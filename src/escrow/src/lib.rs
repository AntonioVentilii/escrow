use ic_cdk::export_candid;
use ic_cdk_macros::{post_upgrade, pre_upgrade};

use crate::{
    api::deals::{
        errors::EscrowError,
        params::{
            AcceptDealArgs, CancelDealArgs, CreateDealArgs, FundDealArgs, ListMyDealsArgs,
            ReclaimDealArgs,
        },
        results::{ClaimableDealView, DealView},
    },
    types::{deal::DealId, ledger_types::Account, state::Config},
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
