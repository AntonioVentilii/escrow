use candid::Principal;
use ic_cdk::api::{canister_self, time};

use super::{deals, disputes};
use crate::{
    api::deals::errors::EscrowError,
    ledger,
    memory::{get_deal, release_lock, try_acquire_lock, with_deal, with_deals},
    types::{
        deal::{Deal, DealFees, DealId, DealStatus, Signature},
        ledger_types::Account,
    },
    validation::{apply_expiry_default_yes, tally_signatures, SignatureTally},
};

/// Processes up to `limit` expired funded deals by dispatching them
/// to the appropriate terminal flow.
///
/// Two flows depending on whether the deal has a bound recipient:
///
/// - **Tip flow** (`recipient = None`): no signatures apply (signing is bound-deal-only). The deal
///   refunds to the payer — the same behaviour the canister has always had on tip expiry.
/// - **Bound deal**: the auto-YES rule (`apply_expiry_default_yes`) treats any `Empty` signature as
///   `Yes` (party that didn't act defaults to release). The resulting tally dispatches:
///   - both effective `Yes` → settle to recipient (via [`deals::execute_accept`] under a per-deal
///     lock; bypasses `validate_can_accept`'s expiry rejection — the sweep IS the canonical
///     post-expiry dispatcher).
///   - both `No` → abort (via [`deals::execute_refund`] with `target_status = Aborted`, same lock
///     pattern).
///   - mixed → auto-open a dispute (via [`disputes::open_post_expiry`], which bypasses the normal
///     `Expired` gate). The opener is the explicit `No`-signer.
///
/// Safe to call repeatedly — idempotent and lock-guarded. Returns
/// the list of deal IDs that were successfully dispatched.
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
        if dispatch_one_expired(deal_id, now).await.is_ok() {
            processed.push(deal_id);
        }
    }

    Ok(processed)
}

/// External entry point for `services::deals::reclaim` to dispatch
/// a single expired bound deal through the auto-YES tally without
/// going through the full sweep loop. Wraps
/// [`dispatch_one_expired`] so the callers can stay decoupled from
/// the per-deal lock management (which is internal to the inner
/// dispatcher and the executors it calls).
pub(crate) async fn dispatch_one_expired_external(
    deal_id: DealId,
    now: u64,
) -> Result<(), EscrowError> {
    dispatch_one_expired(deal_id, now).await
}

/// Inspects a single expired `Funded` deal and routes it to the
/// terminal flow dictated by its signatures (bound deal) or directly
/// to refund (tip).
async fn dispatch_one_expired(deal_id: DealId, now: u64) -> Result<(), EscrowError> {
    let deal = get_deal(deal_id).ok_or(EscrowError::NotFound)?;

    // Concurrent flip after the snapshot was taken — nothing to do.
    if deal.status != DealStatus::Funded {
        return Err(EscrowError::AlreadyFinalised);
    }

    // Tip flow: refund to payer (legacy behaviour). Signing is
    // disabled for tips so signatures are irrelevant here.
    if deal.recipient.is_none() {
        if try_acquire_lock(deal_id).is_err() {
            return Err(EscrowError::ValidationError(
                "Deal is currently being processed".to_owned(),
            ));
        }
        let result = try_refund_tip(deal_id).await;
        release_lock(deal_id);
        return result;
    }

    // Bound deal: apply auto-YES + dispatch. The settle / abort
    // branches call the private `execute_*` helpers directly under
    // a freshly-acquired per-deal lock — going through the public
    // `accept` / `abort` would re-validate `validate_can_accept` /
    // `validate_can_abort`, both of which reject expired deals.
    // The expiry sweep IS the canonical post-expiry dispatcher and
    // must bypass those validators.
    //
    // Mixed dispatches via `disputes::open_post_expiry`, which
    // already takes `allow_expired = true` and acquires its own
    // lock.
    let (eff_payer, eff_recipient) =
        apply_expiry_default_yes(&deal.payer_signature, &deal.recipient_signature);
    match tally_signatures(&eff_payer, &eff_recipient) {
        SignatureTally::BothYes => {
            let recipient = deal.recipient.expect("bound deal in dispatch_one_expired");
            try_acquire_lock(deal_id)?;
            let result = deals::execute_accept(deal_id, &deal, recipient).await;
            release_lock(deal_id);
            result.map(|_| ())
        }
        SignatureTally::BothNo => {
            // Use the canister principal as the abort initiator —
            // neither party "won", the system is just dispatching
            // the agreed-upon outcome.
            try_acquire_lock(deal_id)?;
            let result =
                deals::execute_refund(deal_id, &deal, canister_self(), now, DealStatus::Aborted)
                    .await;
            release_lock(deal_id);
            result.map(|_| ())
        }
        SignatureTally::Mixed => {
            // Mixed at expiry means one party explicitly signed
            // `No`, the other was upgraded from `Empty` to `Yes`
            // by the auto-YES rule. The dissenting `No`-signer is
            // the de-facto dispute opener — record them as such.
            let opener =
                no_signer_for_dispute(&deal.payer_signature, &deal.recipient_signature, &deal)
                    .ok_or(EscrowError::ValidationError(
                        "Mixed tally at expiry has no explicit No-signer".to_owned(),
                    ))?;
            disputes::open_post_expiry(opener, deal_id, now)
                .await
                .map(|_| ())
        }
        SignatureTally::Pending => Err(EscrowError::ValidationError(
            "apply_expiry_default_yes should never leave Pending".to_owned(),
        )),
    }
}

/// Returns the principal of the party who explicitly signed `No`
/// (i.e. the `Mixed` tally's dissenter). Used to attribute the
/// system-opened dispute to the right party in the audit trail.
///
/// Order of precedence is fixed (payer first, then recipient) — the
/// `Mixed` tally guarantees exactly one explicit `No`, so only one
/// branch matches. Returns `None` if neither side has a `No`
/// (defensive — `Mixed` shouldn't be reachable without one).
fn no_signer_for_dispute(
    payer_sig: &Signature,
    recipient_sig: &Signature,
    deal: &Deal,
) -> Option<Principal> {
    if matches!(payer_sig, Signature::No) {
        return deal.payer;
    }
    if matches!(recipient_sig, Signature::No) {
        return deal.recipient;
    }
    None
}

async fn try_refund_tip(deal_id: DealId) -> Result<(), EscrowError> {
    let (ledger_id, subaccount, payer, recipient, amount, fees) = with_deal(deal_id, |deal| {
        if deal.status != DealStatus::Funded {
            return Err(EscrowError::AlreadyFinalised);
        }
        let payer = deal.payer.ok_or(EscrowError::PayerNotSet)?;
        let ledger_id = deal.asset.as_icrc()?;
        Ok((
            ledger_id,
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
