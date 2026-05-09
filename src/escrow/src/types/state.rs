use std::collections::BTreeMap;

use candid::{CandidType, Deserialize};

use super::{
    deal::{Deal, DealId},
    dispute::DisputeConfig,
};

/// Global configuration for the Escrow canister.
///
/// New fields use `Option` for backward-compatible deserialisation from
/// older stable-memory snapshots that lack them.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Config {
    /// Admin-tunable dispute parameters. `None` on legacy snapshots; the
    /// canister falls back to `DisputeConfig::default()` (Q6/Q9/Q10/Q12
    /// defaults) for any missing values when disputes are queried.
    pub dispute_config: Option<DisputeConfig>,
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
}
