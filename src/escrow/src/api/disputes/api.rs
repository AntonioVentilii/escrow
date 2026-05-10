use ic_cdk::api::{msg_caller, time};
use ic_cdk_macros::{query, update};

use super::{
    params::{
        CastVoteArgs, FinalizeDisputeArgs, ListMyDisputesArgs, OpenDisputeArgs, SubmitEvidenceArgs,
    },
    results::{
        CastVoteResult, DisputeView, FinalizeDisputeResult, GetDisputeResult,
        GetPublicDisputeResult, OpenDisputeResult, SubmitEvidenceResult,
    },
};
use crate::{guards::caller_is_not_anonymous, services, types::dispute::DisputeId};

// ---------------------------------------------------------------------------
// Update methods (RFC-001 step 4)
// ---------------------------------------------------------------------------

/// Opens a new dispute on a `Funded` deal. Caller must be `payer` or
/// `recipient`. Funds remain in the escrow subaccount; the deal
/// transitions `Funded → Disputed`. The expiry sweep skips
/// `Disputed` deals (per Q2 contract).
#[update(guard = "caller_is_not_anonymous")]
pub async fn open_dispute(OpenDisputeArgs { deal_id }: OpenDisputeArgs) -> OpenDisputeResult {
    services::disputes::open(msg_caller(), deal_id, time())
        .await
        .into()
}

/// Submits a piece of evidence on a dispute. Caller must be a party
/// of the parent deal or an arbitrator on the panel. Allowed during
/// the `Evidence` phase only. RFC-001 step 5.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn submit_evidence(
    SubmitEvidenceArgs {
        dispute_id,
        note,
        artefact_url,
        artefact_sha256,
    }: SubmitEvidenceArgs,
) -> SubmitEvidenceResult {
    services::disputes::submit_evidence(
        msg_caller(),
        dispute_id,
        note,
        artefact_url,
        artefact_sha256,
        time(),
    )
    .into()
}

/// Casts a vote on a dispute. Caller must be on the panel and
/// currently `Active`. Allowed only during the open voting window
/// (`evidence_deadline_ns <= now < voting_deadline_ns`). Latest-wins
/// semantics — calling repeatedly during the window updates the vote.
/// RFC-001 step 6.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn cast_vote(CastVoteArgs { dispute_id, vote }: CastVoteArgs) -> CastVoteResult {
    services::disputes::cast_vote(msg_caller(), dispute_id, vote, time()).into()
}

/// Force-finalises a dispute past its `voting_deadline_ns`. Anyone
/// (non-anonymous) can call. Idempotent — replays after a successful
/// finalize return the resolved view; partial replays (some
/// arbitrator transfers succeeded, others trapped) skip already-paid
/// panel members. Triggers tally + outcome propagation + ledger
/// transfers + arbitrator score updates. RFC-001 step 7.
#[update(guard = "caller_is_not_anonymous")]
pub async fn finalize_dispute(
    FinalizeDisputeArgs { dispute_id }: FinalizeDisputeArgs,
) -> FinalizeDisputeResult {
    services::disputes::finalize(dispute_id, time())
        .await
        .into()
}

// ---------------------------------------------------------------------------
// Query methods (RFC-001 step 4)
// ---------------------------------------------------------------------------

/// Returns the full dispute view. Caller must be a party of the parent
/// deal or an arbitrator on the panel.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_dispute(dispute_id: DisputeId) -> GetDisputeResult {
    services::disputes::get(msg_caller(), dispute_id).into()
}

/// Returns a reduced public view of a dispute (no party / panel
/// principals, no evidence URLs). Any non-anonymous caller may query.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_public_dispute(dispute_id: DisputeId) -> GetPublicDisputeResult {
    services::disputes::get_public(dispute_id).into()
}

/// Lists disputes the caller is involved with (party of the parent
/// deal or arbitrator on the panel), reverse-chronological by
/// `opened_at_ns`.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn list_my_disputes(
    ListMyDisputesArgs {
        offset,
        limit,
        phase,
    }: ListMyDisputesArgs,
) -> Vec<DisputeView> {
    services::disputes::list_for_caller(
        msg_caller(),
        &ListMyDisputesArgs {
            offset,
            limit,
            phase,
        },
    )
}
