//! Controller-only admin service. Wraps the canister-owned
//! treasury subaccount where every bound deal's `creation_fee`
//! accumulates.
//!
//! The treasury subaccount itself is constant
//! ([`crate::subaccounts::treasury_subaccount`]); the canister
//! never auto-drains it. Only the controller — via
//! `admin_treasury_withdraw` — can move funds out.

use ic_cdk::api::canister_self;

use crate::{
    api::deals::errors::EscrowError,
    ledger,
    subaccounts::treasury_subaccount,
    types::{asset::Asset, ledger_types::Account},
};

/// Returns the canister-owned treasury subaccount as an
/// [`Account`]. Same shape every caller needs (`balance_of` /
/// `transfer`).
#[must_use]
pub fn treasury_account() -> Account {
    Account {
        owner: canister_self(),
        subaccount: Some(treasury_subaccount()),
    }
}

/// Returns the live `icrc1_balance_of` of the treasury subaccount
/// for the given asset.
pub async fn treasury_balance(asset: &Asset) -> Result<u128, EscrowError> {
    let ledger = asset.as_icrc()?;
    ledger::balance_of(ledger, treasury_account()).await
}

/// Drains `amount` of `asset` from the treasury subaccount to
/// `to`. Returns the ledger block index of the resulting
/// `icrc1_transfer`.
///
/// `amount` must exceed the live ledger fee (the ledger burns one
/// fee per transfer); otherwise the underlying ledger call returns
/// an error and we surface it as `EscrowError::TransferFailed`.
pub async fn treasury_withdraw(
    asset: &Asset,
    to: Account,
    amount: u128,
) -> Result<u128, EscrowError> {
    let ledger = asset.as_icrc()?;
    ledger::transfer(ledger, Some(treasury_subaccount()), to, amount).await
}
