use candid::{CandidType, Deserialize, Principal};

/// Minimum non-abstain votes an arbitrator must have cast before a
/// reliability score is reported. Below this threshold, the
/// `ArbitratorProfile::score` field is `None` to avoid noisy early
/// signals.
pub const MIN_VOTES_FOR_SCORE: u32 = 5;

/// Lifecycle status of an arbitrator. Transitions:
///
/// ```text
/// (unregistered) ──register──▶ Active ──admin──▶ Suspended
///                                  │
///                                  └──self──▶ Deregistered (terminal)
/// ```
///
/// `Suspended` and `Deregistered` arbitrators cannot be selected for new
/// disputes, but in-flight assignments are honoured (a non-vote counts as
/// `Vote::Abstain` at finalize time).
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ArbitratorStatus {
    Active,
    Suspended,
    Deregistered,
}

/// Public profile of a registered arbitrator.
///
/// Score-related fields follow these rules:
///
/// | Outcome                | Voter type              | `assigned` | `voted` | `with_majority` |
/// | ---------------------- | ----------------------- | ---------- | ------- | --------------- |
/// | `Settled` / `Refunded` | non-abstain w/ majority | +1         | +1      | +1              |
/// | `Settled` / `Refunded` | non-abstain vs majority | +1         | +1      | +0              |
/// | `Settled` / `Refunded` | abstain                 | +1         | +0      | +0              |
/// | `NoQuorum`             | any (incl. non-abstain) | +1         | +0      | +0              |
/// | `Withdrawn`            | any                     | +1         | +0      | +0              |
///
/// `NoQuorum` and `Withdrawn` deliberately don't update `voted` or
/// `with_majority` — there's no on-canister verdict against which to
/// score votes, and counting them would create a perverse incentive
/// to abstain on hard-to-quorum disputes.
///
/// `score` is computed via [`ArbitratorProfile::compute_score`] and is
/// `None` until `disputes_voted >= MIN_VOTES_FOR_SCORE`.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ArbitratorProfile {
    pub principal: Principal,
    pub registered_at_ns: u64,
    /// Plain-text introduction (max 1 KiB, validated at the boundary).
    pub bio: Option<String>,
    /// Total disputes the arbitrator was selected for.
    pub disputes_assigned: u32,
    /// Disputes the arbitrator submitted a non-abstain vote on, excluding
    /// `NoQuorum` and `Withdrawn` outcomes.
    pub disputes_voted: u32,
    /// Disputes where the arbitrator's non-abstain vote matched the
    /// eventual majority. `NoQuorum` / `Withdrawn` outcomes never
    /// increment this counter.
    pub disputes_with_majority: u32,
    /// 0–100 reliability score, or `None` until enough scored votes
    /// accumulate (`disputes_voted >= MIN_VOTES_FOR_SCORE`).
    pub score: Option<u32>,
    pub status: ArbitratorStatus,
}

impl ArbitratorProfile {
    /// Computes the reliability score from per-arbitrator counters.
    ///
    /// Returns `None` until `disputes_voted >= MIN_VOTES_FOR_SCORE`. Above
    /// the threshold, returns `(disputes_with_majority * 100) /
    /// disputes_voted` clamped into `0..=100`.
    #[must_use]
    pub fn compute_score(disputes_voted: u32, disputes_with_majority: u32) -> Option<u32> {
        if disputes_voted < MIN_VOTES_FOR_SCORE {
            return None;
        }
        // disputes_voted >= 5 ⇒ denom > 0; saturating multiply guards against the
        // (currently impossible) case where with_majority * 100 overflows u32.
        let raw = disputes_with_majority.saturating_mul(100) / disputes_voted;
        Some(raw.min(100))
    }
}

#[cfg(test)]
mod tests {
    use candid::{Decode, Encode, Principal};

    use super::{ArbitratorProfile, ArbitratorStatus, MIN_VOTES_FOR_SCORE};

    #[test]
    fn score_is_none_below_threshold() {
        for voted in 0..MIN_VOTES_FOR_SCORE {
            assert_eq!(
                ArbitratorProfile::compute_score(voted, voted),
                None,
                "voted={voted}",
            );
        }
    }

    #[test]
    fn score_is_some_at_threshold() {
        assert_eq!(
            ArbitratorProfile::compute_score(MIN_VOTES_FOR_SCORE, MIN_VOTES_FOR_SCORE),
            Some(100),
        );
    }

    #[test]
    fn score_is_proportional_above_threshold() {
        assert_eq!(ArbitratorProfile::compute_score(10, 7), Some(70));
        assert_eq!(ArbitratorProfile::compute_score(10, 0), Some(0));
        assert_eq!(ArbitratorProfile::compute_score(20, 13), Some(65));
    }

    #[test]
    fn score_is_clamped_to_100() {
        // with_majority should never exceed voted, but the clamp guards
        // against caller bugs nonetheless.
        assert_eq!(ArbitratorProfile::compute_score(5, 999), Some(100));
    }

    #[test]
    fn arbitrator_status_round_trips_through_candid() {
        for s in [
            ArbitratorStatus::Active,
            ArbitratorStatus::Suspended,
            ArbitratorStatus::Deregistered,
        ] {
            let bytes = Encode!(&s).expect("encode");
            let decoded: ArbitratorStatus = Decode!(&bytes, ArbitratorStatus).expect("decode");
            assert_eq!(s, decoded);
        }
    }

    #[test]
    fn profile_round_trips_through_candid() {
        let profile = ArbitratorProfile {
            principal: Principal::from_slice(&[42]),
            registered_at_ns: 100,
            bio: Some("hello".to_owned()),
            disputes_assigned: 7,
            disputes_voted: 6,
            disputes_with_majority: 4,
            score: Some(66),
            status: ArbitratorStatus::Active,
        };
        let bytes = Encode!(&profile).expect("encode");
        let decoded: ArbitratorProfile = Decode!(&bytes, ArbitratorProfile).expect("decode");
        assert_eq!(decoded.principal, profile.principal);
        assert_eq!(decoded.disputes_assigned, 7);
        assert_eq!(decoded.score, Some(66));
        assert_eq!(decoded.status, ArbitratorStatus::Active);
    }
}
