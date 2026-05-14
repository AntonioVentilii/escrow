use candid::Principal;

use crate::{
    api::deals::errors::EscrowError,
    memory,
    types::{
        deal::{Consent, Deal, DealFees, DealStatus},
        dispute::DisputeConfig,
        state::Config,
    },
};

const MAX_TITLE_LEN: u32 = 200;
const MAX_NOTE_LEN: u32 = 1000;
const MAX_ACTIVE_DEALS_PER_PRINCIPAL: u32 = 50;

/// ~500 years in nanoseconds — the practical u64 ceiling.
const MAX_EXPIRY_WINDOW_NS: u64 = 500 * 365 * 24 * 60 * 60 * 1_000_000_000;

pub fn validate_caller_deal_limit(caller: Principal) -> Result<(), EscrowError> {
    if memory::count_active_deals_for(caller) >= MAX_ACTIVE_DEALS_PER_PRINCIPAL {
        return Err(EscrowError::TooManyActiveDeals {
            max: MAX_ACTIVE_DEALS_PER_PRINCIPAL,
        });
    }
    Ok(())
}

pub fn validate_create(amount: u128, expires_at_ns: u64, now_ns: u64) -> Result<(), EscrowError> {
    if amount == 0 {
        return Err(EscrowError::InvalidAmount);
    }
    if expires_at_ns <= now_ns {
        return Err(EscrowError::InvalidExpiry);
    }
    if expires_at_ns.saturating_sub(now_ns) > MAX_EXPIRY_WINDOW_NS {
        return Err(EscrowError::ExpiryTooFar);
    }
    Ok(())
}

/// Returns the strict floor `amount` must exceed to be viable across
/// every terminal path: happy-path settle/refund AND a dispute opened
/// with the deal's locked panel size.
///
/// Formula: `floor = max(happy, dispute)` where
/// - `happy = escrow_fee + ledger_fee` (settle/refund pays one outgoing ledger fee and retains
///   `escrow_fee` in the subaccount).
/// - `dispute = 2 * dispute_reserve_per_party + (panel_size + 1) * ledger_fee` (panel fan-out is
///   one ledger fee per arbitrator plus one for the prevailing-party transfer; matches the existing
///   `open_dispute` headroom check in `services::disputes`).
///
/// `amount > floor` is required — a deal at exactly the floor would
/// leave zero remainder for the recipient or the prevailing party.
#[must_use]
pub fn compute_min_viable_amount(fees: &DealFees, ledger_fee: u128, panel_size: u32) -> u128 {
    let happy = fees.escrow_fee.saturating_add(ledger_fee);
    let dispute = fees
        .dispute_reserve_per_party
        .saturating_mul(2)
        .saturating_add(ledger_fee.saturating_mul(u128::from(panel_size).saturating_add(1)));
    happy.max(dispute)
}

/// Rejects a `create_deal` whose `amount` is at or below the computed
/// floor. Returns `EscrowError::AmountBelowMinimum { min }` carrying
/// the **smallest acceptable amount** (i.e. `floor + 1`), matching the
/// convention of `AmountTooSmallForArbitration` so callers can retry
/// with the reported value directly.
pub fn validate_min_amount(
    amount: u128,
    fees: &DealFees,
    ledger_fee: u128,
    panel_size: u32,
) -> Result<(), EscrowError> {
    let floor = compute_min_viable_amount(fees, ledger_fee, panel_size);
    if amount <= floor {
        return Err(EscrowError::AmountBelowMinimum {
            min: floor.saturating_add(1),
        });
    }
    Ok(())
}

/// Max byte length of an evidence note.
pub const MAX_EVIDENCE_NOTE_LEN: u32 = 4096;
/// Max byte length of an evidence artefact URL.
pub const MAX_EVIDENCE_URL_LEN: u32 = 2048;
/// SHA-256 length in bytes — invariant for evidence artefact hashes.
pub const SHA256_LEN: usize = 32;

/// Maximum allowed evidence/voting window. `validate_dispute_config`
/// rejects values above this limit so admin can't accidentally make
/// a dispute unclosable by setting a year-long window.
const MAX_DISPUTE_WINDOW_NS: u64 = 30 * 24 * 60 * 60 * 1_000_000_000;

/// Validates a `DisputeConfig` against the invariants documented on
/// the type's fields. Called from `update_config` before persisting
/// a controller-supplied config so the dispute machinery can rely on
/// the invariants at runtime.
///
/// Invariants enforced:
/// - `panel_size >= 3` and `panel_size % 2 == 1` (odd-only — the tally rules assume no tie is
///   possible without an abstention).
/// - `evidence_window_ns > 0` and `voting_window_ns > 0` (zero windows make deadlines pass
///   instantly, leaving no time for submissions / votes).
/// - Both windows `<= MAX_DISPUTE_WINDOW_NS` (~30 days; protects against admin foot-guns).
/// - `arbitration_fee_bps <= 10_000` (100% — anything higher would take more than the disputed
///   amount in fees).
/// - `withdraw_fee_pct <= 100` (percentage; values above 100 would pay arbitrators more than the
///   full arbitration fee).
pub fn validate_dispute_config(cfg: &DisputeConfig) -> Result<(), EscrowError> {
    if cfg.panel_size < 3 {
        return Err(EscrowError::ValidationError(format!(
            "panel_size must be >= 3, got {}",
            cfg.panel_size,
        )));
    }
    if cfg.panel_size.is_multiple_of(2) {
        return Err(EscrowError::ValidationError(format!(
            "panel_size must be odd (no tie possible without abstention), got {}",
            cfg.panel_size,
        )));
    }
    if cfg.min_panel_size < 3 {
        return Err(EscrowError::ValidationError(format!(
            "min_panel_size must be >= 3, got {}",
            cfg.min_panel_size,
        )));
    }
    if cfg.min_panel_size.is_multiple_of(2) {
        return Err(EscrowError::ValidationError(format!(
            "min_panel_size must be odd, got {}",
            cfg.min_panel_size,
        )));
    }
    if cfg.max_panel_size < cfg.min_panel_size {
        return Err(EscrowError::ValidationError(format!(
            "max_panel_size ({}) must be >= min_panel_size ({})",
            cfg.max_panel_size, cfg.min_panel_size,
        )));
    }
    if cfg.max_panel_size.is_multiple_of(2) {
        return Err(EscrowError::ValidationError(format!(
            "max_panel_size must be odd, got {}",
            cfg.max_panel_size,
        )));
    }
    if cfg.panel_size < cfg.min_panel_size || cfg.panel_size > cfg.max_panel_size {
        return Err(EscrowError::ValidationError(format!(
            "panel_size ({}) must be within [min_panel_size ({}), max_panel_size ({})]",
            cfg.panel_size, cfg.min_panel_size, cfg.max_panel_size,
        )));
    }
    if cfg.evidence_window_ns == 0 {
        return Err(EscrowError::ValidationError(
            "evidence_window_ns must be > 0".to_owned(),
        ));
    }
    if cfg.voting_window_ns == 0 {
        return Err(EscrowError::ValidationError(
            "voting_window_ns must be > 0".to_owned(),
        ));
    }
    if cfg.evidence_window_ns > MAX_DISPUTE_WINDOW_NS {
        return Err(EscrowError::ValidationError(format!(
            "evidence_window_ns must be <= {MAX_DISPUTE_WINDOW_NS} (~30 days), got {}",
            cfg.evidence_window_ns,
        )));
    }
    if cfg.voting_window_ns > MAX_DISPUTE_WINDOW_NS {
        return Err(EscrowError::ValidationError(format!(
            "voting_window_ns must be <= {MAX_DISPUTE_WINDOW_NS} (~30 days), got {}",
            cfg.voting_window_ns,
        )));
    }
    if cfg.arbitration_fee_bps > 10_000 {
        return Err(EscrowError::ValidationError(format!(
            "arbitration_fee_bps must be <= 10_000 (100%), got {}",
            cfg.arbitration_fee_bps,
        )));
    }
    if cfg.withdraw_fee_pct > 100 {
        return Err(EscrowError::ValidationError(format!(
            "withdraw_fee_pct must be <= 100, got {}",
            cfg.withdraw_fee_pct,
        )));
    }
    Ok(())
}

/// Validates a top-level [`Config`]. Delegates to
/// [`validate_dispute_config`] for the nested struct.
pub fn validate_config(cfg: &Config) -> Result<(), EscrowError> {
    validate_dispute_config(&cfg.dispute_config)
}

/// Validates a deal creator's per-deal `panel_size` choice against
/// the active `DisputeConfig` bounds.
///
/// `None` is always valid — it means "use whatever
/// `DisputeConfig.panel_size` is current at `open_dispute` time".
///
/// `Some(n)` requires `n` to be:
/// - Odd (no tie possible without an abstention; tally rules require this).
/// - Within `[cfg.min_panel_size, cfg.max_panel_size]`.
///
/// Out-of-range / even values return
/// `EscrowError::PanelSizeOutOfRange { min, max, got }` carrying the
/// active range AND the offending value so the caller can render the
/// rejection (and surface logs) without correlating with the request
/// payload. Both kinds of violation use the same variant — clients
/// can distinguish them by checking whether `got` is in `[min, max]`
/// (even-in-range) or outside (out-of-range).
pub fn validate_panel_size_choice(
    panel_size: Option<u32>,
    cfg: &DisputeConfig,
) -> Result<(), EscrowError> {
    let Some(n) = panel_size else {
        return Ok(());
    };
    let in_range = n >= cfg.min_panel_size && n <= cfg.max_panel_size;
    if !in_range || n.is_multiple_of(2) {
        return Err(EscrowError::PanelSizeOutOfRange {
            min: cfg.min_panel_size,
            max: cfg.max_panel_size,
            got: n,
        });
    }
    Ok(())
}

/// Validates that `principal` is a legal target for arbitrator
/// registration. Rejects:
/// - The anonymous principal — degenerate; cannot vote anyway.
/// - The canister's own principal — would create circular self-arbitration risks if the canister
///   ever became its own caller via timer-driven flows.
pub fn validate_arbitrator_principal(
    principal: Principal,
    canister_id: Principal,
) -> Result<(), EscrowError> {
    if principal == Principal::anonymous() {
        return Err(EscrowError::AnonymousParty);
    }
    if principal == canister_id {
        return Err(EscrowError::ValidationError(
            "cannot register the canister's own principal as an arbitrator".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_metadata(title: Option<&str>, note: Option<&str>) -> Result<(), EscrowError> {
    if let Some(t) = title {
        if t.len() > MAX_TITLE_LEN as usize {
            return Err(EscrowError::MetadataTooLong {
                field: "title".to_owned(),
                max: MAX_TITLE_LEN,
            });
        }
    }
    if let Some(n) = note {
        if n.len() > MAX_NOTE_LEN as usize {
            return Err(EscrowError::MetadataTooLong {
                field: "note".to_owned(),
                max: MAX_NOTE_LEN,
            });
        }
    }
    Ok(())
}

/// Resolves payer/recipient and their initial consent from the caller and
/// supplied args.
///
/// Rules:
/// - `payer` and `recipient` may both be `None`; in that case the caller defaults to payer and the
///   recipient remains unset.
/// - The caller must be one of the resolved parties.
/// - The caller's consent is `Accepted`; the counterparty's is `Pending`.
pub fn resolve_parties(
    caller: Principal,
    payer: Option<Principal>,
    recipient: Option<Principal>,
) -> Result<(Option<Principal>, Option<Principal>, Consent, Consent), EscrowError> {
    if payer.is_some_and(|p| p == Principal::anonymous())
        || recipient.is_some_and(|r| r == Principal::anonymous())
    {
        return Err(EscrowError::AnonymousParty);
    }

    let (payer, recipient) = match (payer, recipient) {
        (None, None) => (Some(caller), None),
        (None, Some(r)) if r == caller => (None, Some(caller)),
        (None, Some(r)) => (Some(caller), Some(r)),
        (Some(p), None) if p == caller => (Some(caller), None),
        (Some(p), None) => (Some(p), Some(caller)),
        (Some(p), Some(r)) => (Some(p), Some(r)),
    };

    if let (Some(p), Some(r)) = (payer, recipient) {
        if p == r {
            return Err(EscrowError::SelfDeal);
        }
    }

    let caller_is_payer = payer == Some(caller);
    let caller_is_recipient = recipient == Some(caller);

    if !caller_is_payer && !caller_is_recipient {
        return Err(EscrowError::NotAuthorised);
    }

    let payer_consent = if caller_is_payer {
        Consent::Accepted
    } else {
        Consent::Pending
    };
    let recipient_consent = if caller_is_recipient {
        Consent::Accepted
    } else {
        Consent::Pending
    };

    Ok((payer, recipient, payer_consent, recipient_consent))
}

/// Returns `true` if the deal is already funded (idempotent success).
/// Returns `Err` if funding is not allowed.
///
/// Funding requires:
/// - Caller is the payer (or becomes the payer for open-payer deals).
/// - Payer consent is `Accepted` (auto-set by funding).
/// - If a recipient is bound, their consent must be `Accepted`.
pub fn validate_can_fund(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    if let Some(p) = deal.payer {
        if p != caller {
            return Err(EscrowError::NotAuthorised);
        }
    }

    match deal.status {
        DealStatus::Created => {}
        DealStatus::Funded | DealStatus::Settled => return Ok(true),
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Created".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    if deal.recipient.is_some() && deal.recipient_consent != Consent::Accepted {
        return Err(EscrowError::ConsentRequired);
    }

    Ok(false)
}

/// Returns `true` if the deal is already settled (idempotent success).
/// Returns `Err` if acceptance is not allowed.
///
/// Acceptance requires:
/// - Deal is `Funded` and not expired.
/// - If a recipient is bound, the caller must match.
/// - For open deals (no bound recipient), a valid claim code is required.
pub fn validate_can_accept(
    deal: &Deal,
    caller: Principal,
    now_ns: u64,
    claim_code: Option<&str>,
) -> Result<bool, EscrowError> {
    match deal.status {
        DealStatus::Settled => return Ok(true),
        DealStatus::Funded => {}
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Funded".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    if deal.expires_at_ns <= now_ns {
        return Err(EscrowError::Expired);
    }

    if let Some(bound) = deal.recipient {
        if bound != caller {
            return Err(EscrowError::RecipientMismatch);
        }
    } else {
        let expected = deal
            .claim_code
            .as_ref()
            .ok_or(EscrowError::ValidationError(
                "Deal has no claim code".to_owned(),
            ))?;
        let provided = claim_code.ok_or(EscrowError::MissingClaimCode)?;
        if provided != expected.as_str() {
            return Err(EscrowError::InvalidClaimCode);
        }
    }

    Ok(false)
}

/// Returns `true` if the deal is already refunded (idempotent success).
/// Returns `Err` if reclaim is not allowed.
pub fn validate_can_reclaim(
    deal: &Deal,
    caller: Principal,
    now_ns: u64,
) -> Result<bool, EscrowError> {
    match deal.payer {
        Some(p) if p != caller => return Err(EscrowError::NotAuthorised),
        None => return Err(EscrowError::PayerNotSet),
        Some(_) => {}
    }
    match deal.status {
        DealStatus::Refunded => return Ok(true),
        DealStatus::Funded => {}
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Funded".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    if deal.expires_at_ns > now_ns {
        return Err(EscrowError::NotExpired);
    }

    Ok(false)
}

pub fn validate_can_cancel(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    let caller_is_party = deal.payer == Some(caller) || deal.recipient == Some(caller);
    if !caller_is_party {
        return Err(EscrowError::NotAuthorised);
    }
    match deal.status {
        DealStatus::Created => Ok(false),
        DealStatus::Cancelled => Ok(true),
        _ => Err(EscrowError::InvalidState {
            expected: "Created".to_owned(),
            actual: format!("{:?}", deal.status),
        }),
    }
}

/// Validates that the caller can consent to a deal.
///
/// Consent is allowed in `Created` or `Funded` states (a counterparty may
/// consent after the other party has already funded).
///
/// Returns the caller's role: `true` if payer, `false` if recipient.
pub fn validate_can_consent(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    match deal.status {
        DealStatus::Created | DealStatus::Funded => {}
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Created or Funded".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    resolve_caller_role(deal, caller)
}

/// Validates that the caller can reject a deal.
///
/// Rejection is only allowed in the `Created` state — once funds are in
/// escrow, the deal must be settled or refunded, not rejected.
///
/// Returns the caller's role: `true` if payer, `false` if recipient.
pub fn validate_can_reject(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    match deal.status {
        DealStatus::Created => {}
        DealStatus::Rejected => {
            return Err(EscrowError::AlreadyFinalised);
        }
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Created".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    resolve_caller_role(deal, caller)
}

/// Validates a single evidence submission at the canister boundary:
/// at least one of `note` / `(artefact_url + artefact_sha256)` present;
/// URL and hash paired; size + length caps.
pub fn validate_evidence(
    note: Option<&str>,
    artefact_url: Option<&str>,
    artefact_sha256: Option<&[u8]>,
) -> Result<(), EscrowError> {
    if note.is_none() && artefact_url.is_none() && artefact_sha256.is_none() {
        return Err(EscrowError::ValidationError(
            "evidence must contain at least a note or an artefact".to_owned(),
        ));
    }

    if artefact_url.is_some() != artefact_sha256.is_some() {
        return Err(EscrowError::ValidationError(
            "artefact_url and artefact_sha256 must be supplied together".to_owned(),
        ));
    }

    if let Some(n) = note {
        if n.len() > MAX_EVIDENCE_NOTE_LEN as usize {
            return Err(EscrowError::EvidenceTooLarge {
                max: MAX_EVIDENCE_NOTE_LEN,
            });
        }
    }

    if let Some(url) = artefact_url {
        if url.len() > MAX_EVIDENCE_URL_LEN as usize {
            // Use the typed `EvidenceTooLarge` variant so callers can
            // pattern-match all evidence size violations uniformly
            // (note overflow uses the same variant). The `max` field
            // tells the caller WHICH limit was breached without having
            // to parse a free-form message.
            return Err(EscrowError::EvidenceTooLarge {
                max: MAX_EVIDENCE_URL_LEN,
            });
        }
    }

    if let Some(hash) = artefact_sha256 {
        if hash.len() != SHA256_LEN {
            return Err(EscrowError::ValidationError(format!(
                "artefact_sha256 must be exactly {SHA256_LEN} bytes",
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Dispute validators
// ---------------------------------------------------------------------------

/// Returns `true` if the deal already has an open dispute (idempotent
/// success — the caller short-circuits and returns the existing dispute
/// view). Returns `Err` if opening a dispute is not allowed.
///
/// `open_dispute` is allowed when:
/// - The deal exists in `Funded` state.
/// - Both `payer` and `recipient` are bound — tip flows (open recipient) are not disputable, since
///   there's no bound counterparty in canister state.
/// - The caller is `payer` or `recipient` — either bound party can open a dispute (symmetric).
/// - The deal has not yet expired.
/// - No dispute is already attached to the deal.
pub fn validate_can_open_dispute(
    deal: &Deal,
    caller: Principal,
    now_ns: u64,
) -> Result<bool, EscrowError> {
    if deal.recipient.is_none() || deal.payer.is_none() {
        return Err(EscrowError::DisputeRequiresBoundRecipient);
    }

    // Caller authorization — payer or recipient only.
    let caller_is_party = deal.payer == Some(caller) || deal.recipient == Some(caller);
    if !caller_is_party {
        return Err(EscrowError::NotAuthorised);
    }

    match deal.status {
        // Idempotent success — short-circuit at the service layer.
        DealStatus::Disputed => return Ok(true),
        DealStatus::Funded => {}
        _ => {
            return Err(EscrowError::InvalidState {
                expected: "Funded".to_owned(),
                actual: format!("{:?}", deal.status),
            })
        }
    }

    if deal.dispute.is_some() {
        return Err(EscrowError::DisputeAlreadyExists);
    }

    // Expiry-at-open check: the auto-refund sweep skips Disputed deals,
    // so we must not let a dispute open after the expiry-claim window
    // has already closed in the recipient's favour.
    if deal.expires_at_ns <= now_ns {
        return Err(EscrowError::Expired);
    }

    Ok(false)
}

fn resolve_caller_role(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    if deal.payer == Some(caller) {
        Ok(true)
    } else if deal.recipient == Some(caller) {
        Ok(false)
    } else {
        Err(EscrowError::NotAuthorised)
    }
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{
        resolve_parties, validate_caller_deal_limit, validate_can_accept, validate_can_cancel,
        validate_can_consent, validate_can_fund, validate_can_reclaim, validate_can_reject,
        validate_create, validate_metadata, MAX_ACTIVE_DEALS_PER_PRINCIPAL, MAX_EXPIRY_WINDOW_NS,
    };
    use crate::{
        api::deals::errors::EscrowError,
        memory::{insert_new_deal, with_deal, with_deals},
        subaccounts::derive_deal_subaccount,
        types::deal::{Consent, Deal, DealMetadata, DealStatus},
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_deal(
        status: DealStatus,
        payer: Option<Principal>,
        recipient: Option<Principal>,
    ) -> Deal {
        Deal {
            id: 1,
            payer,
            recipient,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: payer.or(recipient).unwrap_or(test_principal(1)),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 200,
            status,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: Some("test-code-abc".to_owned()),
            payer_consent: if payer.is_some() {
                Consent::Accepted
            } else {
                Consent::Pending
            },
            recipient_consent: if recipient.is_some() {
                Consent::Accepted
            } else {
                Consent::Pending
            },
            metadata: Some(DealMetadata {
                title: None,
                note: None,
            }),
            dispute: None,
            panel_size: None,
            fees: DealFees::default(),
        }
    }

    // --- resolve_parties ---

    #[test]
    fn resolve_defaults_caller_to_payer() {
        let caller = test_principal(1);
        let (payer, recipient, pc, rc) = resolve_parties(caller, None, None).unwrap();
        assert_eq!(payer, Some(caller));
        assert_eq!(recipient, None);
        assert_eq!(pc, Consent::Accepted);
        assert_eq!(rc, Consent::Pending);
    }

    #[test]
    fn resolve_caller_is_payer_with_recipient() {
        let caller = test_principal(1);
        let recip = test_principal(2);
        let (payer, recipient, pc, rc) = resolve_parties(caller, None, Some(recip)).unwrap();
        assert_eq!(payer, Some(caller));
        assert_eq!(recipient, Some(recip));
        assert_eq!(pc, Consent::Accepted);
        assert_eq!(rc, Consent::Pending);
    }

    #[test]
    fn resolve_caller_is_recipient_invoice() {
        let caller = test_principal(2);
        let pay = test_principal(1);
        let (payer, recipient, pc, rc) = resolve_parties(caller, Some(pay), None).unwrap();
        assert_eq!(payer, Some(pay));
        assert_eq!(recipient, Some(caller));
        assert_eq!(pc, Consent::Pending);
        assert_eq!(rc, Consent::Accepted);
    }

    #[test]
    fn resolve_caller_is_explicit_recipient() {
        let caller = test_principal(2);
        let (payer, recipient, pc, rc) = resolve_parties(caller, None, Some(caller)).unwrap();
        assert_eq!(payer, None);
        assert_eq!(recipient, Some(caller));
        assert_eq!(pc, Consent::Pending);
        assert_eq!(rc, Consent::Accepted);
    }

    #[test]
    fn resolve_rejects_unrelated_caller() {
        let caller = test_principal(3);
        let result = resolve_parties(caller, Some(test_principal(1)), Some(test_principal(2)));
        assert!(matches!(result, Err(EscrowError::NotAuthorised)));
    }

    // --- create ---

    #[test]
    fn create_rejects_zero_amount() {
        assert!(validate_create(0, 200, 100).is_err());
    }

    #[test]
    fn create_rejects_past_expiry() {
        assert!(validate_create(100, 50, 100).is_err());
    }

    #[test]
    fn create_accepts_valid_input() {
        assert!(validate_create(100, 200, 100).is_ok());
    }

    // --- fund ---

    #[test]
    fn fund_ok_when_created() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(!validate_can_fund(&deal, payer).unwrap());
    }

    #[test]
    fn fund_idempotent_when_funded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(validate_can_fund(&deal, payer).unwrap());
    }

    #[test]
    fn fund_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(validate_can_fund(&deal, other).is_err());
    }

    #[test]
    fn fund_rejects_refunded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Refunded, Some(payer), None);
        assert!(validate_can_fund(&deal, payer).is_err());
    }

    #[test]
    fn fund_requires_recipient_consent_when_bound() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let mut deal = make_deal(DealStatus::Created, Some(payer), Some(recip));
        deal.recipient_consent = Consent::Pending;
        assert!(matches!(
            validate_can_fund(&deal, payer),
            Err(EscrowError::ConsentRequired)
        ));
    }

    #[test]
    fn fund_ok_when_recipient_consented() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let mut deal = make_deal(DealStatus::Created, Some(payer), Some(recip));
        deal.recipient_consent = Consent::Accepted;
        assert!(!validate_can_fund(&deal, payer).unwrap());
    }

    // --- accept ---

    #[test]
    fn accept_ok_when_funded_and_not_expired_with_bound_recipient() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), Some(recip));
        assert!(!validate_can_accept(&deal, recip, 150, None).unwrap());
    }

    #[test]
    fn accept_ok_for_open_deal_with_valid_claim_code() {
        let payer = test_principal(1);
        let claimer = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(!validate_can_accept(&deal, claimer, 150, Some("test-code-abc")).unwrap());
    }

    #[test]
    fn accept_rejects_open_deal_without_claim_code() {
        let payer = test_principal(1);
        let claimer = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(matches!(
            validate_can_accept(&deal, claimer, 150, None),
            Err(EscrowError::MissingClaimCode)
        ));
    }

    #[test]
    fn accept_rejects_open_deal_with_wrong_claim_code() {
        let payer = test_principal(1);
        let claimer = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(matches!(
            validate_can_accept(&deal, claimer, 150, Some("wrong-code")),
            Err(EscrowError::InvalidClaimCode)
        ));
    }

    #[test]
    fn accept_rejects_expired() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), Some(recip));
        assert!(validate_can_accept(&deal, recip, 300, None).is_err());
    }

    #[test]
    fn accept_rejects_wrong_recipient() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let other = test_principal(3);
        let deal = make_deal(DealStatus::Funded, Some(payer), Some(recip));
        assert!(matches!(
            validate_can_accept(&deal, other, 150, None),
            Err(EscrowError::RecipientMismatch)
        ));
    }

    #[test]
    fn accept_idempotent_when_settled() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Settled, Some(payer), None);
        assert!(validate_can_accept(&deal, payer, 150, None).unwrap());
    }

    #[test]
    fn accept_rejects_created_deal() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(matches!(
            validate_can_accept(&deal, payer, 150, None),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    // --- reclaim ---

    #[test]
    fn reclaim_ok_when_funded_and_expired() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(!validate_can_reclaim(&deal, payer, 300).unwrap());
    }

    #[test]
    fn reclaim_rejects_before_expiry() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(matches!(
            validate_can_reclaim(&deal, payer, 150),
            Err(EscrowError::NotExpired)
        ));
    }

    #[test]
    fn reclaim_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(validate_can_reclaim(&deal, other, 300).is_err());
    }

    #[test]
    fn reclaim_idempotent_when_refunded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Refunded, Some(payer), None);
        assert!(validate_can_reclaim(&deal, payer, 300).unwrap());
    }

    #[test]
    fn reclaim_rejects_settled_deal() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Settled, Some(payer), None);
        assert!(matches!(
            validate_can_reclaim(&deal, payer, 300),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    // --- cancel ---

    #[test]
    fn cancel_ok_when_created() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(!validate_can_cancel(&deal, payer).unwrap());
    }

    #[test]
    fn cancel_rejects_funded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert!(validate_can_cancel(&deal, payer).is_err());
    }

    // --- consent ---

    #[test]
    fn consent_ok_for_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        let is_payer = validate_can_consent(&deal, payer).unwrap();
        assert!(is_payer);
    }

    #[test]
    fn consent_ok_for_recipient() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Created, Some(payer), Some(recip));
        let is_payer = validate_can_consent(&deal, recip).unwrap();
        assert!(!is_payer);
    }

    #[test]
    fn consent_rejects_stranger() {
        let payer = test_principal(1);
        let stranger = test_principal(3);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(matches!(
            validate_can_consent(&deal, stranger),
            Err(EscrowError::NotAuthorised)
        ));
    }

    #[test]
    fn consent_rejects_settled_deal() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Settled, Some(payer), None);
        assert!(matches!(
            validate_can_consent(&deal, payer),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    // --- reject ---

    #[test]
    fn reject_ok_for_created_deal() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Created, Some(payer), Some(recip));
        let is_payer = validate_can_reject(&deal, recip).unwrap();
        assert!(!is_payer);
    }

    #[test]
    fn reject_not_allowed_on_funded_deal() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Funded, Some(payer), Some(recip));
        assert!(matches!(
            validate_can_reject(&deal, recip),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    #[test]
    fn reject_idempotent_returns_already_finalised() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Rejected, Some(payer), Some(recip));
        assert!(matches!(
            validate_can_reject(&deal, recip),
            Err(EscrowError::AlreadyFinalised)
        ));
    }

    #[test]
    fn reject_rejects_stranger() {
        let payer = test_principal(1);
        let stranger = test_principal(3);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert!(matches!(
            validate_can_reject(&deal, stranger),
            Err(EscrowError::NotAuthorised)
        ));
    }

    // --- self-deal ---

    #[test]
    fn resolve_rejects_self_deal() {
        let caller = test_principal(1);
        let result = resolve_parties(caller, Some(caller), Some(caller));
        assert!(matches!(result, Err(EscrowError::SelfDeal)));
    }

    // --- anonymous party ---

    #[test]
    fn resolve_rejects_anonymous_payer() {
        let caller = test_principal(1);
        let result = resolve_parties(caller, Some(Principal::anonymous()), None);
        assert!(matches!(result, Err(EscrowError::AnonymousParty)));
    }

    #[test]
    fn resolve_rejects_anonymous_recipient() {
        let caller = test_principal(1);
        let result = resolve_parties(caller, None, Some(Principal::anonymous()));
        assert!(matches!(result, Err(EscrowError::AnonymousParty)));
    }

    // --- metadata limits ---

    #[test]
    fn metadata_accepts_valid_input() {
        assert!(validate_metadata(Some("Short title"), Some("A note")).is_ok());
    }

    #[test]
    fn metadata_accepts_none() {
        assert!(validate_metadata(None, None).is_ok());
    }

    #[test]
    fn metadata_rejects_long_title() {
        let long_title = "x".repeat(201);
        assert!(matches!(
            validate_metadata(Some(&long_title), None),
            Err(EscrowError::MetadataTooLong { field, max }) if field == "title" && max == 200
        ));
    }

    #[test]
    fn metadata_rejects_long_note() {
        let long_note = "x".repeat(1001);
        assert!(matches!(
            validate_metadata(None, Some(&long_note)),
            Err(EscrowError::MetadataTooLong { field, max }) if field == "note" && max == 1000
        ));
    }

    // --- expiry window ---

    #[test]
    fn create_rejects_expiry_too_far() {
        let now = 100;
        let too_far = now + MAX_EXPIRY_WINDOW_NS + 1;
        assert!(matches!(
            validate_create(100, too_far, now),
            Err(EscrowError::ExpiryTooFar)
        ));
    }

    #[test]
    fn create_accepts_expiry_at_max_window() {
        let now = 100;
        let at_limit = now + MAX_EXPIRY_WINDOW_NS;
        assert!(validate_create(100, at_limit, now).is_ok());
    }

    // --- active deal cap ---

    fn store_active_deal(creator: Principal) {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer: Some(creator),
            recipient: None,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: creator,
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 200,
            status: DealStatus::Created,
            escrow_subaccount: derive_deal_subaccount(deal_id),
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: None,
            dispute: None,
            panel_size: None,
            fees: DealFees::default(),
        });
    }

    #[test]
    fn deal_limit_allows_under_cap() {
        let creator = test_principal(200);
        store_active_deal(creator);
        assert!(validate_caller_deal_limit(creator).is_ok());
    }

    #[test]
    fn deal_limit_rejects_at_cap() {
        let creator = test_principal(201);
        for _ in 0..MAX_ACTIVE_DEALS_PER_PRINCIPAL {
            store_active_deal(creator);
        }
        assert!(matches!(
            validate_caller_deal_limit(creator),
            Err(EscrowError::TooManyActiveDeals { max }) if max == MAX_ACTIVE_DEALS_PER_PRINCIPAL
        ));
    }

    #[test]
    fn deal_limit_does_not_count_terminal_deals() {
        let creator = test_principal(202);
        for _ in 0..MAX_ACTIVE_DEALS_PER_PRINCIPAL {
            store_active_deal(creator);
        }
        assert!(validate_caller_deal_limit(creator).is_err());

        let first_id = with_deals(|deals| {
            deals
                .values()
                .find(|d| d.created_by == creator && d.status == DealStatus::Created)
                .unwrap()
                .id
        });
        with_deal(first_id, |d| d.status = DealStatus::Cancelled);
        assert!(validate_caller_deal_limit(creator).is_ok());
    }

    // --- validate_dispute_config ---

    use super::{validate_dispute_config, MAX_DISPUTE_WINDOW_NS};
    use crate::types::dispute::DisputeConfig;

    fn validation_err_msg(cfg: &DisputeConfig) -> String {
        match validate_dispute_config(cfg).unwrap_err() {
            EscrowError::ValidationError(msg) => msg,
            other => panic!("expected ValidationError, got: {other:?}"),
        }
    }

    #[test]
    fn dispute_config_default_is_valid() {
        assert!(validate_dispute_config(&DisputeConfig::default()).is_ok());
    }

    #[test]
    fn dispute_config_rejects_panel_size_below_3() {
        for size in [0_u32, 1, 2] {
            let cfg = DisputeConfig {
                panel_size: size,
                ..DisputeConfig::default()
            };
            let msg = validation_err_msg(&cfg);
            assert!(
                msg.contains("panel_size") && msg.contains(">= 3"),
                "size={size}, msg={msg}",
            );
        }
    }

    #[test]
    fn dispute_config_rejects_even_panel_size() {
        // panel_size = 4 passes the >= 3 check but fails the odd check.
        let cfg = DisputeConfig {
            panel_size: 4,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("odd"), "msg={msg}");
    }

    #[test]
    fn dispute_config_accepts_odd_panel_sizes() {
        // For each test value, bump max_panel_size so the
        // panel_size <= max_panel_size invariant doesn't reject the
        // higher cases. The Q6 revisit added the bounds-check.
        for size in [3_u32, 5, 7, 9, 99] {
            let cfg = DisputeConfig {
                panel_size: size,
                min_panel_size: 3,
                max_panel_size: size,
                ..DisputeConfig::default()
            };
            assert!(validate_dispute_config(&cfg).is_ok(), "size={size}");
        }
    }

    #[test]
    fn dispute_config_rejects_zero_windows() {
        let cfg = DisputeConfig {
            evidence_window_ns: 0,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("evidence_window_ns"), "msg={msg}");

        let cfg = DisputeConfig {
            voting_window_ns: 0,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("voting_window_ns"), "msg={msg}");
    }

    #[test]
    fn dispute_config_rejects_oversized_windows() {
        let bad = MAX_DISPUTE_WINDOW_NS + 1;
        let cfg = DisputeConfig {
            evidence_window_ns: bad,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("evidence_window_ns"), "msg={msg}");
        // Error includes the offending value so controller-side
        // debugging doesn't have to guess what was sent.
        assert!(
            msg.contains(&bad.to_string()),
            "expected offending value in msg: {msg}",
        );

        let cfg = DisputeConfig {
            voting_window_ns: bad,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("voting_window_ns"), "msg={msg}");
        assert!(
            msg.contains(&bad.to_string()),
            "expected offending value in msg: {msg}",
        );
    }

    #[test]
    fn dispute_config_rejects_fee_bps_above_100_pct() {
        let cfg = DisputeConfig {
            arbitration_fee_bps: 10_001,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("arbitration_fee_bps"), "msg={msg}");
    }

    #[test]
    fn dispute_config_rejects_withdraw_fee_pct_above_100() {
        let cfg = DisputeConfig {
            withdraw_fee_pct: 101,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("withdraw_fee_pct"), "msg={msg}");
    }

    #[test]
    fn dispute_config_accepts_withdraw_fee_pct_at_boundaries() {
        for pct in [0_u32, 50, 100] {
            let cfg = DisputeConfig {
                withdraw_fee_pct: pct,
                ..DisputeConfig::default()
            };
            assert!(validate_dispute_config(&cfg).is_ok(), "pct={pct}");
        }
    }

    // --- min/max panel_size bounds ---

    #[test]
    fn dispute_config_rejects_min_panel_size_below_3() {
        let cfg = DisputeConfig {
            min_panel_size: 1,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("min_panel_size"), "msg={msg}");
    }

    #[test]
    fn dispute_config_rejects_even_min_panel_size() {
        let cfg = DisputeConfig {
            min_panel_size: 4,
            // Bump max + default panel_size so the only failing
            // invariant is even-min, not the relative-order one.
            max_panel_size: 11,
            panel_size: 5,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(
            msg.contains("min_panel_size") && msg.contains("odd"),
            "msg={msg}"
        );
    }

    #[test]
    fn dispute_config_rejects_max_below_min() {
        let cfg = DisputeConfig {
            min_panel_size: 7,
            max_panel_size: 5,
            panel_size: 7,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(
            msg.contains("max_panel_size") && msg.contains("min_panel_size"),
            "msg={msg}",
        );
    }

    #[test]
    fn dispute_config_rejects_even_max_panel_size() {
        let cfg = DisputeConfig {
            max_panel_size: 10,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(
            msg.contains("max_panel_size") && msg.contains("odd"),
            "msg={msg}"
        );
    }

    #[test]
    fn dispute_config_rejects_default_panel_size_outside_bounds() {
        // panel_size = 13, but max_panel_size = 11 (default).
        let cfg = DisputeConfig {
            panel_size: 13,
            ..DisputeConfig::default()
        };
        let msg = validation_err_msg(&cfg);
        assert!(msg.contains("must be within"), "msg={msg}");
    }

    // --- validate_panel_size_choice ---

    use super::validate_panel_size_choice;

    #[test]
    fn panel_size_choice_none_is_always_valid() {
        let cfg = DisputeConfig::default();
        assert!(validate_panel_size_choice(None, &cfg).is_ok());
    }

    #[test]
    fn panel_size_choice_accepts_within_bounds_and_odd() {
        let cfg = DisputeConfig::default();
        for n in [3_u32, 5, 7, 9, 11] {
            assert!(validate_panel_size_choice(Some(n), &cfg).is_ok(), "n={n}");
        }
    }

    #[test]
    fn panel_size_choice_rejects_below_min() {
        let cfg = DisputeConfig {
            min_panel_size: 5,
            ..DisputeConfig::default()
        };
        match validate_panel_size_choice(Some(3), &cfg).unwrap_err() {
            EscrowError::PanelSizeOutOfRange { min, max, got } => {
                assert_eq!(min, 5);
                assert_eq!(max, 11);
                assert_eq!(got, 3);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn panel_size_choice_rejects_above_max() {
        let cfg = DisputeConfig::default();
        // Default max is 11 — pick 13 to land above it.
        match validate_panel_size_choice(Some(13), &cfg).unwrap_err() {
            EscrowError::PanelSizeOutOfRange { min, max, got } => {
                assert_eq!(min, 3);
                assert_eq!(max, 11);
                assert_eq!(got, 13);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn panel_size_choice_rejects_even_in_range() {
        // 4 is within [3, 11] but even — must be rejected. `got = 4`
        // distinguishes this from out-of-range cases where `got` falls
        // outside `[min, max]`.
        let cfg = DisputeConfig::default();
        match validate_panel_size_choice(Some(4), &cfg).unwrap_err() {
            EscrowError::PanelSizeOutOfRange { min, max, got } => {
                assert_eq!(min, 3);
                assert_eq!(max, 11);
                assert_eq!(got, 4);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    // --- validate_min_amount + compute_min_viable_amount ---

    use super::{compute_min_viable_amount, validate_min_amount};
    use crate::types::deal::DealFees;

    fn fees(escrow_fee: u128, dispute_reserve_per_party: u128) -> DealFees {
        DealFees {
            escrow_fee,
            dispute_reserve_per_party,
            withdraw_fee_pct: 25,
            ledger_fee_at_create: 10_000,
        }
    }

    #[test]
    fn min_amount_floor_for_default_panel() {
        // EF=20_000, LF=10_000, DC/2=5_000 (→ DC=10_000), panel=3.
        // happy = 20_000 + 10_000 = 30_000.
        // dispute = 2 * 5_000 + (3 + 1) * 10_000 = 50_000.
        // floor = max(30_000, 50_000) = 50_000.
        let f = fees(20_000, 5_000);
        assert_eq!(compute_min_viable_amount(&f, 10_000, 3), 50_000);
    }

    #[test]
    fn min_amount_floor_scales_with_panel_size() {
        // Larger panel → more outgoing per-arbitrator ledger fees,
        // so the floor must grow. EF=20_000, LF=10_000, DC/2=5_000.
        // panel=11 → dispute = 10_000 + 12 * 10_000 = 130_000.
        let f = fees(20_000, 5_000);
        assert_eq!(compute_min_viable_amount(&f, 10_000, 11), 130_000);
    }

    #[test]
    fn min_amount_floor_takes_happy_path_when_panel_is_cheap() {
        // Tiny DC + tiny panel → happy path dominates.
        // EF=20_000, LF=10_000, DC/2=0, panel=3.
        // happy = 30_000. dispute = 0 + 4*10_000 = 40_000.
        // floor = max(30_000, 40_000) = 40_000. (Still dispute, even
        // with DC=0, because the per-arbitrator fees count.)
        let f = fees(20_000, 0);
        assert_eq!(compute_min_viable_amount(&f, 10_000, 3), 40_000);
    }

    #[test]
    fn min_amount_accepts_just_above_floor() {
        let f = fees(20_000, 5_000);
        assert!(validate_min_amount(50_001, &f, 10_000, 3).is_ok());
    }

    #[test]
    fn min_amount_rejects_at_floor_and_reports_min_acceptable() {
        // floor is rejected (strict inequality); error reports
        // floor + 1 so the caller can retry with the value as-is.
        let f = fees(20_000, 5_000);
        match validate_min_amount(50_000, &f, 10_000, 3).unwrap_err() {
            EscrowError::AmountBelowMinimum { min } => assert_eq!(min, 50_001),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn min_amount_rejects_below_floor() {
        let f = fees(20_000, 5_000);
        match validate_min_amount(1_000, &f, 10_000, 3).unwrap_err() {
            EscrowError::AmountBelowMinimum { min } => assert_eq!(min, 50_001),
            other => panic!("wrong error: {other:?}"),
        }
    }
}
