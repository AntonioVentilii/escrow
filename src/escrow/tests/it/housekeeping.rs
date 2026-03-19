use core::time::Duration;
use std::{fs, sync::Arc};

use candid::{encode_one, Principal};
use escrow::api::deals::results::ProcessExpiredDealsResult;
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

    pic.advance_time(Duration::from_secs(6 * 60));
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
fn survives_upgrade_with_timer_restart() {
    let (pic, escrow) = setup();

    pic.advance_time(Duration::from_secs(6 * 60));
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

    pic.advance_time(Duration::from_secs(6 * 60));
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
