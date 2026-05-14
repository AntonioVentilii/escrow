use candid::{CandidType, Deserialize, Principal};

use crate::types::dispute::DisputeId;

pub type DealId = u64;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DealStatus {
    Created,
    Funded,
    Settled,
    Refunded,
    Cancelled,
    Rejected,
    /// A dispute is open on this deal. Funds remain in the escrow
    /// subaccount until the dispute resolves to `ArbitratedSettled` /
    /// `ArbitratedRefunded`. The expiry sweep (`services::expiry`)
    /// skips deals in this state.
    Disputed,
    /// Dispute panel voted majority CC, OR both parties agreed
    /// out-of-band on a CC outcome — funds released to recipient.
    /// Distinct from `Settled` so callers can tell arbitrated from
    /// unilateral settlement. Terminal.
    ArbitratedSettled,
    /// Dispute panel voted majority IC, OR both parties agreed
    /// out-of-band on an IC outcome, OR the panel reached no quorum.
    /// Funds refunded to payer. Distinct from `Refunded`. Terminal.
    ArbitratedRefunded,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Consent {
    Pending,
    Accepted,
    Rejected,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealMetadata {
    pub title: Option<String>,
    pub note: Option<String>,
}

/// Per-deal fee snapshot taken at `create_deal` time.
///
/// Every fee the canister will charge against this deal over its
/// lifetime is locked here so that subsequent `update_config` calls
/// cannot retroactively alter the agreed economics. Same pattern as
/// `Deal.panel_size`: the deal terms are a contract at create time.
///
/// `Default` is implemented for test ergonomics — production code
/// always builds a `DealFees` via `services::deals::compute_deal_fees`.
#[derive(CandidType, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DealFees {
    /// Escrow service fee in the deal's token. Charged on every
    /// terminal state (`Settled`, `Refunded`, `Cancelled` with a
    /// committed reserve, `Rejected` with a committed reserve,
    /// `ArbitratedSettled`, `ArbitratedRefunded`). Snapshot of
    /// `Config.escrow_fee` at `create_deal` time.
    pub escrow_fee: u128,

    /// Per-party dispute reserve. Each party pre-commits this amount
    /// before the deal can be `Funded`. Refunded on happy-path
    /// terminal states (minus one `ledger_fee` per outgoing refund
    /// transfer); consumed by the arbitrator panel on
    /// `Disputed → ArbitratedX`. Snapshot of
    /// `compute_arbitration_fee(amount, DisputeConfig) / 2` at
    /// create time.
    pub dispute_reserve_per_party: u128,

    /// Reduced-fee percentage the arbitrator panel receives when
    /// the parties resolve out-of-band via `withdraw_dispute`.
    /// Snapshot of `DisputeConfig.withdraw_fee_pct` at create time.
    pub withdraw_fee_pct: u32,

    /// Ledger `icrc1_fee` value at create time, in the deal's
    /// token. RECORD ONLY — never used for arithmetic. Every
    /// actual transfer re-queries the live fee via
    /// `ledger::fee`. Snapshotted so the audit trail can answer
    /// "what was the user shown at create time?" even if the
    /// ledger later changes its fee. Operator absorbs any drift
    /// between create-time and runtime fees out of `escrow_fee`.
    pub ledger_fee_at_create: u128,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Deal {
    pub id: DealId,
    pub payer: Option<Principal>,
    pub recipient: Option<Principal>,
    pub token_ledger: Principal,
    pub token_symbol: Option<String>,
    pub amount: u128,
    pub created_at_ns: u64,
    pub created_by: Principal,
    pub updated_at_ns: Option<u64>,
    pub updated_by: Option<Principal>,
    pub expires_at_ns: u64,
    pub status: DealStatus,
    pub escrow_subaccount: Vec<u8>,
    pub funded_at_ns: Option<u64>,
    pub settled_at_ns: Option<u64>,
    pub refunded_at_ns: Option<u64>,
    pub funding_tx: Option<u128>,
    pub payout_tx: Option<u128>,
    pub refund_tx: Option<u128>,
    pub claim_code: Option<String>,
    pub payer_consent: Consent,
    pub recipient_consent: Consent,
    pub metadata: Option<DealMetadata>,
    /// `Some(dispute_id)` while a dispute is open on the deal or after
    /// it has resolved (so the audit trail back to the `Dispute` record
    /// survives terminal status). `None` for deals that never went into
    /// dispute. `Option`-wrapped for backward-compat with pre-dispute
    /// stable snapshots.
    pub dispute: Option<DisputeId>,
    /// Per-deal panel size override, chosen by the deal creator at
    /// `create_deal` time. `Some(n)` locks `n` arbitrators for any
    /// dispute opened on this deal regardless of subsequent
    /// `DisputeConfig.panel_size` changes — the deal terms are a
    /// contract at create time. `None` means "use whatever
    /// `DisputeConfig.panel_size` is current at `open_dispute` time"
    /// (the default behaviour for deals created before this field
    /// existed). Validated against `DisputeConfig::min_panel_size` /
    /// `max_panel_size` at create time via
    /// `validation::validate_panel_size_choice`.
    pub panel_size: Option<u32>,
    /// Fee snapshot taken at `create_deal` time. See [`DealFees`].
    pub fees: DealFees,
}
