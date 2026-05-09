use candid::{CandidType, Deserialize, Principal};

use crate::types::dispute::DisputeId;

pub type DealId = u64;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DealStatus {
    Created,
    Funded,
    Settled,
    Refunded,
    Cancelled,
    Rejected,
    /// A dispute is open on this deal (RFC-001 Q1/Q2). Funds remain in the
    /// escrow subaccount until the dispute resolves to
    /// `ArbitratedSettled` / `ArbitratedRefunded`. The expiry sweep
    /// (`services::expiry`) skips deals in this state — see Q2 contract.
    Disputed,
    /// Dispute panel voted majority CC, OR both parties agreed out-of-band
    /// (Q12) on a CC outcome — funds released to recipient. Distinct from
    /// `Settled` so callers can tell arbitrated from unilateral
    /// settlement (Q1). Terminal.
    ArbitratedSettled,
    /// Dispute panel voted majority IC, OR both parties agreed out-of-band
    /// on an IC outcome, OR the panel reached no quorum (Q9 fallback).
    /// Funds refunded to payer. Distinct from `Refunded` (Q1). Terminal.
    ArbitratedRefunded,
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
    /// `Some(dispute_id)` while a dispute is open on the deal or after
    /// it has resolved (so the audit trail back to the `Dispute` record
    /// survives terminal status). `None` for deals that never went into
    /// dispute. New field for RFC-001 — `Option`-wrapped for backward-
    /// compat with pre-RFC-001 stable snapshots.
    pub dispute: Option<DisputeId>,
}
