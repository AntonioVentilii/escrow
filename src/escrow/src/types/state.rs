use std::collections::BTreeMap;

use candid::{CandidType, Deserialize};

use super::deal::{Deal, DealId};

/// Global configuration for the Escrow canister.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Config {}

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
