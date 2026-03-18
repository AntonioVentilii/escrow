use candid::{CandidType, Deserialize};

#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum EscrowError {
    NotFound,
    NotAuthorised,
    InvalidState { expected: String, actual: String },
    AlreadyFinalised,
    NotExpired,
    Expired,
    InvalidAmount,
    InvalidExpiry,
    LedgerError(String),
    TransferFailed(String),
    RecipientMismatch,
    ValidationError(String),
}
