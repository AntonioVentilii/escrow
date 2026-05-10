use candid::{CandidType, Deserialize};

use crate::types::arbitrator::ArbitratorStatus;

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
