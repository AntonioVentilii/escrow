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
        ProcessExpiredDealsResult, ReclaimDealResult, RejectDealResult, SignDealResult,
    },
};
use crate::{
    guards::caller_is_not_anonymous,
    services,
    types::deal::{DealId, Signature},
};

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
/// `Cancelled`. Any reserves already deposited (the receiver's
/// `DC/2` on a 3a deal where consent already moved money) are
/// refunded; the operator retains its `escrow_fee` share when a
/// reserve was on hand. Funded deals cannot be cancelled — use
/// [`reclaim_deal`] after expiry instead.
#[update(guard = "caller_is_not_anonymous")]
pub async fn cancel_deal(CancelDealArgs { deal_id }: CancelDealArgs) -> CancelDealResult {
    services::deals::cancel(msg_caller(), deal_id, time())
        .await
        .into()
}

/// Explicitly consents to a deal's terms.
///
/// The caller must be the payer or recipient. For the bound
/// receiver of a deal in `Created` state, `consent_deal` performs
/// the ICRC-2 deposit of the receiver's `DC/2` dispute reserve
/// into the deal subaccount — receivers must therefore approve
/// the escrow canister to spend at least `DC/2 + ledger_fee`
/// beforehand. Payer consent is a pure state flip (the payer's
/// actual commitment is `fund_deal`, which pulls `amount + DC/2`).
#[update(guard = "caller_is_not_anonymous")]
pub async fn consent_deal(ConsentDealArgs { deal_id }: ConsentDealArgs) -> ConsentDealResult {
    services::deals::consent(msg_caller(), deal_id, time())
        .await
        .into()
}

/// Rejects a deal's terms. The deal transitions to `Rejected` (terminal).
///
/// The caller must be the payer or recipient. Their consent is set
/// to `Rejected` and the deal becomes final. Any reserves already
/// deposited are refunded; the operator retains its `escrow_fee`
/// share when a reserve was on hand.
#[update(guard = "caller_is_not_anonymous")]
pub async fn reject_deal(RejectDealArgs { deal_id }: RejectDealArgs) -> RejectDealResult {
    services::deals::reject(msg_caller(), deal_id, time())
        .await
        .into()
}

/// Records the caller's `Yes` settlement signature on a `Funded`
/// bound deal and dispatches the resulting two-party tally:
///
/// - other party also `Yes` → settle (release to recipient).
/// - other party `No` → auto-open a dispute.
/// - other party still `Empty` → no-op; deal stays `Funded` with the new signature recorded.
///
/// Caller must be the bound payer or recipient. Tip flows
/// (`recipient = None`) reject with `DisputeRequiresBoundRecipient`
/// — use `accept_deal` (with the claim code) to claim a tip.
/// While the deal is still `Funded`, re-signing overwrites the
/// previous vote (latest-wins). At expiry the auto-YES rule (run by
/// the housekeeping sweep) upgrades any unsigned party to `Yes`
/// automatically; calling `sign_yes` after expiry returns `Expired`
/// to make the transition explicit.
///
/// Paired with [`sign_no`] — split into two endpoints (instead of
/// a single `sign_deal(vote)`) to match the canister's
/// `verb + deal_id` convention used by every other deal-action
/// endpoint, and to make "sign with empty vote" unrepresentable
/// at the Candid boundary.
#[update(guard = "caller_is_not_anonymous")]
pub async fn sign_yes(FundDealArgs { deal_id }: FundDealArgs) -> SignDealResult {
    services::deals::sign(msg_caller(), deal_id, Signature::Yes, time())
        .await
        .into()
}

/// Records the caller's `No` settlement signature on a `Funded`
/// bound deal and dispatches the resulting two-party tally:
///
/// - other party also `No` → abort (refund to payer; new `Aborted` terminal).
/// - other party `Yes` → auto-open a dispute.
/// - other party still `Empty` → no-op; deal stays `Funded` with the new signature recorded.
///
/// Same caller / tip / re-sign / post-expiry semantics as
/// [`sign_yes`].
#[update(guard = "caller_is_not_anonymous")]
pub async fn sign_no(FundDealArgs { deal_id }: FundDealArgs) -> SignDealResult {
    services::deals::sign(msg_caller(), deal_id, Signature::No, time())
        .await
        .into()
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
