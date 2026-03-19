use candid::Principal;
use ic_cdk_macros::query;

use super::results::ReliabilityView;
use crate::{guards::caller_is_not_anonymous, services};

/// Returns the reliability score for any principal.
///
/// This is a public query — any authenticated caller may check any
/// principal's score. No authorization required beyond being non-anonymous.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_reliability(principal: Principal) -> ReliabilityView {
    services::reliability::compute(principal).into()
}
