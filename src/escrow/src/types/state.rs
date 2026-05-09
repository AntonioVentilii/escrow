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
    /// RFC-001 step 2 — dispute storage. `None` on legacy snapshots.
    pub disputes: Option<BTreeMap<DisputeId, Dispute>>,
    pub next_dispute_id: Option<DisputeId>,
    /// RFC-001 step 2 — arbitrator registry, keyed by principal.
    pub arbitrators: Option<BTreeMap<Principal, ArbitratorProfile>>,
}
