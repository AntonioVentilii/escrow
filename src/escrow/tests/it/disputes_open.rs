//! Integration tests for `open_dispute`.
//!
//! Covers the canister-boundary checks that are reachable without a
//! real ICRC ledger installed in `pocket-ic`:
//!
//! - The `caller_is_not_anonymous` guard on `open_dispute`.
//! - `open_dispute` returns `NotFound` for unknown `deal_id`s.
//! - `create_deal`'s `panel_size` validator rejects out-of-range / even values — this branch fires
//!   before the create-time `ledger::fee` call so it's safe to exercise against the fake
//!   `Principal([99])` ledger.
//! - Read-side queries (`get_dispute`, `get_public_dispute`, `list_my_disputes`) return sensible
//!   defaults / errors with no disputes in storage.
//!
//! Everything beyond that — happy-path `create_deal` (needs a real
//! `icrc1_fee`), `Funded → Disputed`, panel selection, `dispute_id`
//! wired back to the deal — requires a real ICRC-1/2 ledger canister
//! and lives in the dispute-finalize end-to-end suite. State-machine
//! error paths (`InvalidState`, `DisputeRequiresBoundRecipient`,
//! `NotAuthorised`, `DisputeAlreadyExists`, `Expired`) are unit-tested
//! against `validate_can_open_dispute` in
//! `src/escrow/src/validation.rs`.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        deals::{errors::EscrowError, params::CreateDealArgs, results::CreateDealResult},
        disputes::{params::OpenDisputeArgs, results::OpenDisputeResult},
    },
    types::asset::Asset,
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

// --- per-deal panel_size at create time ---
//
// These tests exercise the `validate_panel_size_choice` branch that
// fires BEFORE the create-time `ledger::fee` call, so they don't
// require a real ICRC ledger at `Principal([99])`. Happy-path
// (in-range, valid panel_size accepted) is covered by the unit tests
// in `validation::tests` and is exercised end-to-end in the
// finalize integration suite where a real ledger is installed.

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
                asset: Asset::Icrc(ledger()),
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
fn create_deal_rejects_panel_size_below_min() {
    let (_pic, escrow) = setup();
    // Default min = 3; n=1 is below.
    match try_create_with_panel_size(&escrow, user(20), Some(1)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 11);
            assert_eq!(got, 1);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn create_deal_rejects_panel_size_above_max() {
    let (_pic, escrow) = setup();
    // Default max = 11; n=13 is above.
    match try_create_with_panel_size(&escrow, user(21), Some(13)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 11);
            assert_eq!(got, 13);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn create_deal_rejects_even_panel_size_in_range() {
    let (_pic, escrow) = setup();
    // n=4 is within [3, 11] but even.
    match try_create_with_panel_size(&escrow, user(22), Some(4)) {
        CreateDealResult::Err(EscrowError::PanelSizeOutOfRange { min, max, got }) => {
            assert_eq!(min, 3);
            assert_eq!(max, 11);
            assert_eq!(got, 4);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

// --- ledger reachability ---

#[test]
fn create_deal_hard_fails_when_ledger_unreachable() {
    // PR #39 removed the `unwrap_or(0)` fallback on `ledger::fee`
    // in `services::deals::create`, so a fake / unreachable
    // ICRC ledger principal now surfaces as `EscrowError::LedgerError`
    // instead of silently snapshotting `ledger_fee_at_create = 0`
    // and creating a deal that can never settle. The fake
    // `Principal::from_slice(&[99])` from the `ledger()` helper
    // has no canister behind it; the inter-canister call to
    // `icrc1_fee` is rejected synchronously. This test locks the
    // behaviour in so a future refactor can't reintroduce the
    // swallow-and-default fallback without flipping a deliberate
    // alarm here.
    let (_pic, escrow) = setup();
    match try_create_with_panel_size(&escrow, user(30), Some(3)) {
        CreateDealResult::Err(EscrowError::LedgerError(_)) => {}
        other => panic!("expected LedgerError, got: {other:?}"),
    }
}

// --- error variants ---

#[test]
fn open_dispute_returns_not_found_for_unknown_deal() {
    let (_pic, escrow) = setup();
    let result: OpenDisputeResult = escrow
        .update(
            user(1),
            "open_dispute",
            (OpenDisputeArgs { deal_id: 9_999_999 },),
        )
        .expect("open_dispute call failed");
    match result {
        OpenDisputeResult::Err(EscrowError::NotFound) => {}
        OpenDisputeResult::Err(e) => panic!("wrong error: {e:?}"),
        OpenDisputeResult::Ok(d) => panic!("unexpected ok: {d:?}"),
    }
}

#[test]
fn open_dispute_rejects_anonymous_caller() {
    // The `caller_is_not_anonymous` guard fires before any deal lookup,
    // so we can probe it with a fabricated deal_id — no `create_deal`
    // (and therefore no real ledger) required.
    let (_pic, escrow) = setup();
    let result: Result<OpenDisputeResult, String> = escrow.update(
        Principal::anonymous(),
        "open_dispute",
        (OpenDisputeArgs { deal_id: 1 },),
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
