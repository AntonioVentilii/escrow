use candid::Principal;
use ic_cdk::{api::time, id};

use crate::{
    api::deals::{
        errors::EscrowError,
        params::CreateDealArgs,
        results::{ClaimableDealView, DealView},
    },
    ledger,
    memory::{
        get_deal as load_deal, insert_new_deal, release_lock, try_acquire_lock, with_deal,
        with_deals,
    },
    subaccounts::derive_deal_subaccount,
    types::{
        deal::{Deal, DealId, DealMetadata, DealStatus},
        ledger_types::Account,
    },
    validation,
};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn create(caller: Principal, args: CreateDealArgs, now: u64) -> Result<DealView, EscrowError> {
    validation::validate_create(args.amount, args.expires_at_ns, now)?;

    let metadata = if args.title.is_some() || args.note.is_some() {
        Some(DealMetadata {
            title: args.title,
            note: args.note,
        })
    } else {
        None
    };

    let deal = insert_new_deal(|deal_id| Deal {
        id: deal_id,
        payer: caller,
        recipient: args.recipient,
        token_ledger: args.token_ledger,
        token_symbol: None,
        amount: args.amount,
        created_at_ns: now,
        created_by: caller,
        updated_at_ns: None,
        updated_by: None,
        expires_at_ns: args.expires_at_ns,
        status: DealStatus::Created,
        escrow_subaccount: derive_deal_subaccount(deal_id),
        funded_at_ns: None,
        completed_at_ns: None,
        refunded_at_ns: None,
        funding_tx: None,
        payout_tx: None,
        refund_tx: None,
        claim_code: None,
        metadata,
    });

    Ok(DealView::from(&deal))
}

pub async fn fund(caller: Principal, deal_id: DealId) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_fund(&deal, caller)?;
    if already_done {
        return Ok(DealView::from(&deal));
    }

    try_acquire_lock(deal_id)?;
    let result = execute_fund(deal_id, &deal, caller).await;
    release_lock(deal_id);
    result
}

pub async fn accept(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_accept(&deal, caller, now)?;
    if already_done {
        return Ok(DealView::from(&deal));
    }

    try_acquire_lock(deal_id)?;
    let result = execute_accept(deal_id, &deal, caller).await;
    release_lock(deal_id);
    result
}

pub async fn reclaim(
    caller: Principal,
    deal_id: DealId,
    now: u64,
) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_reclaim(&deal, caller, now)?;
    if already_done {
        return Ok(DealView::from(&deal));
    }

    try_acquire_lock(deal_id)?;
    let result = execute_reclaim(deal_id, &deal, caller).await;
    release_lock(deal_id);
    result
}

pub fn cancel(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_cancel(&deal, caller)?;
    if already_done {
        return Ok(DealView::from(&deal));
    }

    with_deal(deal_id, |d| {
        d.status = DealStatus::Cancelled;
        d.updated_at_ns = Some(now);
        d.updated_by = Some(caller);
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Returns the full deal view. Caller must be payer or recipient.
pub fn get(caller: Principal, deal_id: DealId) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;
    authorize_deal_participant(&deal, caller)?;
    Ok(DealView::from(&deal))
}

#[must_use]
pub fn list_for_caller(caller: Principal, offset: usize, limit: usize) -> Vec<DealView> {
    with_deals(|deals| {
        let mut matched: Vec<DealView> = deals
            .values()
            .filter(|d| d.created_by == caller || d.payer == caller || d.recipient == Some(caller))
            .map(DealView::from)
            .collect();
        matched.sort_by(|a, b| b.created_at_ns.cmp(&a.created_at_ns));
        matched.into_iter().skip(offset).take(limit).collect()
    })
}

/// Reduced public view for claim/share-link pages (no authorization required).
pub fn get_claimable(deal_id: DealId) -> Result<ClaimableDealView, EscrowError> {
    load_deal(deal_id)
        .map(|d| ClaimableDealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

/// Returns the escrow account for a deal. Caller must be payer or recipient.
pub fn get_escrow_account(caller: Principal, deal_id: DealId) -> Result<Account, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;
    authorize_deal_participant(&deal, caller)?;
    Ok(Account {
        owner: id(),
        subaccount: Some(deal.escrow_subaccount),
    })
}

fn authorize_deal_participant(deal: &Deal, caller: Principal) -> Result<(), EscrowError> {
    if deal.created_by == caller || deal.payer == caller || deal.recipient == Some(caller) {
        return Ok(());
    }
    Err(EscrowError::NotAuthorised)
}

// ---------------------------------------------------------------------------
// Internal async executors (run inside processing lock)
// ---------------------------------------------------------------------------

async fn execute_fund(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
) -> Result<DealView, EscrowError> {
    let escrow_account = Account {
        owner: id(),
        subaccount: Some(deal.escrow_subaccount.clone()),
    };
    let payer_account = Account {
        owner: deal.payer,
        subaccount: None,
    };

    let block_index = ledger::transfer_from(
        deal.token_ledger,
        payer_account,
        escrow_account,
        deal.amount,
    )
    .await?;

    let now = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Created {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(now);
            d.funding_tx = Some(block_index);
            d.updated_at_ns = Some(now);
            d.updated_by = Some(caller);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

async fn execute_accept(
    deal_id: DealId,
    deal: &Deal,
    recipient: Principal,
) -> Result<DealView, EscrowError> {
    with_deal(deal_id, |d| {
        if d.recipient.is_none() {
            d.recipient = Some(recipient);
        }
    });

    let recipient_account = Account {
        owner: recipient,
        subaccount: None,
    };

    let block_index = ledger::transfer(
        deal.token_ledger,
        Some(deal.escrow_subaccount.clone()),
        recipient_account,
        deal.amount,
    )
    .await?;

    let completed_at = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = DealStatus::Completed;
            d.completed_at_ns = Some(completed_at);
            d.payout_tx = Some(block_index);
            d.updated_at_ns = Some(completed_at);
            d.updated_by = Some(recipient);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

async fn execute_reclaim(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
) -> Result<DealView, EscrowError> {
    let payer_account = Account {
        owner: deal.payer,
        subaccount: None,
    };

    let block_index = ledger::transfer(
        deal.token_ledger,
        Some(deal.escrow_subaccount.clone()),
        payer_account,
        deal.amount,
    )
    .await?;

    let now = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = DealStatus::Refunded;
            d.refunded_at_ns = Some(now);
            d.refund_tx = Some(block_index);
            d.updated_at_ns = Some(now);
            d.updated_by = Some(caller);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

// ---------------------------------------------------------------------------
// Tests — sync service functions only (async requires IC runtime)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{cancel, create, get, get_claimable, get_escrow_account, list_for_caller};
    use crate::{
        api::deals::{errors::EscrowError, params::CreateDealArgs},
        types::deal::DealStatus,
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn ledger_principal() -> Principal {
        test_principal(99)
    }

    fn valid_args() -> CreateDealArgs {
        CreateDealArgs {
            amount: 1_000_000,
            token_ledger: ledger_principal(),
            expires_at_ns: 1000,
            recipient: None,
            title: Some("Test".to_owned()),
            note: None,
        }
    }

    #[test]
    fn create_succeeds_with_valid_input() {
        let view = create(test_principal(1), valid_args(), 100).unwrap();
        assert_eq!(view.created_by, test_principal(1));
        assert_eq!(view.payer, test_principal(1));
        assert_eq!(view.amount, 1_000_000);
        assert_eq!(view.status, DealStatus::Created);
        assert_eq!(view.title.as_deref(), Some("Test"));
        assert!(view.updated_at_ns.is_none());
        assert!(view.updated_by.is_none());
    }

    #[test]
    fn create_rejects_zero_amount() {
        let mut args = valid_args();
        args.amount = 0;
        assert!(create(test_principal(1), args, 100).is_err());
    }

    #[test]
    fn create_rejects_past_expiry() {
        let mut args = valid_args();
        args.expires_at_ns = 50;
        assert!(create(test_principal(1), args, 100).is_err());
    }

    #[test]
    fn cancel_succeeds_for_created_deal() {
        let payer = test_principal(1);
        let view = create(payer, valid_args(), 100).unwrap();
        let cancelled = cancel(payer, view.id, 200).unwrap();
        assert_eq!(cancelled.status, DealStatus::Cancelled);
        assert_eq!(cancelled.updated_at_ns, Some(200));
        assert_eq!(cancelled.updated_by, Some(payer));
    }

    #[test]
    fn cancel_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let view = create(payer, valid_args(), 100).unwrap();
        assert!(cancel(other, view.id, 200).is_err());
    }

    #[test]
    fn get_returns_deal_for_payer() {
        let payer = test_principal(1);
        let view = create(payer, valid_args(), 100).unwrap();
        let fetched = get(payer, view.id).unwrap();
        assert_eq!(fetched.id, view.id);
    }

    #[test]
    fn get_returns_deal_for_recipient() {
        let payer = test_principal(1);
        let recipient = test_principal(2);
        let mut args = valid_args();
        args.recipient = Some(recipient);
        let view = create(payer, args, 100).unwrap();
        let fetched = get(recipient, view.id).unwrap();
        assert_eq!(fetched.id, view.id);
    }

    #[test]
    fn get_rejects_unrelated_caller() {
        let payer = test_principal(1);
        let stranger = test_principal(3);
        let view = create(payer, valid_args(), 100).unwrap();
        let err = get(stranger, view.id).unwrap_err();
        assert_eq!(err, EscrowError::NotAuthorised);
    }

    #[test]
    fn get_returns_not_found() {
        assert!(get(test_principal(1), 999_999).is_err());
    }

    #[test]
    fn get_escrow_account_rejects_unrelated_caller() {
        let payer = test_principal(1);
        let stranger = test_principal(3);
        let view = create(payer, valid_args(), 100).unwrap();
        let err = get_escrow_account(stranger, view.id).unwrap_err();
        assert_eq!(err, EscrowError::NotAuthorised);
    }

    #[test]
    fn list_returns_own_deals_only() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let view = create(payer, valid_args(), 100).unwrap();

        let own = list_for_caller(payer, 0, 50);
        assert!(own.iter().any(|d| d.id == view.id));

        let theirs = list_for_caller(other, 0, 50);
        assert!(!theirs.iter().any(|d| d.id == view.id));
    }

    #[test]
    fn get_claimable_hides_sensitive_fields() {
        let payer = test_principal(1);
        let view = create(payer, valid_args(), 100).unwrap();
        let claimable = get_claimable(view.id).unwrap();
        assert!(!claimable.is_recipient_bound);
        assert_eq!(claimable.amount, 1_000_000);
    }
}
