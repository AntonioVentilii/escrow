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
    subaccounts::{derive_deal_subaccount, treasury_subaccount},
    types::{
        deal::{Consent, Deal, DealFees, DealId, DealMetadata, DealStatus, Signature},
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

/// Returns the currently-configured anti-spam creation fee. Caller
/// is responsible for zeroing this for tip flows (no bound
/// counterparty to spam). Defaults sourced from
/// [`crate::types::state::Config::default`].
#[must_use]
pub fn load_creation_fee() -> u128 {
    CONFIG.with(|c| c.borrow().creation_fee)
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
///
/// `creation_fee` is passed in by the caller so tip flows can
/// zero it out — there's no bound counterparty to spam, so no
/// deterrent applies. Bound deals pass through the live
/// `Config.creation_fee` snapshot.
#[must_use]
pub fn compute_deal_fees(
    amount: u128,
    escrow_fee: u128,
    creation_fee: u128,
    dispute_cfg: &DisputeConfig,
    ledger_fee: u128,
) -> DealFees {
    let full_dispute_cost = disputes::compute_arbitration_fee(amount, dispute_cfg);
    DealFees {
        escrow_fee,
        dispute_reserve_per_party: full_dispute_cost.div_ceil(2),
        withdraw_fee_pct: dispute_cfg.withdraw_fee_pct,
        ledger_fee_at_create: ledger_fee,
        creation_fee,
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
    // create-time and runtime fees out of `escrow_fee`. A failure
    // to reach the ledger aborts the create (no stuck deals); the
    // `args.asset` ICRC variant must therefore wrap a real ICRC-1
    // canister principal.
    let token_ledger = args.asset.as_icrc()?;
    let escrow_fee = load_escrow_fee();
    let ledger_fee = ledger::fee(token_ledger).await?;
    // Tips (no bound recipient) skip the anti-spam creation_fee —
    // there's no counterparty to harass with spam invitations.
    // Bound deals pay the snapshotted Config.creation_fee per
    // [`DEFAULT_CREATION_FEE`].
    let creation_fee = if args.recipient.is_some() {
        load_creation_fee()
    } else {
        0
    };
    let fees = compute_deal_fees(
        args.amount,
        escrow_fee,
        creation_fee,
        &dispute_cfg,
        ledger_fee,
    );
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

    let asset_for_deal = args.asset.clone();
    let deal = insert_new_deal(|deal_id| Deal {
        id: deal_id,
        payer,
        recipient,
        asset: asset_for_deal,
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
        payer_signature: Signature::Empty,
        recipient_signature: Signature::Empty,
    });

    // Commit-at-first-action: the creator deposits everything
    // they're on the hook for as part of `create_deal` itself.
    // Three cases driven by `(payer, recipient)`:
    //
    // - **Tip** (`recipient = None`): payer is the creator and the only future actor. Pulls `amount
    //   + DC/2` to the deal subaccount; status flips straight to `Funded` (no counterparty to
    //   consent). No `creation_fee` (no spam target).
    // - **3a payer-creator** (caller is the bound payer): pulls `amount + DC/2` to the deal
    //   subaccount + `creation_fee` to the treasury subaccount. Status stays `Created` until the
    //   recipient consents.
    // - **3b recipient-creator** (caller is the bound recipient and not the payer): pulls `DC/2` to
    //   the deal subaccount + `creation_fee` to the treasury subaccount. Status stays `Created`
    //   until the payer consents.
    //
    // On any deposit failure the deal is rolled forward to
    // `Cancelled`, with whatever was already in the subaccount
    // refunded to the creator (so a partial-approval failure doesn't
    // strand funds).
    if try_acquire_lock(deal.id).is_ok() {
        let result =
            execute_create_time_creator_deposit(deal.id, &deal, caller, token_ledger, now).await;
        release_lock(deal.id);
        result?;
    }

    load_deal(deal.id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

/// Pulls everything the creator is on the hook for into the deal
/// subaccount + treasury subaccount, and (for tips) flips the
/// status to `Funded`. Called exclusively from `create` under the
/// per-deal lock.
///
/// On failure the deal is rolled forward to `Cancelled`, with any
/// already-deposited funds refunded to the creator. The
/// `creation_fee` transfer is the LAST step; if it fails after the
/// big deposit succeeded, the big deposit is refunded back. If the
/// big deposit itself fails, no money has moved and the rollback
/// is a pure status flip.
async fn execute_create_time_creator_deposit(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
    token_ledger: Principal,
    now: u64,
) -> Result<(), EscrowError> {
    let creator_account = Account {
        owner: caller,
        subaccount: None,
    };
    let escrow_account = Account {
        owner: canister_self(),
        subaccount: Some(deal.escrow_subaccount.clone()),
    };
    let treasury_account = Account {
        owner: canister_self(),
        subaccount: Some(treasury_subaccount()),
    };

    // Decide what the creator deposits + whether the deal flips to Funded.
    let is_tip = deal.recipient.is_none();
    let caller_is_payer = deal.payer == Some(caller);
    let reserve = deal.fees.dispute_reserve_per_party;
    // - tip:   payer pulls `amount + DC/2` (legacy fee math; tip recipient gets DC/2 on accept).
    // - 3a:    payer (creator) pulls `amount + DC/2`.
    // - 3b:    recipient (creator) pulls just `DC/2`.
    let big_pull = if caller_is_payer {
        deal.amount.saturating_add(reserve)
    } else {
        reserve
    };
    let creation_fee = deal.fees.creation_fee;

    // Step 1 — pull the big amount to the deal subaccount. If this
    // fails (likely cause: under-approval), no money has moved, so
    // rollback is just a status flip.
    if big_pull > 0 {
        let pull_result = ledger::transfer_from(
            token_ledger,
            creator_account.clone(),
            escrow_account.clone(),
            big_pull,
        )
        .await;
        if pull_result.is_err() {
            with_deal(deal_id, |d| {
                d.status = DealStatus::Cancelled;
                d.updated_at_ns = Some(now);
                d.updated_by = Some(caller);
            });
            // The error variant depends on the case: 3a/tip payer
            // not approving the deal amount maps to
            // `DisputeReserveRequired` (legacy variant covering
            // create-time pull failures). 3b recipient already had
            // the same mapping pre-restructure.
            return Err(EscrowError::DisputeReserveRequired);
        }
    }

    // Step 2 — pull creation_fee to the treasury subaccount. Only
    // applies to bound deals (recipient bound). On failure, refund
    // the big deposit and roll to Cancelled.
    if !is_tip && creation_fee > 0 {
        let fee_pull = ledger::transfer_from(
            token_ledger,
            creator_account.clone(),
            treasury_account,
            creation_fee,
        )
        .await;
        if fee_pull.is_err() {
            // Best-effort refund of the big deposit back to the
            // creator. If THIS transfer fails too, the deal stays
            // Cancelled with funds stuck in the subaccount — the
            // controller can drain it manually via the deal
            // subaccount. That's a documented edge case rather
            // than an invariant violation.
            if big_pull > 0 {
                let live_lf = ledger::fee(token_ledger).await.unwrap_or(0);
                let refund = big_pull.saturating_sub(live_lf);
                if refund > 0 {
                    let _ = ledger::transfer(
                        token_ledger,
                        Some(deal.escrow_subaccount.clone()),
                        creator_account,
                        refund,
                    )
                    .await;
                }
            }
            with_deal(deal_id, |d| {
                d.status = DealStatus::Cancelled;
                d.updated_at_ns = Some(now);
                d.updated_by = Some(caller);
            });
            return Err(EscrowError::CreationFeeRequired);
        }
    }

    // All deposits succeeded. For tips, flip directly to Funded —
    // no counterparty needs to consent. For bound deals, status
    // stays `Created` until the counterparty consents (which, when
    // it fires, will auto-flip the deal to `Funded`).
    with_deal(deal_id, |d| {
        d.updated_at_ns = Some(now);
        d.updated_by = Some(caller);
        if is_tip && d.status == DealStatus::Created {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(now);
        }
    });
    Ok(())
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

    // Bound deals route through the two-signature tally: accepting
    // is conceptually the recipient saying "Yes, release to me".
    // The deal only actually settles when the payer has also signed
    // `Yes` (or the auto-YES rule fires at expiry); otherwise the
    // signature is recorded and the deal stays `Funded`.
    // `validate_can_accept` already enforced `caller == bound
    // recipient` for bound deals, so the role is fixed.
    //
    // Tip flows (recipient unbound) keep the legacy unilateral
    // claim + settle: the caller becomes the recipient via
    // `execute_accept` and the funds release immediately. Tips have
    // no payer-side signature concept so no tally applies.
    if deal.recipient.is_some() && deal.payer.is_some() {
        return record_signature_and_dispatch(deal_id, caller, false, Signature::Yes, now).await;
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

    // Bound deals: route through the expiry auto-tally dispatcher so
    // a manual `reclaim_deal` after expiry produces the same outcome
    // as the housekeeping sweep would. The auto-YES rule (silence =
    // release) means the recipient typically wins by default, NOT
    // the payer — a behaviour change from the legacy
    // `reclaim → Refunded` semantics. Without this routing a payer
    // could race the housekeeping sweep and unilaterally refund
    // themselves on a bound deal where the recipient was about to
    // get auto-settled.
    //
    // The dispatch result depends on the signature state:
    //   - both `Empty` → both auto-`Yes` → settle to recipient.
    //   - one party signed `Yes`, other `Empty` → both effective `Yes` → settle.
    //   - one party signed `No`, other `Empty` → mixed → auto-dispute.
    //   - both signed `No` → abort (refund to payer).
    //   - both signed `Yes` → settle.
    //
    // Tips (recipient unbound) keep the legacy unilateral refund
    // since signatures don't apply to tip flows.
    if deal.recipient.is_some() {
        super::expiry::dispatch_one_expired(deal_id, now).await?;
        return load_deal(deal_id)
            .map(|d| DealView::from(&d))
            .ok_or(EscrowError::NotFound);
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

    // Idempotent short-circuit: caller already consented (and, if
    // they were the counterparty, already deposited their
    // obligation at that earlier consent). Both bound parties end
    // up here when the deal is `Funded` — no further consent work
    // is needed. The check covers the wallet-with-stale-allowance
    // attack vector that previously affected receiver-consent: a
    // generous open allowance could otherwise be drained by
    // accidental (UI retry) or malicious repeated calls, each
    // pulling another deposit.
    let already_consented = if is_payer {
        matches!(deal.payer_consent, Consent::Accepted)
    } else {
        matches!(deal.recipient_consent, Consent::Accepted)
    };
    if already_consented {
        return Ok(DealView::from(&deal));
    }

    // First-time counterparty consent now ALSO pulls their money,
    // per the commit-at-first-action design:
    //
    // - 3a recipient consent: pulls `DC/2` from recipient → deal subaccount.
    // - 3b payer consent: pulls `amount + DC/2` from payer → deal subaccount.
    //
    // Once both consents are `Accepted` the status auto-flips from
    // `Created` to `Funded`, putting the deal in the two-signature
    // tally state.
    try_acquire_lock(deal_id)?;
    let result = execute_counterparty_consent(deal_id, &deal, caller, is_payer, now).await;
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

/// Records the caller's settlement signature on a `Funded` bound
/// deal and dispatches the resulting tally:
///
/// - both `Yes` → settle (release to recipient via [`execute_accept`]).
/// - both `No` → abort (refund to payer via [`execute_refund`] with `target_status = Aborted`).
/// - mixed (`Yes` / `No`) → auto-open a dispute via [`disputes::open`].
/// - one signature still `Empty` → no-op; deal stays `Funded` with the new signature recorded.
///
/// The signature itself is set under the per-deal processing lock
/// (so two concurrent `sign` calls on the same deal serialise their
/// writes). Dispatch happens AFTER releasing the lock — each
/// dispatch path re-acquires the lock and either runs an executor
/// directly (settle / abort) or delegates to `disputes::open` (mixed,
/// which manages its own lock). Each path is idempotent if a racing
/// caller already moved the deal to a terminal state. The signature
/// itself is latest-wins while `Funded` — re-signing with a
/// different vote overwrites; once the tally fires the next sign
/// hits `InvalidState`.
///
/// `vote` must be [`Signature::Yes`] or [`Signature::No`]. The
/// public api wrappers (`sign_yes` / `sign_no`) inject the vote, so
/// `Empty` is unreachable from the Candid boundary; passing `Empty`
/// here from internal code would record an `Empty` signature and
/// always tally to `Pending` (no-op).
pub async fn sign(
    caller: Principal,
    deal_id: DealId,
    vote: Signature,
    now: u64,
) -> Result<DealView, EscrowError> {
    let deal = load_deal(deal_id).ok_or(EscrowError::NotFound)?;
    let is_payer = validation::validate_can_sign(&deal, caller, now)?;
    record_signature_and_dispatch(deal_id, caller, is_payer, vote, now).await
}

/// Sets `caller`'s signature on a `Funded` bound deal under the
/// per-deal lock and dispatches the resulting tally. Shared between
/// `sign` (full new flow) and `accept` (legacy entry that routes
/// to `sign(Yes)` for bound deals). Kept private — the caller is
/// expected to have already validated `validate_can_sign` (or the
/// `accept`-equivalent) so this helper trusts its inputs.
///
/// Lock semantics:
/// - Phase 1 holds the per-deal lock briefly to set the signature atomically with a status re-check
///   (defends against a racing terminal flip between caller validation and write).
/// - Phase 2 dispatches via the existing private executors (`execute_accept`, `execute_refund`)
///   under their own re-acquired locks, or via the public `disputes::open` (Mixed) which acquires
///   its own. Each dispatch path is idempotent if a racing dispatcher already moved the deal to a
///   terminal state.
async fn record_signature_and_dispatch(
    deal_id: DealId,
    caller: Principal,
    is_payer: bool,
    vote: Signature,
    now: u64,
) -> Result<DealView, EscrowError> {
    try_acquire_lock(deal_id)?;
    let set_result: Result<(), EscrowError> = with_deal(deal_id, |d| {
        if d.status != DealStatus::Funded {
            return Err(EscrowError::InvalidState {
                expected: "Funded".to_owned(),
                actual: format!("{:?}", d.status),
            });
        }
        if is_payer {
            d.payer_signature = vote.clone();
        } else {
            d.recipient_signature = vote.clone();
        }
        d.updated_at_ns = Some(now);
        d.updated_by = Some(caller);
        Ok(())
    })
    .ok_or(EscrowError::NotFound)?;
    release_lock(deal_id);
    set_result?;

    let updated = load_deal(deal_id).ok_or(EscrowError::NotFound)?;
    if updated.status != DealStatus::Funded {
        return Ok(DealView::from(&updated));
    }
    match validation::tally_signatures(&updated.payer_signature, &updated.recipient_signature) {
        validation::SignatureTally::BothYes => {
            let recipient = updated.recipient.ok_or(EscrowError::NeitherPartySet)?;
            try_acquire_lock(deal_id)?;
            let result = execute_accept(deal_id, &updated, recipient).await;
            release_lock(deal_id);
            result
        }
        validation::SignatureTally::BothNo => {
            try_acquire_lock(deal_id)?;
            let result = execute_refund(deal_id, &updated, caller, now, DealStatus::Aborted).await;
            release_lock(deal_id);
            result
        }
        validation::SignatureTally::Mixed => {
            // Auto-open a dispute. `disputes::open` acquires its own
            // per-deal lock. If opening fails (e.g.
            // `InsufficientArbitrators` / `AmountTooSmallForArbitration`)
            // the signature is still recorded, the deal stays `Funded`,
            // and the caller can retry by signing again or calling
            // `open_dispute` explicitly.
            disputes::open(caller, deal_id, now).await?;
            load_deal(deal_id)
                .map(|d| DealView::from(&d))
                .ok_or(EscrowError::NotFound)
        }
        validation::SignatureTally::Pending => Ok(DealView::from(&updated)),
    }
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

/// Pulls the counterparty's deposit during `consent_deal` and
/// auto-flips status to `Funded` once both parties have consented.
/// Shared between 3a recipient consent and 3b payer consent — the
/// only thing that varies is **what** they deposit:
///
/// - 3a recipient (`is_payer = false`): pulls `DC/2` to the deal subaccount.
/// - 3b payer (`is_payer = true`): pulls `amount + DC/2` to the deal subaccount.
///
/// On ledger failure the deal stays `Created` with the caller's
/// consent still `Pending` so they can retry after fixing approval.
/// Maps the ledger error to `DisputeReserveRequired` (the
/// canonical "create-time / consent-time pull failed" variant).
async fn execute_counterparty_consent(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
    is_payer: bool,
    now: u64,
) -> Result<DealView, EscrowError> {
    let reserve = deal.fees.dispute_reserve_per_party;
    let pull = if is_payer {
        deal.amount.saturating_add(reserve)
    } else {
        reserve
    };

    if pull > 0 {
        let caller_account = Account {
            owner: caller,
            subaccount: None,
        };
        let escrow_account = Account {
            owner: canister_self(),
            subaccount: Some(deal.escrow_subaccount.clone()),
        };
        let token_ledger = deal.asset.as_icrc()?;
        ledger::transfer_from(token_ledger, caller_account, escrow_account, pull)
            .await
            .map_err(|_| EscrowError::DisputeReserveRequired)?;
    }

    // Set this party's consent + auto-flip status to Funded if both
    // consents are now Accepted (which is the normal case for the
    // counterparty's first consent). The funded_at_ns / funding_tx
    // pair is set on the auto-flip for backward audit-trail
    // compatibility — in this commit-at-first-action world there's
    // no single "funding tx" to point at (funding is split across
    // create_deal and consent_deal), so funding_tx is left None.
    with_deal(deal_id, |d| {
        if is_payer {
            d.payer_consent = Consent::Accepted;
        } else {
            d.recipient_consent = Consent::Accepted;
        }
        d.updated_at_ns = Some(now);
        d.updated_by = Some(caller);
        if d.status == DealStatus::Created
            && matches!(d.payer_consent, Consent::Accepted)
            && matches!(d.recipient_consent, Consent::Accepted)
        {
            d.status = DealStatus::Funded;
            d.funded_at_ns = Some(now);
        }
    });

    load_deal(deal_id)
        .map(|d| DealView::from(&d))
        .ok_or(EscrowError::NotFound)
}

pub(crate) async fn execute_accept(
    deal_id: DealId,
    deal: &Deal,
    recipient: Principal,
) -> Result<DealView, EscrowError> {
    // Defensive: re-check the deal's status under the per-deal lock
    // that the caller is expected to be holding. The `deal`
    // snapshot may be stale if the caller acquired the lock AFTER
    // its own `load_deal` (e.g. the sign dispatcher releases its
    // brief sig-write lock and re-acquires for the executor; the
    // expiry sweep snapshots before locking; the legacy
    // `accept` validates pre-lock). Without this guard the ledger
    // fan-out below would fire against the stale `amount` /
    // `fees` even though another caller already finalised the
    // deal — at best the ledger rejects on insufficient subaccount
    // balance, at worst it surfaces a confusing error to a caller
    // whose intent ("settle this deal") is already satisfied.
    // Idempotent return on a non-`Funded` status is the same shape
    // as `validate_can_accept(Settled) → Ok(true)` upstream.
    let still_funded = with_deal(deal_id, |d| d.status == DealStatus::Funded).unwrap_or(false);
    if !still_funded {
        return load_deal(deal_id)
            .map(|d| DealView::from(&d))
            .ok_or(EscrowError::NotFound);
    }

    with_deal(deal_id, |d| {
        if d.recipient.is_none() {
            d.recipient = Some(recipient);
        }
        d.recipient_consent = Consent::Accepted;
    });

    let token_ledger = deal.asset.as_icrc()?;
    let ledger_fee = ledger::fee(token_ledger).await?;
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
        token_ledger,
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
                token_ledger,
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
    let now = time();
    execute_refund(deal_id, deal, caller, now, DealStatus::Refunded).await
}

/// Refunds a `Funded` deal to the payer using the same fee math
/// as `execute_reclaim` and flips the status to `target_status`
/// (one of [`DealStatus::Refunded`] for expiry / payer-reclaim
/// flows, or [`DealStatus::Aborted`] for the mutual-No tally
/// terminal). Per the project constraint "no fee logic changes for
/// the new terminal", `Aborted` and `Refunded` share the entire
/// fan-out: payer gets `amount − EF + DC/2 − LF` combined, recipient
/// gets `DC/2 − LF`, and the operator's `EF` stays in the subaccount.
///
/// Idempotent: a non-`Funded` deal is left unchanged at the final
/// `with_deal` write — the fee math runs once, the status flip
/// fires once.
pub(crate) async fn execute_refund(
    deal_id: DealId,
    deal: &Deal,
    caller: Principal,
    now_ns: u64,
    target_status: DealStatus,
) -> Result<DealView, EscrowError> {
    // Defensive: re-check status under the per-deal lock the caller
    // is expected to be holding — same rationale as `execute_accept`.
    // Without this guard a stale `deal` snapshot reaching here
    // (sign dispatcher's release-then-reacquire pattern, or the
    // expiry sweep's snapshot-before-lock) would fire the ledger
    // fan-out against a deal that another caller already finalised.
    let still_funded = with_deal(deal_id, |d| d.status == DealStatus::Funded).unwrap_or(false);
    if !still_funded {
        return load_deal(deal_id)
            .map(|d| DealView::from(&d))
            .ok_or(EscrowError::NotFound);
    }

    let payer = deal.payer.ok_or(EscrowError::PayerNotSet)?;
    let token_ledger = deal.asset.as_icrc()?;
    let ledger_fee = ledger::fee(token_ledger).await?;
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
        token_ledger,
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
                token_ledger,
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

    with_deal(deal_id, |d| {
        if d.status == DealStatus::Funded {
            d.status = target_status;
            d.refunded_at_ns = Some(now_ns);
            d.refund_tx = Some(refund_tx);
            d.updated_at_ns = Some(now_ns);
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

    // In the commit-at-first-action model, `Consent::Accepted`
    // implies the party has already deposited their obligation:
    //
    // - Payer Accepted: deposited `amount + DC/2` (whether at create_deal as 3a creator, or at
    //   consent_deal as 3b counterparty).
    // - Recipient Accepted: deposited `DC/2` (whether at create_deal as 3b creator, or at
    //   consent_deal as 3a counterparty).
    //
    // Cancel/reject before both have consented (status still
    // `Created`) refunds whatever was actually deposited. The
    // operator does NOT take `escrow_fee` on a pre-funding
    // termination — same rationale as before. The `creation_fee`
    // is already in the treasury subaccount and stays there
    // (forfeited by design).
    let token_ledger = deal.asset.as_icrc()?;
    let needs_payer_refund =
        matches!(deal.payer_consent, Consent::Accepted) && deal.payer.is_some();
    let needs_recipient_refund = matches!(deal.recipient_consent, Consent::Accepted)
        && deal.recipient.is_some()
        && reserve > 0;
    if needs_payer_refund || needs_recipient_refund {
        let ledger_fee = ledger::fee(token_ledger).await?;

        if needs_payer_refund {
            let payer = deal.payer.expect("checked above");
            let refund = deal
                .amount
                .saturating_add(reserve)
                .saturating_sub(ledger_fee);
            if refund > 0 {
                ledger::transfer(
                    token_ledger,
                    Some(deal.escrow_subaccount.clone()),
                    Account {
                        owner: payer,
                        subaccount: None,
                    },
                    refund,
                )
                .await?;
            }
        }

        if needs_recipient_refund {
            let recipient = deal.recipient.expect("checked above");
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
                    token_ledger,
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
        types::{
            asset::Asset,
            deal::{Consent, Deal, DealFees, DealMetadata, DealStatus, Signature},
        },
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
            asset: Asset::Icrc(ledger_principal()),
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
            payer_signature: Signature::Empty,
            recipient_signature: Signature::Empty,
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
                ..Config::default()
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
        let fees = compute_deal_fees(1_000_000, 20_000, 20_000, &cfg, 10_000);
        assert_eq!(fees.escrow_fee, 20_000);
        assert_eq!(fees.dispute_reserve_per_party, 25_000);
        assert_eq!(fees.withdraw_fee_pct, cfg.withdraw_fee_pct);
        assert_eq!(fees.ledger_fee_at_create, 10_000);
        assert_eq!(fees.creation_fee, 20_000);
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
        let fees = compute_deal_fees(1_000, 20_000, 20_000, &cfg, 10_000);
        assert_eq!(fees.dispute_reserve_per_party, 5_000);
    }

    #[test]
    fn compute_deal_fees_zero_creation_fee_preserved() {
        // Tip flow path passes 0 for creation_fee (no spam
        // counterparty to deter). Snapshot must reflect that.
        let cfg = DisputeConfig::default();
        let fees = compute_deal_fees(1_000_000, 20_000, 0, &cfg, 10_000);
        assert_eq!(fees.creation_fee, 0);
    }

    #[test]
    fn payout_after_fees_subtracts_ef_plus_lf() {
        let fees = DealFees {
            escrow_fee: 20_000,
            dispute_reserve_per_party: 5_000,
            withdraw_fee_pct: 25,
            ledger_fee_at_create: 10_000,
            creation_fee: 20_000,
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
            creation_fee: 0,
        };
        // amount < EF + LF → saturating_sub clamps to 0.
        assert_eq!(payout_after_fees(500_000, &fees, 10_000), 0);
    }
}
