//! Integration tests for `submit_evidence` (RFC-001 step 5).
//!
//! Like `disputes_open`, these only cover the paths reachable without
//! a real ICRC-1/2 ledger installed in pocket-ic. The happy-path
//! "submit evidence on a real Disputed deal" requires a `Funded` deal
//! → `open_dispute` → `submit_evidence`, which in turn needs an actual
//! ledger for the funding step. That end-to-end flow lands in step 7.

use std::sync::Arc;

use candid::Principal;
use escrow::api::{
    deals::errors::EscrowError,
    disputes::{params::SubmitEvidenceArgs, results::SubmitEvidenceResult},
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

fn try_submit(escrow: &PicCanister, caller: Principal, args: SubmitEvidenceArgs) -> SubmitEvidenceResult {
    escrow
        .update(caller, "submit_evidence", (args,))
        .expect("submit_evidence call failed")
}

#[test]
fn submit_evidence_returns_dispute_not_found() {
    let (_pic, escrow) = setup();
    let result = try_submit(
        &escrow,
        user(1),
        SubmitEvidenceArgs {
            dispute_id: 9_999_999,
            note: Some("hello".to_owned()),
            artefact_url: None,
            artefact_sha256: None,
        },
    );
    match result {
        SubmitEvidenceResult::Err(EscrowError::DisputeNotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn submit_evidence_rejects_empty_payload_at_canister_boundary() {
    let (_pic, escrow) = setup();
    // Even before reaching the dispute-not-found check, the validator
    // rejects empty evidence with `ValidationError`. This exercises the
    // canister-boundary length / shape validation (Q8) end-to-end.
    let result = try_submit(
        &escrow,
        user(1),
        SubmitEvidenceArgs {
            dispute_id: 1,
            note: None,
            artefact_url: None,
            artefact_sha256: None,
        },
    );
    match result {
        SubmitEvidenceResult::Err(EscrowError::ValidationError(_)) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn submit_evidence_rejects_url_without_hash() {
    let (_pic, escrow) = setup();
    let result = try_submit(
        &escrow,
        user(1),
        SubmitEvidenceArgs {
            dispute_id: 1,
            note: None,
            artefact_url: Some("https://example.com/proof.pdf".to_owned()),
            artefact_sha256: None,
        },
    );
    match result {
        SubmitEvidenceResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("supplied together"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn submit_evidence_rejects_short_hash() {
    let (_pic, escrow) = setup();
    let result = try_submit(
        &escrow,
        user(1),
        SubmitEvidenceArgs {
            dispute_id: 1,
            note: None,
            artefact_url: Some("https://example.com/x".to_owned()),
            artefact_sha256: Some(vec![0_u8; 16]),
        },
    );
    match result {
        SubmitEvidenceResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("32 bytes"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn submit_evidence_rejects_oversized_note() {
    let (_pic, escrow) = setup();
    let huge = "x".repeat(5000);
    let result = try_submit(
        &escrow,
        user(1),
        SubmitEvidenceArgs {
            dispute_id: 1,
            note: Some(huge),
            artefact_url: None,
            artefact_sha256: None,
        },
    );
    match result {
        SubmitEvidenceResult::Err(EscrowError::EvidenceTooLarge { max }) => {
            assert_eq!(max, 4096);
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn submit_evidence_anonymous_caller_blocked() {
    let (_pic, escrow) = setup();
    let result: Result<SubmitEvidenceResult, String> = escrow.update(
        Principal::anonymous(),
        "submit_evidence",
        (SubmitEvidenceArgs {
            dispute_id: 1,
            note: Some("x".to_owned()),
            artefact_url: None,
            artefact_sha256: None,
        },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}
