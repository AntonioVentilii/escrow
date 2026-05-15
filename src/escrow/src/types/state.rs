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

/// Default anti-spam creation fee, in token base units. Same scale
/// as [`DEFAULT_ESCROW_FEE`] — symbolic deterrent calibrated to
/// `2 × ICP_LEDGER_FEE`. Pulled from the creator at `create_deal`
/// time on bound deals, routed to the controller-controlled
/// treasury subaccount, and never refunded. Tips don't pay this
/// (no bound counterparty to spam).
pub const DEFAULT_CREATION_FEE: u128 = 20_000;

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
    /// Per-deal anti-spam creation fee. Pulled from the creator at
    /// `create_deal` time on bound deals (recipient is `Some`) and
    /// routed to the canister's treasury subaccount. Tips
    /// (`recipient = None`) skip it entirely — there's no
    /// counterparty to spam-harass. Defaults to
    /// [`DEFAULT_CREATION_FEE`]. Snapshotted into each
    /// `Deal.fees.creation_fee` at create time; subsequent
    /// `update_config` changes do not retroactively alter
    /// in-flight deals.
    pub creation_fee: u128,
}

impl Config {
    /// `const`-callable default. Used by the `CONFIG` thread-local
    /// initialiser; `Default::default` delegates here.
    #[must_use]
    pub const fn const_default() -> Self {
        Self {
            dispute_config: DisputeConfig::const_default(),
            escrow_fee: DEFAULT_ESCROW_FEE,
            creation_fee: DEFAULT_CREATION_FEE,
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
