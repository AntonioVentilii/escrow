use core::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use ic_cdk::{storage, trap};

use crate::{
    api::deals::errors::EscrowError,
    types::{
        deal::{Deal, DealId},
        state::{Config, StableState},
    },
};

thread_local! {
    pub static CONFIG: RefCell<Config> = const { RefCell::new(Config { }) };
    static DEALS: RefCell<BTreeMap<DealId, Deal>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_DEAL_ID: RefCell<DealId> = const { RefCell::new(1) };
    /// Transient lock preventing concurrent async processing of the same deal.
    static PROCESSING: RefCell<BTreeSet<DealId>> = const { RefCell::new(BTreeSet::new()) };
}

// --- Deal storage helpers ---

pub fn insert_deal(deal: Deal) {
    DEALS.with(|d| d.borrow_mut().insert(deal.id, deal));
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

#[must_use]
pub fn next_deal_id() -> DealId {
    NEXT_DEAL_ID.with(|id| {
        let mut id = id.borrow_mut();
        let current = *id;
        *id += 1;
        current
    })
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

    let state = StableState {
        config,
        deals: Some(deals),
        next_deal_id: Some(next_deal_id),
    };

    storage::stable_save((state,)).expect("Save failed");
}

pub fn restore_state() {
    let result: Result<(StableState,), String> = storage::stable_restore();

    let state = match result {
        Ok((s,)) => s,
        Err(e) => {
            trap(&format!("Failed to restore stable state: {e:?}"));
        }
    };

    let StableState {
        config,
        deals,
        next_deal_id,
    } = state;

    CONFIG.with(|c| *c.borrow_mut() = config);
    DEALS.with(|d| *d.borrow_mut() = deals.unwrap_or_default());
    NEXT_DEAL_ID.with(|id| *id.borrow_mut() = next_deal_id.unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use crate::types::deal::DealStatus;

    use super::*;

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_test_deal(status: DealStatus) -> Deal {
        let deal_id = next_deal_id();
        Deal {
            id: deal_id,
            payer: test_principal(1),
            recipient: None,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            expires_at_ns: 200,
            status,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            completed_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            metadata: None,
        }
    }

    #[test]
    fn insert_and_retrieve() {
        let deal = make_test_deal(DealStatus::Created);
        let id = deal.id;
        insert_deal(deal);

        let loaded = get_deal(id).expect("deal should exist");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.status, DealStatus::Created);
    }

    #[test]
    fn returns_none_for_unknown_id() {
        assert!(get_deal(999_999).is_none());
    }

    #[test]
    fn deal_id_increments() {
        let a = next_deal_id();
        let b = next_deal_id();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn with_deal_mutates_in_place() {
        let deal = make_test_deal(DealStatus::Created);
        let id = deal.id;
        insert_deal(deal);

        with_deal(id, |d| {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(500);
        });

        let loaded = get_deal(id).unwrap();
        assert_eq!(loaded.status, DealStatus::Funded);
        assert_eq!(loaded.funded_at_ns, Some(500));
    }

    #[test]
    fn with_deals_reads_all() {
        let deal = make_test_deal(DealStatus::Created);
        let id = deal.id;
        insert_deal(deal);

        let found = with_deals(|deals: &BTreeMap<_, _>| deals.values().any(|d| d.id == id));
        assert!(found);
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
}
