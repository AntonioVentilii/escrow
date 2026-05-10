use candid::{CandidType, Deserialize, Principal};

use crate::types::arbitrator::ArbitratorStatus;

/// Arguments for `admin_register_arbitrator`.
///
/// Idempotent — calling for a principal that's already registered:
/// - if currently `Active`, returns the existing profile unchanged.
/// - if currently `Suspended` or `Deregistered`, reactivates the profile (status flips to `Active`)
///   and returns it.
///
/// Score-related counters and `registered_at_ns` are preserved across
/// reactivation; `registered_by` is updated to the calling controller.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminRegisterArbitratorArgs {
    /// The principal being registered as an arbitrator.
    pub principal: Principal,
}

/// Arguments for `admin_set_arbitrator_status`.
///
/// All transitions are allowed (Active ↔ Suspended ↔ Deregistered).
/// A self-transition (e.g. `Active → Active`) is a no-op success.
/// `Deregistered → Active` reactivates a profile previously removed —
/// equivalent to `admin_register_arbitrator` of the same principal.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct AdminSetArbitratorStatusArgs {
    pub principal: Principal,
    pub status: ArbitratorStatus,
}
