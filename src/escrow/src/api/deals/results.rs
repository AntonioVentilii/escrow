use candid::{CandidType, Deserialize, Principal};

use crate::types::deal::{Deal, DealId, DealStatus};

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealView {
    pub id: DealId,
    pub payer: Principal,
    pub recipient: Option<Principal>,
    pub amount: u128,
    pub token_ledger: Principal,
    pub status: DealStatus,
    pub created_at_ns: u64,
    pub expires_at_ns: u64,
    pub escrow_subaccount: Vec<u8>,
    pub title: Option<String>,
    pub note: Option<String>,
    pub funded_at_ns: Option<u64>,
    pub completed_at_ns: Option<u64>,
    pub refunded_at_ns: Option<u64>,
}

impl From<&Deal> for DealView {
    fn from(deal: &Deal) -> Self {
        Self {
            id: deal.id,
            payer: deal.payer,
            recipient: deal.recipient,
            amount: deal.amount,
            token_ledger: deal.token_ledger,
            status: deal.status.clone(),
            created_at_ns: deal.created_at_ns,
            expires_at_ns: deal.expires_at_ns,
            escrow_subaccount: deal.escrow_subaccount.clone(),
            title: deal.metadata.as_ref().and_then(|m| m.title.clone()),
            note: deal.metadata.as_ref().and_then(|m| m.note.clone()),
            funded_at_ns: deal.funded_at_ns,
            completed_at_ns: deal.completed_at_ns,
            refunded_at_ns: deal.refunded_at_ns,
        }
    }
}

/// Reduced view for public claim pages — does not expose payer or internal fields.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ClaimableDealView {
    pub id: DealId,
    pub amount: u128,
    pub token_ledger: Principal,
    pub status: DealStatus,
    pub is_recipient_bound: bool,
    pub expires_at_ns: u64,
    pub title: Option<String>,
    pub note: Option<String>,
}

impl From<&Deal> for ClaimableDealView {
    fn from(deal: &Deal) -> Self {
        Self {
            id: deal.id,
            amount: deal.amount,
            token_ledger: deal.token_ledger,
            status: deal.status.clone(),
            is_recipient_bound: deal.recipient.is_some(),
            expires_at_ns: deal.expires_at_ns,
            title: deal.metadata.as_ref().and_then(|m| m.title.clone()),
            note: deal.metadata.as_ref().and_then(|m| m.note.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use crate::types::deal::{Deal, DealMetadata, DealStatus};

    use super::{ClaimableDealView, DealView};

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    #[test]
    fn deal_view_maps_metadata() {
        let deal = Deal {
            id: 1,
            payer: test_principal(1),
            recipient: None,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1_000_000,
            created_at_ns: 100,
            expires_at_ns: 1000,
            status: DealStatus::Created,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            completed_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            metadata: Some(DealMetadata {
                title: Some("Test tip".to_owned()),
                note: None,
            }),
        };
        let view = DealView::from(&deal);
        assert_eq!(view.title.as_deref(), Some("Test tip"));
        assert!(view.note.is_none());
        assert_eq!(view.escrow_subaccount.len(), 32);
    }

    #[test]
    fn claimable_view_hides_payer() {
        let deal = Deal {
            id: 1,
            payer: test_principal(1),
            recipient: Some(test_principal(2)),
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 500,
            created_at_ns: 100,
            expires_at_ns: 1000,
            status: DealStatus::Funded,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: Some(200),
            completed_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            metadata: None,
        };
        let view = ClaimableDealView::from(&deal);
        assert!(view.is_recipient_bound);
        assert_eq!(view.amount, 500);
    }
}
