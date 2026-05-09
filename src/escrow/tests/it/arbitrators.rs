//! Integration tests for the arbitrator endpoints (RFC-001 step 3).
//!
//! Covers the happy path + idempotency + every error variant the
//! register/deregister/get/list flows can emit, against a real installed
//! canister via `pocket-ic`.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        arbitrators::{
            params::{ListArbitratorsArgs, RegisterArbitratorArgs},
            results::{DeregisterArbitratorResult, RegisterArbitratorResult},
        },
        deals::errors::EscrowError,
    },
    types::arbitrator::{ArbitratorProfile, ArbitratorStatus},
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

fn register(
    escrow: &PicCanister,
    caller: Principal,
    bio: Option<String>,
) -> RegisterArbitratorResult {
    escrow
        .update(
            caller,
            "register_arbitrator",
            (RegisterArbitratorArgs { bio },),
        )
        .expect("update call failed")
}

fn deregister(escrow: &PicCanister, caller: Principal) -> DeregisterArbitratorResult {
    escrow
        .update(caller, "deregister_arbitrator", ())
        .expect("update call failed")
}

fn get(escrow: &PicCanister, caller: Principal, target: Principal) -> Option<ArbitratorProfile> {
    escrow
        .query(caller, "get_arbitrator", (target,))
        .expect("query call failed")
}

fn list(
    escrow: &PicCanister,
    caller: Principal,
    args: ListArbitratorsArgs,
) -> Vec<ArbitratorProfile> {
    escrow
        .query(caller, "list_arbitrators", (args,))
        .expect("query call failed")
}

// --- happy path ---

#[test]
fn register_creates_active_profile() {
    let (_pic, escrow) = setup();
    let caller = user(1);

    let result = register(&escrow, caller, Some("hi".to_owned()));
    let profile = match result {
        RegisterArbitratorResult::Ok(p) => *p,
        RegisterArbitratorResult::Err(e) => panic!("unexpected error: {e:?}"),
    };
    assert_eq!(profile.principal, caller);
    assert_eq!(profile.status, ArbitratorStatus::Active);
    assert_eq!(profile.bio.as_deref(), Some("hi"));
    assert_eq!(profile.disputes_assigned, 0);
}

#[test]
fn get_returns_registered_profile() {
    let (_pic, escrow) = setup();
    let caller = user(2);
    register(&escrow, caller, None);

    let loaded = get(&escrow, caller, caller).expect("registered");
    assert_eq!(loaded.principal, caller);
    assert_eq!(loaded.status, ArbitratorStatus::Active);
}

#[test]
fn get_returns_none_for_unregistered() {
    let (_pic, escrow) = setup();
    let asker = user(3);
    let target = user(4);
    assert!(get(&escrow, asker, target).is_none());
}

#[test]
fn list_returns_registered_arbitrators() {
    let (_pic, escrow) = setup();
    for i in 10..15_u8 {
        register(&escrow, user(i), None);
    }
    let viewer = user(99);
    let all = list(&escrow, viewer, ListArbitratorsArgs::default());
    assert!(all.len() >= 5, "got {} arbitrators", all.len());
}

#[test]
fn list_filters_by_status() {
    let (_pic, escrow) = setup();
    let active = user(20);
    let to_deregister = user(21);
    register(&escrow, active, None);
    register(&escrow, to_deregister, None);
    deregister(&escrow, to_deregister);

    let active_only = list(
        &escrow,
        user(99),
        ListArbitratorsArgs {
            status: Some(ArbitratorStatus::Active),
            ..Default::default()
        },
    );
    assert!(active_only.iter().any(|a| a.principal == active));
    assert!(!active_only.iter().any(|a| a.principal == to_deregister));
}

// --- idempotency ---

#[test]
fn register_is_idempotent() {
    let (_pic, escrow) = setup();
    let caller = user(30);

    let first = match register(&escrow, caller, Some("v1".to_owned())) {
        RegisterArbitratorResult::Ok(p) => *p,
        RegisterArbitratorResult::Err(e) => panic!("unexpected error: {e:?}"),
    };
    let second = match register(&escrow, caller, Some("v2".to_owned())) {
        RegisterArbitratorResult::Ok(p) => *p,
        RegisterArbitratorResult::Err(e) => panic!("unexpected error: {e:?}"),
    };

    assert_eq!(first.principal, second.principal);
    assert_eq!(
        first.registered_at_ns, second.registered_at_ns,
        "registered_at_ns is preserved across re-registration",
    );
    assert_eq!(
        second.bio.as_deref(),
        Some("v2"),
        "bio is updated on re-registration",
    );
}

#[test]
fn register_reactivates_deregistered() {
    let (_pic, escrow) = setup();
    let caller = user(31);

    register(&escrow, caller, None);
    let _ = deregister(&escrow, caller);
    let after_dereg = get(&escrow, caller, caller).unwrap();
    assert_eq!(after_dereg.status, ArbitratorStatus::Deregistered);

    let reactivated = match register(&escrow, caller, None) {
        RegisterArbitratorResult::Ok(p) => *p,
        RegisterArbitratorResult::Err(e) => panic!("unexpected error: {e:?}"),
    };
    assert_eq!(reactivated.status, ArbitratorStatus::Active);
}

#[test]
fn deregister_is_idempotent() {
    let (_pic, escrow) = setup();
    let caller = user(32);
    register(&escrow, caller, None);

    let first = match deregister(&escrow, caller) {
        DeregisterArbitratorResult::Ok(p) => *p,
        DeregisterArbitratorResult::Err(e) => panic!("first dereg failed: {e:?}"),
    };
    let second = match deregister(&escrow, caller) {
        DeregisterArbitratorResult::Ok(p) => *p,
        DeregisterArbitratorResult::Err(e) => panic!("second dereg failed: {e:?}"),
    };
    assert_eq!(first.status, ArbitratorStatus::Deregistered);
    assert_eq!(second.status, ArbitratorStatus::Deregistered);
    assert_eq!(first.principal, second.principal);
}

// --- error variants ---

#[test]
fn deregister_unregistered_returns_not_found() {
    let (_pic, escrow) = setup();
    let result = deregister(&escrow, user(40));
    match result {
        DeregisterArbitratorResult::Err(EscrowError::NotFound) => {}
        DeregisterArbitratorResult::Err(e) => panic!("wrong error: {e:?}"),
        DeregisterArbitratorResult::Ok(p) => panic!("unexpected ok: {p:?}"),
    }
}

#[test]
fn register_oversized_bio_returns_validation_error() {
    let (_pic, escrow) = setup();
    let caller = user(41);
    let huge = "x".repeat(2000);

    let result = register(&escrow, caller, Some(huge));
    match result {
        RegisterArbitratorResult::Err(EscrowError::ValidationError(_)) => {}
        RegisterArbitratorResult::Err(e) => panic!("wrong error: {e:?}"),
        RegisterArbitratorResult::Ok(p) => panic!("unexpected ok: {p:?}"),
    }
}

#[test]
fn anonymous_caller_is_rejected() {
    let (_pic, escrow) = setup();
    let result: Result<RegisterArbitratorResult, String> = escrow.update(
        Principal::anonymous(),
        "register_arbitrator",
        (RegisterArbitratorArgs { bio: None },),
    );
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}
