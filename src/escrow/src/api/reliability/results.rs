use candid::{CandidType, Deserialize};

use crate::services::reliability::ReliabilityScore;

/// Public view of a principal's reliability score.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ReliabilityView {
    /// Reliability percentage (0–100).
    pub score: u32,
    /// Deals that ended positively (Settled or Refunded).
    pub positive: u32,
    /// Total concluded deals (positive + counterparty rejections).
    pub concluded: u32,
}

impl From<ReliabilityScore> for ReliabilityView {
    fn from(r: ReliabilityScore) -> Self {
        Self {
            score: r.score,
            positive: r.positive,
            concluded: r.concluded,
        }
    }
}
