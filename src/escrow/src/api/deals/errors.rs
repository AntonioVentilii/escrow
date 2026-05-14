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
    /// The deal has already reached a terminal state (`Settled`, `Refunded`, `Cancelled`, or
    /// `Rejected`).
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
    /// Payer and recipient cannot be the same principal.
    SelfDeal,
    /// The anonymous principal cannot be used as a deal party.
    AnonymousParty,
    /// A metadata field exceeds its maximum allowed length.
    MetadataTooLong { field: String, max: u32 },
    /// The expiry timestamp is too far in the future.
    ExpiryTooFar,
    /// The caller has too many active (non-terminal) deals.
    TooManyActiveDeals { max: u32 },
    /// The caller's reliability score is below the minimum threshold.
    ReliabilityTooLow { score: u32, threshold: u32 },
    // ---- Dispute resolution + arbitrators ----
    /// `open_dispute` was called on a deal whose `recipient` is `None`
    /// (open-recipient / tip-flow deal). Tip flows cannot be disputed
    /// because there is no bound counterparty in canister state.
    DisputeRequiresBoundRecipient,
    /// `open_dispute` was called on a deal that already has an open or
    /// resolved dispute attached.
    DisputeAlreadyExists,
    /// The requested dispute does not exist.
    DisputeNotFound,
    /// The action requires the dispute to be in a different phase
    /// (`Evidence`, `Voting`, or `Resolved`).
    InvalidDisputePhase {
        /// The phase(s) that would have been valid.
        expected: String,
        /// The phase the dispute is actually in.
        actual: String,
    },
    /// `cast_vote` was called by a principal that is not on the
    /// dispute's selected panel.
    NotAssignedArbitrator,
    /// The eligible arbitrator pool is too small to fill the configured
    /// `panel_size`. Returned by `open_dispute` — the deal stays
    /// `Funded` so the caller can retry later or settle out-of-band.
    InsufficientArbitrators { need: u32, have: u32 },
    /// The arbitrator is `Suspended` or `Deregistered`.
    ArbitratorNotActive,
    /// An evidence submission exceeds the maximum allowed size for
    /// its field. Returned for both `note` overflow (limit
    /// `MAX_EVIDENCE_NOTE_LEN`) and `artefact_url` overflow (limit
    /// `MAX_EVIDENCE_URL_LEN`); the `max` field tells the caller
    /// WHICH limit was breached. Hash-length violations on
    /// `artefact_sha256` use `ValidationError` instead — those are
    /// shape checks ("must be exactly 32 bytes"), not size checks.
    EvidenceTooLarge { max: u32 },
    /// `open_dispute` was called on a deal whose `amount` cannot cover
    /// the configured arbitration fee plus the per-arbitrator ICRC-1
    /// ledger fees. Tiny deals are not disputable.
    AmountTooSmallForArbitration { min: u128 },
    /// `create_deal` was called with an `amount` too small to
    /// cover the escrow fee + per-arbitrator ledger fees + the
    /// full dispute reserve, leaving zero or negative remainder
    /// for the recipient. The `min` field surfaces the
    /// calculated floor so the caller can render the rejection
    /// without recomputing. Mirrors
    /// `AmountTooSmallForArbitration` but lifted to create time
    /// (the latter is checked at `open_dispute` and is kept for
    /// pre-RFC-002 deals that don't carry a fee snapshot). See
    /// [RFC-002 Q3](../../../docs/rfcs/0002-symmetric-escrow-fees.md#q3--minimum-viable-amount).
    AmountBelowMinimum { min: u128 },
    /// `create_deal` was called with a `panel_size` outside the range
    /// `[DisputeConfig.min_panel_size, DisputeConfig.max_panel_size]`,
    /// or a value that is not odd.
    ///
    /// - `min` / `max` surface the active range so the caller can render the allowed window
    ///   without parsing a free-form message.
    /// - `got` carries the value the caller sent so error logs are self-contained — clients don't
    ///   need to correlate the rejection with the request payload to know which value was wrong,
    ///   and a downstream `got` value of `4` (even, in-range) is distinguishable from `got = 11`
    ///   (out-of-range) without re-checking the inputs.
    PanelSizeOutOfRange { min: u32, max: u32, got: u32 },
}
