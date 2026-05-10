use core::time::Duration;
use std::{fs, sync::Arc};

use candid::{encode_one, Principal};
use escrow::api::{
    deals::results::ProcessExpiredDealsResult,
    disputes::{params::ListMyDisputesArgs, results::DisputeView},
};
use pocket_ic::PocketIc;

use crate::utils::pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait};

fn user() -> Principal {
    Principal::from_slice(&[1])
}

fn setup() -> (Arc<PocketIc>, PicCanister) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow").deploy_to(&pic);
    (pic, escrow)
}

#[test]
fn deploys_with_expiry_sweep_timer() {
    let (_pic, _escrow) = setup();
}

#[test]
fn process_expired_returns_empty_when_no_deals() {
    let (_pic, escrow) = setup();

    let result: ProcessExpiredDealsResult = escrow
        .update(user(), "process_expired_deals", (10_u32,))
        .expect("update call failed");

    match result {
        ProcessExpiredDealsResult::Ok(ids) => assert!(ids.is_empty()),
        ProcessExpiredDealsResult::Err(e) => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn canister_healthy_after_sweep_timer_fires() {
    let (pic, escrow) = setup();

    pic.advance_time(Duration::from_mins(6));
    for _ in 0..10 {
        pic.tick();
    }

    let result: ProcessExpiredDealsResult = escrow
        .update(user(), "process_expired_deals", (10_u32,))
        .expect("canister should still be operational after sweep");

    match result {
        ProcessExpiredDealsResult::Ok(ids) => assert!(ids.is_empty()),
        ProcessExpiredDealsResult::Err(e) => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn dispute_sweep_runs_without_panicking() {
    // RFC-001 step 8 — the auto-finalize dispute sweep is wired from
    // `init` and `post_upgrade`. With no disputes in storage (empty
    // canister), the sweep should fire its 5-minute timer, find no
    // due disputes, and exit cleanly without trapping.
    let (pic, escrow) = setup();

    pic.advance_time(Duration::from_mins(6));
    for _ in 0..10 {
        pic.tick();
    }

    // The canister is still operational — `list_my_disputes` returns
    // empty rather than trapping, which would happen if the sweep
    // timer trapped during its first cycle.
    let result: Vec<DisputeView> = escrow
        .query(user(), "list_my_disputes", (ListMyDisputesArgs::default(),))
        .expect("canister should still be operational after dispute sweep");
    assert!(result.is_empty());
}

#[test]
fn survives_upgrade_with_timer_restart() {
    let (pic, escrow) = setup();

    pic.advance_time(Duration::from_mins(6));
    for _ in 0..5 {
        pic.tick();
    }

    let wasm_bytes = fs::read(PicCanister::cargo_wasm_path("escrow")).expect("wasm not found");
    pic.upgrade_canister(
        escrow.canister_id(),
        wasm_bytes,
        encode_one(()).unwrap(),
        None,
    )
    .expect("upgrade failed");

    pic.advance_time(Duration::from_mins(6));
    for _ in 0..10 {
        pic.tick();
    }

    let result: ProcessExpiredDealsResult = escrow
        .update(user(), "process_expired_deals", (10_u32,))
        .expect("canister should be operational after upgrade + sweep");

    match result {
        ProcessExpiredDealsResult::Ok(ids) => assert!(ids.is_empty()),
        ProcessExpiredDealsResult::Err(e) => panic!("unexpected error: {e:?}"),
    }
}
