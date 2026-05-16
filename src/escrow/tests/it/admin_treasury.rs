//! Integration tests for the controller-only treasury endpoints
//! and the create-time `creation_fee → treasury` deposit.

use core::time::Duration;
use std::sync::Arc;

use candid::Principal;
use escrow::{
    api::{
        admin::{
            params::{AdminTreasuryBalanceArgs, AdminTreasuryWithdrawArgs},
            results::{AdminTreasuryBalanceResult, AdminTreasuryWithdrawResult},
        },
        deals::{
            params::CreateDealArgs,
            results::{CreateDealResult, DealView},
        },
    },
    types::{asset::Asset, ledger_types::Account},
};
use pocket_ic::PocketIc;

use crate::utils::{
    icrc_ledger::{IcrcLedger, IcrcLedgerBuilder},
    pic_canister::{PicCanister, PicCanisterBuilder, PicCanisterTrait},
};

const CREATION_FEE: u128 = 20_000;

fn admin() -> Principal {
    Principal::from_slice(&[200])
}

fn payer() -> Principal {
    Principal::from_slice(&[1])
}

fn recipient() -> Principal {
    Principal::from_slice(&[2])
}

fn drain_target() -> Principal {
    Principal::from_slice(&[210])
}

fn setup() -> (Arc<PocketIc>, PicCanister, IcrcLedger) {
    let pic = Arc::new(PocketIc::new());
    let escrow = PicCanisterBuilder::new("escrow")
        .with_controllers(vec![admin()])
        .deploy_to(&pic);
    let ledger = IcrcLedgerBuilder::new()
        .with_initial_balance(payer(), 1_000_000_000_000)
        .with_initial_balance(recipient(), 1_000_000_000_000)
        .deploy_to(&pic);
    (pic, escrow, ledger)
}

/// Returns the canonical treasury subaccount (mirrors
/// `escrow::subaccounts::treasury_subaccount`). Hard-coded here
/// rather than re-imported because the helper is `pub` in the lib
/// — keeping a copy here documents the expected layout at the
/// test boundary.
fn treasury_subaccount() -> Vec<u8> {
    let mut subaccount = vec![0_u8; 32];
    let domain = b"escrow-treasury";
    subaccount[..domain.len()].copy_from_slice(domain);
    subaccount
}

fn far_future(pic: &PocketIc) -> u64 {
    let now_ns = pic.get_time().as_nanos_since_unix_epoch();
    let bump = u64::try_from(Duration::from_hours(1).as_nanos()).expect("1h fits in u64 ns");
    now_ns + bump
}

fn create_3a_bound(
    escrow: &PicCanister,
    ledger: &IcrcLedger,
    amount: u128,
    expires_at_ns: u64,
) -> DealView {
    let lf = ledger.fee;
    // Worst-case: amount + DC/2 (= amount/40 under default DisputeConfig)
    // + creation_fee + 2*LF.
    ledger.approve(
        payer(),
        escrow.canister_id(),
        amount + amount / 40 + CREATION_FEE + 2 * lf + 1,
    );
    let args = CreateDealArgs {
        amount,
        asset: Asset::Icrc(ledger.principal()),
        expires_at_ns,
        payer: Some(payer()),
        recipient: Some(recipient()),
        title: None,
        note: None,
        panel_size: None,
    };
    let result: CreateDealResult = escrow
        .update(payer(), "create_deal", (args,))
        .expect("create_deal call");
    match result {
        CreateDealResult::Ok(view) => *view,
        CreateDealResult::Err(e) => panic!("create_deal: {e:?}"),
    }
}

fn admin_treasury_balance(escrow: &PicCanister, caller: Principal, ledger: Principal) -> u128 {
    let result: AdminTreasuryBalanceResult = escrow
        .update(
            caller,
            "admin_treasury_balance",
            (AdminTreasuryBalanceArgs {
                asset: Asset::Icrc(ledger),
            },),
        )
        .expect("admin_treasury_balance call");
    match result {
        AdminTreasuryBalanceResult::Ok(b) => *b,
        AdminTreasuryBalanceResult::Err(e) => panic!("admin_treasury_balance: {e:?}"),
    }
}

#[test]
fn creation_fee_lands_in_treasury_on_3a_create() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;

    let treasury_pre = ledger.balance_of_subaccount(escrow.canister_id(), treasury_subaccount());
    assert_eq!(treasury_pre, 0, "treasury empty on a fresh canister");

    let _ = create_3a_bound(&escrow, &ledger, amount, far_future(&pic));

    let treasury_post = ledger.balance_of_subaccount(escrow.canister_id(), treasury_subaccount());
    assert_eq!(
        treasury_post, CREATION_FEE,
        "treasury subaccount holds exactly the creation_fee after one bound deal create",
    );
}

#[test]
fn admin_treasury_balance_returns_live_balance() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;

    // Two bound deals → treasury accumulates `2 * CREATION_FEE`.
    let _ = create_3a_bound(&escrow, &ledger, amount, far_future(&pic));
    let _ = create_3a_bound(&escrow, &ledger, amount, far_future(&pic));

    let balance = admin_treasury_balance(&escrow, admin(), ledger.principal());
    assert_eq!(
        balance,
        2 * CREATION_FEE,
        "admin_treasury_balance reflects the cumulative creation_fees",
    );
}

#[test]
fn admin_treasury_balance_rejects_non_controller() {
    let (_pic, escrow, ledger) = setup();
    // Non-controller principal: the payer test principal, definitely
    // not in the controllers list (we only added `admin()` above).
    let result: Result<AdminTreasuryBalanceResult, String> = escrow.update(
        payer(),
        "admin_treasury_balance",
        (AdminTreasuryBalanceArgs {
            asset: Asset::Icrc(ledger.principal()),
        },),
    );
    let err = result.expect_err("non-controller call must be rejected by guard");
    assert!(
        err.contains("not a controller"),
        "expected guard rejection, got: {err}",
    );
}

#[test]
fn admin_treasury_withdraw_drains_to_destination() {
    let (pic, escrow, ledger) = setup();
    let amount: u128 = 1_000_000_000;
    let lf = ledger.fee;

    // Seed the treasury with one bound deal's creation_fee.
    let _ = create_3a_bound(&escrow, &ledger, amount, far_future(&pic));
    assert_eq!(
        admin_treasury_balance(&escrow, admin(), ledger.principal()),
        CREATION_FEE,
    );

    let drain_pre = ledger.balance_of_owner(drain_target());

    // Drain (CREATION_FEE - lf) so the ledger fee fits.
    let drain_amount = CREATION_FEE - lf;
    let result: AdminTreasuryWithdrawResult = escrow
        .update(
            admin(),
            "admin_treasury_withdraw",
            (AdminTreasuryWithdrawArgs {
                asset: Asset::Icrc(ledger.principal()),
                to: Account {
                    owner: drain_target(),
                    subaccount: None,
                },
                amount: drain_amount,
            },),
        )
        .expect("admin_treasury_withdraw call");
    match result {
        AdminTreasuryWithdrawResult::Ok(_block) => {}
        AdminTreasuryWithdrawResult::Err(e) => panic!("admin_treasury_withdraw: {e:?}"),
    }

    // Destination wallet credited the drained amount.
    assert_eq!(
        ledger.balance_of_owner(drain_target()) - drain_pre,
        drain_amount,
        "drain target received the requested amount",
    );
    // Treasury subaccount balance: started at CREATION_FEE, sent
    // (drain_amount + lf) = CREATION_FEE → ends at 0.
    assert_eq!(
        admin_treasury_balance(&escrow, admin(), ledger.principal()),
        0,
        "treasury fully drained",
    );
}

#[test]
fn admin_treasury_withdraw_rejects_non_controller() {
    let (_pic, escrow, ledger) = setup();
    let result: Result<AdminTreasuryWithdrawResult, String> = escrow.update(
        payer(),
        "admin_treasury_withdraw",
        (AdminTreasuryWithdrawArgs {
            asset: Asset::Icrc(ledger.principal()),
            to: Account {
                owner: drain_target(),
                subaccount: None,
            },
            amount: 1,
        },),
    );
    let err = result.expect_err("non-controller call must be rejected by guard");
    assert!(
        err.contains("not a controller"),
        "expected guard rejection, got: {err}",
    );
}
