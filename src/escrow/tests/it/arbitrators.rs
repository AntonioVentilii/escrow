//! Integration tests for the arbitrator endpoints.
//!
//! Curated registration model: only canister controllers can register
//! arbitrators (`admin_register_arbitrator`); the registered principal
//! can self-opt-out (`deregister_arbitrator`); status moderation is
//! admin-only (`admin_set_arbitrator_status`); reads are public.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        admin::{
            params::{AdminRegisterArbitratorArgs, AdminSetArbitratorStatusArgs},
            results::{AdminRegisterArbitratorResult, AdminSetArbitratorStatusResult},
        },
        arbitrators::{params::ListArbitratorsArgs, results::DeregisterArbitratorResult},
        deals::errors::EscrowError,
    },
    types::arbitrator::{ArbitratorProfile, ArbitratorStatus},
};
use pocket_ic::PocketIc;

use crate::utils::pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait};

fn user(id: u8) -> Principal {
    Principal::from_slice(&[id])
}

fn admin() -> Principal {
    user(200)
}

/// Spins up a fresh canister with `admin()` as a controller. All admin
/// endpoints in these tests are called through that principal.
fn setup() -> (Arc<PocketIc>, PicCanister) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow")
        .with_controllers(vec![admin()])
        .deploy_to(&pic);
    (pic, escrow)
}

fn admin_register(escrow: &PicCanister, target: Principal) -> AdminRegisterArbitratorResult {
    escrow
        .update(
            admin(),
            "admin_register_arbitrator",
            (AdminRegisterArbitratorArgs { principal: target },),
        )
        .expect("admin_register_arbitrator call failed")
}

fn admin_set_status(
    escrow: &PicCanister,
    target: Principal,
    status: ArbitratorStatus,
) -> AdminSetArbitratorStatusResult {
    escrow
        .update(
            admin(),
            "admin_set_arbitrator_status",
            (AdminSetArbitratorStatusArgs {
                principal: target,
                status,
            },),
        )
        .expect("admin_set_arbitrator_status call failed")
}

fn deregister(escrow: &PicCanister, caller: Principal) -> DeregisterArbitratorResult {
    escrow
        .update(caller, "deregister_arbitrator", ())
        .expect("deregister_arbitrator call failed")
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

// --- admin_register: happy path ---

#[test]
fn admin_register_creates_active_profile() {
    let (_pic, escrow) = setup();
    let target = user(1);

    let result = admin_register(&escrow, target);
    let profile = match result {
        AdminRegisterArbitratorResult::Ok(p) => *p,
        AdminRegisterArbitratorResult::Err(e) => panic!("unexpected error: {e:?}"),
    };
    assert_eq!(profile.principal, target);
    assert_eq!(profile.registered_by, admin());
    assert_eq!(profile.status, ArbitratorStatus::Active);
    assert_eq!(profile.disputes_assigned, 0);
}

#[test]
fn admin_register_is_idempotent() {
    let (_pic, escrow) = setup();
    let target = user(2);

    let first = match admin_register(&escrow, target) {
        AdminRegisterArbitratorResult::Ok(p) => *p,
        AdminRegisterArbitratorResult::Err(e) => panic!("first failed: {e:?}"),
    };
    let second = match admin_register(&escrow, target) {
        AdminRegisterArbitratorResult::Ok(p) => *p,
        AdminRegisterArbitratorResult::Err(e) => panic!("second failed: {e:?}"),
    };
    assert_eq!(first.principal, second.principal);
    assert_eq!(
        first.registered_at_ns, second.registered_at_ns,
        "registered_at_ns preserved on re-registration",
    );
}

#[test]
fn admin_register_reactivates_deregistered() {
    let (_pic, escrow) = setup();
    let target = user(3);
    admin_register(&escrow, target);
    deregister(&escrow, target);
    let after_dereg = get(&escrow, admin(), target).unwrap();
    assert_eq!(after_dereg.status, ArbitratorStatus::Deregistered);

    let reactivated = match admin_register(&escrow, target) {
        AdminRegisterArbitratorResult::Ok(p) => *p,
        AdminRegisterArbitratorResult::Err(e) => panic!("reactivate failed: {e:?}"),
    };
    assert_eq!(reactivated.status, ArbitratorStatus::Active);
}

// --- admin_register: error variants ---

#[test]
fn admin_register_rejects_non_controller_caller() {
    let (_pic, escrow) = setup();
    let stranger = user(99);

    let result: Result<AdminRegisterArbitratorResult, String> = escrow.update(
        stranger,
        "admin_register_arbitrator",
        (AdminRegisterArbitratorArgs { principal: user(1) },),
    );
    let err = result.expect_err("non-controller should be rejected by guard");
    assert!(err.contains("not a controller"), "got: {err}");
}

#[test]
fn admin_register_rejects_anonymous_target() {
    let (_pic, escrow) = setup();
    let result = admin_register(&escrow, Principal::anonymous());
    match result {
        AdminRegisterArbitratorResult::Err(EscrowError::AnonymousParty) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn admin_register_rejects_canister_self() {
    let (_pic, escrow) = setup();
    let canister_id = escrow.canister_id();
    let result = admin_register(&escrow, canister_id);
    match result {
        AdminRegisterArbitratorResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("canister's own principal"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

// --- admin_set_arbitrator_status ---

#[test]
fn admin_set_status_returns_not_found_for_unregistered() {
    let (_pic, escrow) = setup();
    let result = admin_set_status(&escrow, user(40), ArbitratorStatus::Suspended);
    match result {
        AdminSetArbitratorStatusResult::Err(EscrowError::NotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn admin_set_status_supports_all_transitions() {
    let (_pic, escrow) = setup();
    let target = user(41);
    admin_register(&escrow, target);

    for new_status in [
        ArbitratorStatus::Suspended,
        ArbitratorStatus::Deregistered,
        ArbitratorStatus::Active,
        ArbitratorStatus::Active, // self-transition no-op
    ] {
        let result = admin_set_status(&escrow, target, new_status.clone());
        let p = match result {
            AdminSetArbitratorStatusResult::Ok(p) => *p,
            AdminSetArbitratorStatusResult::Err(e) => panic!("transition failed: {e:?}"),
        };
        assert_eq!(p.status, new_status);
    }
}

#[test]
fn admin_set_status_rejects_non_controller_caller() {
    let (_pic, escrow) = setup();
    let stranger = user(98);
    let result: Result<AdminSetArbitratorStatusResult, String> = escrow.update(
        stranger,
        "admin_set_arbitrator_status",
        (AdminSetArbitratorStatusArgs {
            principal: user(1),
            status: ArbitratorStatus::Suspended,
        },),
    );
    let err = result.expect_err("non-controller should be rejected by guard");
    assert!(err.contains("not a controller"), "got: {err}");
}

// --- deregister (self) ---

#[test]
fn deregister_unregistered_returns_not_found() {
    let (_pic, escrow) = setup();
    let result = deregister(&escrow, user(50));
    match result {
        DeregisterArbitratorResult::Err(EscrowError::NotFound) => {}
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn deregister_is_idempotent() {
    let (_pic, escrow) = setup();
    let target = user(51);
    admin_register(&escrow, target);

    let first = match deregister(&escrow, target) {
        DeregisterArbitratorResult::Ok(p) => *p,
        DeregisterArbitratorResult::Err(e) => panic!("first dereg failed: {e:?}"),
    };
    let second = match deregister(&escrow, target) {
        DeregisterArbitratorResult::Ok(p) => *p,
        DeregisterArbitratorResult::Err(e) => panic!("second dereg failed: {e:?}"),
    };
    assert_eq!(first.status, ArbitratorStatus::Deregistered);
    assert_eq!(second.status, ArbitratorStatus::Deregistered);
}

#[test]
fn deregister_anonymous_caller_blocked() {
    let (_pic, escrow) = setup();
    let result: Result<DeregisterArbitratorResult, String> =
        escrow.update(Principal::anonymous(), "deregister_arbitrator", ());
    let err = result.expect_err("anonymous should be rejected by guard");
    assert!(
        err.contains("Anonymous caller not authorised"),
        "got: {err}",
    );
}

// --- queries ---

#[test]
fn get_returns_registered_profile() {
    let (_pic, escrow) = setup();
    let target = user(60);
    admin_register(&escrow, target);

    let loaded = get(&escrow, user(99), target).expect("registered");
    assert_eq!(loaded.principal, target);
    assert_eq!(loaded.registered_by, admin());
    assert_eq!(loaded.status, ArbitratorStatus::Active);
}

#[test]
fn get_returns_none_for_unregistered() {
    let (_pic, escrow) = setup();
    assert!(get(&escrow, user(70), user(71)).is_none());
}

#[test]
fn list_returns_registered_arbitrators() {
    let (_pic, escrow) = setup();
    for i in 80..85_u8 {
        admin_register(&escrow, user(i));
    }
    let all = list(&escrow, user(99), ListArbitratorsArgs::default());
    assert!(all.len() >= 5, "got {} arbitrators", all.len());
}

#[test]
fn list_filters_by_status() {
    let (_pic, escrow) = setup();
    let active = user(90);
    let to_suspend = user(91);
    admin_register(&escrow, active);
    admin_register(&escrow, to_suspend);
    admin_set_status(&escrow, to_suspend, ArbitratorStatus::Suspended);

    let only_active = list(
        &escrow,
        user(99),
        ListArbitratorsArgs {
            status: Some(ArbitratorStatus::Active),
            ..Default::default()
        },
    );
    assert!(only_active.iter().any(|a| a.principal == active));
    assert!(!only_active.iter().any(|a| a.principal == to_suspend));
}
