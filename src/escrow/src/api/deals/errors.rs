use candid::{CandidType, Deserialize};

/// Canonical error type returned by all deal endpoints.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EscrowError {
    /// The requested deal does not exist.
    NotFound,
    /// The caller is not authorised to perform this action on the deal.
    NotAuthorised,
    /// The deal is not in the expected state for this operation.
    InvalidState {
        /// The state(s) that would have been valid.
        expected: String,
        /// The state the deal is actually in.
        actual: String,
    },
    /// The deal has already reached a terminal state (`Settled`, `Refunded`, or `Cancelled`).
    AlreadyFinalised,
    /// A reclaim was attempted before the deal's expiry deadline.
    NotExpired,
    /// An accept was attempted after the deal's expiry deadline.
    Expired,
    /// The supplied amount is invalid (e.g. zero).
    InvalidAmount,
    /// The supplied expiry timestamp is invalid (e.g. in the past).
    InvalidExpiry,
    /// An error occurred while communicating with the ICRC ledger canister.
    LedgerError(String),
    /// The ledger accepted the call but the transfer itself failed.
    TransferFailed(String),
    /// The caller does not match the deal's bound recipient.
    RecipientMismatch,
    /// A generic validation error with a human-readable message.
    ValidationError(String),
    /// The supplied claim code does not match the deal's claim code.
    InvalidClaimCode,
    /// A claim code is required for open (unbound-recipient) deals.
    MissingClaimCode,
    /// Both parties must consent before this operation can proceed.
    ConsentRequired,
    /// At least one of payer or recipient must be specified.
    NeitherPartySet,
    /// The payer principal is not set for this deal.
    PayerNotSet,
}
