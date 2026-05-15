use ic_cdk::api::{canister_self, time};

use crate::{
    api::deals::errors::EscrowError,
    ledger,
    memory::{release_lock, try_acquire_lock, with_deal, with_deals},
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
    let (ledger_id, subaccount, payer, recipient, amount, fees) = with_deal(deal_id, |deal| {
        if deal.status != DealStatus::Funded {
            return Err(EscrowError::AlreadyFinalised);
        }
        let payer = deal.payer.ok_or(EscrowError::PayerNotSet)?;
        Ok((
            deal.token_ledger,
            deal.escrow_subaccount.clone(),
            payer,
            deal.recipient,
            deal.amount,
            deal.fees.clone(),
        ))
    })
    .ok_or(EscrowError::NotFound)??;

    // Auto-refund mirrors `execute_reclaim`'s fan-out:
    //   - Payer: `amount − EF − LF + (DC/2 − LF)` combined.
    //   - Recipient: `DC/2 − LF` separately.
    // `EF` stays in the deal subaccount as the operator's share.
    let ledger_fee = ledger::fee(ledger_id).await?;
    let (payer_refund, recipient_refund) = refund_amounts(amount, &fees, ledger_fee);

    let refund_tx = ledger::transfer(
        ledger_id,
        Some(subaccount.clone()),
        Account {
            owner: payer,
            subaccount: None,
        },
        payer_refund,
    )
    .await?;

    if recipient_refund > 0 {
        if let Some(recipient) = recipient {
            ledger::transfer(
                ledger_id,
                Some(subaccount),
                Account {
                    owner: recipient,
                    subaccount: None,
                },
                recipient_refund,
            )
            .await?;
        }
    }

    let now = time();
    let canister = canister_self();
    with_deal(deal_id, |deal| {
        if deal.status == DealStatus::Funded {
            deal.status = DealStatus::Refunded;
            deal.refunded_at_ns = Some(now);
            deal.refund_tx = Some(refund_tx);
            deal.updated_at_ns = Some(now);
            deal.updated_by = Some(canister);
        }
    });

    Ok(())
}

/// Returns `(payer_refund, recipient_reserve_refund)` for a deal
/// being auto-refunded by the housekeeping sweep. Mirrors
/// `services::deals::execute_reclaim`'s split so manual and
/// automatic refunds agree on the math.
fn refund_amounts(amount: u128, fees: &DealFees, ledger_fee: u128) -> (u128, u128) {
    let reserve = fees.dispute_reserve_per_party;
    let payer_refund = amount
        .saturating_sub(fees.escrow_fee)
        .saturating_add(reserve)
        .saturating_sub(ledger_fee);
    let recipient_reserve_refund = reserve.saturating_sub(ledger_fee);
    (payer_refund, recipient_reserve_refund)
}
