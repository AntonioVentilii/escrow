use candid::Principal;

use crate::{
    api::deals::errors::EscrowError,
    memory,
    types::deal::{Consent, Deal, DealStatus},
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
/// - At least one of `payer` / `recipient` must be `Some`.
/// - If neither is set, the caller defaults to payer.
/// - The caller must be one of the parties.
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
}
