use ic_cdk::api::{canister_self, msg_caller, time};
use ic_cdk_macros::{query, update};

use super::{
    params::{AdminRegisterArbitratorArgs, AdminSetArbitratorStatusArgs},
    results::{AdminRegisterArbitratorResult, AdminSetArbitratorStatusResult, UpdateConfigResult},
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
