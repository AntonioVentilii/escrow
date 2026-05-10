use candid::{CandidType, Deserialize};

use crate::types::{
    deal::DealId,
    dispute::{DisputeId, DisputePhase, Vote},
};

/// Arguments for `open_dispute`.
///
/// Caller must be `payer` or `recipient` of a `Funded` deal with both
/// parties bound (Q2/Q3). The deal transitions `Funded → Disputed`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct OpenDisputeArgs {
    /// Identifier of the deal to dispute.
    pub deal_id: DealId,
}

/// Arguments for `submit_evidence` (RFC-001 step 5).
///
/// Per Q8: at least one of `note` / `(artefact_url + artefact_sha256)`
/// must be present; URL and hash are paired (one without the other is
/// rejected); `note` <= 4 KiB; `artefact_url` <= 2 KiB;
/// `artefact_sha256` exactly 32 bytes when set.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SubmitEvidenceArgs {
    /// Identifier of the dispute receiving the evidence.
    pub dispute_id: DisputeId,
    /// Free-form note (max 4 KiB).
    pub note: Option<String>,
    /// Off-canister artefact URL (max 2 KiB).
    pub artefact_url: Option<String>,
    /// SHA-256 of the off-canister artefact. Always exactly 32 bytes
    /// when `Some`.
    pub artefact_sha256: Option<Vec<u8>>,
}

/// Arguments for `cast_vote` (RFC-001 step 6).
///
/// Caller must be on the dispute's panel and currently `Active`.
/// Allowed only during the open voting window
/// (`evidence_deadline_ns <= now_ns < voting_deadline_ns`). Latest-wins
/// — calling repeatedly during the window updates the recorded vote.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CastVoteArgs {
    pub dispute_id: DisputeId,
    pub vote: Vote,
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
