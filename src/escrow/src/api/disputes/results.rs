use candid::{CandidType, Deserialize, Principal};

use crate::{
    api::deals::errors::EscrowError,
    types::{
        deal::DealId,
        dispute::{Dispute, DisputeId, DisputeOutcome, DisputePhase, Evidence, PanelMember, Vote},
    },
};

macro_rules! candid_result {
    ($name:ident, $ok:ty) => {
        #[derive(CandidType, Deserialize, Clone, Debug)]
        pub enum $name {
            Ok(Box<$ok>),
            Err(EscrowError),
        }

        impl From<Result<$ok, EscrowError>> for $name {
            fn from(result: Result<$ok, EscrowError>) -> Self {
                match result {
                    Ok(v) => Self::Ok(Box::new(v)),
                    Err(e) => Self::Err(e),
                }
            }
        }
    };
}

candid_result!(OpenDisputeResult, DisputeView);
candid_result!(SubmitEvidenceResult, DisputeView);
candid_result!(CastVoteResult, DisputeView);
candid_result!(FinalizeDisputeResult, DisputeView);
candid_result!(WithdrawDisputeResult, DisputeView);
candid_result!(GetDisputeResult, DisputeView);
candid_result!(GetPublicDisputeResult, PublicDisputeView);

/// Full dispute view returned to authorised callers (parties of the
/// parent deal + arbitrators on the panel). Mirrors the shape of
/// [`crate::types::dispute::Dispute`] one-to-one for simplicity.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DisputeView {
    pub id: DisputeId,
    pub deal_id: DealId,
    pub opened_by: Principal,
    pub opened_at_ns: u64,
    pub phase: DisputePhase,
    pub evidence_deadline_ns: u64,
    pub voting_deadline_ns: u64,
    pub panel: Vec<PanelMember>,
    pub evidence: Vec<Evidence>,
    pub arbitration_fee: u128,
    pub outcome: Option<DisputeOutcome>,
    pub payer_withdraw_proposal: Option<Vote>,
    pub recipient_withdraw_proposal: Option<Vote>,
}

impl From<&Dispute> for DisputeView {
    fn from(d: &Dispute) -> Self {
        Self {
            id: d.id,
            deal_id: d.deal_id,
            opened_by: d.opened_by,
            opened_at_ns: d.opened_at_ns,
            phase: d.phase.clone(),
            evidence_deadline_ns: d.evidence_deadline_ns,
            voting_deadline_ns: d.voting_deadline_ns,
            panel: d.panel.clone(),
            evidence: d.evidence.clone(),
            arbitration_fee: d.arbitration_fee,
            outcome: d.outcome.clone(),
            payer_withdraw_proposal: d.payer_withdraw_proposal.clone(),
            recipient_withdraw_proposal: d.recipient_withdraw_proposal.clone(),
        }
    }
}

/// Reduced public view for status pages (RFC-001 — Candid surface).
///
/// No party principals, no panel principals, no evidence URLs — just
/// the minimum any authenticated caller needs to render a dispute's
/// progress. Exposing the `deal_id` is intentional (it's already public
/// via the ICRC-7 token mapping); exposing the cc/ic/abstain counts
/// after resolution lets external indexers display tally summaries.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct PublicDisputeView {
    pub id: DisputeId,
    pub deal_id: DealId,
    pub phase: DisputePhase,
    pub evidence_deadline_ns: u64,
    pub voting_deadline_ns: u64,
    /// Number of arbitrators on the panel (no principals).
    pub panel_size: u32,
    /// Number of evidence submissions (no URLs).
    pub evidence_count: u32,
    /// Vote counts visible only after the dispute reaches `Resolved`
    /// (per phase-gated info disclosure). `None` while the dispute is
    /// still in `Evidence` / `Voting`.
    pub tally: Option<DisputeTally>,
    /// Outcome — set when phase is `Resolved`.
    pub outcome: Option<DisputeOutcome>,
}

/// Per-outcome vote counts. Mirrors the shape inside
/// `DisputeOutcome::Settled` / `Refunded` / `NoQuorum` so the public view
/// can surface counts independently of the `DisputeOutcome` discriminant.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DisputeTally {
    pub cc: u32,
    pub ic: u32,
    pub abstain: u32,
}

impl From<&Dispute> for PublicDisputeView {
    fn from(d: &Dispute) -> Self {
        let panel_size = u32::try_from(d.panel.len()).unwrap_or(u32::MAX);
        let evidence_count = u32::try_from(d.evidence.len()).unwrap_or(u32::MAX);
        // Phase-gated disclosure: tally only visible after Resolved (per Q11/Q9).
        let tally = if matches!(d.phase, DisputePhase::Resolved) {
            d.outcome.as_ref().map(|o| match o {
                DisputeOutcome::Settled { cc, ic, abstain }
                | DisputeOutcome::Refunded { cc, ic, abstain }
                | DisputeOutcome::NoQuorum { cc, ic, abstain } => DisputeTally {
                    cc: *cc,
                    ic: *ic,
                    abstain: *abstain,
                },
                DisputeOutcome::Withdrawn { .. } => DisputeTally {
                    cc: 0,
                    ic: 0,
                    abstain: 0,
                },
            })
        } else {
            None
        };
        Self {
            id: d.id,
            deal_id: d.deal_id,
            phase: d.phase.clone(),
            evidence_deadline_ns: d.evidence_deadline_ns,
            voting_deadline_ns: d.voting_deadline_ns,
            panel_size,
            evidence_count,
            tally,
            outcome: d.outcome.clone(),
        }
    }
}
