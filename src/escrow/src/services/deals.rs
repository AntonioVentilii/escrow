use core::{cmp::Reverse, fmt::Write};

use candid::Principal;
use ic_cdk::api::{canister_self, time};

use super::{disputes, reliability};
use crate::{
    api::deals::{
        errors::EscrowError,
        params::CreateDealArgs,
        results::{ClaimableDealView, DealView},
    },
    ledger,
    memory::{
        get_deal as load_deal, insert_new_deal, release_lock, try_acquire_lock, with_deal,
        with_deals, CONFIG,
    },
    subaccounts::derive_deal_subaccount,
    types::{
        deal::{Consent, Deal, DealFees, DealId, DealMetadata, DealStatus},
        dispute::DisputeConfig,
        ledger_types::Account,
    },
    validation,
};

// ---------------------------------------------------------------------------
// Fee snapshot + accessors
// ---------------------------------------------------------------------------

/// Returns the currently-configured escrow service fee. Called once
/// per `create_deal` to build the per-deal [`DealFees`] snapshot.
/// Defaults are sourced from [`crate::types::state::Config::default`].
#[must_use]
pub fn load_escrow_fee() -> u128 {
    CONFIG.with(|c| c.borrow().escrow_fee)
}

/// Computes the per-deal fee snapshot from the current configs +
/// the deal's `amount` + the ledger's live fee. The returned
/// [`DealFees`] is stored verbatim on the new deal so subsequent
/// `update_config` calls cannot retroactively change the agreed
/// economics.
///
/// `dispute_reserve_per_party` = `compute_arbitration_fee(amount,
/// dispute_config).div_ceil(2)`. Ceiling division ensures
/// `2 × dispute_reserve_per_party ≥ full_dispute_cost` even when
/// the full cost is odd, so the panel can always be paid in full
/// at finalize time. The (at-most-one-unit) overage on odd fees
/// stays in the deal subaccount and accrues to the operator.
#[must_use]
pub fn compute_deal_fees(
    amount: u128,
    escrow_fee: u128,
    dispute_cfg: &DisputeConfig,
    ledger_fee: u128,
) -> DealFees {
    let full_dispute_cost = disputes::compute_arbitration_fee(amount, dispute_cfg);
    DealFees {
        escrow_fee,
        dispute_reserve_per_party: full_dispute_cost.div_ceil(2),
        withdraw_fee_pct: dispute_cfg.withdraw_fee_pct,
        ledger_fee_at_create: ledger_fee,
    }
}

/// Computes the recipient's payout on a happy-path `Settled` or
/// `Refunded` transition: `amount − escrow_fee − ledger_fee` so
/// the recipient gets exactly the value they were quoted at create
/// time and the escrow subaccount retains `escrow_fee` (which
/// stays locked in the per-deal subaccount as the operator's
/// share; a sweeper is out of scope for now).
///
/// Saturating arithmetic — if `amount < escrow_fee + ledger_fee`
/// the function returns `0`. The `validate_min_amount` check at
/// create time prevents production deals from hitting this case.
#[must_use]
pub fn payout_after_fees(amount: u128, fees: &DealFees, ledger_fee: u128) -> u128 {
    amount
        .saturating_sub(fees.escrow_fee)
        .saturating_sub(ledger_fee)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub async fn create(
    caller: Principal,
    args: CreateDealArgs,
    now: u64,
) -> Result<DealView, EscrowError> {
    validation::validate_create(args.amount, args.expires_at_ns, now)?;
    validation::validate_metadata(args.title.as_deref(), args.note.as_deref())?;
    validation::validate_caller_deal_limit(caller)?;
    reliability::validate(caller)?;

    // Per-deal panel_size override — validate against the active
    // DisputeConfig bounds. None is always valid (= "use whatever
    // canister default applies at open_dispute time"); Some(n) must be
    // odd and within [min_panel_size, max_panel_size]. The validated
    // value is locked into the deal record so subsequent
    // DisputeConfig changes can't retroactively alter the agreed
    // dispute terms.
    let dispute_cfg = disputes::load_dispute_config();
    validation::validate_panel_size_choice(args.panel_size, &dispute_cfg)?;

    let (payer, recipient, payer_consent, recipient_consent) =
        validation::resolve_parties(caller, args.payer, args.recipient)?;

    // Snapshot every fee against the deal at create time. The
    // ledger fee is queried live and stored on the snapshot for
    // audit + the min-amount check, but every subsequent transfer
    // re-queries it — the operator absorbs any drift between
    // create-time and runtime fees out of `escrow_fee`.
    //
    // A failure to reach the ledger here is non-fatal: we fall
    // back to `0` for the snapshot and the min-amount check
    // becomes slightly looser (no `ledger_fee` headroom in the
    // floor). All money-moving operations (`fund`, `accept`,
    // `reclaim`, expiry sweep) re-query the live fee and fail
    // hard if the ledger is unreachable, so a fake / misconfigured
    // `token_ledger` cannot actually drain funds — it just creates
    // a stuck deal that can never settle. This keeps create-time
    // robust against transient ledger flakes without weakening
    // any money-handling invariant.
    let escrow_fee = load_escrow_fee();
    let ledger_fee = ledger::fee(args.token_ledger).await.unwrap_or(0);
    let fees = compute_deal_fees(args.amount, escrow_fee, &dispute_cfg, ledger_fee);
    // For the min-amount floor we use the panel size that will
    // actually be in effect: the deal's locked override if
    // `Some(_)`, otherwise the current canister default.
    let effective_panel_size = args.panel_size.unwrap_or(dispute_cfg.panel_size);
    validation::validate_min_amount(args.amount, &fees, ledger_fee, effective_panel_size)?;

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
        dispute: None,
        panel_size: args.panel_size,
        fees,
    });

    // RFC-002 Case 3b: receiver-creator path. When the caller is
    // the bound recipient (and not also the payer), the receiver's
    // `DC/2` reserve is pulled atomically with the deal creation.
    // On failure the deal is rolled forward to `Cancelled` so we
    // don't leak a `Created` deal that nobody can resolve.
    let caller_is_receiver_only = recipient == Some(caller) && payer != Some(caller);
    if caller_is_receiver_only && deal.fees.dispute_reserve_per_party > 0 {
        let escrow_account = Account {
            owner: canister_self(),
            subaccount: Some(deal.escrow_subaccount.clone()),
        };
        let receiver_account = Account {
            owner: caller,
            subaccount: None,
        };
        let deposit = ledger::transfer_from(
            args.token_ledger,
            receiver_account,
            escrow_account,
            deal.fees.dispute_reserve_per_party,
        )
        .await;
        if deposit.is_ok() {
            // `recipient_consent` is already `Accepted` from
            // `resolve_parties` because the caller is the recipient;
            // just bump the audit timestamps.
            with_deal(deal.id, |d| {
                d.updated_at_ns = Some(now);
                d.updated_by = Some(caller);
            });
        } else {
            // Roll the half-formed deal forward to `Cancelled` so it
            // doesn't sit around as a stuck `Created` record.
            with_deal(deal.id, |d| {
                d.status = DealStatus::Cancelled;
                d.updated_at_ns = Some(now);
                d.updated_by = Some(caller);
            });
            return Err(EscrowError::DisputeReserveRequired);
        }
    }

    load_deal(deal.id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
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

pub async fn cancel(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let already_done = validation::validate_can_cancel(&deal, caller)?;
    if already_done {
        return Ok(DealView::from(&deal));
    }

    try_acquire_lock(deal_id)?;
    let result = execute_terminate(deal_id, &deal, caller, now, DealStatus::Cancelled).await;
    release_lock(deal_id);
    result
}

pub async fn consent(
    caller: Principal,
    deal_id: DealId,
    now: u64,
) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let is_payer = validation::validate_can_consent(&deal, caller)?;

    // Payer consent is still a pure state flip — the payer's actual
    // commitment is `fund_deal`, which pulls `amount + DC/2` and
    // implicitly sets `payer_consent = Accepted`. Idempotent: a
    // repeated payer consent is a no-op.
    if is_payer {
        if !matches!(deal.payer_consent, Consent::Accepted) {
            with_deal(deal_id, |d| {
                d.payer_consent = Consent::Accepted;
                d.updated_at_ns = Some(now);
                d.updated_by = Some(caller);
            });
        }
        return load_deal(deal_id)
            .map(|d| DealView::from(&d))
            .ok_or(EscrowError::NotFound);
    }

    // Receiver consent is idempotent at the canister boundary:
    // a repeated call by an already-consented receiver short-circuits
    // and returns the current view without invoking the ledger.
    // Without this guard a wallet that left a generous allowance
    // open after the first consent could be drained by accidental
    // (UI retry) or malicious repeated calls — each invocation
    // would otherwise pull another `DC/2` via `icrc2_transfer_from`.
    if matches!(deal.recipient_consent, Consent::Accepted) {
        return Ok(DealView::from(&deal));
    }

    // Receiver consent: deposit `DC/2` into the deal subaccount via
    // `icrc2_transfer_from`. Receiver must have approved the canister
    // beforehand for at least `DC/2 + ledger_fee`.
    try_acquire_lock(deal_id)?;
    let result = execute_receiver_consent(deal_id, &deal, caller, now).await;
    release_lock(deal_id);
    result
}

pub async fn reject(caller: Principal, deal_id: DealId, now: u64) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;

    let is_payer = validation::validate_can_reject(&deal, caller)?;

    try_acquire_lock(deal_id)?;
    let result = execute_terminate_with_reject(deal_id, &deal, caller, now, is_payer).await;
    release_lock(deal_id);
    result
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
        matched.sort_by_key(|d| Reverse(d.created_at_ns));
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
        owner: canister_self(),
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

/// Pulls the receiver's `DC/2` reserve into the deal subaccount and
/// flips `recipient_consent` to `Accepted` on success. Mapped errors:
///
/// - `ConsentRequired` is not used here — that's the legacy "consent before fund" check; here the
///   operation IS the consent.
/// - `DisputeReserveRequired` if the ledger refuses the transfer (insufficient allowance /
///   insufficient funds). The deal stays `Created` with `recipient_consent = Pending` so the caller
///   can retry after fixing the approval / balance.
async fn execute_receiver_consent(
    deal_id: DealId,
    deal: &Deal,
    recipient: Principal,
    now: u64,
) -> Result<DealView, EscrowError> {
    let reserve = deal.fees.dispute_reserve_per_party;

    // Zero-reserve deals (DC = 0) skip the ledger round-trip and
    // collapse to a plain state flip. Only happens on synthetic
    // test deals where the dispute config sets the arbitration fee
    // to 0; production deals are gated by `validate_min_amount`.
    if reserve == 0 {
        with_deal(deal_id, |d| {
            d.recipient_consent = Consent::Accepted;
            d.updated_at_ns = Some(now);
            d.updated_by = Some(recipient);
        });
        return load_deal(deal_id)
            .map(|d| DealView::from(&d))
            .ok_or(EscrowError::NotFound);
    }

    let receiver_account = Account {
        owner: recipient,
        subaccount: None,
    };
    let escrow_account = Account {
        owner: canister_self(),
        subaccount: Some(deal.escrow_subaccount.clone()),
    };

    ledger::transfer_from(deal.token_ledger, receiver_account, escrow_account, reserve)
        .await
        .map_err(|_| EscrowError::DisputeReserveRequired)?;

    with_deal(deal_id, |d| {
        d.recipient_consent = Consent::Accepted;
        d.updated_at_ns = Some(now);
        d.updated_by = Some(recipient);
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

async fn execute_fund(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
) -> Result<DealView, EscrowError> {
    // For open-payer deals (invoice flow), use the caller as payer for this
    // transfer attempt but only persist the binding after a successful transfer
    // so a failed ledger call cannot permanently lock the deal.
    let payer = deal.payer.unwrap_or(caller);
    let reserve = deal.fees.dispute_reserve_per_party;
    // Payer's contribution to the escrow subaccount: the deal amount
    // plus the payer's `DC/2` reserve. The receiver's matching half
    // was deposited earlier (at `consent_deal` for 3a, or inside
    // `create_deal` for 3b receiver-creator deals).
    let pull = deal.amount.saturating_add(reserve);

    let escrow_account = Account {
        owner: canister_self(),
        subaccount: Some(deal.escrow_subaccount.clone()),
    };
    let payer_account = Account {
        owner: payer,
        subaccount: None,
    };

    let block_index =
        ledger::transfer_from(deal.token_ledger, payer_account, escrow_account, pull).await?;

    let now = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Created {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(now);
            d.funding_tx = Some(block_index);
            d.updated_at_ns = Some(now);
            d.updated_by = Some(caller);
            if d.payer.is_none() {
                d.payer = Some(payer);
            }
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

    let ledger_fee = ledger::fee(deal.token_ledger).await?;
    let reserve = deal.fees.dispute_reserve_per_party;

    // Fan-out per RFC-002 § Q5:
    //   - Recipient gets `amount − EF + DC/2 − LF` in ONE combined transfer (settlement + reserve
    //     refund, minus the single LF burned on the outbound transfer).
    //   - Payer gets `DC/2 − LF` in a separate transfer.
    // Subaccount math: held `amount + DC` after fund. Outgoing
    // ledger debits: `(amount − EF + DC/2)` (combined transfer
    // amount + 1 LF burned by the ledger) + `DC/2` (payer reserve
    // amount + 1 LF burned), totalling `amount − EF + DC`. Subaccount
    // left with `(amount + DC) − (amount − EF + DC) = EF`.
    let recipient_payout = deal
        .amount
        .saturating_sub(deal.fees.escrow_fee)
        .saturating_add(reserve)
        .saturating_sub(ledger_fee);
    let payer_reserve_refund = reserve.saturating_sub(ledger_fee);

    let payout_tx = ledger::transfer(
        deal.token_ledger,
        Some(deal.escrow_subaccount.clone()),
        Account {
            owner: recipient,
            subaccount: None,
        },
        recipient_payout,
    )
    .await?;

    if payer_reserve_refund > 0 {
        if let Some(payer) = deal.payer {
            ledger::transfer(
                deal.token_ledger,
                Some(deal.escrow_subaccount.clone()),
                Account {
                    owner: payer,
                    subaccount: None,
                },
                payer_reserve_refund,
            )
            .await?;
        }
    }

    let settled_at = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = DealStatus::Settled;
            d.settled_at_ns = Some(settled_at);
            d.payout_tx = Some(payout_tx);
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
    let ledger_fee = ledger::fee(deal.token_ledger).await?;
    let reserve = deal.fees.dispute_reserve_per_party;

    // Symmetric fan-out with `execute_accept`, but the deal amount
    // flows BACK to the payer.
    //   - Payer gets `amount − EF + DC/2 − LF` combined (deal-amount refund + their own reserve
    //     refund, minus one outbound LF).
    //   - Recipient gets `DC/2 − LF` separately.
    let payer_refund = deal
        .amount
        .saturating_sub(deal.fees.escrow_fee)
        .saturating_add(reserve)
        .saturating_sub(ledger_fee);
    let recipient_reserve_refund = reserve.saturating_sub(ledger_fee);

    let refund_tx = ledger::transfer(
        deal.token_ledger,
        Some(deal.escrow_subaccount.clone()),
        Account {
            owner: payer,
            subaccount: None,
        },
        payer_refund,
    )
    .await?;

    if recipient_reserve_refund > 0 {
        if let Some(recipient) = deal.recipient {
            ledger::transfer(
                deal.token_ledger,
                Some(deal.escrow_subaccount.clone()),
                Account {
                    owner: recipient,
                    subaccount: None,
                },
                recipient_reserve_refund,
            )
            .await?;
        }
    }

    let now = time();
    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = DealStatus::Refunded;
            d.refunded_at_ns = Some(now);
            d.refund_tx = Some(refund_tx);
            d.updated_at_ns = Some(now);
            d.updated_by = Some(caller);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

/// Refunds any reserves deposited on a `Created` deal and flips
/// the status to the supplied terminal (`Cancelled` or `Rejected`).
///
/// State at entry:
///   - The deal is `Created`.
///   - The payer has NOT funded (status is `Created`, so `amount` is not in the subaccount).
///   - The receiver MAY have deposited `DC/2` (iff `recipient_consent == Accepted` for a 3a flow,
///     OR the receiver is the deal creator in a 3b flow).
///
/// The receiver gets back their full deposited reserve minus one
/// outgoing ledger fee (`DC/2 − LF`). The operator does NOT take
/// `escrow_fee` on a pre-funding termination — `cancel_deal` /
/// `reject_deal` are callable by either party, so charging `EF`
/// to whatever's in the subaccount would unfairly penalise the
/// non-rejecting side (e.g. payer cancels a `Created` deal where
/// the receiver had already deposited their `DC/2` per RFC-002 §
/// Q5). The operator's revenue model fires only on post-funding
/// terminal states (`Settled`, `Refunded`, `ArbitratedX`); pre-
/// funding terminations are a wash with the operator absorbing
/// the single outgoing ledger fee.
async fn execute_terminate(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
    now: u64,
    new_status: DealStatus,
) -> Result<DealView, EscrowError> {
    let reserve = deal.fees.dispute_reserve_per_party;
    let receiver_deposited = receiver_has_deposited(deal);

    if receiver_deposited && reserve > 0 {
        if let Some(recipient) = deal.recipient {
            let ledger_fee = ledger::fee(deal.token_ledger).await?;
            // `checked_sub` so a pathological `reserve < ledger_fee`
            // configuration surfaces explicitly instead of silently
            // confiscating the receiver's deposit. In production
            // this branch is unreachable — `validate_min_amount`
            // at create time guarantees `DC/2 > 0` and the live
            // `ledger_fee` is bounded by the snapshotted
            // `ledger_fee_at_create` in normal operation.
            let refund = reserve.checked_sub(ledger_fee).ok_or_else(|| {
                EscrowError::ValidationError(format!(
                    "reserve ({reserve}) too small to cover ledger_fee ({ledger_fee}); \
                     refund would underflow",
                ))
            })?;
            if refund > 0 {
                ledger::transfer(
                    deal.token_ledger,
                    Some(deal.escrow_subaccount.clone()),
                    Account {
                        owner: recipient,
                        subaccount: None,
                    },
                    refund,
                )
                .await?;
            }
        }
    }

    with_deal(deal_id, |d| {
        if d.status == DealStatus::Created {
            d.status = new_status;
            d.updated_at_ns = Some(now);
            d.updated_by = Some(caller);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

async fn execute_terminate_with_reject(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
    now: u64,
    is_payer: bool,
) -> Result<DealView, EscrowError> {
    let view = execute_terminate(deal_id, deal, caller, now, DealStatus::Rejected).await?;
    with_deal(deal_id, |d| {
        if is_payer {
            d.payer_consent = Consent::Rejected;
        } else {
            d.recipient_consent = Consent::Rejected;
        }
    });
    Ok(view)
}

/// Returns `true` iff the receiver has actually deposited their
/// `DC/2` reserve. True when the receiver consented (their
/// `consent_deal` performed the `icrc2_transfer_from`) or when the
/// receiver is the creator of the deal — receiver-creator deposits
/// happen atomically inside `create_deal` per RFC-002 Case 3b.
fn receiver_has_deposited(deal: &Deal) -> bool {
    let receiver_is_creator = deal.recipient == Some(deal.created_by);
    receiver_is_creator || matches!(deal.recipient_consent, Consent::Accepted)
}

// ---------------------------------------------------------------------------
// Tests — sync service functions only (async requires IC runtime)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{get, get_claimable, get_escrow_account, list_for_caller};
    use crate::{
        api::deals::errors::EscrowError,
        memory::insert_new_deal,
        subaccounts::derive_deal_subaccount,
        types::deal::{Consent, Deal, DealFees, DealMetadata, DealStatus},
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
            dispute: None,
            panel_size: None,
            fees: DealFees::default(),
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

    // `cancel`, `consent`, and `reject` are now async (they may
    // pull / refund reserves via ICRC-2). Their happy paths and
    // authorisation checks are covered by the integration tests
    // in `tests/it/`; the validator-only invariants live in
    // `validation::tests`.

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
    fn deal_view_contains_claim_code() {
        let payer = test_principal(1);
        let deal = store_tip(payer);
        let view = get(payer, deal.id).unwrap();
        assert_eq!(view.claim_code.as_deref(), Some("test-code-abc"));
    }

    // --- fee snapshot + payout math ---

    use super::{compute_deal_fees, load_escrow_fee, payout_after_fees};
    use crate::{
        memory::CONFIG,
        types::{
            dispute::DisputeConfig,
            state::{Config, DEFAULT_ESCROW_FEE},
        },
    };

    #[test]
    fn load_escrow_fee_returns_default_when_unset() {
        CONFIG.with(|c| {
            *c.borrow_mut() = Config::default();
        });
        assert_eq!(load_escrow_fee(), DEFAULT_ESCROW_FEE);
    }

    #[test]
    fn load_escrow_fee_returns_configured_value() {
        CONFIG.with(|c| {
            *c.borrow_mut() = Config {
                dispute_config: DisputeConfig::default(),
                escrow_fee: 123_456,
            };
        });
        assert_eq!(load_escrow_fee(), 123_456);
        // Reset to default to avoid cross-test pollution.
        CONFIG.with(|c| {
            *c.borrow_mut() = Config::default();
        });
    }

    #[test]
    fn compute_deal_fees_splits_dispute_cost_in_half() {
        // amount = 1_000_000, default DisputeConfig: fee_bps=500 (5%),
        // min_fee=0 → full DC = 50_000, half = 25_000.
        let cfg = DisputeConfig::default();
        let fees = compute_deal_fees(1_000_000, 20_000, &cfg, 10_000);
        assert_eq!(fees.escrow_fee, 20_000);
        assert_eq!(fees.dispute_reserve_per_party, 25_000);
        assert_eq!(fees.withdraw_fee_pct, cfg.withdraw_fee_pct);
        assert_eq!(fees.ledger_fee_at_create, 10_000);
    }

    #[test]
    fn compute_deal_fees_honours_min_fee_floor() {
        // amount = 1_000, fee_bps=500 (5%) → bps_fee = 50,
        // min_fee = 10_000 wins → DC = 10_000, half = 5_000.
        let cfg = DisputeConfig {
            arbitration_fee_bps: 500,
            arbitration_min_fee: 10_000,
            ..DisputeConfig::default()
        };
        let fees = compute_deal_fees(1_000, 20_000, &cfg, 10_000);
        assert_eq!(fees.dispute_reserve_per_party, 5_000);
    }

    #[test]
    fn payout_after_fees_subtracts_ef_plus_lf() {
        let fees = DealFees {
            escrow_fee: 20_000,
            dispute_reserve_per_party: 5_000,
            withdraw_fee_pct: 25,
            ledger_fee_at_create: 10_000,
        };
        // amount = 1_000_000, EF=20_000, live_LF=10_000 → 970_000.
        assert_eq!(payout_after_fees(1_000_000, &fees, 10_000), 970_000);
    }

    #[test]
    fn payout_after_fees_saturates_at_zero_on_underflow() {
        let fees = DealFees {
            escrow_fee: 1_000_000,
            dispute_reserve_per_party: 0,
            withdraw_fee_pct: 25,
            ledger_fee_at_create: 10_000,
        };
        // amount < EF + LF → saturating_sub clamps to 0.
        assert_eq!(payout_after_fees(500_000, &fees, 10_000), 0);
    }
}
