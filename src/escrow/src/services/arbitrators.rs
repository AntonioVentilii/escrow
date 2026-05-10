//! Arbitrator registry service.
//!
//! Owns the lifecycle of the per-principal `ArbitratorProfile` records.
//! Endpoints in `api/arbitrators/api.rs` are thin wrappers over the
//! functions here; per the four-layer rule this module never calls
//! `ic_cdk::api::msg_caller()` directly — the caller is passed in.

use candid::Principal;

use crate::{
    api::{arbitrators::params::ListArbitratorsArgs, deals::errors::EscrowError},
    memory,
    types::arbitrator::{ArbitratorProfile, ArbitratorStatus},
    validation,
};

/// Registers `caller` as an arbitrator, or returns the existing profile
/// if already registered (idempotent).
///
/// Re-registration updates the bio if supplied. The on-chain record's
/// `registered_at_ns` and score-related counters are preserved across
/// re-registration.
pub fn register(
    caller: Principal,
    bio: Option<String>,
    now_ns: u64,
) -> Result<ArbitratorProfile, EscrowError> {
    validation::validate_arbitrator_bio(bio.as_deref())?;

    if let Some(existing) = memory::get_arbitrator(caller) {
        // Idempotent re-registration: refresh bio + reactivate if Suspended/Deregistered.
        // (Deregistration is reversible by re-registering — the canister doesn't
        // permanently bar a principal; admin Suspend remains the bad-actor lever.)
        let updated = ArbitratorProfile {
            bio: bio.or(existing.bio),
            status: ArbitratorStatus::Active,
            ..existing
        };
        memory::upsert_arbitrator(updated.clone());
        return Ok(updated);
    }

    let profile = ArbitratorProfile {
        principal: caller,
        registered_at_ns: now_ns,
        bio,
        disputes_assigned: 0,
        disputes_voted: 0,
        disputes_with_majority: 0,
        score: None,
        status: ArbitratorStatus::Active,
    };
    memory::upsert_arbitrator(profile.clone());
    Ok(profile)
}

/// Marks `caller`'s arbitrator profile as `Deregistered`.
///
/// Returns `EscrowError::NotFound` if the caller isn't registered. Calling
/// `deregister` on an already-deregistered profile is a no-op success
/// (idempotent — matches the canister-wide idempotency contract).
///
/// In-flight assignments are honoured: a non-vote from a
/// deregistered arbitrator counts as `Vote::Abstain` at finalize time.
pub fn deregister(caller: Principal) -> Result<ArbitratorProfile, EscrowError> {
    let existing = memory::get_arbitrator(caller).ok_or(EscrowError::NotFound)?;
    if matches!(existing.status, ArbitratorStatus::Deregistered) {
        return Ok(existing);
    }

    let updated = memory::with_arbitrator(caller, |a| {
        a.status = ArbitratorStatus::Deregistered;
        a.clone()
    })
    .ok_or(EscrowError::NotFound)?;
    Ok(updated)
}

/// Returns the arbitrator profile for `principal`, or `None` if the
/// principal hasn't registered. Public read; no auth beyond non-anonymous.
#[must_use]
pub fn get(principal: Principal) -> Option<ArbitratorProfile> {
    memory::get_arbitrator(principal)
}

/// Lists arbitrators with optional filters + pagination.
///
/// Filters compose as AND: `status` (when set) AND `min_score` (when set).
/// Arbitrators with `score = None` are excluded only when `min_score` is
/// `Some`. Results are ordered by principal (`BTreeMap` iteration order).
#[must_use]
pub fn list(args: &ListArbitratorsArgs) -> Vec<ArbitratorProfile> {
    let offset = args
        .offset
        .and_then(|o| usize::try_from(o).ok())
        .unwrap_or(0);
    let limit = args
        .limit
        .map_or(50_usize, |l| usize::try_from(l.min(100)).unwrap_or(100));

    memory::with_arbitrators(|map| {
        map.values()
            .filter(|a| match &args.status {
                Some(s) => &a.status == s,
                None => true,
            })
            .filter(|a| match args.min_score {
                Some(min) => a.score.is_some_and(|s| s >= min),
                None => true,
            })
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{deregister, get, list, register};
    use crate::{
        api::{arbitrators::params::ListArbitratorsArgs, deals::errors::EscrowError},
        memory::with_arbitrator,
        types::arbitrator::ArbitratorStatus,
    };

    fn principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    #[test]
    fn register_creates_active_profile() {
        let p = principal(101);
        let profile = register(p, Some("hello".to_owned()), 100).unwrap();
        assert_eq!(profile.principal, p);
        assert_eq!(profile.status, ArbitratorStatus::Active);
        assert_eq!(profile.bio.as_deref(), Some("hello"));
        assert_eq!(profile.disputes_assigned, 0);
        assert_eq!(profile.disputes_voted, 0);
        assert_eq!(profile.disputes_with_majority, 0);
        assert!(profile.score.is_none());
        assert_eq!(profile.registered_at_ns, 100);
    }

    #[test]
    fn register_is_idempotent_and_preserves_counters() {
        let p = principal(102);
        let _first = register(p, Some("v1".to_owned()), 100).unwrap();

        // Simulate prior activity.
        with_arbitrator(p, |a| {
            a.disputes_assigned = 7;
            a.disputes_voted = 5;
            a.disputes_with_majority = 4;
        });

        let second = register(p, Some("v2".to_owned()), 999).unwrap();
        assert_eq!(second.bio.as_deref(), Some("v2"), "bio is updated");
        assert_eq!(
            second.registered_at_ns, 100,
            "registered_at_ns is preserved on re-registration",
        );
        assert_eq!(second.disputes_assigned, 7);
        assert_eq!(second.disputes_voted, 5);
        assert_eq!(second.disputes_with_majority, 4);
    }

    #[test]
    fn register_reactivates_deregistered() {
        let p = principal(103);
        register(p, None, 100).unwrap();
        let _ = deregister(p).unwrap();
        assert_eq!(
            get(p).unwrap().status,
            ArbitratorStatus::Deregistered,
            "deregister flips status",
        );
        let reregistered = register(p, None, 200).unwrap();
        assert_eq!(reregistered.status, ArbitratorStatus::Active);
    }

    #[test]
    fn register_rejects_oversized_bio() {
        let p = principal(104);
        let huge = "x".repeat(2000);
        let err = register(p, Some(huge), 100).unwrap_err();
        assert!(matches!(err, EscrowError::ValidationError(_)));
    }

    #[test]
    fn deregister_returns_not_found_for_unregistered() {
        let err = deregister(principal(105)).unwrap_err();
        assert!(matches!(err, EscrowError::NotFound));
    }

    #[test]
    fn deregister_is_idempotent_when_already_deregistered() {
        let p = principal(106);
        register(p, None, 100).unwrap();
        let first = deregister(p).unwrap();
        assert_eq!(first.status, ArbitratorStatus::Deregistered);
        let second = deregister(p).unwrap();
        assert_eq!(second.status, ArbitratorStatus::Deregistered);
    }

    #[test]
    fn get_returns_none_for_unregistered() {
        assert!(get(principal(107)).is_none());
    }

    #[test]
    fn list_filters_by_status() {
        let active = principal(110);
        let suspended = principal(111);
        register(active, None, 100).unwrap();
        register(suspended, None, 100).unwrap();
        with_arbitrator(suspended, |a| a.status = ArbitratorStatus::Suspended);

        let only_active = list(&ListArbitratorsArgs {
            status: Some(ArbitratorStatus::Active),
            ..Default::default()
        });
        assert!(only_active
            .iter()
            .all(|a| a.status == ArbitratorStatus::Active));
        assert!(only_active.iter().any(|a| a.principal == active));
        assert!(!only_active.iter().any(|a| a.principal == suspended));
    }

    #[test]
    fn list_filters_by_min_score() {
        let high = principal(120);
        let low = principal(121);
        let unscored = principal(122);
        register(high, None, 100).unwrap();
        register(low, None, 100).unwrap();
        register(unscored, None, 100).unwrap();
        with_arbitrator(high, |a| a.score = Some(80));
        with_arbitrator(low, |a| a.score = Some(20));

        let above_50 = list(&ListArbitratorsArgs {
            min_score: Some(50),
            ..Default::default()
        });
        assert!(above_50.iter().any(|a| a.principal == high));
        assert!(!above_50.iter().any(|a| a.principal == low));
        assert!(
            !above_50.iter().any(|a| a.principal == unscored),
            "unscored arbitrators excluded when min_score is Some",
        );
    }

    #[test]
    fn list_paginates() {
        for i in 200..210_u8 {
            register(principal(i), None, 100).unwrap();
        }
        let page1 = list(&ListArbitratorsArgs {
            offset: Some(0),
            limit: Some(3),
            ..Default::default()
        });
        let page2 = list(&ListArbitratorsArgs {
            offset: Some(3),
            limit: Some(3),
            ..Default::default()
        });
        assert_eq!(page1.len(), 3);
        assert_eq!(page2.len(), 3);
        // Disjoint slices.
        for a in &page1 {
            assert!(!page2.iter().any(|b| b.principal == a.principal));
        }
    }

    #[test]
    fn list_caps_limit_at_100() {
        for i in 0..150_u8 {
            register(principal(i), None, 100).unwrap();
        }
        let huge = list(&ListArbitratorsArgs {
            offset: Some(0),
            limit: Some(10_000),
            ..Default::default()
        });
        assert!(huge.len() <= 100);
    }
}
