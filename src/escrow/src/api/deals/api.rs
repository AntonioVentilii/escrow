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

/// Creates a new escrow deal with the caller as the payer.
///
/// The deal starts in `Created` state and must be funded separately via
/// [`fund_deal`]. An optional recipient can be specified upfront; if omitted,
/// the recipient is bound on first acceptance (share-link / QR flow).
#[update(guard = "caller_is_not_anonymous")]
pub fn create_deal(args: CreateDealArgs) -> Result<DealView, EscrowError> {
    services::deals::create(caller(), args, time())
}

/// Funds a previously created deal by transferring tokens from the payer's
/// account into the deal's escrow subaccount via ICRC-2 `transfer_from`.
///
/// The deal transitions from `Created` to `Funded`. If the deal is already
/// funded, the current state is returned without performing a second transfer.
#[update(guard = "caller_is_not_anonymous")]
pub async fn fund_deal(args: FundDealArgs) -> Result<DealView, EscrowError> {
    services::deals::fund(caller(), args.deal_id).await
}

/// Accepts (claims) a funded deal, releasing the escrowed tokens to the caller.
///
/// If the deal has no bound recipient, the caller is bound as the recipient on
/// first acceptance. The deal transitions from `Funded` to `Completed`.
#[update(guard = "caller_is_not_anonymous")]
pub async fn accept_deal(args: AcceptDealArgs) -> Result<DealView, EscrowError> {
    services::deals::accept(caller(), args.deal_id, time()).await
}

/// Reclaims escrowed funds from an expired, unclaimed deal back to the payer.
///
/// Only callable after the deal's `expires_at_ns` deadline has passed. The deal
/// transitions from `Funded` to `Refunded`.
#[update(guard = "caller_is_not_anonymous")]
pub async fn reclaim_deal(args: ReclaimDealArgs) -> Result<DealView, EscrowError> {
    services::deals::reclaim(caller(), args.deal_id, time()).await
}

/// Cancels a deal that has not yet been funded.
///
/// Only the original payer may cancel. The deal transitions from `Created` to
/// `Cancelled`. Funded deals cannot be cancelled — use [`reclaim_deal`] after
/// expiry instead.
#[update(guard = "caller_is_not_anonymous")]
#[expect(clippy::needless_pass_by_value)]
pub fn cancel_deal(args: CancelDealArgs) -> Result<DealView, EscrowError> {
    services::deals::cancel(caller(), args.deal_id)
}

/// Batch-processes expired deals by refunding escrowed tokens back to their
/// payers.
///
/// Scans up to `limit` expired-but-still-funded deals and attempts to reclaim
/// each one. Returns the IDs of deals that were successfully refunded.
#[update(guard = "caller_is_not_anonymous")]
pub async fn process_expired_deals(limit: u32) -> Result<Vec<DealId>, EscrowError> {
    services::expiry::process_expired(limit).await
}

// ---------------------------------------------------------------------------
// Query methods
// ---------------------------------------------------------------------------

/// Returns the full deal view for an authorised participant.
///
/// Only the payer or the bound recipient may query a deal's full details.
#[query(guard = "caller_is_not_anonymous")]
pub fn get_deal(deal_id: DealId) -> Result<DealView, EscrowError> {
    services::deals::get(caller(), deal_id)
}

/// Lists all deals where the caller is either the payer or the recipient,
/// ordered by creation time with pagination support.
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
