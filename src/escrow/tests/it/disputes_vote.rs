//! Integration tests for `cast_vote`.
//!
//! Covers the canister-boundary error paths reachable without a real
//! ICRC-1/2 ledger. The happy-path test (registered arbitrator on a
//! real `Disputed` deal panel casts a vote) requires a `Funded` deal
//! to feed `open_dispute`, which in turn needs a ledger — that
//! end-to-end flow lands in step 7.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        deals::errors::EscrowError,
        disputes::{params::CastVoteArgs, results::CastVoteResult},
    },
    types::dispute::Vote,
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

fn try_vote(escrow: &PicCanister, caller: Principal, args: CastVoteArgs) -> CastVoteResult {
    escrow
        .update(caller, "cast_vote", (args,))
        .expect("cast_vote call failed")
}

#[test]
fn cast_vote_returns_dispute_not_found_for_unknown_id() {
    let (_pic, escrow) = setup();
    let result = try_vote(
        &escrow,
        user(1),
        CastVoteArgs {
            dispute_id: 9_999_999,
            vote: Vote::ConcludedCorrectly,
        },
    );
    match result {
        CastVoteResult::Err(EscrowError::DisputeNotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn cast_vote_anonymous_caller_blocked() {
    let (_pic, escrow) = setup();
    let result: Result<CastVoteResult, String> = escrow.update(
        Principal::anonymous(),
        "cast_vote",
        (CastVoteArgs {
            dispute_id: 1,
            vote: Vote::ConcludedCorrectly,
        },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}
