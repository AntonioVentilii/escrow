use ic_cdk::{api::time, caller};
use ic_cdk_macros::{query, update};

use super::{
    errors::EscrowError,
    params::{
        AcceptDealArgs, CancelDealArgs, CreateDealArgs, FundDealArgs, ListMyDealsArgs,
        ReclaimDealArgs,
    },
    results::{ClaimableDealView, DealView},
};
use crate::{
    guards::caller_is_not_anonymous,
    services,
    types::{deal::DealId, ledger_types::Account},
};

// ---------------------------------------------------------------------------
// Update methods
// ---------------------------------------------------------------------------

#[update(guard = "caller_is_not_anonymous")]
pub fn create_deal(args: CreateDealArgs) -> Result<DealView, EscrowError> {
    services::deals::create(caller(), args, time())
}

#[update(guard = "caller_is_not_anonymous")]
pub async fn fund_deal(args: FundDealArgs) -> Result<DealView, EscrowError> {
    services::deals::fund(caller(), args.deal_id).await
}

#[update(guard = "caller_is_not_anonymous")]
pub async fn accept_deal(args: AcceptDealArgs) -> Result<DealView, EscrowError> {
    services::deals::accept(caller(), args.deal_id, time()).await
}

#[update(guard = "caller_is_not_anonymous")]
pub async fn reclaim_deal(args: ReclaimDealArgs) -> Result<DealView, EscrowError> {
    services::deals::reclaim(caller(), args.deal_id, time()).await
}

#[update(guard = "caller_is_not_anonymous")]
#[expect(clippy::needless_pass_by_value)]
pub fn cancel_deal(args: CancelDealArgs) -> Result<DealView, EscrowError> {
    services::deals::cancel(caller(), args.deal_id)
}

#[update(guard = "caller_is_not_anonymous")]
pub async fn process_expired_deals(limit: u32) -> Result<Vec<DealId>, EscrowError> {
    services::expiry::process_expired(limit).await
}

// ---------------------------------------------------------------------------
// Query methods
// ---------------------------------------------------------------------------

#[query(guard = "caller_is_not_anonymous")]
pub fn get_deal(deal_id: DealId) -> Result<DealView, EscrowError> {
    services::deals::get(caller(), deal_id)
}

#[query(guard = "caller_is_not_anonymous")]
#[expect(clippy::needless_pass_by_value, clippy::cast_possible_truncation)]
#[must_use]
pub fn list_my_deals(args: ListMyDealsArgs) -> Vec<DealView> {
    let offset = args.offset.unwrap_or(0) as usize;
    let limit = args.limit.unwrap_or(50) as usize;
    services::deals::list_for_caller(caller(), offset, limit)
}

/// Reduced public view for claim/share-link pages.
/// Returns limited info (no payer, no internal fields). Any authenticated
/// caller may query this — authorization is intentionally open so a
/// not-yet-bound recipient can preview the tip before accepting.
#[query(guard = "caller_is_not_anonymous")]
pub fn get_claimable_deal(deal_id: DealId) -> Result<ClaimableDealView, EscrowError> {
    services::deals::get_claimable(deal_id)
}

/// Returns the escrow `Account` (canister principal + deal subaccount) for a deal.
#[query(guard = "caller_is_not_anonymous")]
pub fn get_escrow_account(deal_id: DealId) -> Result<Account, EscrowError> {
    services::deals::get_escrow_account(caller(), deal_id)
}
