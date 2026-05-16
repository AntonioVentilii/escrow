use candid::{CandidType, Deserialize};

use crate::{
    api::deals::errors::EscrowError,
    types::{arbitrator::ArbitratorProfile, state::Config},
};

/// Public, read-only snapshot of the canister's current fee schedule.
///
/// Returned by the unguarded `get_fees` query so any caller (wallets,
/// frontends, explorers) can quote the live economics without going
/// through the controller-gated `config` endpoint, which exposes
/// operational dispute parameters (panel sizes, windows, eligibility
/// thresholds) that aren't relevant to consumers.
///
/// Mirrors the fee-bearing fields of [`Config`] one-to-one. New deals
/// snapshot the same values into `Deal.fees` at `create_deal` time
/// (see `services::deals::compute_deal_fees`) — subsequent
/// `update_config` calls never retroactively alter in-flight deals.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct FeesView {
    /// Per-deal escrow service fee in the deal's token. Charged on
    /// every terminal state. Mirrors [`Config::escrow_fee`].
    pub escrow_fee: u128,
    /// Per-deal anti-spam creation fee. Pulled from the creator at
    /// `create_deal` time on bound deals and routed to the treasury
    /// subaccount; tips (`recipient = None`) skip it. Mirrors
    /// [`Config::creation_fee`].
    pub creation_fee: u128,
    /// Arbitration fee in basis points of the disputed amount
    /// (`10_000` = 100%). Mirrors
    /// [`crate::types::dispute::DisputeConfig::arbitration_fee_bps`].
    pub arbitration_fee_bps: u32,
    /// Minimum arbitration fee in the deal's token. The effective
    /// arbitration fee is
    /// `max(arbitration_min_fee, amount * arbitration_fee_bps / 10_000)`.
    /// Mirrors
    /// [`crate::types::dispute::DisputeConfig::arbitration_min_fee`].
    pub arbitration_min_fee: u128,
    /// Reduced percentage of the arbitration fee paid to the panel
    /// when both parties resolve out-of-band via `withdraw_dispute`.
    /// Mirrors
    /// [`crate::types::dispute::DisputeConfig::withdraw_fee_pct`].
    pub withdraw_fee_pct: u32,
}

impl From<&Config> for FeesView {
    fn from(cfg: &Config) -> Self {
        Self {
            escrow_fee: cfg.escrow_fee,
            creation_fee: cfg.creation_fee,
            arbitration_fee_bps: cfg.dispute_config.arbitration_fee_bps,
            arbitration_min_fee: cfg.dispute_config.arbitration_min_fee,
            withdraw_fee_pct: cfg.dispute_config.withdraw_fee_pct,
        }
    }
}

macro_rules! candid_result {
    ($name:ident, $ok:ty) => {
        #[derive(CandidType, Deserialize, Clone, Debug)]
        pub enum $name {
            Ok(Box<$ok>),
            Err(EscrowError),
        }

        impl From<Result<$ok, EscrowError>> for $name {
            fn from(result: Result<$ok, EscrowError>) -> Self {
                match result {
                    Ok(v) => Self::Ok(Box::new(v)),
                    Err(e) => Self::Err(e),
                }
            }
        }
    };
}

candid_result!(AdminRegisterArbitratorResult, ArbitratorProfile);
candid_result!(AdminSetArbitratorStatusResult, ArbitratorProfile);
candid_result!(AdminTreasuryBalanceResult, u128);
candid_result!(AdminTreasuryWithdrawResult, u128);

/// Outcome of `update_config`. `Ok` on successful validation +
/// persistence; `Err` carries the validation failure.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum UpdateConfigResult {
    Ok,
    Err(EscrowError),
}

impl From<Result<(), EscrowError>> for UpdateConfigResult {
    fn from(result: Result<(), EscrowError>) -> Self {
        match result {
            Ok(()) => Self::Ok,
            Err(e) => Self::Err(e),
        }
    }
}
