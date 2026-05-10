use candid::{Nat, Principal};
use ic_cdk::{export_candid, init, post_upgrade, pre_upgrade};

use crate::{
    api::{
        arbitrators::{
            params::{ListArbitratorsArgs, RegisterArbitratorArgs},
            results::{DeregisterArbitratorResult, RegisterArbitratorResult},
        },
        deals::{
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
        disputes::{
            params::{
                CastVoteArgs, FinalizeDisputeArgs, ListMyDisputesArgs, OpenDisputeArgs,
                SubmitEvidenceArgs, WithdrawDisputeArgs,
            },
            results::{
                CastVoteResult, DisputeView, FinalizeDisputeResult, GetDisputeResult,
                GetPublicDisputeResult, OpenDisputeResult, SubmitEvidenceResult,
                WithdrawDisputeResult,
            },
        },
        reliability::results::ReliabilityView,
    },
    types::{
        arbitrator::ArbitratorProfile,
        deal::DealId,
        dispute::DisputeId,
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

#[init]
fn init() {
    services::housekeeping::start_expiry_sweep();
    services::housekeeping::start_dispute_sweep();
}

#[pre_upgrade]
fn pre_upgrade() {
    memory::save_state();
}

#[post_upgrade]
fn post_upgrade() {
    memory::restore_state();

    services::housekeeping::start_expiry_sweep();
    services::housekeeping::start_dispute_sweep();
}

export_candid!();
