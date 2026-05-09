use candid::{CandidType, Deserialize};

use crate::types::arbitrator::ArbitratorStatus;

/// Arguments for `register_arbitrator`.
///
/// Idempotent — calling `register_arbitrator` for a principal that is
/// already registered returns the existing profile rather than erroring
/// (per RFC-001 Q4 decision). The `bio` is updated on re-registration.
#[derive(CandidType, Deserialize, Clone, Debug, Default)]
pub struct RegisterArbitratorArgs {
    /// Plain-text introduction (max 1 KiB at the canister boundary).
    pub bio: Option<String>,
}

/// Pagination + filter arguments for `list_arbitrators`.
#[derive(CandidType, Deserialize, Clone, Debug, Default)]
pub struct ListArbitratorsArgs {
    /// Number of arbitrators to skip (0-based). Defaults to `0`.
    pub offset: Option<u64>,
    /// Maximum number of arbitrators to return. Defaults to `50`, capped
    /// at `100`.
    pub limit: Option<u64>,
    /// Filter by status. `None` returns all statuses.
    pub status: Option<ArbitratorStatus>,
    /// Filter by minimum reliability score. Arbitrators with `score = None`
    /// are included only when `min_score` is `None`.
    pub min_score: Option<u32>,
}
