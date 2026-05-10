use core::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use candid::Principal;
use ic_cdk::{storage, trap};

use crate::{
    api::deals::errors::EscrowError,
    types::{
        arbitrator::ArbitratorProfile,
        deal::{Deal, DealId, DealStatus},
        dispute::{Dispute, DisputeId},
        state::{Config, StableState},
    },
};

thread_local! {
    pub static CONFIG: RefCell<Config> = const { RefCell::new(Config { dispute_config: None }) };
    static DEALS: RefCell<BTreeMap<DealId, Deal>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_DEAL_ID: RefCell<DealId> = const { RefCell::new(1) };
    /// Transient lock preventing concurrent async processing of the same deal.
    static PROCESSING: RefCell<BTreeSet<DealId>> = const { RefCell::new(BTreeSet::new()) };
    static DISPUTES: RefCell<BTreeMap<DisputeId, Dispute>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_DISPUTE_ID: RefCell<DisputeId> = const { RefCell::new(1) };
    static ARBITRATORS: RefCell<BTreeMap<Principal, ArbitratorProfile>> = const { RefCell::new(BTreeMap::new()) };
}

// --- Deal storage ---

/// Atomically allocates a unique `DealId`, builds the deal via `build`, and
/// inserts it into the store.
///
/// This is the **only** public way to create a deal — guaranteeing that every
/// stored deal has a unique, canister-assigned ID.  The `build` closure
/// receives the freshly allocated ID so it can derive subaccounts and populate
/// the struct.  After `build` returns, the deal's `id` field is forcibly set
/// to the allocated value (belt-and-suspenders) before insertion.
pub fn insert_new_deal(build: impl FnOnce(DealId) -> Deal) -> Deal {
    let deal_id = allocate_deal_id();
    let mut deal = build(deal_id);
    deal.id = deal_id;
    DEALS.with(|d| d.borrow_mut().insert(deal_id, deal.clone()));
    deal
}

#[must_use]
pub fn get_deal(deal_id: DealId) -> Option<Deal> {
    DEALS.with(|d| d.borrow().get(&deal_id).cloned())
}

/// Runs `f` with a mutable reference to the deal, returning `Some(R)` if the deal exists.
pub fn with_deal<R>(deal_id: DealId, f: impl FnOnce(&mut Deal) -> R) -> Option<R> {
    DEALS.with(|d| d.borrow_mut().get_mut(&deal_id).map(f))
}

/// Runs `f` with a read-only reference to the full deal map.
pub fn with_deals<R>(f: impl FnOnce(&BTreeMap<DealId, Deal>) -> R) -> R {
    DEALS.with(|d| f(&d.borrow()))
}

/// Returns the total number of deals in storage.
///
/// Used by the ICRC-7 layer for `icrc7_total_supply`.
#[must_use]
pub fn deal_count() -> u64 {
    DEALS.with(|d| d.borrow().len() as u64)
}

/// Counts non-terminal deals created by `principal`.
///
/// Terminal statuses are: `Settled`, `Refunded`, `Cancelled`,
/// `Rejected`, `ArbitratedSettled`, `ArbitratedRefunded`. `Disputed`
/// is **non-terminal** — funds are still in escrow pending resolution.
#[must_use]
pub fn count_active_deals_for(principal: Principal) -> u32 {
    DEALS.with(|d| {
        u32::try_from(
            d.borrow()
                .values()
                .filter(|deal| {
                    deal.created_by == principal
                        && !matches!(
                            deal.status,
                            DealStatus::Settled
                                | DealStatus::Refunded
                                | DealStatus::Cancelled
                                | DealStatus::Rejected
                                | DealStatus::ArbitratedSettled
                                | DealStatus::ArbitratedRefunded
                        )
                })
                .count(),
        )
        .unwrap_or(u32::MAX)
    })
}

/// Returns `(positive, concluded)` deal counts for a principal's reliability.
///
/// - **positive**: deals with status `Settled` or `Refunded`.
/// - **concluded**: positive + counterparty rejections (where `updated_by != created_by`).
///
/// Deals that are `Created`, `Funded`, `Cancelled`, or self-rejected are excluded.
#[must_use]
pub fn compute_reliability_for(principal: Principal) -> (u32, u32) {
    DEALS.with(|d| {
        let (mut positive, mut concluded) = (0_u32, 0_u32);
        for deal in d.borrow().values() {
            if deal.created_by != principal {
                continue;
            }
            match deal.status {
                // Arbitrated outcomes count as positive just like their
                // unilateral counterparts — successful resolution via
                // arbitration is still a "deal that ended in a fund-
                // release decision".
                DealStatus::Settled
                | DealStatus::Refunded
                | DealStatus::ArbitratedSettled
                | DealStatus::ArbitratedRefunded => {
                    positive = positive.saturating_add(1);
                    concluded = concluded.saturating_add(1);
                }
                DealStatus::Rejected if deal.updated_by.is_some_and(|by| by != principal) => {
                    concluded = concluded.saturating_add(1);
                }
                _ => {}
            }
        }
        (positive, concluded)
    })
}

fn allocate_deal_id() -> DealId {
    NEXT_DEAL_ID.with(|id| {
        let mut id = id.borrow_mut();
        let current = *id;
        *id = current.checked_add(1).expect("DealId overflow");
        current
    })
}

// --- Dispute storage ---

/// Atomically allocates a unique `DisputeId`, builds the dispute via `build`,
/// and inserts it into the store.
///
/// The **only** public way to create a dispute — guarantees every stored
/// dispute has a canister-assigned ID. The `build` closure receives the
/// freshly allocated ID; after `build` returns, the dispute's `id` field
/// is forcibly set to the allocated value before insertion (matches the
/// `insert_new_deal` belt-and-suspenders pattern).
pub fn insert_new_dispute(build: impl FnOnce(DisputeId) -> Dispute) -> Dispute {
    let dispute_id = allocate_dispute_id();
    let mut dispute = build(dispute_id);
    dispute.id = dispute_id;
    DISPUTES.with(|d| d.borrow_mut().insert(dispute_id, dispute.clone()));
    dispute
}

#[must_use]
pub fn get_dispute(dispute_id: DisputeId) -> Option<Dispute> {
    DISPUTES.with(|d| d.borrow().get(&dispute_id).cloned())
}

/// Runs `f` with a mutable reference to the dispute, returning `Some(R)`
/// if the dispute exists.
pub fn with_dispute<R>(dispute_id: DisputeId, f: impl FnOnce(&mut Dispute) -> R) -> Option<R> {
    DISPUTES.with(|d| d.borrow_mut().get_mut(&dispute_id).map(f))
}

/// Runs `f` with a read-only reference to the full dispute map.
pub fn with_disputes<R>(f: impl FnOnce(&BTreeMap<DisputeId, Dispute>) -> R) -> R {
    DISPUTES.with(|d| f(&d.borrow()))
}

/// Runs `f` with a mutable reference to the full dispute map. Used by the
/// auto-finalize sweep to iterate without per-call clone.
pub fn with_disputes_mut<R>(f: impl FnOnce(&mut BTreeMap<DisputeId, Dispute>) -> R) -> R {
    DISPUTES.with(|d| f(&mut d.borrow_mut()))
}

#[must_use]
pub fn dispute_count() -> u64 {
    DISPUTES.with(|d| d.borrow().len() as u64)
}

fn allocate_dispute_id() -> DisputeId {
    NEXT_DISPUTE_ID.with(|id| {
        let mut id = id.borrow_mut();
        let current = *id;
        *id = current.checked_add(1).expect("DisputeId overflow");
        current
    })
}

// --- Arbitrator storage ---

/// Inserts or replaces an arbitrator profile keyed by principal.
///
/// Used by `services::arbitrators::admin_register` (idempotent
/// re-registration that reactivates Suspended/Deregistered profiles)
/// and by `services::disputes::apply_score_updates` (called from
/// `finalize` to bump per-arbitrator score counters).
pub fn upsert_arbitrator(profile: ArbitratorProfile) {
    ARBITRATORS.with(|a| a.borrow_mut().insert(profile.principal, profile));
}

#[must_use]
pub fn get_arbitrator(principal: Principal) -> Option<ArbitratorProfile> {
    ARBITRATORS.with(|a| a.borrow().get(&principal).cloned())
}

/// Runs `f` with a mutable reference to the arbitrator profile, returning
/// `Some(R)` if the arbitrator exists.
pub fn with_arbitrator<R>(
    principal: Principal,
    f: impl FnOnce(&mut ArbitratorProfile) -> R,
) -> Option<R> {
    ARBITRATORS.with(|a| a.borrow_mut().get_mut(&principal).map(f))
}

/// Runs `f` with a read-only reference to the full arbitrator map. Used by
/// `services::arbitrators::select_panel` to enumerate eligible arbitrators.
pub fn with_arbitrators<R>(f: impl FnOnce(&BTreeMap<Principal, ArbitratorProfile>) -> R) -> R {
    ARBITRATORS.with(|a| f(&a.borrow()))
}

#[must_use]
pub fn arbitrator_count() -> u64 {
    ARBITRATORS.with(|a| a.borrow().len() as u64)
}

// --- Processing lock ---

pub fn try_acquire_lock(deal_id: DealId) -> Result<(), EscrowError> {
    PROCESSING.with(|p| {
        if p.borrow().contains(&deal_id) {
            Err(EscrowError::ValidationError(
                "Deal is currently being processed".to_owned(),
            ))
        } else {
            p.borrow_mut().insert(deal_id);
            Ok(())
        }
    })
}

pub fn release_lock(deal_id: DealId) {
    PROCESSING.with(|p| {
        p.borrow_mut().remove(&deal_id);
    });
}

// --- Stable storage ---

pub fn save_state() {
    let config: Config = CONFIG.with(|c| c.borrow().clone());
    let deals = DEALS.with(|d| d.borrow().clone());
    let next_deal_id = NEXT_DEAL_ID.with(|id| *id.borrow());
    let disputes = DISPUTES.with(|d| d.borrow().clone());
    let next_dispute_id = NEXT_DISPUTE_ID.with(|id| *id.borrow());
    let arbitrators = ARBITRATORS.with(|a| a.borrow().clone());

    let state = StableState {
        config,
        deals: Some(deals),
        next_deal_id: Some(next_deal_id),
        disputes: Some(disputes),
        next_dispute_id: Some(next_dispute_id),
        arbitrators: Some(arbitrators),
    };

    storage::stable_save((state,)).expect("Save failed");
}

pub fn restore_state() {
    let result: Result<(StableState,), String> = storage::stable_restore();

    let state = match result {
        Ok((s,)) => s,
        Err(e) => {
            trap(format!("Failed to restore stable state: {e:?}"));
        }
    };

    let StableState {
        config,
        deals,
        next_deal_id,
        disputes,
        next_dispute_id,
        arbitrators,
    } = state;

    CONFIG.with(|c| *c.borrow_mut() = config);
    DEALS.with(|d| *d.borrow_mut() = deals.unwrap_or_default());
    NEXT_DEAL_ID.with(|id| *id.borrow_mut() = next_deal_id.unwrap_or(1));
    DISPUTES.with(|d| *d.borrow_mut() = disputes.unwrap_or_default());
    NEXT_DISPUTE_ID.with(|id| *id.borrow_mut() = next_dispute_id.unwrap_or(1));
    ARBITRATORS.with(|a| *a.borrow_mut() = arbitrators.unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use candid::Principal;

    use super::{get_deal, insert_new_deal, release_lock, try_acquire_lock, with_deal, with_deals};
    use crate::types::deal::{Consent, Deal, DealStatus};

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_stored_deal(status: DealStatus) -> Deal {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer: Some(test_principal(1)),
            recipient: None,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: test_principal(1),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 200,
            status,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: None,
            dispute: None,
        })
    }

    #[test]
    fn insert_and_retrieve() {
        let deal = make_stored_deal(DealStatus::Created);
        let loaded = get_deal(deal.id).expect("deal should exist");
        assert_eq!(loaded.id, deal.id);
        assert_eq!(loaded.status, DealStatus::Created);
    }

    #[test]
    fn returns_none_for_unknown_id() {
        assert!(get_deal(999_999).is_none());
    }

    #[test]
    fn ids_are_sequential() {
        let a = make_stored_deal(DealStatus::Created);
        let b = make_stored_deal(DealStatus::Created);
        assert_eq!(b.id, a.id + 1);
    }

    #[test]
    fn ids_are_globally_unique() {
        let mut seen = BTreeSet::new();
        for _ in 0..100 {
            let deal = make_stored_deal(DealStatus::Created);
            assert!(seen.insert(deal.id), "duplicate DealId: {}", deal.id);
        }
    }

    #[test]
    fn builder_cannot_forge_id() {
        let deal = insert_new_deal(|_deal_id| Deal {
            id: 999_999_999,
            payer: Some(test_principal(1)),
            recipient: None,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: test_principal(1),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 200,
            status: DealStatus::Created,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: None,
            dispute: None,
        });
        assert_ne!(deal.id, 999_999_999);
        assert!(get_deal(deal.id).is_some());
        assert!(get_deal(999_999_999).is_none());
    }

    #[test]
    fn with_deal_mutates_in_place() {
        let deal = make_stored_deal(DealStatus::Created);
        with_deal(deal.id, |d| {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(500);
        });

        let loaded = get_deal(deal.id).unwrap();
        assert_eq!(loaded.status, DealStatus::Funded);
        assert_eq!(loaded.funded_at_ns, Some(500));
    }

    #[test]
    fn with_deals_reads_all() {
        let deal = make_stored_deal(DealStatus::Created);
        let found = with_deals(|deals: &BTreeMap<_, _>| deals.values().any(|d| d.id == deal.id));
        assert!(found);
    }

    #[test]
    fn deal_count_reflects_insertions() {
        let before = super::deal_count();
        make_stored_deal(DealStatus::Created);
        make_stored_deal(DealStatus::Funded);
        assert_eq!(super::deal_count(), before + 2);
    }

    #[test]
    fn lock_prevents_double_processing() {
        let id = 9_999_999;
        assert!(try_acquire_lock(id).is_ok());
        assert!(try_acquire_lock(id).is_err());
        release_lock(id);
        assert!(try_acquire_lock(id).is_ok());
        release_lock(id);
    }

    // --- Dispute storage ---

    use super::{
        arbitrator_count, dispute_count, get_arbitrator, get_dispute, insert_new_dispute,
        upsert_arbitrator, with_arbitrator, with_arbitrators, with_dispute, with_disputes,
    };
    use crate::types::{
        arbitrator::{ArbitratorProfile, ArbitratorStatus},
        dispute::{Dispute, DisputeOutcome, DisputePhase, PanelMember, Vote},
    };

    fn make_dispute(deal_id: u64) -> Dispute {
        insert_new_dispute(|dispute_id| Dispute {
            id: dispute_id,
            deal_id,
            opened_by: test_principal(1),
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 200,
            voting_deadline_ns: 300,
            panel: vec![PanelMember {
                principal: test_principal(2),
                vote: None,
                paid_at_ns: None,
                payout_tx: None,
            }],
            evidence: vec![],
            arbitration_fee: 1_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        })
    }

    #[test]
    fn insert_and_retrieve_dispute() {
        let dispute = make_dispute(7);
        let loaded = get_dispute(dispute.id).expect("dispute should exist");
        assert_eq!(loaded.id, dispute.id);
        assert_eq!(loaded.deal_id, 7);
    }

    #[test]
    fn dispute_ids_are_sequential() {
        let a = make_dispute(1);
        let b = make_dispute(2);
        assert_eq!(b.id, a.id + 1);
    }

    #[test]
    fn dispute_count_reflects_inserts() {
        let before = dispute_count();
        make_dispute(1);
        make_dispute(2);
        assert_eq!(dispute_count(), before + 2);
    }

    #[test]
    fn with_dispute_mutates_in_place() {
        let dispute = make_dispute(99);
        let res = with_dispute(dispute.id, |d| {
            d.phase = DisputePhase::Voting;
            d.outcome = Some(DisputeOutcome::Settled {
                cc: 2,
                ic: 1,
                abstain: 0,
            });
            d.panel[0].vote = Some(Vote::ConcludedCorrectly);
            "ok"
        });
        assert_eq!(res, Some("ok"));
        let reloaded = get_dispute(dispute.id).unwrap();
        assert_eq!(reloaded.phase, DisputePhase::Voting);
        assert!(matches!(
            reloaded.outcome,
            Some(DisputeOutcome::Settled { .. })
        ));
        assert_eq!(reloaded.panel[0].vote, Some(Vote::ConcludedCorrectly));
    }

    #[test]
    fn with_dispute_returns_none_for_unknown() {
        assert!(with_dispute(9_999_999, |_| ()).is_none());
    }

    #[test]
    fn with_disputes_iterates_all() {
        make_dispute(11);
        make_dispute(12);
        let count = with_disputes(|map| map.values().filter(|d| d.deal_id >= 11).count());
        assert!(count >= 2);
    }

    // --- Arbitrator storage ---

    fn make_arbitrator(p: u8) -> ArbitratorProfile {
        let profile = ArbitratorProfile {
            principal: test_principal(p),
            registered_at_ns: 100,
            registered_by: test_principal(200),
            disputes_assigned: 0,
            disputes_voted: 0,
            disputes_with_majority: 0,
            score: None,
            status: ArbitratorStatus::Active,
        };
        upsert_arbitrator(profile.clone());
        profile
    }

    #[test]
    fn upsert_and_retrieve_arbitrator() {
        let profile = make_arbitrator(50);
        let loaded = get_arbitrator(profile.principal).expect("present");
        assert_eq!(loaded.principal, profile.principal);
        assert_eq!(loaded.status, ArbitratorStatus::Active);
    }

    #[test]
    fn upsert_replaces_existing() {
        let profile = make_arbitrator(51);
        let mut updated = profile.clone();
        updated.disputes_assigned = 5;
        updated.status = ArbitratorStatus::Suspended;
        upsert_arbitrator(updated);
        let loaded = get_arbitrator(profile.principal).unwrap();
        assert_eq!(loaded.disputes_assigned, 5);
        assert_eq!(loaded.status, ArbitratorStatus::Suspended);
    }

    #[test]
    fn with_arbitrator_mutates_in_place() {
        let profile = make_arbitrator(52);
        let res = with_arbitrator(profile.principal, |a| {
            a.disputes_assigned += 1;
            a.disputes_voted += 1;
            a.disputes_with_majority += 1;
        });
        assert!(res.is_some());
        let loaded = get_arbitrator(profile.principal).unwrap();
        assert_eq!(loaded.disputes_assigned, 1);
        assert_eq!(loaded.disputes_voted, 1);
        assert_eq!(loaded.disputes_with_majority, 1);
    }

    #[test]
    fn arbitrator_count_reflects_inserts() {
        let before = arbitrator_count();
        make_arbitrator(60);
        make_arbitrator(61);
        assert_eq!(arbitrator_count(), before + 2);
    }

    #[test]
    fn with_arbitrators_filters_active() {
        make_arbitrator(70);
        make_arbitrator(71);
        with_arbitrator(test_principal(71), |a| {
            a.status = ArbitratorStatus::Suspended;
        });
        let active = with_arbitrators(|map| {
            map.values()
                .filter(|a| {
                    matches!(a.status, ArbitratorStatus::Active)
                        && (a.principal == test_principal(70) || a.principal == test_principal(71))
                })
                .count()
        });
        assert_eq!(active, 1);
    }
}
