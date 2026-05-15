use candid::{CandidType, Deserialize, Principal};

use crate::types::{asset::Asset, dispute::DisputeId};

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
    /// Both parties signed `No` on a `Funded` bound deal — mutual
    /// agreement that the off-chain part of the deal did NOT happen.
    /// Funds returned to the payer using the same fee math as
    /// `Refunded` (`escrow_fee` retained, `ledger_fee` burned per
    /// transfer); the per-party dispute reserves are returned to
    /// each side. Distinct from `Refunded` (expiry-driven) and
    /// `ArbitratedRefunded` (dispute-driven) so the audit trail
    /// records WHY the deal didn't settle. Terminal.
    Aborted,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Consent {
    Pending,
    Accepted,
    Rejected,
}

/// Per-party feedback signature recorded on a `Funded` bound deal.
/// Drives the settlement tally (see `services::deals::tally_signatures`).
///
/// - `Empty`: no decision yet. Default for both parties when the deal becomes `Funded`. At expiry,
///   any `Empty` signature is treated as `Yes` for tally purposes (silence = release).
/// - `Yes`: the party affirms the deal completed correctly off-chain. Both parties on `Yes` →
///   `Settled` (release to recipient).
/// - `No`: the party affirms the deal did NOT complete correctly. Both parties on `No` → `Aborted`
///   (refund to payer). Mixed `Yes`/`No` → auto-`Disputed` (panel arbitration).
///
/// Tip flows (`recipient = None`) never carry signatures: the tip
/// model has no bound counterparty to sign for, and signing endpoints
/// reject tip deals with `DisputeRequiresBoundRecipient`.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum Signature {
    #[default]
    Empty,
    Yes,
    No,
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
    /// Settlement asset for this deal. Today always
    /// [`Asset::Icrc`]; future variants extend the canister's
    /// settlement domain without forcing a Candid-breaking field
    /// rename.
    pub asset: Asset,
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
    /// survives terminal status). `None` for deals that never went
    /// into dispute.
    pub dispute: Option<DisputeId>,
    /// Per-deal panel size override, chosen by the deal creator at
    /// `create_deal` time. `Some(n)` locks `n` arbitrators for any
    /// dispute opened on this deal regardless of subsequent
    /// `DisputeConfig.panel_size` changes — the deal terms are a
    /// contract at create time. `None` means "use whatever
    /// `DisputeConfig.panel_size` is current at `open_dispute` time".
    /// Validated against `DisputeConfig::min_panel_size` /
    /// `max_panel_size` at create time via
    /// `validation::validate_panel_size_choice`.
    pub panel_size: Option<u32>,
    /// Fee snapshot taken at `create_deal` time. See [`DealFees`].
    pub fees: DealFees,
    /// Payer's settlement signature. Defaults to [`Signature::Empty`]
    /// at `Funded` time and stays `Empty` until the payer calls
    /// `sign_yes` / `sign_no`. Together with `recipient_signature` it
    /// drives the settlement tally — see [`Signature`] and
    /// `services::deals::tally_signatures`.
    pub payer_signature: Signature,
    /// Recipient's settlement signature. Mirrors
    /// [`Self::payer_signature`].
    pub recipient_signature: Signature,
}
