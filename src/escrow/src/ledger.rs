use candid::{Nat, Principal};
use ic_cdk::call::Call;

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

    let response = Call::unbounded_wait(ledger, "icrc2_transfer_from")
        .with_args(&(args,))
        .await
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    let (inner_result,): (Result<Nat, TransferFromError>,) = response
        .candid_tuple()
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    match inner_result {
        Ok(block_index) => nat_to_u128(&block_index),
        Err(e) => Err(EscrowError::TransferFailed(format!("{e:?}"))),
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

    let response = Call::unbounded_wait(ledger, "icrc1_transfer")
        .with_args(&(args,))
        .await
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    let (inner_result,): (Result<Nat, TransferError>,) = response
        .candid_tuple()
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    match inner_result {
        Ok(block_index) => nat_to_u128(&block_index),
        Err(e) => Err(EscrowError::TransferFailed(format!("{e:?}"))),
    }
}

/// Queries `icrc1_fee` on `ledger`. Returns the per-transfer fee
/// (in token units) the ledger will charge on subsequent
/// `icrc1_transfer` / `icrc2_transfer_from` calls.
///
/// Used by `services::disputes::finalize` (RFC-001 step 7) to size
/// the prevailing-party payout so the per-arbitrator transfers'
/// ledger fees are absorbed by the prevailing party (Q10 refinement #1).
pub async fn fee(ledger: Principal) -> Result<u128, EscrowError> {
    let response = Call::unbounded_wait(ledger, "icrc1_fee")
        .await
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    let (fee_nat,): (Nat,) = response
        .candid_tuple()
        .map_err(|e| EscrowError::LedgerError(format!("icrc1_fee decode failed: {e:?}")))?;

    nat_to_u128(&fee_nat)
}

/// Calls the IC management canister to obtain 32 bytes of cryptographic randomness.
pub async fn raw_rand() -> Result<(Vec<u8>,), EscrowError> {
    let response = Call::unbounded_wait(Principal::management_canister(), "raw_rand")
        .await
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    response
        .candid_tuple()
        .map_err(|e| EscrowError::LedgerError(format!("raw_rand decode failed: {e:?}")))
}

fn nat_to_u128(nat: &Nat) -> Result<u128, EscrowError> {
    let s = nat.0.to_string();
    s.parse::<u128>()
        .map_err(|_| EscrowError::LedgerError("Block index exceeds u128".to_owned()))
}
