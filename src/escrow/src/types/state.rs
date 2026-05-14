use std::collections::BTreeMap;

use candid::{CandidType, Deserialize, Principal};

use super::{
    arbitrator::ArbitratorProfile,
    deal::{Deal, DealId},
    dispute::{Dispute, DisputeConfig, DisputeId},
};

/// Global configuration for the Escrow canister.
///
/// New fields use `Option` for backward-compatible deserialisation from
/// older stable-memory snapshots that lack them.
#[derive(CandidType, Deserialize, Clone, Debug, Default)]
pub struct Config {
    /// Admin-tunable dispute parameters. The fallback is whole-struct,
    /// not per-field — when `dispute_config` is `None` (legacy
    /// snapshots; fresh deployments before `update_config` is first
    /// called), `services::disputes::load_dispute_config` returns
    /// `DisputeConfig::default()`. Once a controller calls
    /// `update_config` with a `Some(_)` value, every field comes from
    /// that struct (including any fields the controller wants set to
    /// the default value — there's no per-field "leave unchanged"
    /// merge mechanism).
    pub dispute_config: Option<DisputeConfig>,
    /// Per-deal escrow service fee, in the deal's token. Charged on
    /// every terminal state. `None` on legacy stable snapshots
    /// — `services::deals::load_escrow_fee` returns the default
    /// (`DEFAULT_ESCROW_FEE`) in that case. Once a controller calls
    /// `update_config` with a `Some(_)` value the snapshot at
    /// `create_deal` time uses that. Subsequent changes do not
    /// retroactively alter in-flight deals — the create-time
    /// snapshot on each `Deal` is the contract.
    pub escrow_fee: Option<u128>,
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
