//! Integration tests for `update_config` with `dispute_config`
//! validation.

use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        admin::results::{FeesView, UpdateConfigResult},
        deals::errors::EscrowError,
    },
    types::{dispute::DisputeConfig, state::Config},
};
use pocket_ic::PocketIc;

use crate::utils::pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait};

fn admin() -> Principal {
    Principal::from_slice(&[200])
}

fn setup() -> (Arc<PocketIc>, PicCanister) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow")
        .with_controllers(vec![admin()])
        .deploy_to(&pic);
    (pic, escrow)
}

fn try_update_config(escrow: &PicCanister, cfg: Config) -> UpdateConfigResult {
    escrow
        .update(admin(), "update_config", (cfg,))
        .expect("update_config call failed")
}

fn read_config(escrow: &PicCanister) -> Config {
    escrow
        .query(admin(), "config", ())
        .expect("config query failed")
}

#[test]
fn update_config_accepts_default_dispute_config() {
    let (_pic, escrow) = setup();
    let cfg = Config::default();
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Ok => {}
        UpdateConfigResult::Err(e) => panic!("expected Ok, got {e:?}"),
    }
    let stored = read_config(&escrow);
    assert_eq!(stored.dispute_config.panel_size, 3);
}

#[test]
fn update_config_rejects_even_panel_size() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            panel_size: 4,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("odd"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_panel_size_below_3() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            panel_size: 1,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("panel_size"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_zero_voting_window() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            voting_window_ns: 0,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("voting_window_ns"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_withdraw_fee_pct_over_100() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            withdraw_fee_pct: 200,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("withdraw_fee_pct"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_fee_bps_above_100_pct() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            arbitration_fee_bps: 10_001,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("arbitration_fee_bps"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_does_not_persist_invalid_config() {
    let (_pic, escrow) = setup();
    // First land a known-good config so we have a baseline.
    try_update_config(&escrow, Config::default());
    let baseline = read_config(&escrow);
    assert_eq!(baseline.dispute_config.panel_size, 3);

    // Now try to land a bad config; must reject without overwriting.
    let bad = Config {
        dispute_config: DisputeConfig {
            panel_size: 0,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    let result = try_update_config(&escrow, bad);
    assert!(matches!(result, UpdateConfigResult::Err(_)));

    // Baseline must still be there.
    let after = read_config(&escrow);
    assert_eq!(after.dispute_config.panel_size, 3);
}

#[test]
fn update_config_rejects_max_panel_below_min() {
    let (_pic, escrow) = setup();
    let cfg = Config {
        dispute_config: DisputeConfig {
            min_panel_size: 7,
            max_panel_size: 5,
            panel_size: 7,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(
                msg.contains("max_panel_size") && msg.contains("min_panel_size"),
                "msg: {msg}",
            );
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_default_panel_size_outside_bounds() {
    let (_pic, escrow) = setup();
    // panel_size = 13 but max_panel_size still defaults to 11.
    let cfg = Config {
        dispute_config: DisputeConfig {
            panel_size: 13,
            ..DisputeConfig::default()
        },
        ..Config::default()
    };
    match try_update_config(&escrow, cfg) {
        UpdateConfigResult::Err(EscrowError::ValidationError(msg)) => {
            assert!(msg.contains("must be within"), "msg: {msg}");
        }
        other => panic!("wrong response: {other:?}"),
    }
}

#[test]
fn update_config_rejects_non_controller_caller() {
    let (_pic, escrow) = setup();
    let stranger = Principal::from_slice(&[99]);
    let cfg = Config::default();
    let result: Result<UpdateConfigResult, String> =
        escrow.update(stranger, "update_config", (cfg,));
    let err = result.expect_err("non-controller should be rejected by guard");
    assert!(err.contains("not a controller"), "got: {err}");
}

fn read_fees(escrow: &PicCanister, caller: Principal) -> FeesView {
    escrow
        .query(caller, "get_fees", ())
        .expect("get_fees query failed")
}

#[test]
fn get_fees_returns_default_schedule_for_any_caller() {
    let (_pic, escrow) = setup();
    let default_cfg = Config::default();
    let stranger = Principal::from_slice(&[99]);
    let fees = read_fees(&escrow, stranger);
    assert_eq!(fees.escrow_fee, default_cfg.escrow_fee);
    assert_eq!(fees.creation_fee, default_cfg.creation_fee);
    assert_eq!(
        fees.arbitration_fee_bps,
        default_cfg.dispute_config.arbitration_fee_bps
    );
    assert_eq!(
        fees.arbitration_min_fee,
        default_cfg.dispute_config.arbitration_min_fee
    );
    assert_eq!(
        fees.withdraw_fee_pct,
        default_cfg.dispute_config.withdraw_fee_pct
    );
}

#[test]
fn get_fees_is_callable_by_anonymous() {
    let (_pic, escrow) = setup();
    let fees = read_fees(&escrow, Principal::anonymous());
    let default_cfg = Config::default();
    assert_eq!(fees.escrow_fee, default_cfg.escrow_fee);
    assert_eq!(fees.creation_fee, default_cfg.creation_fee);
}

#[test]
fn get_fees_reflects_update_config_changes() {
    let (_pic, escrow) = setup();
    let new_cfg = Config {
        escrow_fee: 12_345,
        creation_fee: 67_890,
        dispute_config: DisputeConfig {
            arbitration_fee_bps: 750,
            arbitration_min_fee: 111_222,
            withdraw_fee_pct: 33,
            ..DisputeConfig::default()
        },
    };
    match try_update_config(&escrow, new_cfg.clone()) {
        UpdateConfigResult::Ok => {}
        UpdateConfigResult::Err(e) => panic!("expected Ok, got {e:?}"),
    }
    let fees = read_fees(&escrow, admin());
    assert_eq!(fees.escrow_fee, new_cfg.escrow_fee);
    assert_eq!(fees.creation_fee, new_cfg.creation_fee);
    assert_eq!(
        fees.arbitration_fee_bps,
        new_cfg.dispute_config.arbitration_fee_bps
    );
    assert_eq!(
        fees.arbitration_min_fee,
        new_cfg.dispute_config.arbitration_min_fee
    );
    assert_eq!(
        fees.withdraw_fee_pct,
        new_cfg.dispute_config.withdraw_fee_pct
    );
}
