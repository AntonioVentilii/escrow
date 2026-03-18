use candid::{CandidType, Deserialize, Principal};

use crate::types::deal::DealId;

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CreateDealArgs {
    pub amount: u128,
    pub token_ledger: Principal,
    pub expires_at_ns: u64,
    pub recipient: Option<Principal>,
    pub title: Option<String>,
    pub note: Option<String>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct FundDealArgs {
    pub deal_id: DealId,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AcceptDealArgs {
    pub deal_id: DealId,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ReclaimDealArgs {
    pub deal_id: DealId,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CancelDealArgs {
    pub deal_id: DealId,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ListMyDealsArgs {
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}
