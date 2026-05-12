//! Arbitrator registry service.
//!
//! Curated model: only canister controllers can add arbitrators to the
//! pool (`admin_register`); the registered principal can self-opt-out
//! (`deregister`). Status moderation is admin-only
//! (`admin_set_status`). Read endpoints (`get`, `list`) are public.

use candid::Principal;

use crate::{
    api::{arbitrators::params::ListArbitratorsArgs, deals::errors::EscrowError},
    memory,
    types::arbitrator::{ArbitratorProfile, ArbitratorStatus},
    validation,
};

/// Admin-side registration. Idempotent — re-registering an existing
/// principal returns the existing profile (reactivating if it was
/// `Suspended` or `Deregistered`); score counters and
/// `registered_at_ns` are preserved across reactivation.
///
/// `caller_admin` is the controller principal making the registration
/// call; it's recorded on the profile as `registered_by` for audit.
/// `canister_id` is used by the validator to reject the canister's
/// own principal as a registration target.
pub fn admin_register(
    caller_admin: Principal,
    target: Principal,
    canister_id: Principal,
    now_ns: u64,
) -> Result<ArbitratorProfile, EscrowError> {
    validation::validate_arbitrator_principal(target, canister_id)?;

    if let Some(existing) = memory::get_arbitrator(target) {
        // Idempotent: refresh `registered_by` to the calling admin (so
        // the audit trail reflects the most recent curation event) and
        // reactivate if Suspended/Deregistered. Counters + first-seen
        // timestamp preserved.
        let updated = ArbitratorProfile {
            registered_by: caller_admin,
            status: ArbitratorStatus::Active,
            ..existing
        };
        memory::upsert_arbitrator(updated.clone());
        return Ok(updated);
    }

    let profile = ArbitratorProfile {
        principal: target,
        registered_at_ns: now_ns,
        registered_by: caller_admin,
        disputes_assigned: 0,
        disputes_voted: 0,
        disputes_with_majority: 0,
        score: None,
        status: ArbitratorStatus::Active,
    };
    memory::upsert_arbitrator(profile.clone());
    Ok(profile)
}

/// Admin-side status flip. All transitions are allowed; the canister
/// doesn't model a state machine on `ArbitratorStatus` (there's no
/// invariant that depends on the *order* of transitions, only on the
/// current value). Self-transitions are no-op success.
///
/// Returns `EscrowError::NotFound` if `target` isn't registered.
pub fn admin_set_status(
    target: Principal,
    new_status: ArbitratorStatus,
) -> Result<ArbitratorProfile, EscrowError> {
    let updated = memory::with_arbitrator(target, move |a| {
        a.status = new_status;
        a.clone()
    })
    .ok_or(EscrowError::NotFound)?;
    Ok(updated)
}

/// Self-deregister. Caller's profile flips to `Deregistered`. In-flight
/// assignments are honoured (a non-vote then counts as `Vote::Abstain`
/// at finalize time).
///
/// Returns `EscrowError::NotFound` if the caller isn't registered.
/// Idempotent: calling on an already-deregistered profile returns the
/// existing profile unchanged.
///
/// To re-enter the pool the caller must be re-registered by an admin
/// via `admin_register` — the curated model does not allow
/// self-resurrection.
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
/// principal hasn't been registered. Public read; no auth beyond
/// non-anonymous.
#[must_use]
pub fn get(principal: Principal) -> Option<ArbitratorProfile> {
    memory::get_arbitrator(principal)
}

/// Lists arbitrators with optional filters + pagination.
///
/// Filters compose as AND: `status` (when set) AND `min_score` (when
/// set). Arbitrators with `score = None` are excluded only when
/// `min_score` is `Some`. Results are ordered by principal
/// (`BTreeMap` iteration order).
#[must_use]
pub fn list(args: &ListArbitratorsArgs) -> Vec<ArbitratorProfile> {
    // On wasm32 (32-bit usize), `offset > u32::MAX` overflows the
    // try_from. Saturate to `usize::MAX` so an oversized offset
    // yields an empty page (paginated past the end), matching the
    // shape of `api/deals/api.rs::list_my_deals`. The previous
    // `unwrap_or(0)` silently reset to page 0 — wrong-shaped result.
    let offset = args
        .offset
        .map_or(0_usize, |o| usize::try_from(o).unwrap_or(usize::MAX));
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

    use super::{admin_register, admin_set_status, deregister, get, list};
    use crate::{
        api::{arbitrators::params::ListArbitratorsArgs, deals::errors::EscrowError},
        memory::with_arbitrator,
        types::arbitrator::ArbitratorStatus,
    };

    fn principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn admin() -> Principal {
        Principal::from_slice(&[200])
    }

    fn canister() -> Principal {
        Principal::management_canister()
    }

    // --- admin_register ---

    #[test]
    fn admin_register_creates_active_profile() {
        let p = principal(101);
        let profile = admin_register(admin(), p, canister(), 100).unwrap();
        assert_eq!(profile.principal, p);
        assert_eq!(profile.registered_by, admin());
        assert_eq!(profile.status, ArbitratorStatus::Active);
        assert_eq!(profile.registered_at_ns, 100);
        assert_eq!(profile.disputes_assigned, 0);
    }

    #[test]
    fn admin_register_is_idempotent_and_preserves_counters() {
        let p = principal(102);
        let _first = admin_register(admin(), p, canister(), 100).unwrap();

        with_arbitrator(p, |a| {
            a.disputes_assigned = 7;
            a.disputes_voted = 5;
            a.disputes_with_majority = 4;
        });

        let admin2 = principal(201);
        let second = admin_register(admin2, p, canister(), 999).unwrap();
        assert_eq!(
            second.registered_by, admin2,
            "registered_by reflects most recent admin"
        );
        assert_eq!(
            second.registered_at_ns, 100,
            "first-seen timestamp preserved on re-registration",
        );
        assert_eq!(second.disputes_assigned, 7);
        assert_eq!(second.disputes_voted, 5);
        assert_eq!(second.disputes_with_majority, 4);
    }

    #[test]
    fn admin_register_reactivates_suspended() {
        let p = principal(103);
        admin_register(admin(), p, canister(), 100).unwrap();
        admin_set_status(p, ArbitratorStatus::Suspended).unwrap();
        let reactivated = admin_register(admin(), p, canister(), 200).unwrap();
        assert_eq!(reactivated.status, ArbitratorStatus::Active);
    }

    #[test]
    fn admin_register_reactivates_deregistered() {
        let p = principal(104);
        admin_register(admin(), p, canister(), 100).unwrap();
        deregister(p).unwrap();
        assert_eq!(get(p).unwrap().status, ArbitratorStatus::Deregistered);
        let reactivated = admin_register(admin(), p, canister(), 200).unwrap();
        assert_eq!(reactivated.status, ArbitratorStatus::Active);
    }

    #[test]
    fn admin_register_rejects_anonymous_target() {
        let err = admin_register(admin(), Principal::anonymous(), canister(), 100).unwrap_err();
        assert!(matches!(err, EscrowError::AnonymousParty));
    }

    #[test]
    fn admin_register_rejects_canister_self() {
        let err = admin_register(admin(), canister(), canister(), 100).unwrap_err();
        match err {
            EscrowError::ValidationError(msg) => {
                assert!(msg.contains("canister's own principal"), "msg: {msg}");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    // --- admin_set_status ---

    #[test]
    fn admin_set_status_returns_not_found_for_unregistered() {
        let err = admin_set_status(principal(110), ArbitratorStatus::Suspended).unwrap_err();
        assert!(matches!(err, EscrowError::NotFound));
    }

    #[test]
    fn admin_set_status_supports_all_transitions() {
        let p = principal(111);
        admin_register(admin(), p, canister(), 100).unwrap();
        // Active → Suspended.
        let v = admin_set_status(p, ArbitratorStatus::Suspended).unwrap();
        assert_eq!(v.status, ArbitratorStatus::Suspended);
        // Suspended → Deregistered.
        let v = admin_set_status(p, ArbitratorStatus::Deregistered).unwrap();
        assert_eq!(v.status, ArbitratorStatus::Deregistered);
        // Deregistered → Active (reactivation).
        let v = admin_set_status(p, ArbitratorStatus::Active).unwrap();
        assert_eq!(v.status, ArbitratorStatus::Active);
        // Active → Active (self-transition no-op).
        let v = admin_set_status(p, ArbitratorStatus::Active).unwrap();
        assert_eq!(v.status, ArbitratorStatus::Active);
    }

    // --- deregister (self) ---

    #[test]
    fn deregister_returns_not_found_for_unregistered() {
        let err = deregister(principal(120)).unwrap_err();
        assert!(matches!(err, EscrowError::NotFound));
    }

    #[test]
    fn deregister_is_idempotent_when_already_deregistered() {
        let p = principal(121);
        admin_register(admin(), p, canister(), 100).unwrap();
        let first = deregister(p).unwrap();
        assert_eq!(first.status, ArbitratorStatus::Deregistered);
        let second = deregister(p).unwrap();
        assert_eq!(second.status, ArbitratorStatus::Deregistered);
    }

    // --- queries ---

    #[test]
    fn get_returns_none_for_unregistered() {
        assert!(get(principal(130)).is_none());
    }

    #[test]
    fn list_filters_by_status() {
        let active = principal(140);
        let suspended = principal(141);
        admin_register(admin(), active, canister(), 100).unwrap();
        admin_register(admin(), suspended, canister(), 100).unwrap();
        admin_set_status(suspended, ArbitratorStatus::Suspended).unwrap();

        let only_active = list(&ListArbitratorsArgs {
            status: Some(ArbitratorStatus::Active),
            ..Default::default()
        });
        assert!(only_active.iter().any(|a| a.principal == active));
        assert!(!only_active.iter().any(|a| a.principal == suspended));
    }

    #[test]
    fn list_filters_by_min_score() {
        let high = principal(150);
        let low = principal(151);
        let unscored = principal(152);
        admin_register(admin(), high, canister(), 100).unwrap();
        admin_register(admin(), low, canister(), 100).unwrap();
        admin_register(admin(), unscored, canister(), 100).unwrap();
        with_arbitrator(high, |a| a.score = Some(80));
        with_arbitrator(low, |a| a.score = Some(20));

        let above_50 = list(&ListArbitratorsArgs {
            min_score: Some(50),
            ..Default::default()
        });
        assert!(above_50.iter().any(|a| a.principal == high));
        assert!(!above_50.iter().any(|a| a.principal == low));
        assert!(!above_50.iter().any(|a| a.principal == unscored));
    }

    #[test]
    fn list_paginates() {
        for i in 200..210_u8 {
            admin_register(admin(), principal(i), canister(), 100).unwrap();
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
        for a in &page1 {
            assert!(!page2.iter().any(|b| b.principal == a.principal));
        }
    }

    #[test]
    fn list_caps_limit_at_100() {
        // Skip principal byte 4 — it collides with `Principal::anonymous()`
        // which the registration validator rejects.
        for i in 5..160_u8 {
            admin_register(admin(), principal(i), canister(), 100).unwrap();
        }
        let huge = list(&ListArbitratorsArgs {
            offset: Some(0),
            limit: Some(10_000),
            ..Default::default()
        });
        assert!(huge.len() <= 100);
    }

    #[test]
    fn list_oversized_offset_returns_empty_page() {
        // wasm32 has 32-bit usize, so any `offset > u32::MAX` overflows
        // the `usize::try_from(u64)` conversion. The previous shape
        // (`unwrap_or(0)`) silently reset to page 0 — wrong: the caller
        // asked for "skip 18 quintillion entries" and should get an
        // empty page. We saturate to `usize::MAX` so `Iterator::skip`
        // exhausts the (much shorter) backing iterator and yields zero
        // results.
        admin_register(admin(), principal(170), canister(), 100).unwrap();
        let beyond = list(&ListArbitratorsArgs {
            offset: Some(u64::MAX),
            limit: Some(50),
            ..Default::default()
        });
        assert!(
            beyond.is_empty(),
            "oversized offset must yield empty page, got {} entries",
            beyond.len(),
        );
    }
}
