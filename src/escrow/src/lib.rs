use candid::Nat;
use ic_cdk::export_candid;
use ic_cdk_macros::{post_upgrade, pre_upgrade};

use crate::{
    api::deals::{
        params::{
            AcceptDealArgs, CancelDealArgs, ConsentDealArgs, CreateDealArgs, FundDealArgs,
            ListMyDealsArgs, ReclaimDealArgs, RejectDealArgs,
        },
        results::{
            AcceptDealResult, CancelDealResult, ConsentDealResult, CreateDealResult, DealView,
            FundDealResult, GetClaimableDealResult, GetDealResult, GetEscrowAccountResult,
            ProcessExpiredDealsResult, ReclaimDealResult, RejectDealResult,
        },
    },
    types::{
        deal::DealId,
        icrc7::{Icrc7TransferArg, Icrc7TransferResponse, SupportedStandard, Value},
        ledger_types::Account,
        state::Config,
    },
};

pub mod api;
pub mod guards;
pub mod ledger;
pub mod memory;
pub mod services;
pub mod subaccounts;
pub mod types;
pub mod validation;

#[pre_upgrade]
fn pre_upgrade() {
    memory::save_state();
}

#[post_upgrade]
fn post_upgrade() {
    memory::restore_state();
}

export_candid!();
