use core::fmt::Write;

use candid::Principal;
use ic_cdk::{api::time, id};

use super::reliability;
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
        deal::{Consent, Deal, DealId, DealMetadata, DealStatus},
        ledger_types::Account,
    },
    validation,
};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub async fn create(
    caller: Principal,
    args: CreateDealArgs,
    now: u64,
) -> Result<DealView, EscrowError> {
    validation::validate_caller_deal_limit(caller)?;
    reliability::validate(caller)?;
    validation::validate_create(args.amount, args.expires_at_ns, now)?;
    validation::validate_metadata(args.title.as_deref(), args.note.as_deref())?;

    let (payer, recipient, payer_consent, recipient_consent) =
        validation::resolve_parties(caller, args.payer, args.recipient)?;

    let claim_code = generate_claim_code().await?;

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
        payer,
        recipient,
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
        settled_at_ns: None,
        refunded_at_ns: None,
        funding_tx: None,
        payout_tx: None,
        refund_tx: None,
        claim_code: Some(claim_code),
        payer_consent,
        recipient_consent,
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

pub async fn accept(
    caller: Principal,
    deal_id: DealId,
    now: u64,
    claim_code: Option<String>,
) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_accept(&deal, caller, now, claim_code.as_deref())?;
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

pub fn consent(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let is_payer = validation::validate_can_consent(&deal, caller)?;

    with_deal(deal_id, |d| {
        if is_payer {
            d.payer_consent = Consent::Accepted;
        } else {
            d.recipient_consent = Consent::Accepted;
        }
        d.updated_at_ns = Some(now);
        d.updated_by = Some(caller);
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

pub fn reject(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let is_payer = validation::validate_can_reject(&deal, caller)?;

    with_deal(deal_id, |d| {
        if is_payer {
            d.payer_consent = Consent::Rejected;
        } else {
            d.recipient_consent = Consent::Rejected;
        }
        d.status = DealStatus::Rejected;
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
            .filter(|d| {
                d.created_by == caller || d.payer == Some(caller) || d.recipient == Some(caller)
            })
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
    if deal.created_by == caller || deal.payer == Some(caller) || deal.recipient == Some(caller) {
        return Ok(());
    }
    Err(EscrowError::NotAuthorised)
}

// ---------------------------------------------------------------------------
// Claim code generation
// ---------------------------------------------------------------------------

async fn generate_claim_code() -> Result<String, EscrowError> {
    let (random_bytes,): (Vec<u8>,) = ledger::raw_rand().await?;

    let hex = random_bytes
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        });

    Ok(hex)
}

// ---------------------------------------------------------------------------
// Internal async executors (run inside processing lock)
// ---------------------------------------------------------------------------

async fn execute_fund(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
) -> Result<DealView, EscrowError> {
    // Bind the payer if this is an open-payer deal (invoice flow).
    // Done inside the lock so no partial mutation if the transfer fails.
    if deal.payer.is_none() {
        with_deal(deal_id, |d| {
            d.payer = Some(caller);
            d.payer_consent = Consent::Accepted;
        });
    }

    let payer = deal.payer.unwrap_or(caller);

    let escrow_account = Account {
        owner: id(),
        subaccount: Some(deal.escrow_subaccount.clone()),
    };
    let payer_account = Account {
        owner: payer,
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
            d.payer_consent = Consent::Accepted;
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
        d.recipient_consent = Consent::Accepted;
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

    let settled_at = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = DealStatus::Settled;
            d.settled_at_ns = Some(settled_at);
            d.payout_tx = Some(block_index);
            d.updated_at_ns = Some(settled_at);
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
    let payer = deal.payer.ok_or(EscrowError::PayerNotSet)?;

    let payer_account = Account {
        owner: payer,
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

    use super::{cancel, consent, get, get_claimable, get_escrow_account, list_for_caller, reject};
    use crate::{
        api::deals::errors::EscrowError,
        memory::insert_new_deal,
        subaccounts::derive_deal_subaccount,
        types::deal::{Consent, Deal, DealMetadata, DealStatus},
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn ledger_principal() -> Principal {
        test_principal(99)
    }

    fn store_deal(
        payer: Option<Principal>,
        recipient: Option<Principal>,
        status: DealStatus,
        payer_consent: Consent,
        recipient_consent: Consent,
    ) -> Deal {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer,
            recipient,
            token_ledger: ledger_principal(),
            token_symbol: None,
            amount: 1_000_000,
            created_at_ns: 100,
            created_by: payer.or(recipient).unwrap_or(test_principal(1)),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 1000,
            status,
            escrow_subaccount: derive_deal_subaccount(deal_id),
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: Some("test-code-abc".to_owned()),
            payer_consent,
            recipient_consent,
            metadata: Some(DealMetadata {
                title: Some("Test".to_owned()),
                note: None,
            }),
        })
    }

    fn store_tip(payer: Principal) -> Deal {
        store_deal(
            Some(payer),
            None,
            DealStatus::Created,
            Consent::Accepted,
            Consent::Pending,
        )
    }

    #[test]
    fn cancel_succeeds_for_created_deal() {
        let payer = test_principal(1);
        let deal = store_tip(payer);
        let cancelled = cancel(payer, deal.id, 200).unwrap();
        assert_eq!(cancelled.status, DealStatus::Cancelled);
        assert_eq!(cancelled.updated_at_ns, Some(200));
        assert_eq!(cancelled.updated_by, Some(payer));
    }

    #[test]
    fn cancel_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = store_tip(payer);
        assert!(cancel(other, deal.id, 200).is_err());
    }

    #[test]
    fn get_returns_deal_for_payer() {
        let payer = test_principal(1);
        let deal = store_tip(payer);
        let fetched = get(payer, deal.id).unwrap();
        assert_eq!(fetched.id, deal.id);
    }

    #[test]
    fn get_returns_deal_for_recipient() {
        let payer = test_principal(1);
        let recipient = test_principal(2);
        let deal = store_deal(
            Some(payer),
            Some(recipient),
            DealStatus::Created,
            Consent::Accepted,
            Consent::Pending,
        );
        let fetched = get(recipient, deal.id).unwrap();
        assert_eq!(fetched.id, deal.id);
    }

    #[test]
    fn get_rejects_unrelated_caller() {
        let payer = test_principal(1);
        let stranger = test_principal(3);
        let deal = store_tip(payer);
        let err = get(stranger, deal.id).unwrap_err();
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
        let deal = store_tip(payer);
        let err = get_escrow_account(stranger, deal.id).unwrap_err();
        assert_eq!(err, EscrowError::NotAuthorised);
    }

    #[test]
    fn list_returns_own_deals_only() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = store_tip(payer);

        let own = list_for_caller(payer, 0, 50);
        assert!(own.iter().any(|d| d.id == deal.id));

        let theirs = list_for_caller(other, 0, 50);
        assert!(!theirs.iter().any(|d| d.id == deal.id));
    }

    #[test]
    fn get_claimable_hides_sensitive_fields() {
        let payer = test_principal(1);
        let deal = store_tip(payer);
        let claimable = get_claimable(deal.id).unwrap();
        assert!(!claimable.is_recipient_bound);
        assert_eq!(claimable.amount, 1_000_000);
    }

    #[test]
    fn consent_sets_payer_consent() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = store_deal(
            Some(payer),
            Some(recip),
            DealStatus::Created,
            Consent::Pending,
            Consent::Accepted,
        );
        let updated = consent(payer, deal.id, 200).unwrap();
        assert_eq!(updated.payer_consent, Consent::Accepted);
    }

    #[test]
    fn consent_sets_recipient_consent() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = store_deal(
            Some(payer),
            Some(recip),
            DealStatus::Created,
            Consent::Accepted,
            Consent::Pending,
        );
        let updated = consent(recip, deal.id, 200).unwrap();
        assert_eq!(updated.recipient_consent, Consent::Accepted);
    }

    #[test]
    fn reject_transitions_to_rejected() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = store_deal(
            Some(payer),
            Some(recip),
            DealStatus::Created,
            Consent::Accepted,
            Consent::Pending,
        );
        let updated = reject(recip, deal.id, 200).unwrap();
        assert_eq!(updated.status, DealStatus::Rejected);
        assert_eq!(updated.recipient_consent, Consent::Rejected);
    }

    #[test]
    fn deal_view_contains_claim_code() {
        let payer = test_principal(1);
        let deal = store_tip(payer);
        let view = get(payer, deal.id).unwrap();
        assert_eq!(view.claim_code.as_deref(), Some("test-code-abc"));
    }
}
