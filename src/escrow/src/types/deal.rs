use candid::{CandidType, Deserialize, Principal};

pub type DealId = u64;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DealStatus {
    Created,
    Funded,
    Settled,
    Refunded,
    Cancelled,
    Rejected,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Consent {
    Pending,
    Accepted,
    Rejected,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealMetadata {
    pub title: Option<String>,
    pub note: Option<String>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Deal {
    pub id: DealId,
    pub payer: Option<Principal>,
    pub recipient: Option<Principal>,
    pub token_ledger: Principal,
    pub token_symbol: Option<String>,
    pub amount: u128,
    pub created_at_ns: u64,
    pub created_by: Principal,
    pub updated_at_ns: Option<u64>,
    pub updated_by: Option<Principal>,
    pub expires_at_ns: u64,
    pub status: DealStatus,
    pub escrow_subaccount: Vec<u8>,
    pub funded_at_ns: Option<u64>,
    pub settled_at_ns: Option<u64>,
    pub refunded_at_ns: Option<u64>,
    pub funding_tx: Option<u128>,
    pub payout_tx: Option<u128>,
    pub refund_tx: Option<u128>,
    pub claim_code: Option<String>,
    pub payer_consent: Consent,
    pub recipient_consent: Consent,
    pub metadata: Option<DealMetadata>,
}
