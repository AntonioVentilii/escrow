use candid::{CandidType, Deserialize, Principal};

use crate::types::{arbitrator::ArbitratorStatus, asset::Asset, ledger_types::Account};

/// Arguments for `admin_register_arbitrator`.
///
/// Idempotent — calling for an already-registered principal returns
/// the existing profile, with two side effects on every successful
/// call (regardless of prior status):
///
/// - Status is set to `Active` (reactivating `Suspended` / `Deregistered` profiles, no-op for
///   already-`Active`).
/// - `registered_by` is refreshed to the calling controller, so the audit trail reflects the most
///   recent curation event.
///
/// Score-related counters and `registered_at_ns` are preserved across
/// re-registration.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminRegisterArbitratorArgs {
    /// The principal being registered as an arbitrator.
    pub principal: Principal,
}

/// Arguments for `admin_set_arbitrator_status`.
///
/// All transitions are allowed (Active ↔ Suspended ↔ Deregistered).
/// A self-transition (e.g. `Active → Active`) is a no-op success.
/// `Deregistered → Active` reactivates a profile previously removed
/// — but unlike `admin_register_arbitrator` it does NOT refresh
/// `registered_by` (the audit trail still points at the original
/// admin that first added the arbitrator) and does NOT re-run the
/// `validate_arbitrator_principal` check (the principal was already
/// validated at original registration time). To both reactivate AND
/// refresh the audit trail, use `admin_register_arbitrator` instead.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminSetArbitratorStatusArgs {
    pub principal: Principal,
    pub status: ArbitratorStatus,
}

/// Arguments for `admin_treasury_balance` — the controller-only
/// query that returns how much of `asset` is currently sitting in
/// the canister's treasury subaccount (where every deal's
/// `creation_fee` accumulates). One asset per call so the
/// controller can iterate ledgers explicitly.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminTreasuryBalanceArgs {
    /// Settlement asset to query. Today only [`Asset::Icrc`]; the
    /// query reaches the underlying ledger via
    /// [`crate::ledger::balance_of`].
    pub asset: Asset,
}

/// Arguments for `admin_treasury_withdraw` — the controller-only
/// drain endpoint. Pulls `amount` of `asset` from the treasury
/// subaccount and `icrc1_transfer`s it to `to`. Caller is
/// responsible for sizing `amount` against the live treasury
/// balance (`admin_treasury_balance`); under-funded withdrawals
/// surface as `EscrowError::TransferFailed` from the ledger.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminTreasuryWithdrawArgs {
    pub asset: Asset,
    /// Destination account. Typically a controller's own account
    /// or a downstream treasury / ops wallet.
    pub to: Account,
    /// Amount to transfer in `asset`'s base units. Must be
    /// `> ledger_fee` (the ledger burns one fee per transfer).
    pub amount: u128,
}
