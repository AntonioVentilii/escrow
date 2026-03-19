use candid::{CandidType, Deserialize, Int, Nat};

use super::{
    deal::{Deal, DealStatus},
    ledger_types::Account,
};

/// Generic metadata value following the ICRC-16 specification.
///
/// Used for both collection-level and token-level metadata in the ICRC-7 NFT
/// interface.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub enum Value {
    Nat(Nat),
    Int(Int),
    Text(String),
    Blob(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(String, Value)>),
}

/// ICRC-7 transfer argument for a single token.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Icrc7TransferArg {
    pub from_subaccount: Option<Vec<u8>>,
    pub to: Account,
    pub token_id: Nat,
    pub memo: Option<Vec<u8>>,
    pub created_at_time: Option<u64>,
}

/// ICRC-7 transfer error.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum Icrc7TransferError {
    NonExistingTokenId,
    InvalidRecipient,
    Unauthorized,
    TooOld,
    CreatedInFuture { ledger_time: u64 },
    Duplicate { duplicate_of: Nat },
    GenericError { error_code: Nat, message: String },
    GenericBatchError { error_code: Nat, message: String },
}

/// Explicit result type for `icrc7_transfer` to avoid generic `Result` names
/// in the generated Candid interface.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum Icrc7TransferResponse {
    Ok(Nat),
    Err(Icrc7TransferError),
}

/// ICRC-10 supported standard descriptor.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SupportedStandard {
    pub name: String,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Collection-level constants
// ---------------------------------------------------------------------------

pub const COLLECTION_NAME: &str = "Escrow Deals";
pub const COLLECTION_SYMBOL: &str = "ESCROW";
pub const COLLECTION_DESCRIPTION: &str =
    "Non-fungible tokens representing escrow deals on the Internet Computer";

pub const DEFAULT_TAKE: u64 = 50;
pub const MAX_TAKE: u64 = 500;
pub const MAX_QUERY_BATCH_SIZE: u64 = 100;

// ---------------------------------------------------------------------------
// Ownership helpers
// ---------------------------------------------------------------------------

/// Computes the ICRC-7 owner of a deal token.
///
/// Settled deals are owned by the recipient; all other states (including
/// terminal ones like `Refunded` and `Cancelled`) are owned by the payer.
/// Falls back to `created_by` when the relevant principal is not set.
#[must_use]
pub fn token_owner(deal: &Deal) -> Account {
    let owner = match deal.status {
        DealStatus::Settled => deal
            .recipient
            .or(deal.payer)
            .unwrap_or(deal.created_by),
        DealStatus::Created
        | DealStatus::Funded
        | DealStatus::Refunded
        | DealStatus::Cancelled
        | DealStatus::Rejected => deal.payer.unwrap_or(deal.created_by),
    };
    Account {
        owner,
        subaccount: None,
    }
}

/// Returns `true` if `account` matches the token owner of `deal`.
///
/// Ownership is always assigned to the default subaccount (`None` / all-zeros),
/// so any account with a non-default subaccount will never match.
#[must_use]
pub fn account_owns_token(account: &Account, deal: &Deal) -> bool {
    if !is_default_subaccount(&account.subaccount) {
        return false;
    }
    let owner = token_owner(deal);
    account.owner == owner.owner
}

/// Checks whether a subaccount is the default (None or all-zeros).
///
/// Per ICRC-1, `None` and `Some([0; 32])` denote the same default account.
#[must_use]
pub fn is_default_subaccount(sub: &Option<Vec<u8>>) -> bool {
    match sub {
        None => true,
        Some(bytes) => bytes.iter().all(|&b| b == 0),
    }
}

// ---------------------------------------------------------------------------
// Metadata builders
// ---------------------------------------------------------------------------

/// Builds the ICRC-7 metadata map for a single deal token.
#[must_use]
pub fn deal_to_metadata(deal: &Deal) -> Vec<(String, Value)> {
    let mut meta = vec![
        (
            "icrc7:name".to_owned(),
            Value::Text(format!("Escrow Deal #{}", deal.id)),
        ),
        (
            "escrow:status".to_owned(),
            Value::Text(format!("{:?}", deal.status)),
        ),
        (
            "escrow:amount".to_owned(),
            Value::Nat(Nat::from(deal.amount)),
        ),
        (
            "escrow:token_ledger".to_owned(),
            Value::Text(deal.token_ledger.to_text()),
        ),
        (
            "escrow:expires_at_ns".to_owned(),
            Value::Nat(Nat::from(deal.expires_at_ns)),
        ),
        (
            "escrow:created_at_ns".to_owned(),
            Value::Nat(Nat::from(deal.created_at_ns)),
        ),
        (
            "escrow:escrow_subaccount".to_owned(),
            Value::Blob(deal.escrow_subaccount.clone()),
        ),
        (
            "escrow:payer_consent".to_owned(),
            Value::Text(format!("{:?}", deal.payer_consent)),
        ),
        (
            "escrow:recipient_consent".to_owned(),
            Value::Text(format!("{:?}", deal.recipient_consent)),
        ),
    ];

    if let Some(payer) = deal.payer {
        meta.push(("escrow:payer".to_owned(), Value::Text(payer.to_text())));
    }

    if let Some(recipient) = deal.recipient {
        meta.push((
            "escrow:recipient".to_owned(),
            Value::Text(recipient.to_text()),
        ));
    }

    if let Some(ref sym) = deal.token_symbol {
        meta.push(("escrow:token_symbol".to_owned(), Value::Text(sym.clone())));
    }

    if let Some(funded) = deal.funded_at_ns {
        meta.push((
            "escrow:funded_at_ns".to_owned(),
            Value::Nat(Nat::from(funded)),
        ));
    }

    if let Some(settled) = deal.settled_at_ns {
        meta.push((
            "escrow:settled_at_ns".to_owned(),
            Value::Nat(Nat::from(settled)),
        ));
    }

    if let Some(refunded) = deal.refunded_at_ns {
        meta.push((
            "escrow:refunded_at_ns".to_owned(),
            Value::Nat(Nat::from(refunded)),
        ));
    }

    if let Some(ref metadata) = deal.metadata {
        if let Some(ref title) = metadata.title {
            meta.push(("escrow:title".to_owned(), Value::Text(title.clone())));
        }
        if let Some(ref note) = metadata.note {
            meta.push(("escrow:note".to_owned(), Value::Text(note.clone())));
        }
    }

    meta
}

/// Builds ICRC-7 collection-level metadata.
#[must_use]
pub fn collection_metadata(total_supply: u64) -> Vec<(String, Value)> {
    vec![
        (
            "icrc7:name".to_owned(),
            Value::Text(COLLECTION_NAME.to_owned()),
        ),
        (
            "icrc7:symbol".to_owned(),
            Value::Text(COLLECTION_SYMBOL.to_owned()),
        ),
        (
            "icrc7:description".to_owned(),
            Value::Text(COLLECTION_DESCRIPTION.to_owned()),
        ),
        (
            "icrc7:total_supply".to_owned(),
            Value::Nat(Nat::from(total_supply)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use candid::{Nat, Principal};

    use super::{
        account_owns_token, collection_metadata, deal_to_metadata, is_default_subaccount,
        token_owner, Value, COLLECTION_NAME,
    };
    use crate::types::{
        deal::{Consent, Deal, DealMetadata, DealStatus},
        ledger_types::Account,
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn make_deal(status: DealStatus, payer: Option<Principal>, recipient: Option<Principal>) -> Deal {
        Deal {
            id: 1,
            payer,
            recipient,
            token_ledger: test_principal(99),
            token_symbol: None,
            amount: 1000,
            created_at_ns: 100,
            created_by: payer.unwrap_or(test_principal(1)),
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
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: None,
        }
    }

    // --- token_owner ---

    #[test]
    fn owner_of_created_is_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_of_funded_is_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Funded, Some(payer), None);
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_of_settled_is_recipient() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let deal = make_deal(DealStatus::Settled, Some(payer), Some(recip));
        assert_eq!(token_owner(&deal).owner, recip);
    }

    #[test]
    fn owner_of_settled_falls_back_to_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Settled, Some(payer), None);
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_of_refunded_is_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Refunded, Some(payer), Some(test_principal(2)));
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_of_cancelled_is_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Cancelled, Some(payer), None);
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_of_rejected_is_payer() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Rejected, Some(payer), Some(test_principal(2)));
        assert_eq!(token_owner(&deal).owner, payer);
    }

    #[test]
    fn owner_falls_back_to_created_by_when_payer_none() {
        let mut deal = make_deal(DealStatus::Created, None, Some(test_principal(2)));
        deal.created_by = test_principal(2);
        assert_eq!(token_owner(&deal).owner, test_principal(2));
    }

    #[test]
    fn owner_subaccount_is_none() {
        let deal = make_deal(DealStatus::Created, Some(test_principal(1)), None);
        assert!(token_owner(&deal).subaccount.is_none());
    }

    // --- account_owns_token ---

    #[test]
    fn matching_principal_owns_token() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        let account = Account {
            owner: payer,
            subaccount: None,
        };
        assert!(account_owns_token(&account, &deal));
    }

    #[test]
    fn account_with_subaccount_never_owns() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        let account = Account {
            owner: payer,
            subaccount: Some(vec![1_u8; 32]),
        };
        assert!(!account_owns_token(&account, &deal));
    }

    #[test]
    fn account_with_zero_subaccount_owns() {
        let payer = test_principal(1);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        let account = Account {
            owner: payer,
            subaccount: Some(vec![0_u8; 32]),
        };
        assert!(account_owns_token(&account, &deal));
    }

    #[test]
    fn different_principal_does_not_own() {
        let payer = test_principal(1);
        let other = test_principal(2);
        let deal = make_deal(DealStatus::Created, Some(payer), None);
        let account = Account {
            owner: other,
            subaccount: None,
        };
        assert!(!account_owns_token(&account, &deal));
    }

    // --- is_default_subaccount ---

    #[test]
    fn none_is_default() {
        assert!(is_default_subaccount(&None));
    }

    #[test]
    fn all_zeros_is_default() {
        assert!(is_default_subaccount(&Some(vec![0_u8; 32])));
    }

    #[test]
    fn nonzero_is_not_default() {
        assert!(!is_default_subaccount(&Some(vec![1_u8; 32])));
    }

    #[test]
    fn empty_vec_is_default() {
        assert!(is_default_subaccount(&Some(vec![])));
    }

    // --- deal_to_metadata ---

    #[test]
    fn metadata_contains_required_fields() {
        let deal = make_deal(DealStatus::Funded, Some(test_principal(1)), None);
        let meta = deal_to_metadata(&deal);
        let keys: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"icrc7:name"));
        assert!(keys.contains(&"escrow:status"));
        assert!(keys.contains(&"escrow:payer"));
        assert!(keys.contains(&"escrow:amount"));
        assert!(keys.contains(&"escrow:token_ledger"));
        assert!(keys.contains(&"escrow:expires_at_ns"));
        assert!(keys.contains(&"escrow:created_at_ns"));
        assert!(keys.contains(&"escrow:escrow_subaccount"));
        assert!(keys.contains(&"escrow:payer_consent"));
        assert!(keys.contains(&"escrow:recipient_consent"));
    }

    #[test]
    fn metadata_includes_optional_fields_when_present() {
        let payer = test_principal(1);
        let recip = test_principal(2);
        let mut deal = make_deal(DealStatus::Settled, Some(payer), Some(recip));
        deal.funded_at_ns = Some(150);
        deal.settled_at_ns = Some(180);
        deal.metadata = Some(DealMetadata {
            title: Some("Coffee tip".to_owned()),
            note: Some("Thanks!".to_owned()),
        });
        let meta = deal_to_metadata(&deal);
        let keys: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"escrow:recipient"));
        assert!(keys.contains(&"escrow:funded_at_ns"));
        assert!(keys.contains(&"escrow:settled_at_ns"));
        assert!(keys.contains(&"escrow:title"));
        assert!(keys.contains(&"escrow:note"));
    }

    #[test]
    fn metadata_omits_unset_optional_fields() {
        let deal = make_deal(DealStatus::Created, Some(test_principal(1)), None);
        let meta = deal_to_metadata(&deal);
        let keys: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
        assert!(!keys.contains(&"escrow:recipient"));
        assert!(!keys.contains(&"escrow:funded_at_ns"));
        assert!(!keys.contains(&"escrow:settled_at_ns"));
        assert!(!keys.contains(&"escrow:refunded_at_ns"));
        assert!(!keys.contains(&"escrow:token_symbol"));
        assert!(!keys.contains(&"escrow:title"));
        assert!(!keys.contains(&"escrow:note"));
    }

    #[test]
    fn metadata_name_contains_deal_id() {
        let deal = make_deal(DealStatus::Created, Some(test_principal(1)), None);
        let meta = deal_to_metadata(&deal);
        let name_val = meta.iter().find(|(k, _)| k == "icrc7:name").map(|(_, v)| v);
        assert_eq!(name_val, Some(&Value::Text("Escrow Deal #1".to_owned())));
    }

    #[test]
    fn metadata_amount_as_nat() {
        let mut deal = make_deal(DealStatus::Created, Some(test_principal(1)), None);
        deal.amount = 42_000;
        let meta = deal_to_metadata(&deal);
        let amount_val = meta
            .iter()
            .find(|(k, _)| k == "escrow:amount")
            .map(|(_, v)| v);
        assert_eq!(amount_val, Some(&Value::Nat(Nat::from(42_000_u64))));
    }

    // --- collection_metadata ---

    #[test]
    fn collection_metadata_structure() {
        let meta = collection_metadata(10);
        let keys: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"icrc7:name"));
        assert!(keys.contains(&"icrc7:symbol"));
        assert!(keys.contains(&"icrc7:description"));
        assert!(keys.contains(&"icrc7:total_supply"));
    }

    #[test]
    fn collection_metadata_name_value() {
        let meta = collection_metadata(0);
        let name_val = meta.iter().find(|(k, _)| k == "icrc7:name").map(|(_, v)| v);
        assert_eq!(name_val, Some(&Value::Text(COLLECTION_NAME.to_owned())));
    }

    #[test]
    fn collection_metadata_supply_reflects_input() {
        let meta = collection_metadata(42);
        let supply_val = meta
            .iter()
            .find(|(k, _)| k == "icrc7:total_supply")
            .map(|(_, v)| v);
        assert_eq!(supply_val, Some(&Value::Nat(Nat::from(42_u64))));
    }
}
