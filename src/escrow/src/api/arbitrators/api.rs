use candid::Principal;
use ic_cdk::api::{msg_caller, time};
use ic_cdk_macros::{query, update};

use super::{
    params::{ListArbitratorsArgs, RegisterArbitratorArgs},
    results::{DeregisterArbitratorResult, RegisterArbitratorResult},
};
use crate::{guards::caller_is_not_anonymous, services, types::arbitrator::ArbitratorProfile};

// ---------------------------------------------------------------------------
// Update methods (RFC-001 step 3)
// ---------------------------------------------------------------------------

/// Self-registers the caller as an arbitrator. Idempotent — re-registration
/// returns the existing profile (with the new bio if supplied) and
/// reactivates a previously deregistered profile.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn register_arbitrator(args: RegisterArbitratorArgs) -> RegisterArbitratorResult {
    services::arbitrators::register(msg_caller(), args.bio, time()).into()
}

/// Self-deregisters the caller's arbitrator profile. In-flight assignments
/// are honoured — a non-vote from a deregistered arbitrator counts as
/// `Vote::Abstain` at finalize time (RFC-001 Q5).
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn deregister_arbitrator() -> DeregisterArbitratorResult {
    services::arbitrators::deregister(msg_caller()).into()
}

// ---------------------------------------------------------------------------
// Query methods (RFC-001 step 3)
// ---------------------------------------------------------------------------

/// Returns the arbitrator profile for `principal`, or `None` if the
/// principal hasn't registered. Public read; any non-anonymous caller
/// may query any principal.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_arbitrator(principal: Principal) -> Option<ArbitratorProfile> {
    services::arbitrators::get(principal)
}

/// Lists registered arbitrators with optional `status` and `min_score`
/// filters and pagination.
///
/// Destructured at the signature so clippy's `needless_pass_by_value` is
/// satisfied — Candid decodes into owned values, then we hand the
/// re-assembled struct to the service by reference.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn list_arbitrators(
    ListArbitratorsArgs {
        offset,
        limit,
        status,
        min_score,
    }: ListArbitratorsArgs,
) -> Vec<ArbitratorProfile> {
    services::arbitrators::list(&ListArbitratorsArgs {
        offset,
        limit,
        status,
        min_score,
    })
}
