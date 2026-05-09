use ic_cdk::api::{msg_caller, time};
use ic_cdk_macros::{query, update};

use super::{
    params::{
        AcceptDealArgs, CancelDealArgs, ConsentDealArgs, CreateDealArgs, FundDealArgs,
        ListMyDealsArgs, ReclaimDealArgs, RejectDealArgs,
    },
    results::{
        AcceptDealResult, CancelDealResult, ConsentDealResult, CreateDealResult, DealView,
        FundDealResult, GetClaimableDealResult, GetDealResult, GetEscrowAccountResult,
        ProcessExpiredDealsResult, ReclaimDealResult, RejectDealResult,
    },
};
use crate::{guards::caller_is_not_anonymous, services, types::deal::DealId};

// ---------------------------------------------------------------------------
// Update methods
// ---------------------------------------------------------------------------

/// Creates a new escrow deal.
///
/// The caller is automatically assigned as one of the parties based on the
/// supplied `payer` and `recipient` fields. Their consent is set to `Accepted`;
/// the counterparty's consent starts as `Pending`.
///
/// A cryptographically random claim code is generated and returned in the
/// `DealView`. This code must be included in QR / share links so that an
/// unbound recipient can later claim the deal.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub async fn create_deal(args: CreateDealArgs) -> CreateDealResult {
    services::deals::create(msg_caller(), args, time())
        .await
        .into()
}

/// Funds a previously created deal by transferring tokens from the payer's
/// account into the deal's escrow subaccount via ICRC-2 `transfer_from`.
///
/// The deal transitions from `Created` to `Funded`. Funding implicitly sets
/// the payer's consent to `Accepted`. For deals with a known recipient, the
/// recipient must have consented first.
#[update(guard = "caller_is_not_anonymous")]
pub async fn fund_deal(FundDealArgs { deal_id }: FundDealArgs) -> FundDealResult {
    services::deals::fund(msg_caller(), deal_id).await.into()
}

/// Accepts (claims) a funded deal, releasing the escrowed tokens to the caller.
///
/// If the deal has no bound recipient, the caller must supply the correct
/// `claim_code`. The caller is bound as the recipient and their consent is
/// automatically set to `Accepted`. The deal transitions from `Funded` to
/// `Settled`.
#[update(guard = "caller_is_not_anonymous")]
pub async fn accept_deal(
    AcceptDealArgs {
        deal_id,
        claim_code,
    }: AcceptDealArgs,
) -> AcceptDealResult {
    services::deals::accept(msg_caller(), deal_id, time(), claim_code)
        .await
        .into()
}

/// Reclaims escrowed funds from an expired, unclaimed deal back to the payer.
///
/// Only callable after the deal's `expires_at_ns` deadline has passed. The deal
/// transitions from `Funded` to `Refunded`.
#[update(guard = "caller_is_not_anonymous")]
pub async fn reclaim_deal(ReclaimDealArgs { deal_id }: ReclaimDealArgs) -> ReclaimDealResult {
    services::deals::reclaim(msg_caller(), deal_id, time())
        .await
        .into()
}

/// Cancels a deal that has not yet been funded.
///
/// Either party may cancel. The deal transitions from `Created` to
/// `Cancelled`. Funded deals cannot be cancelled — use [`reclaim_deal`] after
/// expiry instead.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn cancel_deal(CancelDealArgs { deal_id }: CancelDealArgs) -> CancelDealResult {
    services::deals::cancel(msg_caller(), deal_id, time()).into()
}

/// Explicitly consents to a deal's terms.
///
/// The caller must be the payer or recipient. Sets their consent to `Accepted`.
/// Both parties must consent before the payer can fund a deal with a known
/// recipient.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn consent_deal(ConsentDealArgs { deal_id }: ConsentDealArgs) -> ConsentDealResult {
    services::deals::consent(msg_caller(), deal_id, time()).into()
}

/// Rejects a deal's terms. The deal transitions to `Rejected` (terminal).
///
/// The caller must be the payer or recipient. Their consent is set to
/// `Rejected` and the deal becomes final.
#[update(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn reject_deal(RejectDealArgs { deal_id }: RejectDealArgs) -> RejectDealResult {
    services::deals::reject(msg_caller(), deal_id, time()).into()
}

/// Batch-processes expired deals by refunding escrowed tokens back to their
/// payers.
///
/// Scans up to `limit` expired-but-still-funded deals and attempts to reclaim
/// each one. Returns the IDs of deals that were successfully refunded.
#[update(guard = "caller_is_not_anonymous")]
pub async fn process_expired_deals(limit: u32) -> ProcessExpiredDealsResult {
    services::expiry::process_expired(limit).await.into()
}

// ---------------------------------------------------------------------------
// Query methods
// ---------------------------------------------------------------------------

/// Returns the full deal view for an authorised participant.
///
/// Only the payer or the bound recipient may query a deal's full details.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_deal(deal_id: DealId) -> GetDealResult {
    services::deals::get(msg_caller(), deal_id).into()
}

/// Lists all deals where the caller is either the payer or the recipient,
/// ordered by creation time with pagination support.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn list_my_deals(ListMyDealsArgs { offset, limit }: ListMyDealsArgs) -> Vec<DealView> {
    let offset_u64 = offset.unwrap_or(0);
    let offset_usize = usize::try_from(offset_u64).unwrap_or(usize::MAX);
    let limit_u64 = limit.unwrap_or(50).min(100);
    let limit_usize = usize::try_from(limit_u64).unwrap_or(100);
    services::deals::list_for_caller(msg_caller(), offset_usize, limit_usize)
}

/// Reduced public view for claim/share-link pages.
/// Returns limited info (no payer, no claim code, no internal fields). Any
/// authenticated caller may query this — authorization is intentionally open
/// so a not-yet-bound recipient can preview the deal before accepting.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_claimable_deal(deal_id: DealId) -> GetClaimableDealResult {
    services::deals::get_claimable(deal_id).into()
}

/// Returns the escrow `Account` (canister principal + deal subaccount) for a deal.
#[query(guard = "caller_is_not_anonymous")]
#[must_use]
pub fn get_escrow_account(deal_id: DealId) -> GetEscrowAccountResult {
    services::deals::get_escrow_account(msg_caller(), deal_id).into()
}
