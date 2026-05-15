use candid::{CandidType, Deserialize, Principal};

use super::errors::EscrowError;
use crate::types::{
    asset::Asset,
    deal::{Consent, Deal, DealFees, DealId, DealStatus, Signature},
    dispute::DisputeId,
    ledger_types::Account,
};

macro_rules! candid_result {
    ($name:ident, $ok:ty) => {
        #[derive(CandidType, Deserialize, Clone, Debug)]
        pub enum $name {
            Ok(Box<$ok>),
            Err(EscrowError),
        }

        impl From<Result<$ok, EscrowError>> for $name {
            fn from(result: Result<$ok, EscrowError>) -> Self {
                match result {
                    Ok(v) => Self::Ok(Box::new(v)),
                    Err(e) => Self::Err(e),
                }
            }
        }
    };
}

candid_result!(CreateDealResult, DealView);
candid_result!(FundDealResult, DealView);
candid_result!(AcceptDealResult, DealView);
candid_result!(ReclaimDealResult, DealView);
candid_result!(CancelDealResult, DealView);
candid_result!(ConsentDealResult, DealView);
candid_result!(RejectDealResult, DealView);
candid_result!(SignDealResult, DealView);
candid_result!(GetDealResult, DealView);
candid_result!(GetClaimableDealResult, ClaimableDealView);
candid_result!(GetEscrowAccountResult, Account);
candid_result!(ProcessExpiredDealsResult, Vec<DealId>);

/// Full deal view returned to authorised participants (payer or recipient).
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealView {
    /// Unique deal identifier.
    pub id: DealId,
    /// Principal of the payer, or `None` if not yet assigned.
    pub payer: Option<Principal>,
    /// Principal of the recipient, or `None` if not yet bound.
    pub recipient: Option<Principal>,
    /// Escrowed token amount.
    pub amount: u128,
    /// Settlement asset for this deal. Today always
    /// [`Asset::Icrc`]; the enum exists so future settlement
    /// domains can be added without renaming this field.
    pub asset: Asset,
    /// Current lifecycle status of the deal.
    pub status: DealStatus,
    /// Nanosecond UTC timestamp when the deal was created.
    pub created_at_ns: u64,
    /// Principal who created the deal.
    pub created_by: Principal,
    /// Nanosecond UTC timestamp when the deal was last updated.
    pub updated_at_ns: Option<u64>,
    /// Principal who last updated the deal.
    pub updated_by: Option<Principal>,
    /// Nanosecond UTC timestamp after which the deal expires.
    pub expires_at_ns: u64,
    /// 32-byte ledger subaccount that holds the escrowed funds.
    pub escrow_subaccount: Vec<u8>,
    /// Optional short title.
    pub title: Option<String>,
    /// Optional note or message.
    pub note: Option<String>,
    /// Timestamp when the deal was funded, if applicable.
    pub funded_at_ns: Option<u64>,
    /// Timestamp when the deal was settled (funds released), if applicable.
    pub settled_at_ns: Option<u64>,
    /// Timestamp when the deal was refunded, if applicable.
    pub refunded_at_ns: Option<u64>,
    /// Payer's consent to the deal terms.
    pub payer_consent: Consent,
    /// Recipient's consent to the deal terms.
    pub recipient_consent: Consent,
    /// Claim code for sharing via QR / link. Only visible to authorised
    /// participants; never exposed in the public claimable view.
    pub claim_code: Option<String>,
    /// Identifier of the attached dispute, if any. `Some(_)` while a
    /// dispute is open or after it has resolved (audit-trail link to
    /// the `Dispute` record); `None` for deals that never went into
    /// dispute.
    pub dispute: Option<DisputeId>,
    /// Per-deal arbitrator panel size override chosen by the creator
    /// at `create_deal` time. `Some(n)` locks `n` arbitrators for
    /// any dispute on this deal; `None` means "use whatever
    /// `DisputeConfig.panel_size` is current when the dispute opens".
    /// Surfaced in the public view so a counterparty can see the
    /// committed dispute terms before consenting.
    pub panel_size: Option<u32>,
    /// Per-deal fee snapshot taken at `create_deal` time —
    /// `escrow_fee`, per-party dispute reserve, withdraw-fee
    /// percentage, and create-time ledger fee. Frontends should
    /// render these for transparent quoting (the recipient's
    /// expected payout is `amount - fees.escrow_fee - live ledger
    /// fee`).
    pub fees: DealFees,
    /// Payer's settlement signature on a `Funded` bound deal —
    /// `Empty` until the payer calls `sign_deal`. Tip flows
    /// (recipient unbound) always carry `Empty`. See [`Signature`]
    /// for tally semantics.
    pub payer_signature: Signature,
    /// Recipient's settlement signature; mirrors
    /// [`Self::payer_signature`].
    pub recipient_signature: Signature,
}

impl From<&Deal> for DealView {
    fn from(deal: &Deal) -> Self {
        Self {
            id: deal.id,
            payer: deal.payer,
            recipient: deal.recipient,
            amount: deal.amount,
            asset: deal.asset.clone(),
            status: deal.status.clone(),
            created_at_ns: deal.created_at_ns,
            created_by: deal.created_by,
            updated_at_ns: deal.updated_at_ns,
            updated_by: deal.updated_by,
            expires_at_ns: deal.expires_at_ns,
            escrow_subaccount: deal.escrow_subaccount.clone(),
            title: deal.metadata.as_ref().and_then(|m| m.title.clone()),
            note: deal.metadata.as_ref().and_then(|m| m.note.clone()),
            funded_at_ns: deal.funded_at_ns,
            settled_at_ns: deal.settled_at_ns,
            refunded_at_ns: deal.refunded_at_ns,
            payer_consent: deal.payer_consent.clone(),
            recipient_consent: deal.recipient_consent.clone(),
            claim_code: deal.claim_code.clone(),
            dispute: deal.dispute,
            panel_size: deal.panel_size,
            fees: deal.fees.clone(),
            payer_signature: deal.payer_signature.clone(),
            recipient_signature: deal.recipient_signature.clone(),
        }
    }
}

/// Reduced view for public claim pages — does not expose payer, claim code,
/// or internal fields.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ClaimableDealView {
    /// Unique deal identifier.
    pub id: DealId,
    /// Escrowed token amount.
    pub amount: u128,
    /// Settlement asset for this deal. Today always
    /// [`Asset::Icrc`].
    pub asset: Asset,
    /// Current lifecycle status of the deal.
    pub status: DealStatus,
    /// Whether a recipient principal has already been bound to this deal.
    pub is_recipient_bound: bool,
    /// Nanosecond UTC timestamp after which the deal expires.
    pub expires_at_ns: u64,
    /// Optional short title.
    pub title: Option<String>,
    /// Optional note or message.
    pub note: Option<String>,
}

impl From<&Deal> for ClaimableDealView {
    fn from(deal: &Deal) -> Self {
        Self {
            id: deal.id,
            amount: deal.amount,
            asset: deal.asset.clone(),
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

    use super::{ClaimableDealView, DealView};
    use crate::types::{
        asset::Asset,
        deal::{Consent, Deal, DealFees, DealMetadata, DealStatus, Signature},
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    #[test]
    fn deal_view_maps_metadata() {
        let deal = Deal {
            id: 1,
            payer: Some(test_principal(1)),
            recipient: None,
            asset: Asset::Icrc(test_principal(99)),
            amount: 1_000_000,
            created_at_ns: 100,
            created_by: test_principal(1),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 1000,
            status: DealStatus::Created,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: None,
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: Some("abc123".to_owned()),
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: Some(DealMetadata {
                title: Some("Test tip".to_owned()),
                note: None,
            }),
            dispute: None,
            panel_size: None,
            fees: DealFees::default(),
            payer_signature: Signature::Empty,
            recipient_signature: Signature::Empty,
        };
        let view = DealView::from(&deal);
        assert_eq!(view.title.as_deref(), Some("Test tip"));
        assert!(view.note.is_none());
        assert_eq!(view.escrow_subaccount.len(), 32);
        assert_eq!(view.created_by, test_principal(1));
        assert!(view.updated_at_ns.is_none());
        assert!(view.updated_by.is_none());
        assert_eq!(view.payer_consent, Consent::Accepted);
        assert_eq!(view.recipient_consent, Consent::Pending);
        assert_eq!(view.claim_code.as_deref(), Some("abc123"));
        assert!(view.dispute.is_none());
    }

    #[test]
    fn claimable_view_hides_payer_and_claim_code() {
        let deal = Deal {
            id: 1,
            payer: Some(test_principal(1)),
            recipient: Some(test_principal(2)),
            asset: Asset::Icrc(test_principal(99)),
            amount: 500,
            created_at_ns: 100,
            created_by: test_principal(1),
            updated_at_ns: None,
            updated_by: None,
            expires_at_ns: 1000,
            status: DealStatus::Funded,
            escrow_subaccount: vec![0_u8; 32],
            funded_at_ns: Some(200),
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: Some("secret".to_owned()),
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Accepted,
            metadata: None,
            dispute: None,
            panel_size: None,
            fees: DealFees::default(),
            payer_signature: Signature::Empty,
            recipient_signature: Signature::Empty,
        };
        let view = ClaimableDealView::from(&deal);
        assert!(view.is_recipient_bound);
        assert_eq!(view.amount, 500);
    }
}
