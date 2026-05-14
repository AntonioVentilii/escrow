use candid::{CandidType, Deserialize, Principal};

use crate::types::deal::DealId;

/// Identifier for a dispute. Allocated atomically by the storage layer
/// in [`crate::memory`].
pub type DisputeId = u64;

/// Lifecycle phase of a dispute. The phase advances on a strict timeline
/// driven by `evidence_deadline_ns` / `voting_deadline_ns` (set at
/// `open_dispute` time using the windows in [`DisputeConfig`]).
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DisputePhase {
    /// Evidence submission window. Both parties + arbitrators may post
    /// evidence; voting is closed.
    Evidence,
    /// Voting window. Evidence is frozen, arbitrators cast their votes.
    Voting,
    /// Tally finalised; outcome propagated to the parent `Deal`. Terminal.
    Resolved,
}

/// A single arbitrator's vote on a dispute, or — at the canister boundary —
/// the outcome a party proposes via `withdraw_dispute`. The validator
/// at the boundary rejects `Abstain` for out-of-band proposals.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Vote {
    /// "Concluded Correctly" — release funds to recipient.
    ConcludedCorrectly,
    /// "Incorrectly Concluded" — refund payer.
    IncorrectlyConcluded,
    /// Arbitrator abstained. Counts toward `disputes_assigned` but never
    /// toward `disputes_voted` / `disputes_with_majority` for the
    /// reliability score.
    Abstain,
}

/// A piece of evidence attached to a dispute.
///
/// Artefacts are stored **off-canister**: the canister only
/// records a URL + SHA-256 commitment + an optional short note. Length
/// limits are enforced at the canister boundary; this struct itself is
/// purely a data carrier.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Evidence {
    pub submitter: Principal,
    pub submitted_at_ns: u64,
    /// Free-form note (max 4 KiB at the boundary →
    /// [`EscrowError::EvidenceTooLarge`]).
    pub note: Option<String>,
    /// Off-canister artefact URL (max 2 KiB at the boundary).
    pub artefact_url: Option<String>,
    /// SHA-256 of the off-canister artefact. Always exactly 32 bytes when
    /// `Some` — the validator rejects other lengths.
    pub artefact_sha256: Option<Vec<u8>>,
}

/// One member of the dispute panel.
///
/// Mirrors the `Deal.{funded_at_ns, funding_tx, settled_at_ns, payout_tx}`
/// pattern: `paid_at_ns` + `payout_tx` are populated when the per-arbitrator
/// fee transfer succeeds at finalize time, making the fan-out payout
/// replay-safe.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct PanelMember {
    pub principal: Principal,
    /// `None` until the arbitrator calls `cast_vote` during the Voting phase.
    pub vote: Option<Vote>,
    /// `None` until the arbitrator's slice of the arbitration fee has been
    /// successfully transferred via the ICRC-1 ledger at finalize.
    pub paid_at_ns: Option<u64>,
    /// ICRC-1 ledger block index of the per-arbitrator payout transfer.
    pub payout_tx: Option<u128>,
}

/// Outcome of a resolved dispute. Set on the `Dispute` record at finalize
/// time. The mapping to `DealStatus` is:
///
/// | `DisputeOutcome`                                 | Resulting `DealStatus`     |
/// | ------------------------------------------------ | -------------------------- |
/// | `Settled { … }`                                  | `ArbitratedSettled`        |
/// | `Refunded { … }`                                 | `ArbitratedRefunded`       |
/// | `NoQuorum { … }`                                 | `ArbitratedRefunded` (no-quorum fallback) |
/// | `Withdrawn { agreed: ConcludedCorrectly }`       | `ArbitratedSettled`        |
/// | `Withdrawn { agreed: IncorrectlyConcluded }`     | `ArbitratedRefunded`       |
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DisputeOutcome {
    /// Majority CC — funds released to recipient.
    Settled { cc: u32, ic: u32, abstain: u32 },
    /// Majority IC — funds refunded to payer.
    Refunded { cc: u32, ic: u32, abstain: u32 },
    /// Voting deadline reached without enough non-abstain votes.
    /// Falls back to refunding the payer (status quo ante).
    NoQuorum { cc: u32, ic: u32, abstain: u32 },
    /// Both parties agreed out-of-band on `agreed` via `withdraw_dispute`.
    /// Arbitrators receive the reduced `withdraw_fee_pct` slice of the fee.
    Withdrawn { agreed: Vote },
}

/// A single dispute attached to a deal.
///
/// Created by `open_dispute`, advanced through phases by
/// `submit_evidence` / `cast_vote`, resolved by `finalize_dispute` /
/// `withdraw_dispute`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Dispute {
    pub id: DisputeId,
    pub deal_id: DealId,
    pub opened_by: Principal,
    pub opened_at_ns: u64,
    pub phase: DisputePhase,
    /// End of the Evidence window. Set once at `open_dispute` time.
    pub evidence_deadline_ns: u64,
    /// End of the Voting window. Set once at `open_dispute` time.
    pub voting_deadline_ns: u64,
    /// Panel selected for this dispute. Committed once at `open_dispute`
    /// time — no re-selection on tally.
    pub panel: Vec<PanelMember>,
    /// Evidence submissions in submission order.
    pub evidence: Vec<Evidence>,
    /// Arbitration fee in the deal's token, computed at `open_dispute` and
    /// frozen for the dispute's lifetime. Sourced from the disputed amount
    /// in the escrow subaccount.
    pub arbitration_fee: u128,
    /// Tally + outcome. `None` until the dispute is `Resolved`.
    pub outcome: Option<DisputeOutcome>,
    /// Payer's out-of-band withdrawal proposal. Resolution fires when
    /// both party fields are `Some` and equal.
    pub payer_withdraw_proposal: Option<Vote>,
    /// Recipient's out-of-band withdrawal proposal.
    pub recipient_withdraw_proposal: Option<Vote>,
}

/// Admin-tunable dispute configuration. Lives nested in [`crate::types::state::Config`].
///
/// All windows are nanoseconds (canister convention); fee bps follows the
/// standard ICRC bps convention (`10_000` = 100%).
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DisputeConfig {
    /// Default number of arbitrators selected per dispute when the
    /// deal creator did not pick a per-deal `panel_size`. Must be odd
    /// and within `[min_panel_size, max_panel_size]`.
    /// `validation::validate_dispute_config` enforces all invariants
    /// when `update_config` is called.
    pub panel_size: u32,
    /// Lower bound on the panel sizes a deal creator may request via
    /// `CreateDealArgs.panel_size`. Must be odd and `>= 3` (odd-only
    /// is required by the tally rules — no tie possible without an
    /// abstention; the `>= 3` floor is the smallest meaningful jury).
    pub min_panel_size: u32,
    /// Upper bound on the panel sizes a deal creator may request.
    /// Must be odd and `>= min_panel_size`. Bounds the cost (each
    /// extra arbitrator adds an ICRC-1 ledger fee at finalize) and
    /// the latency to fill the panel from the eligible pool.
    pub max_panel_size: u32,
    /// Length of the Evidence phase, in nanoseconds (default 3 days).
    pub evidence_window_ns: u64,
    /// Length of the Voting phase, in nanoseconds (default 2 days).
    pub voting_window_ns: u64,
    /// Arbitration fee in basis points of the disputed amount
    /// (default 500 = 5%). Combined with [`Self::arbitration_min_fee`].
    pub arbitration_fee_bps: u32,
    /// Minimum arbitration fee in the deal's token. The effective fee
    /// at `open_dispute` is `max(arbitration_min_fee, amount *
    /// arbitration_fee_bps / 10_000)`.
    pub arbitration_min_fee: u128,
    /// Percentage of the arbitration fee paid to the panel when both
    /// parties resolve out-of-band via `withdraw_dispute` (default
    /// 25). `validation::validate_dispute_config` rejects values
    /// `> 100` at `update_config` time;
    /// `services::disputes::withdraw_finalize_locked` also clamps
    /// defensively at the use site so a hypothetical bad config that
    /// somehow slipped through (e.g. a future migration) can't
    /// over-pay arbitrators.
    pub withdraw_fee_pct: u32,
    /// Optional minimum arbitrator score required to be eligible for
    /// selection (Sybil filter). `None` = bootstrap mode (every active
    /// arbitrator is eligible; default).
    pub min_arbitrator_score: Option<u32>,
}

impl DisputeConfig {
    /// `const`-callable default. Used to initialise the `CONFIG`
    /// thread-local in `const` context; `Default::default` delegates
    /// here so runtime callers and the static initialiser stay in
    /// sync.
    #[must_use]
    pub const fn const_default() -> Self {
        const NANOS_PER_DAY: u64 = 24 * 60 * 60 * 1_000_000_000;
        Self {
            panel_size: 3,
            min_panel_size: 3,
            // Default 11 (4 odd choices: 3, 5, 7, 9, 11) lets the
            // Figma 3 / 7 / 11 picker triplet work without admin
            // intervention. Earlier draft used 9 which rejected the
            // "Slow" Figma option.
            max_panel_size: 11,
            evidence_window_ns: 3 * NANOS_PER_DAY,
            voting_window_ns: 2 * NANOS_PER_DAY,
            arbitration_fee_bps: 500,
            arbitration_min_fee: 0,
            withdraw_fee_pct: 25,
            min_arbitrator_score: None,
        }
    }
}

impl Default for DisputeConfig {
    fn default() -> Self {
        Self::const_default()
    }
}

#[cfg(test)]
mod tests {
    use candid::{Decode, Encode, Principal};

    use super::{
        Dispute, DisputeConfig, DisputeOutcome, DisputePhase, Evidence, PanelMember, Vote,
    };

    #[test]
    fn dispute_config_defaults_match_locked_decisions() {
        let cfg = DisputeConfig::default();
        assert_eq!(cfg.panel_size, 3);
        assert_eq!(cfg.min_panel_size, 3);
        assert_eq!(cfg.max_panel_size, 11);
        assert_eq!(cfg.evidence_window_ns, 3 * 24 * 60 * 60 * 1_000_000_000);
        assert_eq!(cfg.voting_window_ns, 2 * 24 * 60 * 60 * 1_000_000_000);
        assert_eq!(cfg.arbitration_fee_bps, 500);
        assert_eq!(cfg.arbitration_min_fee, 0);
        assert_eq!(cfg.withdraw_fee_pct, 25);
        assert!(cfg.min_arbitrator_score.is_none());
    }

    #[test]
    fn vote_round_trips_through_candid() {
        for v in [
            Vote::ConcludedCorrectly,
            Vote::IncorrectlyConcluded,
            Vote::Abstain,
        ] {
            let bytes = Encode!(&v).expect("encode");
            let decoded: Vote = Decode!(&bytes, Vote).expect("decode");
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn phase_round_trips_through_candid() {
        for p in [
            DisputePhase::Evidence,
            DisputePhase::Voting,
            DisputePhase::Resolved,
        ] {
            let bytes = Encode!(&p).expect("encode");
            let decoded: DisputePhase = Decode!(&bytes, DisputePhase).expect("decode");
            assert_eq!(p, decoded);
        }
    }

    #[test]
    fn outcome_round_trips_through_candid() {
        let cases = [
            DisputeOutcome::Settled {
                cc: 2,
                ic: 1,
                abstain: 0,
            },
            DisputeOutcome::Refunded {
                cc: 1,
                ic: 2,
                abstain: 0,
            },
            DisputeOutcome::NoQuorum {
                cc: 0,
                ic: 0,
                abstain: 3,
            },
            DisputeOutcome::Withdrawn {
                agreed: Vote::ConcludedCorrectly,
            },
        ];
        for o in cases {
            let bytes = Encode!(&o).expect("encode");
            let decoded: DisputeOutcome = Decode!(&bytes, DisputeOutcome).expect("decode");
            assert_eq!(o, decoded);
        }
    }

    #[test]
    fn dispute_round_trips_through_candid() {
        let dispute = Dispute {
            id: 1,
            deal_id: 42,
            opened_by: Principal::from_slice(&[1]),
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 200,
            voting_deadline_ns: 300,
            panel: vec![PanelMember {
                principal: Principal::from_slice(&[2]),
                vote: None,
                paid_at_ns: None,
                payout_tx: None,
            }],
            evidence: vec![Evidence {
                submitter: Principal::from_slice(&[1]),
                submitted_at_ns: 110,
                note: Some("hello".to_owned()),
                artefact_url: None,
                artefact_sha256: None,
            }],
            arbitration_fee: 1_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        };
        let bytes = Encode!(&dispute).expect("encode");
        let decoded: Dispute = Decode!(&bytes, Dispute).expect("decode");
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.deal_id, 42);
        assert_eq!(decoded.panel.len(), 1);
        assert_eq!(decoded.evidence.len(), 1);
    }
}
