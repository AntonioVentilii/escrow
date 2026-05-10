//! Integration tests for `withdraw_dispute`.
//!
//! Like the rest of the dispute pocket-ic suite, these only cover the
//! canister-boundary error paths reachable without a real ICRC-1/2
//! ledger. Full happy-path testing (both parties propose matching
//! outcomes → reduced-fee fan-out → deal moves to `ArbitratedSettled`
//! / `ArbitratedRefunded`) requires an actual ledger canister installed
//! in pocket-ic plus a `Funded` deal — that infrastructure is out of
//! scope for this PR.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        deals::errors::EscrowError,
        disputes::{params::WithdrawDisputeArgs, results::WithdrawDisputeResult},
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

fn try_withdraw(
    escrow: &PicCanister,
    caller: Principal,
    args: WithdrawDisputeArgs,
) -> WithdrawDisputeResult {
    escrow
        .update(caller, "withdraw_dispute", (args,))
        .expect("withdraw_dispute call failed")
}

#[test]
fn withdraw_dispute_returns_dispute_not_found() {
    let (_pic, escrow) = setup();
    let result = try_withdraw(
        &escrow,
        user(1),
        WithdrawDisputeArgs {
            dispute_id: 9_999_999,
            proposal: Some(Vote::ConcludedCorrectly),
        },
    );
    match result {
        WithdrawDisputeResult::Err(EscrowError::DisputeNotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn withdraw_dispute_rejects_abstain_proposal() {
    let (_pic, escrow) = setup();
    // Abstain is a vote concept, not an out-of-band agreement —
    // rejected at the canister boundary before the dispute lookup.
    let result = try_withdraw(
        &escrow,
        user(1),
        WithdrawDisputeArgs {
            dispute_id: 1,
            proposal: Some(Vote::Abstain),
        },
    );
    match result {
        WithdrawDisputeResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("Abstain"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn withdraw_dispute_anonymous_caller_blocked() {
    let (_pic, escrow) = setup();
    let result: Result<WithdrawDisputeResult, String> = escrow.update(
        Principal::anonymous(),
        "withdraw_dispute",
        (WithdrawDisputeArgs {
            dispute_id: 1,
            proposal: Some(Vote::ConcludedCorrectly),
        },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}
