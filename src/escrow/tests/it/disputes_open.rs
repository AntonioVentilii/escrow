//! Integration tests for `open_dispute`.
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
        panel_size: None,
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
        panel_size: None,
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

// --- per-deal panel_size at create time ---

fn try_create_with_panel_size(
    escrow: &PicCanister,
    caller: Principal,
    panel_size: Option<u32>,
) -> CreateDealResult {
    escrow
        .update(
            caller,
            "create_deal",
            (CreateDealArgs {
                amount: 1_000_000,
                token_ledger: ledger(),
                expires_at_ns: u64::MAX / 2,
                payer: Some(caller),
                recipient: Some(user(2)),
                title: None,
                note: None,
                panel_size,
            },),
        )
        .expect("create_deal call failed")
}

#[test]
fn create_deal_accepts_none_panel_size() {
    // None = "use canister default at open_dispute time"; should pass
    // validation regardless of the canister's current DisputeConfig.
    let (_pic, escrow) = setup();
    match try_create_with_panel_size(&escrow, user(1), None) {
        CreateDealResult::Ok(view) => {
            assert!(
                view.panel_size.is_none(),
                "view.panel_size: {:?}",
                view.panel_size
            );
        }
        CreateDealResult::Err(e) => panic!("expected Ok, got: {e:?}"),
    }
}

#[test]
fn create_deal_accepts_in_range_odd_panel_size() {
    // Defaults: min=3, max=9. 3, 5, 7, 9 all valid.
    let (_pic, escrow) = setup();
    for (i, n) in [3_u32, 5, 7, 9].into_iter().enumerate() {
        // i ∈ 0..4, well within u8.
        let caller = user(10_u8.saturating_add(u8::try_from(i).unwrap_or(0)));
        match try_create_with_panel_size(&escrow, caller, Some(n)) {
            CreateDealResult::Ok(view) => {
                assert_eq!(view.panel_size, Some(n), "n={n}");
            }
            CreateDealResult::Err(e) => panic!("n={n}: expected Ok, got: {e:?}"),
        }
    }
}

#[test]
fn create_deal_rejects_panel_size_below_min() {
    let (_pic, escrow) = setup();
    // Default min = 3; n=1 is below.
    match try_create_with_panel_size(&escrow, user(20), Some(1)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 9);
            assert_eq!(got, 1);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn create_deal_rejects_panel_size_above_max() {
    let (_pic, escrow) = setup();
    // Default max = 9; n=11 is above.
    match try_create_with_panel_size(&escrow, user(21), Some(11)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 9);
            assert_eq!(got, 11);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn create_deal_rejects_even_panel_size_in_range() {
    let (_pic, escrow) = setup();
    // n=4 is within [3, 9] but even.
    match try_create_with_panel_size(&escrow, user(22), Some(4)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 9);
            assert_eq!(got, 4);
        }
        other => panic!("wrong response: {other:?}"),
    }
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
