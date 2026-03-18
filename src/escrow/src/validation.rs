use candid::Principal;

use crate::{
    api::deals::errors::EscrowError,
    types::deal::{Deal, DealStatus},
};

pub fn validate_create(amount: u128, expires_at_ns: u64, now_ns: u64) -> Result<(), EscrowError> {
    if amount == 0 {
        return Err(EscrowError::InvalidAmount);
    }
    if expires_at_ns <= now_ns {
        return Err(EscrowError::InvalidExpiry);
    }
    Ok(())
}

/// Returns `true` if the deal is already funded (idempotent success).
/// Returns `Err` if funding is not allowed.
pub fn validate_can_fund(deal: &Deal, caller: Principal) -> Result<bool, EscrowError> {
    if deal.payer != caller {
        return Err(EscrowError::NotAuthorised);
    }
    match deal.status {
        DealStatus::Created => Ok(false),
        DealStatus::Funded | DealStatus::Completed => Ok(true),
        _ => Err(EscrowError::InvalidState {
            expected: "Created".to_owned(),
            actual: format!("{:?}", deal.status),
        }),
    }
}

/// Returns `true` if the deal is already completed (idempotent success).
/// Returns `Err` if acceptance is not allowed.
pub fn validate_can_accept(
    deal: &Deal,
    caller: Principal,
    now_ns: u64,
) -> Result<bool, EscrowError> {
    match deal.status {
        DealStatus::Completed => return Ok(true),
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
    if deal.payer != caller {
        return Err(EscrowError::NotAuthorised);
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
    if deal.payer != caller {
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

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::{
        validate_can_accept, validate_can_cancel, validate_can_fund, validate_can_reclaim,
        validate_create,
    };
    use crate::{
        api::deals::errors::EscrowError,
        types::deal::{Deal, DealMetadata, DealStatus},
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_deal(status: DealStatus, payer: Principal, recipient: Option<Principal>) -> Deal {
        Deal {
            id: 1,
            payer,
            recipient,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: payer,
            updated_at_ns: None,
            updated_by: None,
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
            metadata: Some(DealMetadata {
                title: None,
                note: None,
            }),
        }
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
        let deal = make_deal(DealStatus::Created, payer, None);
        assert!(!validate_can_fund(&deal, payer).unwrap());
    }

    #[test]
    fn fund_idempotent_when_funded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(validate_can_fund(&deal, payer).unwrap());
    }

    #[test]
    fn fund_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = make_deal(DealStatus::Created, payer, None);
        assert!(validate_can_fund(&deal, other).is_err());
    }

    #[test]
    fn fund_rejects_refunded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Refunded, payer, None);
        assert!(validate_can_fund(&deal, payer).is_err());
    }

    // --- accept ---

    #[test]
    fn accept_ok_when_funded_and_not_expired() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(!validate_can_accept(&deal, recip, 150).unwrap());
    }

    #[test]
    fn accept_rejects_expired() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(validate_can_accept(&deal, recip, 300).is_err());
    }

    #[test]
    fn accept_rejects_wrong_recipient() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let other = test_principal(3);
        let deal = make_deal(DealStatus::Funded, payer, Some(recip));
        assert!(matches!(
            validate_can_accept(&deal, other, 150),
            Err(EscrowError::RecipientMismatch)
        ));
    }

    #[test]
    fn accept_idempotent_when_completed() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Completed, payer, None);
        assert!(validate_can_accept(&deal, payer, 150).unwrap());
    }

    #[test]
    fn accept_rejects_created_deal() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, payer, None);
        assert!(matches!(
            validate_can_accept(&deal, payer, 150),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    // --- reclaim ---

    #[test]
    fn reclaim_ok_when_funded_and_expired() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(!validate_can_reclaim(&deal, payer, 300).unwrap());
    }

    #[test]
    fn reclaim_rejects_before_expiry() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(matches!(
            validate_can_reclaim(&deal, payer, 150),
            Err(EscrowError::NotExpired)
        ));
    }

    #[test]
    fn reclaim_rejects_non_payer() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(validate_can_reclaim(&deal, other, 300).is_err());
    }

    #[test]
    fn reclaim_idempotent_when_refunded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Refunded, payer, None);
        assert!(validate_can_reclaim(&deal, payer, 300).unwrap());
    }

    #[test]
    fn reclaim_rejects_completed_deal() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Completed, payer, None);
        assert!(matches!(
            validate_can_reclaim(&deal, payer, 300),
            Err(EscrowError::InvalidState { .. })
        ));
    }

    // --- cancel ---

    #[test]
    fn cancel_ok_when_created() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, payer, None);
        assert!(!validate_can_cancel(&deal, payer).unwrap());
    }

    #[test]
    fn cancel_rejects_funded() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, payer, None);
        assert!(validate_can_cancel(&deal, payer).is_err());
    }
}
