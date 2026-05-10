//! Integration tests for `open_dispute` (RFC-001 step 4).
//!
//! Covers the error paths reachable without a real ICRC ledger:
//! - `NotFound` on unknown `deal_id`.
//! - `InvalidState` on `Created` deals (no funds at risk yet).
//! - `DisputeRequiresBoundRecipient` on tip-flow deals.
//! - `NotAuthorised` for unrelated callers.
//! - Anonymous caller blocked by guard.
//!
//! The full happy-path test (`Funded → Disputed`, panel selection,
//! `dispute_id` wired back to the deal) requires a deal that's actually
//! `Funded` — which requires an ICRC-1/2 ledger canister installed in
//! `pocket-ic`. That infrastructure lands in step 7 (finalize) where
//! actual transfers happen; the open-dispute happy path will be
//! exercised end-to-end alongside it.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        deals::{
            errors::EscrowError,
            params::CreateDealArgs,
            results::{CreateDealResult, DealView},
        },
        disputes::{params::OpenDisputeArgs, results::OpenDisputeResult},
    },
    types::deal::DealStatus,
};
use pocket_ic::PocketIc;

use crate::utils::pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait};

fn user(id: u8) -> Principal {
    Principal::from_slice(&[id])
}

fn ledger() -> Principal {
    Principal::from_slice(&[99])
}

fn setup() -> (Arc<PocketIc>, PicCanister) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow").deploy_to(&pic);
    (pic, escrow)
}

fn create_bound_deal(escrow: &PicCanister, payer: Principal, recipient: Principal) -> DealView {
    let args = CreateDealArgs {
        amount: 1_000_000,
        token_ledger: ledger(),
        // Far-future expiry so we don't trip the Expired check.
        expires_at_ns: u64::MAX / 2,
        payer: Some(payer),
        recipient: Some(recipient),
        title: None,
        note: None,
    };
    let result: CreateDealResult = escrow
        .update(payer, "create_deal", (args,))
        .expect("create_deal call failed");
    match result {
        CreateDealResult::Ok(view) => *view,
        CreateDealResult::Err(e) => panic!("create_deal returned error: {e:?}"),
    }
}

fn create_tip_deal(escrow: &PicCanister, payer: Principal) -> DealView {
    let args = CreateDealArgs {
        amount: 1_000_000,
        token_ledger: ledger(),
        expires_at_ns: u64::MAX / 2,
        payer: Some(payer),
        recipient: None,
        title: None,
        note: None,
    };
    let result: CreateDealResult = escrow
        .update(payer, "create_deal", (args,))
        .expect("create_deal call failed");
    match result {
        CreateDealResult::Ok(view) => *view,
        CreateDealResult::Err(e) => panic!("create_deal returned error: {e:?}"),
    }
}

fn try_open_dispute(escrow: &PicCanister, caller: Principal, deal_id: u64) -> OpenDisputeResult {
    escrow
        .update(caller, "open_dispute", (OpenDisputeArgs { deal_id },))
        .expect("open_dispute call failed")
}

// --- error variants ---

#[test]
fn open_dispute_returns_not_found_for_unknown_deal() {
    let (_pic, escrow) = setup();
    let result = try_open_dispute(&escrow, user(1), 9_999_999);
    match result {
        OpenDisputeResult::Err(EscrowError::NotFound) => {}
        OpenDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        OpenDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn open_dispute_rejects_created_deal() {
    let (_pic, escrow) = setup();
    let payer = user(1);
    let recipient = user(2);
    let deal = create_bound_deal(&escrow, payer, recipient);
    assert_eq!(deal.status, DealStatus::Created);

    // Both parties present, but deal is still Created (no funding) →
    // InvalidState should fire (Created is not Funded).
    let result = try_open_dispute(&escrow, payer, deal.id);
    match result {
        OpenDisputeResult::Err(EscrowError::InvalidState { expected, actual }) => {
            assert!(expected.contains("Funded"), "expected: {expected}");
            assert!(actual.contains("Created"), "actual: {actual}");
        }
        OpenDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        OpenDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn open_dispute_rejects_tip_flow_deal() {
    let (_pic, escrow) = setup();
    let payer = user(1);
    let deal = create_tip_deal(&escrow, payer);

    // Tip-flow deals have recipient = None → DisputeRequiresBoundRecipient.
    // (This fires before the Funded check because the bound-recipient check
    // is the first guard in `validate_can_open_dispute`.)
    let result = try_open_dispute(&escrow, payer, deal.id);
    match result {
        OpenDisputeResult::Err(EscrowError::DisputeRequiresBoundRecipient) => {}
        OpenDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        OpenDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn open_dispute_rejects_unrelated_caller() {
    let (_pic, escrow) = setup();
    let payer = user(1);
    let recipient = user(2);
    let stranger = user(99);
    let deal = create_bound_deal(&escrow, payer, recipient);

    let result = try_open_dispute(&escrow, stranger, deal.id);
    match result {
        OpenDisputeResult::Err(EscrowError::NotAuthorised) => {}
        OpenDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        OpenDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn open_dispute_rejects_anonymous_caller() {
    let (_pic, escrow) = setup();
    let payer = user(1);
    let recipient = user(2);
    let deal = create_bound_deal(&escrow, payer, recipient);

    let result: Result<OpenDisputeResult, String> = escrow.update(
        Principal::anonymous(),
        "open_dispute",
        (OpenDisputeArgs { deal_id: deal.id },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}

// --- queries ---

#[test]
fn get_dispute_returns_not_found_for_unknown_id() {
    use escrow::api::disputes::results::GetDisputeResult;
    let (_pic, escrow) = setup();
    let result: GetDisputeResult = escrow
        .query(user(1), "get_dispute", (9_999_999_u64,))
        .expect("query failed");
    match result {
        GetDisputeResult::Err(EscrowError::DisputeNotFound) => {}
        GetDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        GetDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn get_public_dispute_returns_not_found_for_unknown_id() {
    use escrow::api::disputes::results::GetPublicDisputeResult;
    let (_pic, escrow) = setup();
    let result: GetPublicDisputeResult = escrow
        .query(user(1), "get_public_dispute", (9_999_999_u64,))
        .expect("query failed");
    match result {
        GetPublicDisputeResult::Err(EscrowError::DisputeNotFound) => {}
        GetPublicDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        GetPublicDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn list_my_disputes_returns_empty_when_none() {
    use escrow::api::disputes::{params::ListMyDisputesArgs, results::DisputeView};
    let (_pic, escrow) = setup();
    let result: Vec<DisputeView> = escrow
        .query(
            user(1),
            "list_my_disputes",
            (ListMyDisputesArgs::default(),),
        )
        .expect("query failed");
    assert!(result.is_empty());
}
