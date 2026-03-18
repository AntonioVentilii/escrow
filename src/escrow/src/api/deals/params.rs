use candid::{CandidType, Deserialize, Principal};

use crate::types::deal::DealId;

/// Arguments for creating a new tip deal.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CreateDealArgs {
    /// Token amount to escrow (must be > 0).
    pub amount: u128,
    /// Principal of the ICRC-1/ICRC-2 token ledger canister.
    pub token_ledger: Principal,
    /// Nanosecond UTC timestamp after which the deal expires and becomes reclaimable.
    pub expires_at_ns: u64,
    /// Optional recipient principal. If `None`, the recipient is bound on first acceptance (QR /
    /// share-link flow).
    pub recipient: Option<Principal>,
    /// Optional short title displayed on claim pages.
    pub title: Option<String>,
    /// Optional note or message attached to the tip.
    pub note: Option<String>,
}

/// Arguments for funding a created deal via ICRC-2 `transfer_from`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct FundDealArgs {
    /// Identifier of the deal to fund.
    pub deal_id: DealId,
}

/// Arguments for accepting (claiming) a funded deal.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AcceptDealArgs {
    /// Identifier of the deal to accept.
    pub deal_id: DealId,
}

/// Arguments for reclaiming funds from an expired, unclaimed deal.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ReclaimDealArgs {
    /// Identifier of the deal to reclaim.
    pub deal_id: DealId,
}

/// Arguments for cancelling an unfunded deal.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CancelDealArgs {
    /// Identifier of the deal to cancel.
    pub deal_id: DealId,
}

/// Pagination arguments for listing the caller's deals.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ListMyDealsArgs {
    /// Number of deals to skip (0-based). Defaults to `0`.
    pub offset: Option<u64>,
    /// Maximum number of deals to return. Defaults to `50`.
    pub limit: Option<u64>,
}
