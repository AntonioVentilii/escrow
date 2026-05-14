use ic_cdk::api::{canister_self, time};

use crate::{
    api::deals::errors::EscrowError,
    ledger,
    memory::{release_lock, try_acquire_lock, with_deal, with_deals},
    services::deals::payout_after_fees,
    types::{
        deal::{DealFees, DealId, DealStatus},
        ledger_types::Account,
    },
};

/// Processes up to `limit` expired funded deals by refunding the payer.
///
/// Safe to call repeatedly — idempotent and lock-guarded.
/// Returns the list of deal IDs that were successfully refunded.
pub async fn process_expired(limit: u32) -> Result<Vec<DealId>, EscrowError> {
    let now = time();

    let expired_ids: Vec<DealId> = with_deals(|deals| {
        deals
            .values()
            .filter(|d| d.status == DealStatus::Funded && d.expires_at_ns <= now)
            .take(limit as usize)
            .map(|d| d.id)
            .collect()
    });

    let mut processed = Vec::new();

    for deal_id in expired_ids {
        if try_acquire_lock(deal_id).is_err() {
            continue;
        }

        let refund_result = try_refund_deal(deal_id).await;
        release_lock(deal_id);

        if refund_result.is_ok() {
            processed.push(deal_id);
        }
    }

    Ok(processed)
}

async fn try_refund_deal(deal_id: DealId) -> Result<(), EscrowError> {
    let (ledger_id, subaccount, payer, amount, fees) = with_deal(deal_id, |deal| {
        if deal.status != DealStatus::Funded {
            return Err(EscrowError::AlreadyFinalised);
        }
        let payer = deal.payer.ok_or(EscrowError::PayerNotSet)?;
        Ok((
            deal.token_ledger,
            deal.escrow_subaccount.clone(),
            payer,
            deal.amount,
            deal.fees.clone(),
        ))
    })
    .ok_or(EscrowError::NotFound)??;

    let payer_account = Account {
        owner: payer,
        subaccount: None,
    };

    // RFC-002: auto-refund is symmetric with manual `reclaim_deal` —
    // escrow keeps `escrow_fee`, refund covers `amount - EF - LF`.
    // The `fees` snapshot is cloned out of the deal so we don't hold
    // a borrow over the await.
    let ledger_fee = ledger::fee(ledger_id).await?;
    let fees_ref: Option<&DealFees> = fees.as_ref();
    let refund = payout_after_fees(amount, fees_ref, ledger_fee);

    let block_index = ledger::transfer(ledger_id, Some(subaccount), payer_account, refund).await?;

    let now = time();
    let canister = canister_self();
    with_deal(deal_id, |deal| {
        if deal.status == DealStatus::Funded {
            deal.status = DealStatus::Refunded;
            deal.refunded_at_ns = Some(now);
            deal.refund_tx = Some(block_index);
            deal.updated_at_ns = Some(now);
            deal.updated_by = Some(canister);
        }
    });

    Ok(())
}
