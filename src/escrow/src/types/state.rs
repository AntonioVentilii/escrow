use std::collections::BTreeMap;

use candid::{CandidType, Deserialize, Principal};

use super::{
    arbitrator::ArbitratorProfile,
    deal::{Deal, DealId},
    dispute::{Dispute, DisputeConfig, DisputeId},
};

/// Default escrow service fee, in token base units. Calibrated to
/// `2 × ICP_LEDGER_FEE` (= `20_000` e8s for ICP). Used by
/// [`Config::default`] for fresh deployments that haven't been
/// admin-configured yet. Denominated in the deal's token; non-ICP
/// ledgers with materially different transfer-fee scales may need
/// the controller to call `update_config` with a token-appropriate
/// override before deals on that token can be created.
pub const DEFAULT_ESCROW_FEE: u128 = 20_000;

/// Global configuration for the Escrow canister.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Config {
    /// Admin-tunable dispute parameters. The fallback is whole-struct,
    /// not per-field — when `dispute_config` is `None`,
    /// `services::disputes::load_dispute_config` returns
    /// `DisputeConfig::default()`. Once a controller calls
    /// `update_config` with a `Some(_)` value, every field comes from
    /// that struct (including any fields the controller wants set to
    /// the default value — there's no per-field "leave unchanged"
    /// merge mechanism).
    pub dispute_config: Option<DisputeConfig>,
    /// Per-deal escrow service fee, in the deal's token. Charged on
    /// every terminal state. Defaults to [`DEFAULT_ESCROW_FEE`].
    /// Snapshotted into each `Deal.fees.escrow_fee` at `create_deal`
    /// time; subsequent `update_config` changes do not retroactively
    /// alter in-flight deals.
    pub escrow_fee: u128,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dispute_config: None,
            escrow_fee: DEFAULT_ESCROW_FEE,
        }
    }
}

/// Represents the complete state of the Escrow canister for persistence.
///
/// New fields use `Option` for backward-compatible deserialization
/// from older stable-memory snapshots that lack them.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct StableState {
    pub config: Config,
    pub deals: Option<BTreeMap<DealId, Deal>>,
    pub next_deal_id: Option<DealId>,
    /// Dispute storage. `None` on pre-dispute-feature snapshots.
    pub disputes: Option<BTreeMap<DisputeId, Dispute>>,
    pub next_dispute_id: Option<DisputeId>,
    /// Arbitrator registry, keyed by principal.
    pub arbitrators: Option<BTreeMap<Principal, ArbitratorProfile>>,
}
