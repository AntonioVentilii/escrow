use candid::{CandidType, Deserialize};

/// Global configuration for the Escrow canister.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Config {}

/// Represents the complete state of the Escrow canister for persistence (V2).
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct StableState {
    /// The global configuration.
    pub config: Config,
}
