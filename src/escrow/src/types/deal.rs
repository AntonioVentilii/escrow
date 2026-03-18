use candid::{CandidType, Deserialize, Principal};

pub type DealId = u64;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DealStatus {
    Created,
    Funded,
    Completed,
    Refunded,
    Cancelled,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealMetadata {
    pub title: Option<String>,
    pub note: Option<String>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Deal {
    pub id: DealId,
    pub payer: Principal,
    pub recipient: Option<Principal>,
    pub token_ledger: Principal,
    pub token_symbol: Option<String>,
    pub amount: u128,
    pub created_at_ns: u64,
    pub expires_at_ns: u64,
    pub status: DealStatus,
    pub escrow_subaccount: Vec<u8>,
    pub funded_at_ns: Option<u64>,
    pub completed_at_ns: Option<u64>,
    pub refunded_at_ns: Option<u64>,
    pub funding_tx: Option<u128>,
    pub payout_tx: Option<u128>,
    pub refund_tx: Option<u128>,
    pub claim_code: Option<String>,
    pub metadata: Option<DealMetadata>,
}
