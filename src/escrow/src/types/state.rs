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
    /// Admin-tunable dispute parameters. Fallback is whole-struct
    /// (`update_config` replaces the entire `DisputeConfig` — there
    /// is no per-field "leave unchanged" merge mechanism).
    pub dispute_config: DisputeConfig,
    /// Per-deal escrow service fee, in the deal's token. Charged on
    /// every terminal state. Defaults to [`DEFAULT_ESCROW_FEE`].
    /// Snapshotted into each `Deal.fees.escrow_fee` at `create_deal`
    /// time; subsequent `update_config` changes do not retroactively
    /// alter in-flight deals.
    pub escrow_fee: u128,
}

impl Config {
    /// `const`-callable default. Used by the `CONFIG` thread-local
    /// initialiser; `Default::default` delegates here.
    #[must_use]
    pub const fn const_default() -> Self {
        Self {
            dispute_config: DisputeConfig::const_default(),
            escrow_fee: DEFAULT_ESCROW_FEE,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::const_default()
    }
}

/// Complete state of the Escrow canister for persistence.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct StableState {
    pub config: Config,
    pub deals: BTreeMap<DealId, Deal>,
    pub next_deal_id: DealId,
    pub disputes: BTreeMap<DisputeId, Dispute>,
    pub next_dispute_id: DisputeId,
    pub arbitrators: BTreeMap<Principal, ArbitratorProfile>,
}
