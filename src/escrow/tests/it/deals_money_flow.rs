//! Integration tests for the money-flow on `accept_deal`,
//! `reclaim_deal`, and `process_expired_deals` against a real
//! ICRC-1 / ICRC-2 ledger installed in pocket-ic.
//!
//! Asserts the post-RFC-002 invariants that the unit tests can
//! only check in isolation:
//!
//!   - On `Settled`, the recipient receives exactly `amount − escrow_fee − ledger_fee` and the deal
//!     subaccount retains exactly `escrow_fee`.
//!   - On `Refunded` via `reclaim_deal`, the payer receives `amount − escrow_fee − ledger_fee` and
//!     the subaccount retains `escrow_fee`.
//!   - On auto-refund via `process_expired_deals`, the same math holds via the housekeeping path.

use core::time::Duration;
use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::deals::{
        params::{AcceptDealArgs, ConsentDealArgs, CreateDealArgs, FundDealArgs, ReclaimDealArgs},
        results::{
            AcceptDealResult, ConsentDealResult, CreateDealResult, DealView, FundDealResult,
            ProcessExpiredDealsResult, ReclaimDealResult,
        },
    },
    types::deal::DealStatus,
};
use pocket_ic::PocketIc;

use crate::utils::{
    icrc_ledger::{IcrcLedger, IcrcLedgerBuilder},
    pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait},
};

// ---------------------------------------------------------------------------
// Test principals
// ---------------------------------------------------------------------------

fn payer() -> Principal {
    Principal::from_slice(&[1])
}

fn recipient() -> Principal {
    Principal::from_slice(&[2])
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

/// Spins up a fresh pocket-ic instance with both the escrow canister
/// and the ICRC-1 ledger installed. Pre-funds `payer` with a generous
/// balance so they can fund deals + cover ledger fees.
fn setup() -> (Arc<PocketIc>, PicCanister, IcrcLedger) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow").deploy_to(&pic);
    let ledger = IcrcLedgerBuilder::new()
        .with_initial_balance(payer(), 1_000_000_000_000)
        .deploy_to(&pic);
    (pic, escrow, ledger)
}

fn create_bound_deal(
    escrow: &PicCanister,
    ledger: &IcrcLedger,
    amount: u128,
    expires_at_ns: u64,
) -> DealView {
    let args = CreateDealArgs {
        amount,
        token_ledger: ledger.principal(),
        expires_at_ns,
        payer: Some(payer()),
        recipient: Some(recipient()),
        title: None,
        note: None,
        panel_size: None,
    };
    let result: CreateDealResult = escrow
        .update(payer(), "create_deal", (args,))
        .expect("create_deal call");
    match result {
        CreateDealResult::Ok(view) => *view,
        CreateDealResult::Err(e) => panic!("create_deal: {e:?}"),
    }
}

/// Counterparty signals consent on a bound deal. In PR-1 this is a
/// pure state mutation (no ledger calls); it becomes async + ICRC-2
/// in PR-2b. The fixture exists today so the existing test stays
/// stable across that transition.
fn consent(escrow: &PicCanister, caller: Principal, deal_id: u64) {
    let result: ConsentDealResult = escrow
        .update(caller, "consent_deal", (ConsentDealArgs { deal_id },))
        .expect("consent_deal call");
    match result {
        ConsentDealResult::Ok(_) => {}
        ConsentDealResult::Err(e) => panic!("consent_deal: {e:?}"),
    }
}

fn fund(escrow: &PicCanister, caller: Principal, deal_id: u64) -> DealView {
    let result: FundDealResult = escrow
        .update(caller, "fund_deal", (FundDealArgs { deal_id },))
        .expect("fund_deal call");
    match result {
        FundDealResult::Ok(view) => *view,
        FundDealResult::Err(e) => panic!("fund_deal: {e:?}"),
    }
}

fn accept(escrow: &PicCanister, caller: Principal, deal_id: u64) -> DealView {
    let args = AcceptDealArgs {
        deal_id,
        claim_code: None,
    };
    let result: AcceptDealResult = escrow
        .update(caller, "accept_deal", (args,))
        .expect("accept_deal call");
    match result {
        AcceptDealResult::Ok(view) => *view,
        AcceptDealResult::Err(e) => panic!("accept_deal: {e:?}"),
    }
}

fn reclaim(escrow: &PicCanister, caller: Principal, deal_id: u64) -> DealView {
    let result: ReclaimDealResult = escrow
        .update(caller, "reclaim_deal", (ReclaimDealArgs { deal_id },))
        .expect("reclaim_deal call");
    match result {
        ReclaimDealResult::Ok(view) => *view,
        ReclaimDealResult::Err(e) => panic!("reclaim_deal: {e:?}"),
    }
}

fn process_expired(escrow: &PicCanister, caller: Principal, limit: u32) -> Vec<u64> {
    let result: ProcessExpiredDealsResult = escrow
        .update(caller, "process_expired_deals", (limit,))
        .expect("process_expired_deals call");
    match result {
        ProcessExpiredDealsResult::Ok(ids) => *ids,
        ProcessExpiredDealsResult::Err(e) => panic!("process_expired_deals: {e:?}"),
    }
}

/// Returns a nanosecond timestamp comfortably in the future relative
/// to pocket-ic's deterministic clock at fresh-canister time.
fn far_future(pic: &PocketIc) -> u64 {
    let now_ns = pic.get_time().as_nanos_since_unix_epoch();
    let bump = u64::try_from(Duration::from_hours(1).as_nanos()).expect("1h fits in u64 ns");
    now_ns + bump
}

/// One-minute expiry — close enough to advance past in the test loop.
fn short_expiry(pic: &PocketIc) -> u64 {
    let now_ns = pic.get_time().as_nanos_since_unix_epoch();
    let bump = u64::try_from(Duration::from_mins(1).as_nanos()).expect("1m fits in u64 ns");
    now_ns + bump
}

// ---------------------------------------------------------------------------
// Happy-path: accept_deal settles with the snapshotted fee math
// ---------------------------------------------------------------------------

#[test]
fn accept_deal_settles_recipient_net_amount_minus_ef_and_lf() {
    let (pic, escrow, ledger) = setup();

    let amount: u128 = 1_000_000_000;
    let expires_at_ns = far_future(&pic);
    let deal = create_bound_deal(&escrow, &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let escrow_subaccount = deal.escrow_subaccount.clone();

    // PR-1 sync consent (mutation only — no ledger calls yet).
    consent(&escrow, recipient(), deal.id);

    // Payer approves the escrow canister to pull `amount + ledger_fee`.
    ledger.approve(payer(), escrow.canister_id(), amount + ledger.fee);

    let payer_balance_pre_fund = ledger.balance_of_owner(payer());

    let funded = fund(&escrow, payer(), deal.id);
    assert_eq!(funded.status, DealStatus::Funded);

    // Payer wallet debited `amount + ledger_fee` on `icrc2_transfer_from`
    // (the ICRC-2 fee is paid by the FROM side) PLUS another `ledger_fee`
    // on the prior `icrc2_approve`. Approve cost was already paid before
    // the snapshot, so the delta we measure here is the transfer_from
    // portion only.
    let payer_balance_post_fund = ledger.balance_of_owner(payer());
    assert_eq!(
        payer_balance_pre_fund - payer_balance_post_fund,
        amount + ledger.fee,
        "payer wallet should be debited amount + ledger_fee on transfer_from",
    );

    // Subaccount holds exactly `amount` — the ICRC-2 fee is burned,
    // not credited to the destination.
    let subaccount_post_fund =
        ledger.balance_of_subaccount(escrow.canister_id(), escrow_subaccount.clone());
    assert_eq!(
        subaccount_post_fund, amount,
        "deal subaccount should hold exactly the deal amount after fund",
    );

    let recipient_balance_pre_accept = ledger.balance_of_owner(recipient());

    let settled = accept(&escrow, recipient(), deal.id);
    assert_eq!(settled.status, DealStatus::Settled);

    // Recipient nets `amount − escrow_fee − ledger_fee` exactly.
    let recipient_balance_post_accept = ledger.balance_of_owner(recipient());
    assert_eq!(
        recipient_balance_post_accept - recipient_balance_pre_accept,
        amount - escrow_fee - ledger.fee,
        "recipient should receive amount − EF − LF on settle",
    );

    // Deal subaccount retains exactly `escrow_fee` (the operator's share).
    let subaccount_post_accept =
        ledger.balance_of_subaccount(escrow.canister_id(), escrow_subaccount);
    assert_eq!(
        subaccount_post_accept, escrow_fee,
        "deal subaccount should retain exactly EF after settle",
    );
}

// ---------------------------------------------------------------------------
// Happy-path: reclaim_deal refunds with the same fee math
// ---------------------------------------------------------------------------

#[test]
fn reclaim_deal_refunds_payer_net_amount_minus_ef_and_lf() {
    let (pic, escrow, ledger) = setup();

    let amount: u128 = 1_000_000_000;
    let expires_at_ns = short_expiry(&pic);

    let deal = create_bound_deal(&escrow, &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let escrow_subaccount = deal.escrow_subaccount.clone();

    consent(&escrow, recipient(), deal.id);
    ledger.approve(payer(), escrow.canister_id(), amount + ledger.fee);
    let funded = fund(&escrow, payer(), deal.id);
    assert_eq!(funded.status, DealStatus::Funded);

    // Advance past expiry and let the canister observe the new time.
    pic.advance_time(Duration::from_mins(2));
    pic.tick();

    let payer_balance_pre_reclaim = ledger.balance_of_owner(payer());

    let refunded = reclaim(&escrow, payer(), deal.id);
    assert_eq!(refunded.status, DealStatus::Refunded);

    let payer_balance_post_reclaim = ledger.balance_of_owner(payer());
    assert_eq!(
        payer_balance_post_reclaim - payer_balance_pre_reclaim,
        amount - escrow_fee - ledger.fee,
        "payer should be refunded amount − EF − LF on reclaim",
    );

    let subaccount_post_reclaim =
        ledger.balance_of_subaccount(escrow.canister_id(), escrow_subaccount);
    assert_eq!(
        subaccount_post_reclaim, escrow_fee,
        "deal subaccount should retain exactly EF after reclaim",
    );
}

// ---------------------------------------------------------------------------
// Auto-refund: process_expired_deals
// ---------------------------------------------------------------------------

#[test]
fn process_expired_deals_auto_refunds_payer() {
    let (pic, escrow, ledger) = setup();

    let amount: u128 = 1_000_000_000;
    let expires_at_ns = short_expiry(&pic);

    let deal = create_bound_deal(&escrow, &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let escrow_subaccount = deal.escrow_subaccount.clone();

    consent(&escrow, recipient(), deal.id);
    ledger.approve(payer(), escrow.canister_id(), amount + ledger.fee);
    fund(&escrow, payer(), deal.id);

    pic.advance_time(Duration::from_mins(2));
    pic.tick();

    let payer_balance_pre_sweep = ledger.balance_of_owner(payer());

    let processed = process_expired(&escrow, payer(), 10);
    assert_eq!(
        processed,
        vec![deal.id],
        "the expired deal id should be returned in the processed list",
    );

    let payer_balance_post_sweep = ledger.balance_of_owner(payer());
    assert_eq!(
        payer_balance_post_sweep - payer_balance_pre_sweep,
        amount - escrow_fee - ledger.fee,
        "payer should be refunded amount − EF − LF on auto-sweep",
    );

    let subaccount_post_sweep =
        ledger.balance_of_subaccount(escrow.canister_id(), escrow_subaccount);
    assert_eq!(
        subaccount_post_sweep, escrow_fee,
        "deal subaccount should retain exactly EF after auto-sweep",
    );
}
