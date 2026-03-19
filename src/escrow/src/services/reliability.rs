use candid::Principal;

use crate::{api::deals::errors::EscrowError, memory};

const MIN_CONCLUDED_DEALS: u32 = 5;
const MIN_RELIABILITY_PERCENT: u32 = 25;

/// Reliability score for a principal, computed from concluded deal outcomes.
pub struct ReliabilityScore {
    /// Percentage 0–100 (100 for new users with fewer than `MIN_CONCLUDED_DEALS`).
    pub score: u32,
    /// Deals that ended positively (Settled or Refunded).
    pub positive: u32,
    /// Total concluded deals (positive + counterparty rejections).
    pub concluded: u32,
}

/// Computes the reliability score for a principal.
///
/// Scans all deals where `created_by == principal` and counts:
/// - **positive**: `Settled` or `Refunded`
/// - **concluded**: positive + rejections performed by the counterparty
///
/// Users with fewer than [`MIN_CONCLUDED_DEALS`] concluded deals get 100%.
#[must_use]
pub fn compute(principal: Principal) -> ReliabilityScore {
    let (positive, concluded) = memory::compute_reliability_for(principal);
    let score = if concluded < MIN_CONCLUDED_DEALS {
        100
    } else {
        positive * 100 / concluded
    };
    ReliabilityScore {
        score,
        positive,
        concluded,
    }
}

/// Blocks deal creation when the caller's reliability drops below the threshold.
pub fn validate(caller: Principal) -> Result<(), EscrowError> {
    let reliability = compute(caller);
    if reliability.concluded >= MIN_CONCLUDED_DEALS && reliability.score < MIN_RELIABILITY_PERCENT {
        return Err(EscrowError::ReliabilityTooLow {
            score: reliability.score,
            threshold: MIN_RELIABILITY_PERCENT,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{compute, validate, MIN_CONCLUDED_DEALS, MIN_RELIABILITY_PERCENT};
    use crate::{
        api::deals::errors::EscrowError,
        memory::insert_new_deal,
        subaccounts::derive_deal_subaccount,
        types::deal::{Consent, Deal, DealStatus},
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn store_concluded_deal(creator: Principal, status: DealStatus, updated_by: Principal) {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer: Some(creator),
            recipient: Some(test_principal(250)),
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: creator,
            updated_at_ns: Some(200),
            updated_by: Some(updated_by),
            expires_at_ns: 300,
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
        });
    }

    #[test]
    fn new_user_scores_100() {
        let creator = test_principal(210);
        let r = compute(creator);
        assert_eq!(r.score, 100);
        assert_eq!(r.concluded, 0);
    }

    #[test]
    fn under_min_concluded_scores_100() {
        let creator = test_principal(211);
        let other = test_principal(212);
        for _ in 0..2 {
            store_concluded_deal(creator, DealStatus::Settled, other);
        }
        for _ in 0..2 {
            store_concluded_deal(creator, DealStatus::Rejected, other);
        }
        let r = compute(creator);
        assert_eq!(r.score, 100);
        assert_eq!(r.concluded, 4);
    }

    #[test]
    fn above_threshold_passes() {
        let creator = test_principal(213);
        let other = test_principal(214);
        for _ in 0..4 {
            store_concluded_deal(creator, DealStatus::Settled, other);
        }
        store_concluded_deal(creator, DealStatus::Rejected, other);
        let r = compute(creator);
        assert_eq!(r.score, 80);
        assert!(validate(creator).is_ok());
    }

    #[test]
    fn below_threshold_blocks() {
        let creator = test_principal(215);
        let other = test_principal(216);
        store_concluded_deal(creator, DealStatus::Settled, other);
        for _ in 0..4 {
            store_concluded_deal(creator, DealStatus::Rejected, other);
        }
        let r = compute(creator);
        assert_eq!(r.score, 20);
        assert!(matches!(
            validate(creator),
            Err(EscrowError::ReliabilityTooLow { score, threshold })
                if score == 20 && threshold == MIN_RELIABILITY_PERCENT
        ));
    }

    #[test]
    fn self_rejections_ignored() {
        let creator = test_principal(217);
        let other = test_principal(218);
        store_concluded_deal(creator, DealStatus::Settled, other);
        for _ in 0..10 {
            store_concluded_deal(creator, DealStatus::Rejected, creator);
        }
        let r = compute(creator);
        assert_eq!(r.concluded, 1);
        assert_eq!(r.score, 100);
    }

    #[test]
    fn in_progress_deals_ignored() {
        let creator = test_principal(219);
        let other = test_principal(220);
        store_concluded_deal(creator, DealStatus::Settled, other);
        for _ in 0..4 {
            store_concluded_deal(creator, DealStatus::Rejected, other);
        }
        for status in [
            DealStatus::Created,
            DealStatus::Funded,
            DealStatus::Cancelled,
        ] {
            for _ in 0..33 {
                store_concluded_deal(creator, status.clone(), other);
            }
        }
        let r = compute(creator);
        assert_eq!(r.score, 20);
        assert_eq!(r.concluded, 5);
    }

    #[test]
    fn refunded_counts_as_positive() {
        let creator = test_principal(221);
        let other = test_principal(222);
        for _ in 0..3 {
            store_concluded_deal(creator, DealStatus::Refunded, other);
        }
        for _ in 0..2 {
            store_concluded_deal(creator, DealStatus::Rejected, other);
        }
        let r = compute(creator);
        assert_eq!(r.score, 60);
        assert_eq!(r.positive, 3);
        assert_eq!(r.concluded, 5);
    }

    #[test]
    fn validate_allows_under_min_concluded() {
        let creator = test_principal(223);
        let other = test_principal(224);
        for _ in 0..(MIN_CONCLUDED_DEALS - 1) {
            store_concluded_deal(creator, DealStatus::Rejected, other);
        }
        assert!(validate(creator).is_ok());
    }
}
