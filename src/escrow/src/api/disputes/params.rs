use candid::{CandidType, Deserialize};

use crate::types::{deal::DealId, dispute::DisputePhase};

/// Arguments for `open_dispute`.
///
/// Caller must be `payer` or `recipient` of a `Funded` deal with both
/// parties bound (Q2/Q3). The deal transitions `Funded → Disputed`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct OpenDisputeArgs {
    /// Identifier of the deal to dispute.
    pub deal_id: DealId,
}

/// Pagination + filter arguments for `list_my_disputes`.
#[derive(CandidType, Deserialize, Clone, Debug, Default)]
pub struct ListMyDisputesArgs {
    /// Number of disputes to skip (0-based). Defaults to `0`.
    pub offset: Option<u64>,
    /// Maximum number of disputes to return. Defaults to `50`, capped
    /// at `100`.
    pub limit: Option<u64>,
    /// Filter by phase. `None` returns disputes in all phases.
    pub phase: Option<DisputePhase>,
}
