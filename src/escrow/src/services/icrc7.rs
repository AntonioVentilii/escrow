use candid::Nat;

use crate::{
    memory,
    types::{
        deal::DealId,
        icrc7::{
            self, account_owns_token, Icrc7TransferArg, Icrc7TransferError, Icrc7TransferResponse,
            SupportedStandard, Value, COLLECTION_DESCRIPTION, COLLECTION_NAME, COLLECTION_SYMBOL,
            DEFAULT_TAKE, MAX_QUERY_BATCH_SIZE, MAX_TAKE,
        },
        ledger_types::Account,
    },
};

// ---------------------------------------------------------------------------
// Collection-level queries
// ---------------------------------------------------------------------------

/// Human-readable name of the NFT collection.
#[must_use]
pub fn name() -> String {
    COLLECTION_NAME.to_owned()
}

/// Ticker symbol of the NFT collection.
#[must_use]
pub fn symbol() -> String {
    COLLECTION_SYMBOL.to_owned()
}

/// Human-readable description of the NFT collection.
#[must_use]
pub fn description() -> Option<String> {
    Some(COLLECTION_DESCRIPTION.to_owned())
}

/// Collection logo (currently unset).
#[must_use]
pub fn logo() -> Option<String> {
    None
}

/// Total number of deal NFTs in existence (one per deal, never burned).
#[must_use]
pub fn total_supply() -> Nat {
    Nat::from(memory::deal_count())
}

/// Maximum supply cap. `None` means unlimited.
#[must_use]
pub fn supply_cap() -> Option<Nat> {
    None
}

/// Maximum number of token IDs accepted in a single batch query.
#[must_use]
pub fn max_query_batch_size() -> Option<Nat> {
    Some(Nat::from(MAX_QUERY_BATCH_SIZE))
}

/// Maximum number of transfer args accepted in a single batch update.
#[must_use]
pub fn max_update_batch_size() -> Option<Nat> {
    None
}

/// Default page size for `tokens` / `tokens_of` when `take` is omitted.
#[must_use]
pub fn default_take_value() -> Option<Nat> {
    Some(Nat::from(DEFAULT_TAKE))
}

/// Maximum page size for `tokens` / `tokens_of`.
#[must_use]
pub fn max_take_value() -> Option<Nat> {
    Some(Nat::from(MAX_TAKE))
}

/// Maximum memo size. `None` means no explicit limit.
#[must_use]
pub fn max_memo_size() -> Option<Nat> {
    None
}

/// Whether batch transfers are atomic. `None` means unspecified.
#[must_use]
pub fn atomic_batch_transfers() -> Option<bool> {
    None
}

/// Transaction deduplication window. `None` means unspecified.
#[must_use]
pub fn tx_window() -> Option<Nat> {
    None
}

/// Permitted time drift for deduplication. `None` means unspecified.
#[must_use]
pub fn permitted_drift() -> Option<Nat> {
    None
}

/// Collection-level metadata as an ICRC-16 key-value map.
#[must_use]
pub fn collection_metadata() -> Vec<(String, Value)> {
    icrc7::collection_metadata(memory::deal_count())
}

// ---------------------------------------------------------------------------
// Token-level queries
// ---------------------------------------------------------------------------

/// Returns metadata for each requested token ID.
///
/// Unknown IDs yield `None`.  The input is silently capped at
/// `MAX_QUERY_BATCH_SIZE`.
#[must_use]
pub fn token_metadata(token_ids: &[Nat]) -> Vec<Option<Vec<(String, Value)>>> {
    let capped = cap_batch(token_ids);
    capped
        .iter()
        .map(|id| {
            nat_to_deal_id(id)
                .and_then(memory::get_deal)
                .map(|d| icrc7::deal_to_metadata(&d))
        })
        .collect()
}

/// Returns the owner `Account` for each requested token ID.
///
/// Unknown IDs yield `None`.  The input is silently capped at
/// `MAX_QUERY_BATCH_SIZE`.
#[must_use]
pub fn owner_of(token_ids: &[Nat]) -> Vec<Option<Account>> {
    let capped = cap_batch(token_ids);
    capped
        .iter()
        .map(|id| {
            nat_to_deal_id(id)
                .and_then(memory::get_deal)
                .map(|d| icrc7::token_owner(&d))
        })
        .collect()
}

/// Returns the number of tokens owned by each requested account.
#[must_use]
pub fn balance_of(accounts: &[Account]) -> Vec<Nat> {
    memory::with_deals(|deals| {
        accounts
            .iter()
            .map(|account| {
                let count = deals
                    .values()
                    .filter(|d| account_owns_token(account, d))
                    .count();
                Nat::from(count as u64)
            })
            .collect()
    })
}

/// Returns a page of token IDs in ascending order.
///
/// `prev` is the last ID the caller received (exclusive cursor).
/// `take` limits the page size (capped at `MAX_TAKE`, defaults to
/// `DEFAULT_TAKE`).
#[must_use]
pub fn tokens(prev: Option<&Nat>, take: Option<&Nat>) -> Vec<Nat> {
    let effective_take = effective_take(take);
    let Some(start) = start_after_id(prev) else {
        return vec![];
    };

    memory::with_deals(|deals| {
        deals
            .range(start..)
            .take(effective_take)
            .map(|(id, _)| Nat::from(*id))
            .collect()
    })
}

/// Returns a page of token IDs owned by `account` in ascending order.
///
/// See [`tokens`] for cursor / take semantics.
#[must_use]
pub fn tokens_of(account: &Account, prev: Option<&Nat>, take: Option<&Nat>) -> Vec<Nat> {
    let effective_take = effective_take(take);
    let Some(start) = start_after_id(prev) else {
        return vec![];
    };

    memory::with_deals(|deals| {
        deals
            .range(start..)
            .filter(|(_, d)| account_owns_token(account, d))
            .take(effective_take)
            .map(|(id, _)| Nat::from(*id))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Transfer (always rejected — ownership managed through escrow operations)
// ---------------------------------------------------------------------------

/// Rejects every transfer request with a `GenericError`.
///
/// Deal ownership is managed exclusively through escrow operations
/// (`accept_deal`, `reclaim_deal`, etc.), not via direct ICRC-7 transfers.
#[must_use]
pub fn transfer(args: &[Icrc7TransferArg]) -> Vec<Option<Icrc7TransferResponse>> {
    args.iter()
        .map(|_| {
            Some(Icrc7TransferResponse::Err(
                Icrc7TransferError::GenericError {
                    error_code: Nat::from(1_u64),
                    message: "Direct transfers are disabled; use escrow operations \
                              (accept_deal, reclaim_deal) to manage deal ownership"
                        .to_owned(),
                },
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ICRC-10 supported standards
// ---------------------------------------------------------------------------

/// Returns the ICRC standards supported by this canister (ICRC-7, ICRC-10).
#[must_use]
pub fn supported_standards() -> Vec<SupportedStandard> {
    vec![
        SupportedStandard {
            name: "ICRC-7".to_owned(),
            url: "https://github.com/dfinity/ICRC/ICRCs/ICRC-7".to_owned(),
        },
        SupportedStandard {
            name: "ICRC-10".to_owned(),
            url: "https://github.com/dfinity/ICRC/ICRCs/ICRC-10".to_owned(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn nat_to_deal_id(nat: &Nat) -> Option<DealId> {
    nat.0.to_string().parse::<DealId>().ok()
}

fn effective_take(take: Option<&Nat>) -> usize {
    let raw = take
        .and_then(nat_to_deal_id)
        .map_or(DEFAULT_TAKE, |t| t.min(MAX_TAKE));
    usize::try_from(raw).unwrap_or(usize::MAX)
}

/// Computes the (inclusive) start key from an exclusive `prev` cursor.
///
/// Returns `None` when the cursor overflows `u64`, signalling an empty result.
fn start_after_id(prev: Option<&Nat>) -> Option<DealId> {
    match prev {
        None => Some(0),
        Some(p) => nat_to_deal_id(p).and_then(|id| id.checked_add(1)),
    }
}

fn cap_batch(ids: &[Nat]) -> &[Nat] {
    let limit = usize::try_from(MAX_QUERY_BATCH_SIZE).unwrap_or(usize::MAX);
    if ids.len() > limit {
        &ids[..limit]
    } else {
        ids
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use candid::{Nat, Principal};

    use super::{
        balance_of, collection_metadata, name, owner_of, supported_standards, symbol,
        token_metadata, tokens, tokens_of, total_supply, transfer,
    };
    use crate::{
        memory::insert_new_deal,
        types::{
            deal::{Consent, Deal, DealMetadata, DealStatus},
            icrc7::{Icrc7TransferArg, Icrc7TransferResponse, Value, COLLECTION_NAME},
            ledger_types::Account,
        },
    };

    fn test_principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    fn store_deal(status: DealStatus, payer: Principal, recipient: Option<Principal>) -> Deal {
        insert_new_deal(|deal_id| Deal {
            id: deal_id,
            payer: Some(payer),
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
            settled_at_ns: None,
            refunded_at_ns: None,
            funding_tx: None,
            payout_tx: None,
            refund_tx: None,
            claim_code: None,
            payer_consent: Consent::Accepted,
            recipient_consent: Consent::Pending,
            metadata: Some(DealMetadata {
                title: Some("Tip".to_owned()),
                note: None,
            }),
        })
    }

    // --- Collection-level ---

    #[test]
    fn name_returns_constant() {
        assert_eq!(name(), COLLECTION_NAME);
    }

    #[test]
    fn symbol_returns_constant() {
        assert_eq!(symbol(), "ESCROW");
    }

    #[test]
    fn total_supply_reflects_deals() {
        let before = total_supply();
        store_deal(DealStatus::Created, test_principal(10), None);
        store_deal(DealStatus::Funded, test_principal(10), None);
        let after = total_supply();
        assert_eq!(after, before + Nat::from(2_u64));
    }

    #[test]
    fn collection_metadata_has_name() {
        let meta = collection_metadata();
        let name_val = meta.iter().find(|(k, _)| k == "icrc7:name").map(|(_, v)| v);
        assert_eq!(name_val, Some(&Value::Text(COLLECTION_NAME.to_owned())));
    }

    // --- Token metadata ---

    #[test]
    fn metadata_for_existing_deal() {
        let deal = store_deal(DealStatus::Created, test_principal(11), None);
        let result = token_metadata(&[Nat::from(deal.id)]);
        assert_eq!(result.len(), 1);
        let meta = result[0].as_ref().expect("metadata should exist");
        let has_name = meta.iter().any(|(k, _)| k == "icrc7:name");
        assert!(has_name);
    }

    #[test]
    fn metadata_for_unknown_id() {
        let result = token_metadata(&[Nat::from(999_999_999_u64)]);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_none());
    }

    #[test]
    fn metadata_batch_mixed() {
        let deal = store_deal(DealStatus::Funded, test_principal(12), None);
        let result = token_metadata(&[Nat::from(deal.id), Nat::from(999_999_998_u64)]);
        assert_eq!(result.len(), 2);
        assert!(result[0].is_some());
        assert!(result[1].is_none());
    }

    // --- Owner of ---

    #[test]
    fn owner_of_created_deal() {
        let payer = test_principal(13);
        let deal = store_deal(DealStatus::Created, payer, None);
        let result = owner_of(&[Nat::from(deal.id)]);
        assert_eq!(result.len(), 1);
        let account = result[0].as_ref().expect("owner should exist");
        assert_eq!(account.owner, payer);
        assert!(account.subaccount.is_none());
    }

    #[test]
    fn owner_of_settled_deal() {
        let payer = test_principal(14);
        let recip = test_principal(15);
        let deal = store_deal(DealStatus::Settled, payer, Some(recip));
        let result = owner_of(&[Nat::from(deal.id)]);
        assert_eq!(result[0].as_ref().unwrap().owner, recip);
    }

    #[test]
    fn owner_of_unknown_id() {
        let result = owner_of(&[Nat::from(999_999_997_u64)]);
        assert!(result[0].is_none());
    }

    // --- Balance of ---

    #[test]
    fn balance_of_counts_correctly() {
        let payer = test_principal(16);
        store_deal(DealStatus::Created, payer, None);
        store_deal(DealStatus::Funded, payer, None);

        let account = Account {
            owner: payer,
            subaccount: None,
        };
        let result = balance_of(&[account]);
        let balance: u64 = result[0].0.to_string().parse().unwrap();
        assert!(balance >= 2);
    }

    #[test]
    fn balance_of_subaccount_returns_zero() {
        let payer = test_principal(17);
        store_deal(DealStatus::Created, payer, None);

        let account = Account {
            owner: payer,
            subaccount: Some(vec![1_u8; 32]),
        };
        let result = balance_of(&[account]);
        assert_eq!(result[0], Nat::from(0_u64));
    }

    #[test]
    fn balance_of_multiple_accounts() {
        let a = test_principal(18);
        let b = test_principal(19);
        store_deal(DealStatus::Created, a, None);

        let accounts = vec![
            Account {
                owner: a,
                subaccount: None,
            },
            Account {
                owner: b,
                subaccount: None,
            },
        ];
        let result = balance_of(&accounts);
        assert_eq!(result.len(), 2);
        let a_balance: u64 = result[0].0.to_string().parse().unwrap();
        assert!(a_balance >= 1);
    }

    // --- Tokens pagination ---

    #[test]
    fn tokens_returns_ids_in_order() {
        let d1 = store_deal(DealStatus::Created, test_principal(20), None);
        let d2 = store_deal(DealStatus::Created, test_principal(20), None);
        let ids = tokens(None, Some(&Nat::from(500_u64)));
        assert!(ids.contains(&Nat::from(d1.id)));
        assert!(ids.contains(&Nat::from(d2.id)));

        let positions: Vec<usize> = ids
            .iter()
            .enumerate()
            .filter(|(_, id)| *id == &Nat::from(d1.id) || *id == &Nat::from(d2.id))
            .map(|(i, _)| i)
            .collect();
        if positions.len() == 2 {
            assert!(positions[0] < positions[1]);
        }
    }

    #[test]
    fn tokens_with_prev_cursor() {
        let d1 = store_deal(DealStatus::Created, test_principal(21), None);
        let d2 = store_deal(DealStatus::Created, test_principal(21), None);
        let ids = tokens(Some(&Nat::from(d1.id)), Some(&Nat::from(500_u64)));
        assert!(!ids.contains(&Nat::from(d1.id)));
        assert!(ids.contains(&Nat::from(d2.id)));
    }

    #[test]
    fn tokens_respects_take_limit() {
        store_deal(DealStatus::Created, test_principal(22), None);
        store_deal(DealStatus::Created, test_principal(22), None);
        let ids = tokens(None, Some(&Nat::from(1_u64)));
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn tokens_empty_when_cursor_past_end() {
        let ids = tokens(Some(&Nat::from(u64::MAX - 1)), Some(&Nat::from(50_u64)));
        assert!(ids.is_empty());
    }

    // --- Tokens of (filtered) ---

    #[test]
    fn tokens_of_filters_by_owner() {
        let owner = test_principal(23);
        let other = test_principal(24);
        let d1 = store_deal(DealStatus::Created, owner, None);
        store_deal(DealStatus::Created, other, None);

        let account = Account {
            owner,
            subaccount: None,
        };
        let ids = tokens_of(&account, None, Some(&Nat::from(500_u64)));
        assert!(ids.contains(&Nat::from(d1.id)));
    }

    #[test]
    fn tokens_of_with_subaccount_returns_empty() {
        let owner = test_principal(25);
        store_deal(DealStatus::Created, owner, None);
        let account = Account {
            owner,
            subaccount: Some(vec![1_u8; 32]),
        };
        let ids = tokens_of(&account, None, Some(&Nat::from(500_u64)));
        assert!(ids.is_empty());
    }

    #[test]
    fn tokens_of_pagination() {
        let owner = test_principal(26);
        let d1 = store_deal(DealStatus::Created, owner, None);
        let d2 = store_deal(DealStatus::Created, owner, None);

        let account = Account {
            owner,
            subaccount: None,
        };
        let page1 = tokens_of(&account, None, Some(&Nat::from(1_u64)));
        assert_eq!(page1.len(), 1);

        let page2 = tokens_of(&account, Some(&page1[0]), Some(&Nat::from(500_u64)));
        assert!(page2.contains(&Nat::from(d2.id)));
        assert!(!page2.contains(&Nat::from(d1.id)));
    }

    // --- Transfer (always rejects) ---

    #[test]
    fn transfer_rejects_all() {
        let args = vec![Icrc7TransferArg {
            from_subaccount: None,
            to: Account {
                owner: test_principal(30),
                subaccount: None,
            },
            token_id: Nat::from(1_u64),
            memo: None,
            created_at_time: None,
        }];
        let result = transfer(&args);
        assert_eq!(result.len(), 1);
        let resp = result[0].as_ref().expect("response should be present");
        assert!(matches!(resp, Icrc7TransferResponse::Err(_)));
    }

    // --- Supported standards ---

    #[test]
    fn standards_include_icrc7() {
        let stds = supported_standards();
        assert!(stds.iter().any(|s| s.name == "ICRC-7"));
    }

    #[test]
    fn standards_include_icrc10() {
        let stds = supported_standards();
        assert!(stds.iter().any(|s| s.name == "ICRC-10"));
    }
}
