use ic_cdk::api::{canister_self, msg_caller, time};
use ic_cdk_macros::{query, update};

use super::{
    params::{
        AdminRegisterArbitratorArgs, AdminSetArbitratorStatusArgs, AdminTreasuryBalanceArgs,
        AdminTreasuryWithdrawArgs,
    },
    results::{
        AdminRegisterArbitratorResult, AdminSetArbitratorStatusResult, AdminTreasuryBalanceResult,
        AdminTreasuryWithdrawResult, UpdateConfigResult,
    },
};
use crate::{guards::caller_is_controller, memory::CONFIG, services, validation, Config};

/// Returns the current global configuration of the Escrow canister.
///
/// This method is gated to canister controllers.
#[query(guard = "caller_is_controller")]
#[must_use]
pub fn config() -> Config {
    CONFIG.with(|c| c.borrow().clone())
}

/// Updates the global configuration for the Escrow canister.
///
/// Validates `config.dispute_config` against the invariants documented
/// on `DisputeConfig` before persisting. Rejects with
/// `EscrowError::ValidationError` on invalid input
/// rather than letting bad config poison the dispute machinery at
/// runtime (e.g. even `panel_size`, zero windows, fee bps > 100%).
///
/// This method is gated to canister controllers.
#[update(guard = "caller_is_controller")]
#[must_use]
pub fn update_config(config: Config) -> UpdateConfigResult {
    if let Err(e) = validation::validate_config(&config) {
        return UpdateConfigResult::Err(e);
    }
    CONFIG.with(|c| {
        *c.borrow_mut() = config;
    });
    UpdateConfigResult::Ok
}

/// Registers `args.principal` as an arbitrator. Curated registration —
/// only canister controllers can add arbitrators to the pool.
///
/// Idempotent: re-calling for an already-registered principal returns
/// the existing profile (and reactivates it if it was `Suspended` or
/// `Deregistered`). Score counters and `registered_at_ns` are
/// preserved across reactivation; `registered_by` is updated to the
/// calling controller.
#[update(guard = "caller_is_controller")]
#[must_use]
pub fn admin_register_arbitrator(
    AdminRegisterArbitratorArgs { principal }: AdminRegisterArbitratorArgs,
) -> AdminRegisterArbitratorResult {
    services::arbitrators::admin_register(msg_caller(), principal, canister_self(), time()).into()
}

/// Sets an arbitrator's status. All transitions are allowed (Active ↔
/// Suspended ↔ Deregistered). Self-transitions are no-op success.
///
/// Returns `EscrowError::NotFound` if the target principal isn't
/// registered as an arbitrator.
#[update(guard = "caller_is_controller")]
#[must_use]
pub fn admin_set_arbitrator_status(
    args: AdminSetArbitratorStatusArgs,
) -> AdminSetArbitratorStatusResult {
    let AdminSetArbitratorStatusArgs { principal, status } = args;
    services::arbitrators::admin_set_status(principal, status).into()
}

/// Returns the live `icrc1_balance_of` of the canister-owned
/// treasury subaccount for the requested asset. Every bound deal's
/// `creation_fee` accumulates here and stays until a controller
/// drains it via `admin_treasury_withdraw`.
///
/// This method is gated to canister controllers — the treasury
/// balance is operationally sensitive (it reveals total accumulated
/// anti-spam fees across all deals).
#[update(guard = "caller_is_controller")]
#[must_use]
pub async fn admin_treasury_balance(
    AdminTreasuryBalanceArgs { asset }: AdminTreasuryBalanceArgs,
) -> AdminTreasuryBalanceResult {
    services::admin::treasury_balance(&asset).await.into()
}

/// Drains `amount` of `asset` from the treasury subaccount to
/// `to` via `icrc1_transfer`. Returns the ledger block index on
/// success.
///
/// Caller is responsible for sizing `amount` against the live
/// treasury balance — under-funded withdrawals surface as
/// `EscrowError::TransferFailed` from the ledger.
///
/// This method is gated to canister controllers.
#[update(guard = "caller_is_controller")]
#[must_use]
pub async fn admin_treasury_withdraw(
    AdminTreasuryWithdrawArgs { asset, to, amount }: AdminTreasuryWithdrawArgs,
) -> AdminTreasuryWithdrawResult {
    services::admin::treasury_withdraw(&asset, to, amount)
        .await
        .into()
}
