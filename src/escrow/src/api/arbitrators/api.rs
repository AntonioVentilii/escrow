use candid::Principal;
use ic_cdk::api::msg_caller;
use ic_cdk_macros::{query, update};

use super::{params::ListArbitratorsArgs, results::DeregisterArbitratorResult};
use crate::{guards::caller_is_not_anonymous, services, types::arbitrator::ArbitratorProfile};

// ---------------------------------------------------------------------------
// Update methods
// ---------------------------------------------------------------------------

/// Self-deregisters the caller's arbitrator profile. Opt-out is a
/// fundamental right that doesn't require admin intervention. The
/// status flips to `Deregistered`; in-flight assignments are honoured
/// (a non-vote then counts as `Vote::Abstain` at finalize time).
///
/// To re-enter the pool the caller must be re-registered by an admin
/// via `admin_register_arbitrator` — the curated registration model
/// (admin chooses who's in) does not allow self-resurrection.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn deregister_arbitrator() -> DeregisterArbitratorResult {
    services::arbitrators::deregister(msg_caller()).into()
}

// ---------------------------------------------------------------------------
// Query methods
// ---------------------------------------------------------------------------

/// Returns the arbitrator profile for `principal`, or `None` if the
/// principal hasn't been registered. Public read; any non-anonymous
/// caller may query any principal.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_arbitrator(principal: Principal) -> Option<ArbitratorProfile> {
    services::arbitrators::get(principal)
}

/// Lists registered arbitrators with optional `status` and `min_score`
/// filters and pagination.
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
