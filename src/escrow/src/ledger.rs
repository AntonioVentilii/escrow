use candid::{Nat, Principal};
use ic_cdk::call;

use crate::{
    api::deals::errors::EscrowError,
    types::ledger_types::{
        Account, TransferArg, TransferError, TransferFromArgs, TransferFromError,
    },
};

/// Transfers tokens from `from` to `to` via ICRC-2 `transfer_from`.
///
/// The escrow canister must have been approved as spender beforehand.
/// Returns the ledger block index on success.
pub async fn transfer_from(
    ledger: Principal,
    from: Account,
    to: Account,
    amount: u128,
) -> Result<u128, EscrowError> {
    let args = TransferFromArgs {
        spender_subaccount: None,
        from,
        to,
        amount: Nat::from(amount),
        fee: None,
        memo: None,
        created_at_time: None,
    };

    let result: Result<(Result<Nat, TransferFromError>,), _> =
        call(ledger, "icrc2_transfer_from", (args,)).await;

    match result {
        Ok((Ok(block_index),)) => nat_to_u128(&block_index),
        Ok((Err(e),)) => Err(EscrowError::TransferFailed(format!("{e:?}"))),
        Err((code, msg)) => Err(EscrowError::LedgerError(format!("{code:?}: {msg}"))),
    }
}

/// Transfers tokens from a canister-owned subaccount via ICRC-1 `transfer`.
///
/// Used for settlement (to recipient) and refund (back to payer).
/// Returns the ledger block index on success.
pub async fn transfer(
    ledger: Principal,
    from_subaccount: Option<Vec<u8>>,
    to: Account,
    amount: u128,
) -> Result<u128, EscrowError> {
    let args = TransferArg {
        from_subaccount,
        to,
        amount: Nat::from(amount),
        fee: None,
        memo: None,
        created_at_time: None,
    };

    let result: Result<(Result<Nat, TransferError>,), _> =
        call(ledger, "icrc1_transfer", (args,)).await;

    match result {
        Ok((Ok(block_index),)) => nat_to_u128(&block_index),
        Ok((Err(e),)) => Err(EscrowError::TransferFailed(format!("{e:?}"))),
        Err((code, msg)) => Err(EscrowError::LedgerError(format!("{code:?}: {msg}"))),
    }
}

fn nat_to_u128(nat: &Nat) -> Result<u128, EscrowError> {
    let s = nat.0.to_string();
    s.parse::<u128>()
        .map_err(|_| EscrowError::LedgerError("Block index exceeds u128".to_owned()))
}
