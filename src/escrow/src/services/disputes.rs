//! Dispute service (RFC-001 step 4).
//!
//! Owns the dispute lifecycle: opening, panel selection, queries.
//! `submit_evidence` / `cast_vote` / `finalize_dispute` /
//! `withdraw_dispute` land in subsequent steps.

use core::cmp::Reverse;

use candid::Principal;

use crate::{
    api::{
        deals::errors::EscrowError,
        disputes::{
            params::ListMyDisputesArgs,
            results::{DisputeView, PublicDisputeView},
        },
    },
    ledger,
    memory::{
        get_deal, get_dispute as load_dispute, insert_new_dispute, release_lock, try_acquire_lock,
        with_arbitrator, with_arbitrators, with_deal, with_disputes, CONFIG,
    },
    types::{
        arbitrator::ArbitratorStatus,
        deal::{Deal, DealId, DealStatus},
        dispute::{Dispute, DisputeConfig, DisputeId, DisputePhase, PanelMember},
    },
    validation,
};

/// Computes the arbitration fee from `amount` per the Q10 formula:
/// `max(MIN_FEE, amount * FEE_BPS / 10_000)`.
///
/// Saturating arithmetic — overflow on huge amounts is clamped to
/// `u128::MAX` rather than panicking.
#[must_use]
pub fn compute_arbitration_fee(amount: u128, cfg: &DisputeConfig) -> u128 {
    let bps_fee = amount.saturating_mul(u128::from(cfg.arbitration_fee_bps)) / 10_000;
    bps_fee.max(cfg.arbitration_min_fee)
}

/// Returns the eligible arbitrator pool for a dispute on `deal`, with
/// per-arbitrator selection weights (Q5):
/// - Active status only.
/// - Excludes `payer` and `recipient` of the disputed deal.
/// - Excludes arbitrators below `min_arbitrator_score` when set.
/// - Weight = `score.unwrap_or(0).max(1)` so unscored arbitrators get a non-zero base weight (= 1)
///   per the Q5 decision.
#[must_use]
pub fn eligible_arbitrators(deal: &Deal, cfg: &DisputeConfig) -> Vec<(Principal, u32)> {
    with_arbitrators(|map| {
        map.values()
            .filter(|a| matches!(a.status, ArbitratorStatus::Active))
            .filter(|a| Some(a.principal) != deal.payer && Some(a.principal) != deal.recipient)
            .filter(|a| match cfg.min_arbitrator_score {
                Some(min) => a.score.is_some_and(|s| s >= min),
                None => true,
            })
            .map(|a| {
                let weight = a.score.unwrap_or(0).max(1);
                (a.principal, weight)
            })
            .collect()
    })
}

/// Pure weighted-random-without-replacement selector.
///
/// Takes a precomputed `eligible` list of `(principal, weight)` pairs,
/// the desired `panel_size`, and a slice of random bytes from
/// `ledger::raw_rand`. Returns the selected panel in selection order.
///
/// The function is deterministic given the same `randomness` slice — that
/// makes the selection auditable and easily unit-testable. Each draw
/// consumes 8 bytes (`u64`) from `randomness`; if the slice is too short
/// the function falls back to deterministic chunk-rotation, which is
/// fine for v1 (we always pass 32 `raw_rand` bytes for `panel_size = 3`).
///
/// Returns fewer than `panel_size` principals only when the eligible
/// pool is smaller than `panel_size` — callers (e.g. `open_dispute`)
/// gate this case via `EscrowError::InsufficientArbitrators` *before*
/// calling the selector.
#[must_use]
pub fn select_panel(
    mut eligible: Vec<(Principal, u32)>,
    panel_size: u32,
    randomness: &[u8],
) -> Vec<Principal> {
    let panel_size = panel_size as usize;
    let mut selected = Vec::with_capacity(panel_size);
    let mut cursor = 0_usize;

    while selected.len() < panel_size && !eligible.is_empty() {
        let total_weight: u128 = eligible.iter().map(|(_, w)| u128::from(*w)).sum();
        if total_weight == 0 {
            break;
        }

        let pick = u128::from(next_u64(randomness, cursor)) % total_weight;
        cursor = cursor.wrapping_add(8);

        let mut acc: u128 = 0;
        let mut chosen_idx: usize = eligible.len() - 1;
        for (idx, (_, w)) in eligible.iter().enumerate() {
            acc += u128::from(*w);
            if pick < acc {
                chosen_idx = idx;
                break;
            }
        }

        let (principal, _) = eligible.swap_remove(chosen_idx);
        selected.push(principal);
    }

    selected
}

fn next_u64(bytes: &[u8], cursor: usize) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut buf = [0_u8; 8];
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = bytes[(cursor + i) % bytes.len()];
    }
    u64::from_le_bytes(buf)
}

/// Reads `Config::dispute_config` with a fallback to `DisputeConfig::default()`
/// when admin hasn't set one (legacy snapshots, fresh deployments).
#[must_use]
pub fn load_dispute_config() -> DisputeConfig {
    CONFIG.with(|c| c.borrow().dispute_config.clone().unwrap_or_default())
}

/// Opens a new dispute on `deal_id`. RFC-001 step 4.
///
/// On success: creates a `Dispute` with phase `Evidence`, transitions
/// the deal `Funded → Disputed`, sets `Deal.dispute = Some(dispute_id)`,
/// and increments `disputes_assigned` for each panel arbitrator.
///
/// Idempotent: calling `open` on a deal that's already `Disputed`
/// returns the existing `DisputeView` (the Q9 deadlines are NOT reset
/// — the original timeline is preserved).
pub async fn open(
    caller: Principal,
    deal_id: DealId,
    now_ns: u64,
) -> Result<DisputeView, EscrowError> {
    let deal = get_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_open_dispute(&deal, caller, now_ns)?;
    if already_done {
        // Idempotent: deal is already Disputed; load and return the existing dispute.
        let existing_id = deal.dispute.ok_or(EscrowError::DisputeNotFound)?;
        let dispute = load_dispute(existing_id).ok_or(EscrowError::DisputeNotFound)?;
        return Ok(DisputeView::from(&dispute));
    }

    let cfg = load_dispute_config();

    // Fee math: validator already checks Funded + bound; here we ensure the
    // amount can cover at least the configured fee with at least 1 unit
    // remaining for the prevailing party. Per-arbitrator ledger fees are
    // absorbed at finalize (Q10 refinement #1) — the validator can't know
    // the exact ICRC-1 fee without an inter-canister call, so we keep the
    // amount-too-small check conservative here.
    let fee = compute_arbitration_fee(deal.amount, &cfg);
    if deal.amount <= fee {
        return Err(EscrowError::AmountTooSmallForArbitration {
            min: fee.saturating_add(1),
        });
    }

    // Eligible-pool gate (sync, before async raw_rand to fail fast).
    let eligible = eligible_arbitrators(&deal, &cfg);
    let needed = cfg.panel_size;
    let have = u32::try_from(eligible.len()).unwrap_or(u32::MAX);
    if have < needed {
        return Err(EscrowError::InsufficientArbitrators { need: needed, have });
    }

    try_acquire_lock(deal_id)?;
    let result = open_locked(deal, caller, now_ns, cfg, eligible, needed).await;
    release_lock(deal_id);
    result
}

async fn open_locked(
    deal: Deal,
    caller: Principal,
    now_ns: u64,
    cfg: DisputeConfig,
    eligible: Vec<(Principal, u32)>,
    panel_size: u32,
) -> Result<DisputeView, EscrowError> {
    let (random_bytes,): (Vec<u8>,) = ledger::raw_rand().await?;
    let panel_principals = select_panel(eligible, panel_size, &random_bytes);

    if panel_principals.len() < panel_size as usize {
        // Should be unreachable — we checked the pool size before raw_rand —
        // but guard the contract explicitly rather than emit a malformed
        // dispute record.
        return Err(EscrowError::InsufficientArbitrators {
            need: panel_size,
            have: u32::try_from(panel_principals.len()).unwrap_or(u32::MAX),
        });
    }

    let panel: Vec<PanelMember> = panel_principals
        .iter()
        .map(|p| PanelMember {
            principal: *p,
            vote: None,
            paid_at_ns: None,
            payout_tx: None,
        })
        .collect();

    let arbitration_fee = compute_arbitration_fee(deal.amount, &cfg);
    let evidence_deadline_ns = now_ns.saturating_add(cfg.evidence_window_ns);
    let voting_deadline_ns = evidence_deadline_ns.saturating_add(cfg.voting_window_ns);

    let dispute = insert_new_dispute(|dispute_id| Dispute {
        id: dispute_id,
        deal_id: deal.id,
        opened_by: caller,
        opened_at_ns: now_ns,
        phase: DisputePhase::Evidence,
        evidence_deadline_ns,
        voting_deadline_ns,
        panel: panel.clone(),
        evidence: vec![],
        arbitration_fee,
        outcome: None,
        payer_withdraw_proposal: None,
        recipient_withdraw_proposal: None,
    });

    // Wire the dispute back to the deal + transition to Disputed.
    with_deal(deal.id, |d| {
        d.status = DealStatus::Disputed;
        d.dispute = Some(dispute.id);
        d.updated_at_ns = Some(now_ns);
        d.updated_by = Some(caller);
    });

    // Bump `disputes_assigned` for each panel member (Q11 — assigned counter
    // tracks all panel selections, including future NoQuorum / Withdrawn).
    for member in &panel {
        with_arbitrator(member.principal, |a| {
            a.disputes_assigned = a.disputes_assigned.saturating_add(1);
        });
    }

    Ok(DisputeView::from(&dispute))
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Returns the full dispute view. Caller must be a party of the parent
/// deal or an arbitrator on the panel.
pub fn get(caller: Principal, dispute_id: DisputeId) -> Result<DisputeView, EscrowError> {
    let dispute = load_dispute(dispute_id).ok_or(EscrowError::DisputeNotFound)?;
    let deal = get_deal(dispute.deal_id).ok_or(EscrowError::NotFound)?;
    authorize_dispute_view(&dispute, &deal, caller)?;
    Ok(DisputeView::from(&dispute))
}

/// Returns the reduced public dispute view. Any non-anonymous caller
/// may query — there is no participant authorization on this endpoint
/// (it deliberately omits party / panel principals + evidence URLs).
pub fn get_public(dispute_id: DisputeId) -> Result<PublicDisputeView, EscrowError> {
    let dispute = load_dispute(dispute_id).ok_or(EscrowError::DisputeNotFound)?;
    Ok(PublicDisputeView::from(&dispute))
}

/// Lists disputes the caller is involved with (party of the parent deal
/// or arbitrator on the panel), reverse-chronological by `opened_at_ns`.
#[must_use]
pub fn list_for_caller(caller: Principal, args: &ListMyDisputesArgs) -> Vec<DisputeView> {
    let offset = args
        .offset
        .and_then(|o| usize::try_from(o).ok())
        .unwrap_or(0);
    let limit = args
        .limit
        .map_or(50_usize, |l| usize::try_from(l.min(100)).unwrap_or(100));

    with_disputes(|disputes| {
        let mut matched: Vec<DisputeView> = disputes
            .values()
            .filter(|d| match &args.phase {
                Some(p) => &d.phase == p,
                None => true,
            })
            .filter(|d| {
                let deal = get_deal(d.deal_id);
                let on_panel = d.panel.iter().any(|m| m.principal == caller);
                let party = deal.is_some_and(|deal| {
                    deal.payer == Some(caller) || deal.recipient == Some(caller)
                });
                on_panel || party
            })
            .map(DisputeView::from)
            .collect();
        matched.sort_by_key(|d| Reverse(d.opened_at_ns));
        matched.into_iter().skip(offset).take(limit).collect()
    })
}

fn authorize_dispute_view(
    dispute: &Dispute,
    deal: &Deal,
    caller: Principal,
) -> Result<(), EscrowError> {
    let on_panel = dispute.panel.iter().any(|m| m.principal == caller);
    let party = deal.payer == Some(caller) || deal.recipient == Some(caller);
    if on_panel || party {
        Ok(())
    } else {
        Err(EscrowError::NotAuthorised)
    }
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{
        compute_arbitration_fee, eligible_arbitrators, get, get_public, list_for_caller,
        load_dispute_config, select_panel,
    };
    use crate::{
        api::{deals::errors::EscrowError, disputes::params::ListMyDisputesArgs},
        memory::{
            get_arbitrator, insert_new_deal, insert_new_dispute, upsert_arbitrator, with_arbitrator,
        },
        subaccounts::derive_deal_subaccount,
        types::{
            arbitrator::{ArbitratorProfile, ArbitratorStatus},
            deal::{Consent, Deal, DealStatus},
            dispute::{Dispute, DisputeConfig, DisputePhase, PanelMember},
        },
    };

    fn principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_arbitrator(id: u8, score: Option<u32>, status: ArbitratorStatus) -> Principal {
        let p = principal(id);
        upsert_arbitrator(ArbitratorProfile {
            principal: p,
            registered_at_ns: 100,
            bio: None,
            disputes_assigned: 0,
            disputes_voted: 0,
            disputes_with_majority: 0,
            score,
            status,
        });
        p
    }

    fn make_deal(payer: Principal, recipient: Principal, status: DealStatus) -> Deal {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer: Some(payer),
            recipient: Some(recipient),
            token_ledger: principal(99),
            token_symbol: None,
            amount: 1_000_000,
            created_at_ns: 100,
            created_by: payer,
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 10_000,
            status,
            escrow_subaccount: derive_deal_subaccount(deal_id),
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Accepted,
            metadata: None,
            dispute: None,
        })
    }

    // --- compute_arbitration_fee ---

    #[test]
    fn fee_uses_bps_when_above_min() {
        let cfg = DisputeConfig {
            arbitration_fee_bps: 500,
            arbitration_min_fee: 0,
            ..DisputeConfig::default()
        };
        assert_eq!(compute_arbitration_fee(1_000_000, &cfg), 50_000);
    }

    #[test]
    fn fee_uses_min_when_bps_falls_below() {
        let cfg = DisputeConfig {
            arbitration_fee_bps: 100,
            arbitration_min_fee: 10_000,
            ..DisputeConfig::default()
        };
        // 1_000 * 100 / 10_000 = 10. min_fee floor wins.
        assert_eq!(compute_arbitration_fee(1_000, &cfg), 10_000);
    }

    #[test]
    fn fee_saturates_on_huge_amounts() {
        let cfg = DisputeConfig {
            arbitration_fee_bps: 10_000,
            arbitration_min_fee: 0,
            ..DisputeConfig::default()
        };
        let fee = compute_arbitration_fee(u128::MAX, &cfg);
        // FEE_BPS=10000 => effectively 100% of amount, so huge inputs cap at u128::MAX/10000*10000
        // The point is that we don't panic on overflow.
        assert!(fee > 0);
    }

    // --- select_panel (pure) ---

    #[test]
    fn select_returns_empty_when_eligible_empty() {
        let panel = select_panel(vec![], 3, &[1, 2, 3, 4]);
        assert!(panel.is_empty());
    }

    #[test]
    fn select_returns_all_when_pool_smaller_than_panel() {
        let eligible = vec![(principal(1), 1), (principal(2), 1)];
        let panel = select_panel(eligible, 3, &[0_u8; 32]);
        assert_eq!(panel.len(), 2, "selector returns what's available");
    }

    #[test]
    fn select_returns_panel_size_when_pool_large_enough() {
        let eligible: Vec<_> = (1_u8..=10_u8).map(|i| (principal(i), 1)).collect();
        let panel = select_panel(
            eligible,
            3,
            &[7, 13, 21, 42, 99, 100, 250, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        );
        assert_eq!(panel.len(), 3);
    }

    #[test]
    fn select_no_duplicates() {
        let eligible: Vec<_> = (1_u8..=10_u8).map(|i| (principal(i), 5)).collect();
        let panel = select_panel(eligible, 5, &[1; 64]);
        let mut sorted = panel.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), panel.len(), "no duplicates: {panel:?}");
    }

    #[test]
    fn select_is_deterministic_for_same_randomness() {
        let pool: Vec<_> = (1_u8..=10_u8).map(|i| (principal(i), 1)).collect();
        let bytes = [42_u8; 32];
        let a = select_panel(pool.clone(), 3, &bytes);
        let b = select_panel(pool, 3, &bytes);
        assert_eq!(a, b);
    }

    #[test]
    fn select_weighted_favours_higher_weights() {
        // Run many iterations with different seeds; the high-weight arbitrator
        // should be selected more often than the low-weight one.
        let high = principal(1);
        let low = principal(2);
        let mut high_count = 0;
        let mut low_count = 0;
        for seed in 0_u8..=200_u8 {
            let bytes = [seed; 16];
            let panel = select_panel(vec![(high, 100), (low, 1)], 1, &bytes);
            if panel == [high] {
                high_count += 1;
            } else if panel == [low] {
                low_count += 1;
            }
        }
        assert!(
            high_count > low_count * 5,
            "high={high_count}, low={low_count}"
        );
    }

    // --- eligible_arbitrators ---

    #[test]
    fn eligible_excludes_payer_and_recipient() {
        let payer = principal(80);
        let recipient = principal(81);
        // Register the parties as arbitrators (something a malicious actor would try).
        make_arbitrator(80, None, ArbitratorStatus::Active);
        make_arbitrator(81, None, ArbitratorStatus::Active);
        // Plus a few unrelated arbitrators.
        for i in 82_u8..=85_u8 {
            make_arbitrator(i, None, ArbitratorStatus::Active);
        }
        let deal = make_deal(payer, recipient, DealStatus::Funded);
        let cfg = DisputeConfig::default();
        let eligible = eligible_arbitrators(&deal, &cfg);
        assert!(!eligible.iter().any(|(p, _)| *p == payer));
        assert!(!eligible.iter().any(|(p, _)| *p == recipient));
        assert!(eligible.len() >= 4);
    }

    #[test]
    fn eligible_excludes_inactive_arbitrators() {
        let payer = principal(90);
        let recipient = principal(91);
        let active = make_arbitrator(92, None, ArbitratorStatus::Active);
        let suspended = make_arbitrator(93, None, ArbitratorStatus::Suspended);
        let deregistered = make_arbitrator(94, None, ArbitratorStatus::Deregistered);
        let deal = make_deal(payer, recipient, DealStatus::Funded);
        let cfg = DisputeConfig::default();
        let eligible = eligible_arbitrators(&deal, &cfg);
        assert!(eligible.iter().any(|(p, _)| *p == active));
        assert!(!eligible.iter().any(|(p, _)| *p == suspended));
        assert!(!eligible.iter().any(|(p, _)| *p == deregistered));
    }

    #[test]
    fn eligible_filters_by_min_score() {
        let payer = principal(100);
        let recipient = principal(101);
        let high = make_arbitrator(102, Some(80), ArbitratorStatus::Active);
        let low = make_arbitrator(103, Some(20), ArbitratorStatus::Active);
        let unscored = make_arbitrator(104, None, ArbitratorStatus::Active);
        let deal = make_deal(payer, recipient, DealStatus::Funded);
        let cfg = DisputeConfig {
            min_arbitrator_score: Some(50),
            ..DisputeConfig::default()
        };
        let eligible = eligible_arbitrators(&deal, &cfg);
        assert!(eligible.iter().any(|(p, _)| *p == high));
        assert!(!eligible.iter().any(|(p, _)| *p == low));
        assert!(
            !eligible.iter().any(|(p, _)| *p == unscored),
            "unscored excluded when min_score is Some",
        );
    }

    #[test]
    fn eligible_includes_unscored_when_no_min_score() {
        let payer = principal(110);
        let recipient = principal(111);
        let unscored = make_arbitrator(112, None, ArbitratorStatus::Active);
        let deal = make_deal(payer, recipient, DealStatus::Funded);
        let cfg = DisputeConfig {
            min_arbitrator_score: None,
            ..DisputeConfig::default()
        };
        let eligible = eligible_arbitrators(&deal, &cfg);
        let entry = eligible.iter().find(|(p, _)| *p == unscored);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().1, 1, "unscored gets base weight = 1");
    }

    // --- queries (sync) ---

    #[test]
    fn get_returns_dispute_for_party() {
        let payer = principal(120);
        let recipient = principal(121);
        let deal = make_deal(payer, recipient, DealStatus::Disputed);
        let dispute = insert_new_dispute(|id| Dispute {
            id,
            deal_id: deal.id,
            opened_by: payer,
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 1000,
            voting_deadline_ns: 2000,
            panel: vec![PanelMember {
                principal: principal(150),
                vote: None,
                paid_at_ns: None,
                payout_tx: None,
            }],
            evidence: vec![],
            arbitration_fee: 50_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        });
        let view = get(payer, dispute.id).unwrap();
        assert_eq!(view.id, dispute.id);
    }

    #[test]
    fn get_returns_dispute_for_panel_member() {
        let payer = principal(130);
        let recipient = principal(131);
        let arbitrator = principal(150);
        let deal = make_deal(payer, recipient, DealStatus::Disputed);
        let dispute = insert_new_dispute(|id| Dispute {
            id,
            deal_id: deal.id,
            opened_by: payer,
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 1000,
            voting_deadline_ns: 2000,
            panel: vec![PanelMember {
                principal: arbitrator,
                vote: None,
                paid_at_ns: None,
                payout_tx: None,
            }],
            evidence: vec![],
            arbitration_fee: 50_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        });
        assert!(get(arbitrator, dispute.id).is_ok());
    }

    #[test]
    fn get_rejects_unrelated_caller() {
        let payer = principal(140);
        let recipient = principal(141);
        let stranger = principal(199);
        let deal = make_deal(payer, recipient, DealStatus::Disputed);
        let dispute = insert_new_dispute(|id| Dispute {
            id,
            deal_id: deal.id,
            opened_by: payer,
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 1000,
            voting_deadline_ns: 2000,
            panel: vec![],
            evidence: vec![],
            arbitration_fee: 50_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        });
        let err = get(stranger, dispute.id).unwrap_err();
        assert_eq!(err, EscrowError::NotAuthorised);
    }

    #[test]
    fn get_returns_dispute_not_found() {
        let err = get(principal(1), 999_999).unwrap_err();
        assert_eq!(err, EscrowError::DisputeNotFound);
    }

    #[test]
    fn get_public_works_without_authorization() {
        let payer = principal(160);
        let recipient = principal(161);
        let deal = make_deal(payer, recipient, DealStatus::Disputed);
        let dispute = insert_new_dispute(|id| Dispute {
            id,
            deal_id: deal.id,
            opened_by: payer,
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 1000,
            voting_deadline_ns: 2000,
            panel: vec![],
            evidence: vec![],
            arbitration_fee: 50_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        });
        let stranger = principal(200);
        let view = get_public(dispute.id).unwrap();
        assert_eq!(view.id, dispute.id);
        // Caller is irrelevant for public — no auth check.
        let _ = stranger;
        // Tally is None pre-resolution.
        assert!(view.tally.is_none());
    }

    #[test]
    fn list_for_caller_filters_by_party() {
        let payer = principal(170);
        let recipient = principal(171);
        let other = principal(172);
        let deal = make_deal(payer, recipient, DealStatus::Disputed);
        let _ = insert_new_dispute(|id| Dispute {
            id,
            deal_id: deal.id,
            opened_by: payer,
            opened_at_ns: 100,
            phase: DisputePhase::Evidence,
            evidence_deadline_ns: 1000,
            voting_deadline_ns: 2000,
            panel: vec![],
            evidence: vec![],
            arbitration_fee: 50_000,
            outcome: None,
            payer_withdraw_proposal: None,
            recipient_withdraw_proposal: None,
        });
        let mine = list_for_caller(payer, &ListMyDisputesArgs::default());
        assert!(mine.iter().any(|d| d.deal_id == deal.id));
        let theirs = list_for_caller(other, &ListMyDisputesArgs::default());
        assert!(!theirs.iter().any(|d| d.deal_id == deal.id));
    }

    // --- load_dispute_config ---

    #[test]
    fn load_dispute_config_falls_back_to_default() {
        // CONFIG starts with dispute_config = None per memory.rs init.
        let cfg = load_dispute_config();
        assert_eq!(cfg.panel_size, DisputeConfig::default().panel_size);
    }

    // --- assigned counter sanity (single-arbitrator case) ---

    #[test]
    fn arbitrator_assigned_counter_helper() {
        // We don't test `open()` directly here (async + raw_rand not available
        // in unit tests). Instead, confirm the helper math used by `open_locked`
        // when bumping `disputes_assigned` after panel commit.
        let arb = make_arbitrator(180, None, ArbitratorStatus::Active);
        with_arbitrator(arb, |a| {
            a.disputes_assigned = a.disputes_assigned.saturating_add(1);
        });
        let loaded = get_arbitrator(arb).unwrap();
        assert_eq!(loaded.disputes_assigned, 1);
    }
}
