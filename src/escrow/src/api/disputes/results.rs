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

/// Reduced public view for status pages.
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
        // Phase-gated disclosure: tally + outcome only visible after
        // the dispute reaches `Resolved`. `withdraw_dispute` early-
        // sets `dispute.outcome = Some(Withdrawn { … })` BEFORE
        // flipping `phase` to `Resolved` (the early-set is what
        // serialises against concurrent `submit_evidence` /
        // `cast_vote` — see `services::disputes::withdraw`); during
        // that async window the public view must NOT leak the
        // outcome, otherwise external observers learn the resolution
        // before the canister has finished applying it. Gating both
        // fields on `phase == Resolved` keeps the public view
        // monotonic with the dispute's externally-visible state.
        let resolved = matches!(d.phase, DisputePhase::Resolved);
        let outcome = if resolved { d.outcome.clone() } else { None };
        let tally = if resolved {
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
            outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::PublicDisputeView;
    use crate::types::dispute::{Dispute, DisputeOutcome, DisputePhase, Vote};

    fn make_dispute(phase: DisputePhase, outcome: Option<DisputeOutcome>) -> Dispute {
        Dispute {
            id: 1,
            deal_id: 42,
            opened_by: Principal::from_slice(&[1]),
            opened_at_ns: 100,
            phase,
            evidence_deadline_ns: 200,
            voting_deadline_ns: 300,
            panel: vec![],
            evidence: vec![],
            arbitration_fee: 0,
            outcome,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        }
    }

    #[test]
    fn public_view_hides_outcome_during_evidence_phase() {
        // Reproduces the scenario flagged in PR #29 review: `withdraw`
        // early-sets `outcome = Some(Withdrawn { ... })` before
        // flipping `phase` to `Resolved`. During the async payout
        // window, the public view must not leak the outcome.
        let dispute = make_dispute(
            DisputePhase::Evidence,
            Some(DisputeOutcome::Withdrawn {
                agreed: Vote::ConcludedCorrectly,
            }),
        );
        let view = PublicDisputeView::from(&dispute);
        assert!(
            view.outcome.is_none(),
            "outcome must be hidden pre-Resolved"
        );
        assert!(view.tally.is_none(), "tally must be hidden pre-Resolved");
    }

    #[test]
    fn public_view_hides_outcome_during_voting_phase() {
        let dispute = make_dispute(
            DisputePhase::Voting,
            Some(DisputeOutcome::Settled {
                cc: 2,
                ic: 1,
                abstain: 0,
            }),
        );
        let view = PublicDisputeView::from(&dispute);
        assert!(view.outcome.is_none());
        assert!(view.tally.is_none());
    }

    #[test]
    fn public_view_reveals_outcome_when_resolved() {
        let dispute = make_dispute(
            DisputePhase::Resolved,
            Some(DisputeOutcome::Settled {
                cc: 2,
                ic: 1,
                abstain: 0,
            }),
        );
        let view = PublicDisputeView::from(&dispute);
        assert!(matches!(view.outcome, Some(DisputeOutcome::Settled { .. })));
        assert!(view.tally.is_some());
    }
}
