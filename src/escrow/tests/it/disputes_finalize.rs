//! Integration tests for `finalize_dispute` (RFC-001 step 7).
//!
//! Like the other dispute pocket-ic suites, these only cover the
//! canister-boundary error paths reachable without a real ICRC-1/2
//! ledger. Full happy-path testing (`Funded` deal → `open_dispute` →
//! `cast_vote` ×3 → `finalize_dispute` → `ArbitratedSettled` /
//! `ArbitratedRefunded` with arbitrator fee fan-out) requires an
//! actual ledger canister installed in pocket-ic plus approval flows.
//! That infrastructure is out of scope for this RFC-001
//! implementation PR and will land as a separate test PR.
//!
//! Coverage here:
//! - `DisputeNotFound` on unknown id.
//! - Anonymous caller blocked by guard.

use std::sync::Arc;

use candid::Principal;
use escrow::api::{
    deals::errors::EscrowError,
    disputes::{params::FinalizeDisputeArgs, results::FinalizeDisputeResult},
};
use pocket_ic::PocketIc;

use crate::utils::pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait};

fn user(id: u8) -> Principal {
    Principal::from_slice(&[id])
}

fn setup() -> (Arc<PocketIc>, PicCanister) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow").deploy_to(&pic);
    (pic, escrow)
}

#[test]
fn finalize_dispute_returns_dispute_not_found() {
    let (_pic, escrow) = setup();
    let result: FinalizeDisputeResult = escrow
        .update(
            user(1),
            "finalize_dispute",
            (FinalizeDisputeArgs {
                dispute_id: 9_999_999,
            },),
        )
        .expect("update call failed");
    match result {
        FinalizeDisputeResult::Err(EscrowError::DisputeNotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn finalize_dispute_anonymous_caller_blocked() {
    let (_pic, escrow) = setup();
    let result: Result<FinalizeDisputeResult, String> = escrow.update(
        Principal::anonymous(),
        "finalize_dispute",
        (FinalizeDisputeArgs { dispute_id: 1 },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}
