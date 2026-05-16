//! Integration tests for the money-flow on `accept_deal`,
//! `reclaim_deal`, `process_expired_deals`, `consent_deal`,
//! `reject_deal`, and the receiver-creator (3b) variant of
//! `create_deal`, all driven against a real ICRC-1 / ICRC-2 ledger
//! installed in pocket-ic.
//!
//! Asserts the RFC-002 two-sided-reserve invariants the unit tests
//! cannot check in isolation:
//!
//!   - Receiver's `DC/2` is pulled from their wallet on `consent_deal` (3a) or atomically inside
//!     `create_deal` (3b).
//!   - Payer's `amount + DC/2` is pulled on `fund_deal`.
//!   - On `Settled`, the recipient receives `amount − escrow_fee − ledger_fee + (DC/2 −
//!     ledger_fee)` in one combined transfer; the payer recovers `DC/2 − ledger_fee` separately;
//!     the deal subaccount retains exactly `escrow_fee`.
//!   - On `Refunded`, the payer recovers `amount − escrow_fee − ledger_fee + (DC/2 − ledger_fee)`
//!     combined and the recipient recovers `DC/2 − ledger_fee` separately.
//!   - On `Rejected` (or `Cancelled`) after receiver consent, the receiver's deposited `DC/2` is
//!     refunded minus one outgoing ledger fee. The operator does NOT charge `escrow_fee` on
//!     pre-funding terminations (RFC-002 § Q5) — both parties can trigger the cancel/reject so
//!     charging the side that happens to have a deposit would unfairly penalise the non-rejector.

use core::time::Duration;
use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::deals::{
        errors::EscrowError,
        params::{
            AcceptDealArgs, ConsentDealArgs, CreateDealArgs, FundDealArgs, ReclaimDealArgs,
            RejectDealArgs,
        },
        results::{
            AcceptDealResult, ConsentDealResult, CreateDealResult, DealView, GetDealResult,
            ProcessExpiredDealsResult, ReclaimDealResult, RejectDealResult, SignDealResult,
        },
    },
    types::{
        asset::Asset,
        deal::{DealStatus, Signature},
    },
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
/// and the ICRC-1 ledger installed. Pre-funds both `payer` and
/// `recipient` so each can cover their respective `DC/2` deposit +
/// ledger fees under the RFC-002 two-sided flow.
fn setup() -> (Arc<PocketIc>, PicCanister, IcrcLedger) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow").deploy_to(&pic);
    let ledger = IcrcLedgerBuilder::new()
        .with_initial_balance(payer(), 1_000_000_000_000)
        .with_initial_balance(recipient(), 1_000_000_000_000)
        .deploy_to(&pic);
    (pic, escrow, ledger)
}

fn create_bound_deal_as(
    escrow: &PicCanister,
    creator: Principal,
    ledger: &IcrcLedger,
    amount: u128,
    expires_at_ns: u64,
) -> DealView {
    let args = CreateDealArgs {
        amount,
        asset: Asset::Icrc(ledger.principal()),
        expires_at_ns,
        payer: Some(payer()),
        recipient: Some(recipient()),
        title: None,
        note: None,
        panel_size: None,
    };
    let result: CreateDealResult = escrow
        .update(creator, "create_deal", (args,))
        .expect("create_deal call");
    match result {
        CreateDealResult::Ok(view) => *view,
        CreateDealResult::Err(e) => panic!("create_deal: {e:?}"),
    }
}

fn try_create_bound_deal_as(
    escrow: &PicCanister,
    creator: Principal,
    ledger: &IcrcLedger,
    amount: u128,
    expires_at_ns: u64,
) -> CreateDealResult {
    let args = CreateDealArgs {
        amount,
        asset: Asset::Icrc(ledger.principal()),
        expires_at_ns,
        payer: Some(payer()),
        recipient: Some(recipient()),
        title: None,
        note: None,
        panel_size: None,
    };
    escrow
        .update(creator, "create_deal", (args,))
        .expect("create_deal call")
}

/// Calls `consent_deal` from `caller`. The receiver path performs an
/// ICRC-2 `transfer_from` of `DC/2`; callers must have approved the
/// escrow canister beforehand.
fn consent(escrow: &PicCanister, caller: Principal, deal_id: u64) {
    let result: ConsentDealResult = escrow
        .update(caller, "consent_deal", (ConsentDealArgs { deal_id },))
        .expect("consent_deal call");
    match result {
        ConsentDealResult::Ok(_) => {}
        ConsentDealResult::Err(e) => panic!("consent_deal: {e:?}"),
    }
}

fn try_consent(escrow: &PicCanister, caller: Principal, deal_id: u64) -> ConsentDealResult {
    escrow
        .update(caller, "consent_deal", (ConsentDealArgs { deal_id },))
        .expect("consent_deal call")
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

fn reject(escrow: &PicCanister, caller: Principal, deal_id: u64) -> DealView {
    let result: RejectDealResult = escrow
        .update(caller, "reject_deal", (RejectDealArgs { deal_id },))
        .expect("reject_deal call");
    match result {
        RejectDealResult::Ok(view) => *view,
        RejectDealResult::Err(e) => panic!("reject_deal: {e:?}"),
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

/// Calls either `sign_yes` or `sign_no` depending on `vote`. Both
/// take the same `FundDealArgs` payload — the verb is encoded in
/// the endpoint name.
fn sign(escrow: &PicCanister, caller: Principal, deal_id: u64, vote: &Signature) -> DealView {
    let method = match vote {
        Signature::Yes => "sign_yes",
        Signature::No => "sign_no",
        Signature::Empty => panic!("sign helper: Empty is not a callable vote"),
    };
    let result: SignDealResult = escrow
        .update(caller, method, (FundDealArgs { deal_id },))
        .unwrap_or_else(|e| panic!("{method} call: {e:?}"));
    match result {
        SignDealResult::Ok(view) => *view,
        SignDealResult::Err(e) => panic!("{method}: {e:?}"),
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

/// Default `Config.creation_fee` from `escrow::types::state`. Tests
/// that need to pre-approve the creator's deposit hard-code this
/// rather than reading it from the canister, so the approval call
/// can fire BEFORE `create_deal`.
const CREATION_FEE: u128 = 20_000;

/// Conservative pre-approval headroom for a 3a payer-creator: the
/// payer needs to cover `amount + DC/2 + creation_fee + 2*LF`
/// (two `transfer_from`s at create — one to the deal subaccount,
/// one to the treasury). Under the default `DisputeConfig`
/// (`arbitration_fee_bps = 500` = 5%), `DC = amount * 5 / 100`
/// and `DC/2 = amount / 40`.
fn payer_create_approval(amount: u128, lf: u128) -> u128 {
    let worst_case_dc_half = amount / 40;
    amount + worst_case_dc_half + CREATION_FEE + 2 * lf + 1
}

/// Conservative pre-approval headroom for a 3b recipient-creator:
/// `DC/2 + creation_fee + 2*LF`. Same two-`transfer_from` pattern.
fn recipient_create_approval(amount: u128, lf: u128) -> u128 {
    let worst_case_dc_half = amount / 40;
    worst_case_dc_half + CREATION_FEE + 2 * lf + 1
}

/// End-to-end helper: create a 3a payer-creator deal and have the
/// recipient consent, leaving the deal in `Funded` state ready for
/// the two-signature tally. Returns the `DealView` after consent
/// (status `Funded`, both consents `Accepted`, both signatures
/// `Empty`).
fn setup_funded_3a_deal(
    pic: &PocketIc,
    escrow: &PicCanister,
    ledger: &IcrcLedger,
    amount: u128,
    expires_at_ns: u64,
) -> DealView {
    let lf = ledger.fee;
    ledger.approve(
        payer(),
        escrow.canister_id(),
        payer_create_approval(amount, lf),
    );
    let deal = create_bound_deal_as(escrow, payer(), ledger, amount, expires_at_ns);
    let dc_half = deal.fees.dispute_reserve_per_party;
    ledger.approve(recipient(), escrow.canister_id(), dc_half + lf);
    consent(escrow, recipient(), deal.id);
    // pic.tick is unused here but the parameter is kept for callers that need a fresh time read.
    let _ = pic;
    use_get_deal(escrow, payer(), deal.id)
}

/// Convenience: refresh the latest `DealView` from canister state.
fn use_get_deal(escrow: &PicCanister, caller: Principal, deal_id: u64) -> DealView {
    let result: GetDealResult = escrow
        .query(caller, "get_deal", (deal_id,))
        .expect("get_deal call");
    match result {
        GetDealResult::Ok(view) => *view,
        GetDealResult::Err(e) => panic!("get_deal: {e:?}"),
    }
}

// ---------------------------------------------------------------------------
// 3a happy path: payer-creator deposits everything at create →
// recipient consent (deposits DC/2 + auto-flips to Funded) →
// both sign Yes → settle
// ---------------------------------------------------------------------------

#[test]
fn accept_deal_3a_settles_with_two_sided_reserve_math() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;
    let expires_at_ns = far_future(&pic);

    // Pre-approve everything the payer-creator owes at create
    // time: `amount + DC/2 + creation_fee + 2*LF` (two
    // `transfer_from`s — one to the deal subaccount, one to the
    // treasury — each burning 1 LF).
    ledger.approve(
        payer(),
        escrow.canister_id(),
        payer_create_approval(amount, lf),
    );

    let payer_balance_pre_create = ledger.balance_of_owner(payer());
    let deal = create_bound_deal_as(&escrow, payer(), &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let creation_fee = deal.fees.creation_fee;
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();

    // Payer wallet debited `amount + DC/2 + creation_fee + 2*LF`
    // at create — two transfer_froms (one to deal subaccount, one
    // to treasury), each burning 1 LF.
    assert_eq!(
        payer_balance_pre_create - ledger.balance_of_owner(payer()),
        amount + dc_half + creation_fee + 2 * lf,
        "payer wallet debited amount + DC/2 + creation_fee + 2*LF on create",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount.clone()),
        amount + dc_half,
        "subaccount holds amount + DC/2 after payer's create-time deposit",
    );
    assert_eq!(deal.status, DealStatus::Created);

    // Recipient consent now ALSO pulls their DC/2 + auto-flips status.
    ledger.approve(recipient(), escrow.canister_id(), dc_half + lf);
    let recipient_balance_pre_consent = ledger.balance_of_owner(recipient());
    consent(&escrow, recipient(), deal.id);
    assert_eq!(
        recipient_balance_pre_consent - ledger.balance_of_owner(recipient()),
        dc_half + lf,
        "recipient wallet debited DC/2 + LF on consent",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount.clone()),
        amount + 2 * dc_half,
        "subaccount holds amount + DC after both parties deposited",
    );

    let recipient_pre = ledger.balance_of_owner(recipient());
    let payer_pre = ledger.balance_of_owner(payer());

    // Two-signature tally: recipient's accept_deal records Yes;
    // payer's sign_yes makes both Yes → Settled.
    let after_recipient_sign = accept(&escrow, recipient(), deal.id);
    assert_eq!(
        after_recipient_sign.status,
        DealStatus::Funded,
        "deal stays Funded until both parties sign Yes",
    );
    assert_eq!(after_recipient_sign.recipient_signature, Signature::Yes);
    assert_eq!(after_recipient_sign.payer_signature, Signature::Empty);

    let settled = sign(&escrow, payer(), deal.id, &Signature::Yes);
    assert_eq!(settled.status, DealStatus::Settled);

    // Recipient gets `amount − EF + DC/2 − LF`.
    assert_eq!(
        ledger.balance_of_owner(recipient()) - recipient_pre,
        amount - escrow_fee + dc_half - lf,
        "recipient nets amount − EF + DC/2 − LF on settle",
    );
    // Payer gets their `DC/2 − LF` reserve refund.
    assert_eq!(
        ledger.balance_of_owner(payer()) - payer_pre,
        dc_half - lf,
        "payer recovers DC/2 − LF on settle",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        escrow_fee,
        "subaccount retains exactly EF after settle",
    );
}

// ---------------------------------------------------------------------------
// 3b happy path: receiver-creator deposits DC/2 + creation_fee at create →
// payer consent (deposits amount + DC/2 + auto-flips to Funded) →
// both sign Yes → settle
// ---------------------------------------------------------------------------

#[test]
fn create_deal_3b_receiver_creator_deposits_reserve_atomically() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    // Receiver pre-approves DC/2 + creation_fee + 2*LF for the two
    // `transfer_from`s (one to deal subaccount, one to treasury).
    ledger.approve(
        recipient(),
        escrow.canister_id(),
        recipient_create_approval(amount, lf),
    );

    let recipient_pre = ledger.balance_of_owner(recipient());
    let deal = create_bound_deal_as(&escrow, recipient(), &ledger, amount, far_future(&pic));
    let dc_half = deal.fees.dispute_reserve_per_party;
    let creation_fee = deal.fees.creation_fee;
    let subaccount = deal.escrow_subaccount.clone();

    assert_eq!(
        recipient_pre - ledger.balance_of_owner(recipient()),
        dc_half + creation_fee + 2 * lf,
        "receiver-creator wallet debited DC/2 + creation_fee + 2*LF on create",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        dc_half,
        "subaccount holds receiver's DC/2 immediately after create",
    );
    // Receiver's consent was auto-set to Accepted by resolve_parties.
    // Deal is `Created`, waiting for the payer to consent.
    assert_eq!(deal.status, DealStatus::Created);
}

#[test]
fn create_deal_3b_returns_dispute_reserve_required_without_approval() {
    let (pic, escrow, ledger) = setup();
    // Recipient does NOT approve. create_deal should fail and the
    // partially-inserted deal should be rolled forward to Cancelled.
    let result = try_create_bound_deal_as(
        &escrow,
        recipient(),
        &ledger,
        1_000_000_000,
        far_future(&pic),
    );
    match result {
        CreateDealResult::Err(EscrowError::DisputeReserveRequired) => {}
        other => panic!("expected DisputeReserveRequired, got {other:?}"),
    }
}

#[test]
fn create_deal_3a_returns_dispute_reserve_required_without_approval() {
    let (pic, escrow, ledger) = setup();
    // Symmetric coverage: payer-creator must also pre-approve at
    // create time. Without an approval the create call fails and
    // the half-formed deal is rolled forward to Cancelled.
    let result =
        try_create_bound_deal_as(&escrow, payer(), &ledger, 1_000_000_000, far_future(&pic));
    match result {
        CreateDealResult::Err(EscrowError::DisputeReserveRequired) => {}
        other => panic!("expected DisputeReserveRequired, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Auto-YES expiry: bound deals where neither party signed default to
// `Yes` for both parties at expiry → settle to recipient (silence =
// release). Both `reclaim_deal` (manual, payer-initiated) and the
// `process_expired_deals` housekeeping sweep dispatch through the
// same `services::expiry::dispatch_one_expired` path so they produce
// identical settlement money flow.
// ---------------------------------------------------------------------------

#[test]
fn reclaim_deal_auto_settles_bound_deal_when_both_silent_at_expiry() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;
    let expires_at_ns = short_expiry(&pic);

    let deal = setup_funded_3a_deal(&pic, &escrow, &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();

    pic.advance_time(Duration::from_mins(2));
    pic.tick();

    let payer_pre = ledger.balance_of_owner(payer());
    let recipient_pre = ledger.balance_of_owner(recipient());

    // Manual reclaim by the payer on a bound deal AFTER expiry now
    // routes through the expiry auto-tally dispatcher. With both
    // signatures `Empty`, the auto-YES rule upgrades both to `Yes`,
    // tally is BothYes → settle to recipient. The payer does NOT
    // get a refund — this is the diagram's "silence = release"
    // behaviour, opposite to the legacy `reclaim → Refunded`.
    let settled = reclaim(&escrow, payer(), deal.id);
    assert_eq!(settled.status, DealStatus::Settled);

    // Recipient gets `amount − EF + DC/2 − LF`.
    assert_eq!(
        ledger.balance_of_owner(recipient()) - recipient_pre,
        amount - escrow_fee + dc_half - lf,
        "recipient nets amount − EF + DC/2 − LF on auto-YES settle at expiry",
    );
    // Payer gets back only their `DC/2 − LF` reserve.
    assert_eq!(
        ledger.balance_of_owner(payer()) - payer_pre,
        dc_half - lf,
        "payer recovers only DC/2 − LF on auto-YES settle at expiry",
    );
    // Subaccount retains exactly EF.
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        escrow_fee,
        "subaccount retains exactly EF after auto-YES settle",
    );
}

#[test]
fn process_expired_deals_auto_settles_bound_deal_when_both_silent_at_expiry() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;
    let expires_at_ns = short_expiry(&pic);

    let deal = setup_funded_3a_deal(&pic, &escrow, &ledger, amount, expires_at_ns);
    let escrow_fee = deal.fees.escrow_fee;
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();

    pic.advance_time(Duration::from_mins(2));
    pic.tick();

    let payer_pre = ledger.balance_of_owner(payer());
    let recipient_pre = ledger.balance_of_owner(recipient());

    let processed = process_expired(&escrow, payer(), 10);
    assert_eq!(processed, vec![deal.id]);

    // Same money flow as the manual reclaim path — both go through
    // `services::expiry::dispatch_one_expired`.
    assert_eq!(
        ledger.balance_of_owner(recipient()) - recipient_pre,
        amount - escrow_fee + dc_half - lf,
        "housekeeping sweep settles to recipient under auto-YES",
    );
    assert_eq!(
        ledger.balance_of_owner(payer()) - payer_pre,
        dc_half - lf,
        "payer recovers only DC/2 − LF on auto-YES settle",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        escrow_fee,
        "subaccount retains exactly EF after auto-YES settle",
    );
}

// ---------------------------------------------------------------------------
// Two-signature tally — happy paths (pre-expiry, mutual decision)
// ---------------------------------------------------------------------------

#[test]
fn sign_both_no_aborts_with_refund_money_flow() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    let deal = setup_funded_3a_deal(&pic, &escrow, &ledger, amount, far_future(&pic));
    let escrow_fee = deal.fees.escrow_fee;
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();

    let payer_pre = ledger.balance_of_owner(payer());
    let recipient_pre = ledger.balance_of_owner(recipient());

    // Both parties explicitly sign `No` → BothNo tally → Aborted.
    // Fee math is identical to `Refunded` (project constraint: no
    // fee logic changes for the new terminal). Payer recovers
    // `amount − EF + DC/2 − LF`, recipient recovers `DC/2 − LF`,
    // subaccount retains EF.
    let after_payer = sign(&escrow, payer(), deal.id, &Signature::No);
    assert_eq!(after_payer.status, DealStatus::Funded);
    assert_eq!(after_payer.payer_signature, Signature::No);

    let aborted = sign(&escrow, recipient(), deal.id, &Signature::No);
    assert_eq!(
        aborted.status,
        DealStatus::Aborted,
        "BothNo tally aborts the deal (new terminal status)",
    );

    assert_eq!(
        ledger.balance_of_owner(payer()) - payer_pre,
        amount - escrow_fee + dc_half - lf,
        "payer recovers amount − EF + DC/2 − LF on Aborted (mirrors Refunded)",
    );
    assert_eq!(
        ledger.balance_of_owner(recipient()) - recipient_pre,
        dc_half - lf,
        "recipient recovers DC/2 − LF on Aborted",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        escrow_fee,
        "subaccount retains exactly EF after Aborted",
    );
}

#[test]
fn sign_mixed_auto_opens_dispute() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;

    let deal = setup_funded_3a_deal(&pic, &escrow, &ledger, amount, far_future(&pic));

    // Recipient signs Yes; deal stays Funded (Pending tally).
    let after_recipient = sign(&escrow, recipient(), deal.id, &Signature::Yes);
    assert_eq!(after_recipient.status, DealStatus::Funded);

    // Payer signs No → Mixed tally → auto-open dispute. The deal
    // would land in `Disputed` if the eligible-arbitrator pool is
    // non-empty; with no arbitrators registered in this test
    // setup, the auto-open returns `InsufficientArbitrators` and
    // the deal stays `Funded` with both signatures recorded —
    // the caller can retry by signing again or registering
    // arbitrators and calling `open_dispute` explicitly.
    let result: SignDealResult = escrow
        .update(payer(), "sign_no", (FundDealArgs { deal_id: deal.id },))
        .expect("sign_no call");
    match result {
        SignDealResult::Err(EscrowError::InsufficientArbitrators { need, have }) => {
            assert!(have < need, "want < need, got need={need} have={have}");
            // Signature was still recorded under Phase 1 of sign().
            let view = use_get_deal(&escrow, payer(), deal.id);
            assert_eq!(view.payer_signature, Signature::No);
            assert_eq!(view.recipient_signature, Signature::Yes);
            assert_eq!(view.status, DealStatus::Funded);
        }
        other => panic!(
            "expected InsufficientArbitrators (no arbitrators registered in this test); got {other:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// Reject path: receiver consents (deposits DC/2) then rejects
// ---------------------------------------------------------------------------

#[test]
fn reject_after_receiver_consent_refunds_minus_ledger_fee() {
    // 3a payer-creator deposits at create. Recipient then consents
    // (deposits DC/2) — both consents Accepted and the auto-flip
    // takes the deal to Funded. Once Funded, `reject_deal` is no
    // longer applicable (validator rejects with InvalidState); use
    // `sign_no` to abort post-funding instead. So this test now
    // covers the pre-recipient-consent rejection path: payer
    // creates, recipient REJECTS without consenting, and the
    // payer's create-time deposit is refunded minus one outgoing
    // LF. creation_fee stays in the treasury (forfeited per design).
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    ledger.approve(
        payer(),
        escrow.canister_id(),
        payer_create_approval(amount, lf),
    );
    let deal = create_bound_deal_as(&escrow, payer(), &ledger, amount, far_future(&pic));
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();
    assert_eq!(deal.status, DealStatus::Created);

    let payer_pre = ledger.balance_of_owner(payer());

    // Recipient rejects WITHOUT consenting. Subaccount holds the
    // payer's `amount + DC/2`; the rejection refunds it back to the
    // payer minus one outgoing ledger fee. The operator does NOT
    // take `escrow_fee` (pre-funding terminal). `creation_fee` is
    // already in the treasury subaccount and stays there.
    let rejected = reject(&escrow, recipient(), deal.id);
    assert_eq!(rejected.status, DealStatus::Rejected);

    assert_eq!(
        ledger.balance_of_owner(payer()) - payer_pre,
        amount + dc_half - lf,
        "payer recovers amount + DC/2 − LF when recipient rejects pre-consent",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        0,
        "subaccount empty after reject (no EF on pre-funding terminal)",
    );
}

// ---------------------------------------------------------------------------
// Cross-party reject: payer rejects after receiver consented
// (RFC-002 § Q5: the non-rejector's deposit is refunded in full
// minus the outgoing LF; the rejecting party does not get to
// confiscate the other side's reserve).
// ---------------------------------------------------------------------------

#[test]
fn reject_by_payer_3b_refunds_recipient_deposit() {
    // Symmetric coverage for 3b: recipient creates and deposits
    // their `DC/2 + creation_fee` at create. Payer then rejects
    // BEFORE consenting. The recipient's `DC/2` deposit refunds
    // minus one outgoing LF. creation_fee stays in treasury.
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    ledger.approve(
        recipient(),
        escrow.canister_id(),
        recipient_create_approval(amount, lf),
    );
    let deal = create_bound_deal_as(&escrow, recipient(), &ledger, amount, far_future(&pic));
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount = deal.escrow_subaccount.clone();

    let recipient_pre = ledger.balance_of_owner(recipient());

    let rejected = reject(&escrow, payer(), deal.id);
    assert_eq!(rejected.status, DealStatus::Rejected);

    assert_eq!(
        ledger.balance_of_owner(recipient()) - recipient_pre,
        dc_half - lf,
        "recipient-creator recovers DC/2 − LF when payer rejects (no EF confiscation)",
    );
    assert_eq!(
        ledger.balance_of_subaccount(escrow.canister_id(), subaccount),
        0,
        "subaccount empty after cross-party reject (creation_fee already in treasury)",
    );
}

// (The legacy `reject_before_any_deposit_is_free` test was
// removed in the commit-at-first-action restructure: the creator
// now ALWAYS deposits at create_deal — there's no
// "deal-with-zero-deposits" pre-funding state to reject from.
// `cancel_deal` / `reject_deal` from the creator side now refund
// what they deposited, see `reject_after_receiver_consent_refunds_minus_ledger_fee`.)

// ---------------------------------------------------------------------------
// consent_deal is idempotent — a second call from an
// already-consented receiver does NOT pull another `DC/2`, even
// when the receiver's approval is still open.
// ---------------------------------------------------------------------------

#[test]
fn consent_deal_is_idempotent_for_already_consented_receiver() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    // 3a setup: payer creates and deposits everything at create.
    ledger.approve(
        payer(),
        escrow.canister_id(),
        payer_create_approval(amount, lf),
    );
    let deal = create_bound_deal_as(&escrow, payer(), &ledger, amount, far_future(&pic));
    let dc_half = deal.fees.dispute_reserve_per_party;
    let subaccount_after_create =
        ledger.balance_of_subaccount(escrow.canister_id(), deal.escrow_subaccount.clone());
    assert_eq!(subaccount_after_create, amount + dc_half);

    // Approve generously — twice the DC/2 so a second pull WOULD
    // succeed at the ledger level if the canister tried.
    ledger.approve(recipient(), escrow.canister_id(), (dc_half + lf) * 2);

    let recipient_pre_consent = ledger.balance_of_owner(recipient());
    consent(&escrow, recipient(), deal.id);
    let recipient_after_first = ledger.balance_of_owner(recipient());
    let subaccount_after_first =
        ledger.balance_of_subaccount(escrow.canister_id(), deal.escrow_subaccount.clone());

    // First consent moved DC/2 + LF out of the receiver and DC/2
    // into the subaccount on top of the payer's existing
    // `amount + DC/2` deposit. Status auto-flipped to Funded.
    assert_eq!(recipient_pre_consent - recipient_after_first, dc_half + lf);
    assert_eq!(subaccount_after_first, amount + 2 * dc_half);

    // Second consent: should be a no-op (no ledger calls). The
    // idempotency guard inside `consent_deal` short-circuits when
    // `recipient_consent` is already `Accepted`, regardless of
    // whether the wallet still has a generous open allowance.
    consent(&escrow, recipient(), deal.id);
    let recipient_after_second = ledger.balance_of_owner(recipient());
    let subaccount_after_second =
        ledger.balance_of_subaccount(escrow.canister_id(), deal.escrow_subaccount);

    assert_eq!(
        recipient_after_second, recipient_after_first,
        "second consent must not pull another DC/2 from the receiver",
    );
    assert_eq!(
        subaccount_after_second, subaccount_after_first,
        "subaccount must not double-credit on a redundant consent",
    );
}

// ---------------------------------------------------------------------------
// consent_deal returns DisputeReserveRequired without approval
// ---------------------------------------------------------------------------

#[test]
fn consent_deal_without_approval_returns_dispute_reserve_required() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    // Payer pre-approves and creates a 3a deal — needed because
    // `consent_deal` requires the deal to exist.
    ledger.approve(
        payer(),
        escrow.canister_id(),
        payer_create_approval(amount, lf),
    );
    let deal = create_bound_deal_as(&escrow, payer(), &ledger, amount, far_future(&pic));

    // Receiver has balance (per the setup) but never approved the
    // escrow canister — `icrc2_transfer_from` rejects on missing
    // allowance, which we map to `DisputeReserveRequired`.
    let result = try_consent(&escrow, recipient(), deal.id);
    match result {
        ConsentDealResult::Err(EscrowError::DisputeReserveRequired) => {}
        other => panic!("expected DisputeReserveRequired, got {other:?}"),
    }
}
