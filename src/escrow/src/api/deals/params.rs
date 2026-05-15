use candid::{CandidType, Deserialize, Principal};

use crate::types::{asset::Asset, deal::DealId};

/// Arguments for creating a new deal.
///
/// If both `payer` and `recipient` are `None`, the caller defaults to payer
/// and the recipient is bound on first acceptance (tip / share-link flow).
/// If neither resolved party is the caller, the call is rejected. The
/// caller's consent is automatically set to `Accepted`; the counterparty's
/// consent starts as `Pending`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CreateDealArgs {
    /// Token amount to escrow (must be > 0). Denominated in the
    /// base units of the deal's [`Asset`] (e.g. e8s for ICP).
    pub amount: u128,
    /// Settlement asset for this deal. Today the canister only
    /// handles [`Asset::Icrc`] — clients should pass
    /// `Asset::Icrc(<ledger principal>)`. The variant exists so
    /// future settlement domains (EVM, Solana, …) can be added
    /// without renaming this field.
    pub asset: Asset,
    /// Nanosecond UTC timestamp after which the deal expires and becomes reclaimable.
    pub expires_at_ns: u64,
    /// Optional payer principal. If `None`, defaults to the caller when the
    /// caller is not the recipient.
    pub payer: Option<Principal>,
    /// Optional recipient principal. If `None`, the recipient is bound on first
    /// acceptance (QR / share-link flow).
    pub recipient: Option<Principal>,
    /// Optional short title displayed on claim pages.
    pub title: Option<String>,
    /// Optional note or message attached to the deal.
    pub note: Option<String>,
    /// Optional per-deal arbitrator panel size. If `Some(n)`, any
    /// dispute opened on this deal will use `n` arbitrators regardless
    /// of subsequent `DisputeConfig.panel_size` changes — the deal
    /// terms are a contract at create time. `None` falls back to
    /// `DisputeConfig.panel_size` at `open_dispute` time.
    ///
    /// Validation at create time: must be odd, must be within
    /// `[DisputeConfig.min_panel_size, DisputeConfig.max_panel_size]`.
    /// Out-of-range values return `EscrowError::PanelSizeOutOfRange`.
    pub panel_size: Option<u32>,
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
    /// Claim code required for open (unbound-recipient) deals. Must match the
    /// code generated at deal creation. Not required when the caller is the
    /// deal's bound recipient.
    pub claim_code: Option<String>,
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

/// Arguments for explicitly consenting to a deal's terms.
///
/// The caller must be the payer or recipient. Their consent is set to
/// `Accepted`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ConsentDealArgs {
    /// Identifier of the deal to consent to.
    pub deal_id: DealId,
}

/// Arguments for rejecting a deal's terms.
///
/// The caller must be the payer or recipient. The deal transitions to
/// `Rejected` (terminal).
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct RejectDealArgs {
    /// Identifier of the deal to reject.
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
