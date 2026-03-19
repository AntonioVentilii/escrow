use candid::Nat;
use ic_cdk_macros::{query, update};

use crate::{
    guards::caller_is_not_anonymous,
    services,
    types::{
        icrc7::{Icrc7TransferArg, Icrc7TransferResponse, SupportedStandard, Value},
        ledger_types::Account,
    },
};

// ---------------------------------------------------------------------------
// ICRC-7 collection-level queries
// ---------------------------------------------------------------------------

/// Returns the human-readable name of the NFT collection (`"Escrow Deals"`).
#[query]
#[must_use]
pub fn icrc7_name() -> String {
    services::icrc7::name()
}

/// Returns the ticker symbol of the NFT collection (`"ESCROW"`).
#[query]
#[must_use]
pub fn icrc7_symbol() -> String {
    services::icrc7::symbol()
}

/// Returns a human-readable description of the NFT collection.
#[query]
#[must_use]
pub fn icrc7_description() -> Option<String> {
    services::icrc7::description()
}

/// Returns the collection logo. Currently `None`.
#[query]
#[must_use]
pub fn icrc7_logo() -> Option<String> {
    services::icrc7::logo()
}

/// Returns the total number of deal NFTs that have been minted (one per deal).
#[query]
#[must_use]
pub fn icrc7_total_supply() -> Nat {
    services::icrc7::total_supply()
}

/// Returns the maximum number of deal NFTs that can ever exist.
///
/// Currently `None` (unlimited).
#[query]
#[must_use]
pub fn icrc7_supply_cap() -> Option<Nat> {
    services::icrc7::supply_cap()
}

/// Returns the maximum number of token IDs accepted by `icrc7_token_metadata`
/// and `icrc7_owner_of` in a single call.
#[query]
#[must_use]
pub fn icrc7_max_query_batch_size() -> Option<Nat> {
    services::icrc7::max_query_batch_size()
}

/// Returns the maximum number of transfer args accepted by `icrc7_transfer`
/// in a single call. Currently `None`.
#[query]
#[must_use]
pub fn icrc7_max_update_batch_size() -> Option<Nat> {
    services::icrc7::max_update_batch_size()
}

/// Returns the default page size for `icrc7_tokens` / `icrc7_tokens_of`
/// when the caller omits the `take` argument.
#[query]
#[must_use]
pub fn icrc7_default_take_value() -> Option<Nat> {
    services::icrc7::default_take_value()
}

/// Returns the maximum page size for `icrc7_tokens` / `icrc7_tokens_of`.
#[query]
#[must_use]
pub fn icrc7_max_take_value() -> Option<Nat> {
    services::icrc7::max_take_value()
}

/// Returns the maximum memo size accepted in transfer arguments.
///
/// Currently `None` (no explicit limit).
#[query]
#[must_use]
pub fn icrc7_max_memo_size() -> Option<Nat> {
    services::icrc7::max_memo_size()
}

/// Whether batch transfers are executed atomically. Currently `None`.
#[query]
#[must_use]
pub fn icrc7_atomic_batch_transfers() -> Option<bool> {
    services::icrc7::atomic_batch_transfers()
}

/// Returns the transaction deduplication window. Currently `None`.
#[query]
#[must_use]
pub fn icrc7_tx_window() -> Option<Nat> {
    services::icrc7::tx_window()
}

/// Returns the permitted time drift for deduplication. Currently `None`.
#[query]
#[must_use]
pub fn icrc7_permitted_drift() -> Option<Nat> {
    services::icrc7::permitted_drift()
}

/// Returns collection-level metadata as an ICRC-16 key-value map.
///
/// Includes `icrc7:name`, `icrc7:symbol`, `icrc7:description`, and
/// `icrc7:total_supply`.
#[query]
#[must_use]
pub fn icrc7_collection_metadata() -> Vec<(String, Value)> {
    services::icrc7::collection_metadata()
}

// ---------------------------------------------------------------------------
// ICRC-7 token-level queries
// ---------------------------------------------------------------------------

/// Returns per-token metadata for each requested token ID.
///
/// Each deal's metadata includes `icrc7:name`, `escrow:status`,
/// `escrow:payer`, `escrow:amount`, `escrow:token_ledger`, and other
/// deal-specific fields. Unknown IDs produce `None` in the result vector.
#[query]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_token_metadata(token_ids: Vec<Nat>) -> Vec<Option<Vec<(String, Value)>>> {
    services::icrc7::token_metadata(&token_ids)
}

/// Returns the owner account for each requested token ID.
///
/// Ownership follows deal lifecycle: the payer owns the token in all states
/// except `Completed`, where the recipient becomes the owner.
/// Unknown IDs produce `None`.
#[query]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_owner_of(token_ids: Vec<Nat>) -> Vec<Option<Account>> {
    services::icrc7::owner_of(&token_ids)
}

/// Returns the number of deal NFTs owned by each requested account.
///
/// Accounts with a non-default subaccount always return `0`.
#[query]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_balance_of(accounts: Vec<Account>) -> Vec<Nat> {
    services::icrc7::balance_of(&accounts)
}

/// Returns a page of token IDs in ascending order.
///
/// `prev` is the last token ID the caller received (exclusive cursor).
/// `take` limits the page size (defaults to 50, capped at 500).
#[query]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_tokens(prev: Option<Nat>, take: Option<Nat>) -> Vec<Nat> {
    services::icrc7::tokens(prev.as_ref(), take.as_ref())
}

/// Returns a page of token IDs owned by `account`, in ascending order.
///
/// See [`icrc7_tokens`] for cursor / take semantics.
#[query]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_tokens_of(account: Account, prev: Option<Nat>, take: Option<Nat>) -> Vec<Nat> {
    services::icrc7::tokens_of(&account, prev.as_ref(), take.as_ref())
}

// ---------------------------------------------------------------------------
// ICRC-7 transfer (always rejected — escrow manages ownership)
// ---------------------------------------------------------------------------

/// Rejects all transfer attempts with a `GenericError`.
///
/// Deal ownership transitions are managed exclusively through escrow
/// operations (`accept_deal`, `reclaim_deal`, …), not via direct ICRC-7
/// transfers.
#[update(guard = "caller_is_not_anonymous")]
#[expect(clippy::needless_pass_by_value)]
#[must_use]
pub fn icrc7_transfer(args: Vec<Icrc7TransferArg>) -> Vec<Option<Icrc7TransferResponse>> {
    services::icrc7::transfer(&args)
}

// ---------------------------------------------------------------------------
// ICRC-10 supported standards
// ---------------------------------------------------------------------------

/// Returns the list of ICRC standards supported by this canister.
///
/// Currently reports ICRC-7 (NFT) and ICRC-10 (supported-standards discovery).
#[query]
#[must_use]
pub fn icrc10_supported_standards() -> Vec<SupportedStandard> {
    services::icrc7::supported_standards()
}
