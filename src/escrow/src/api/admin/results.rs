use candid::{CandidType, Deserialize};

use crate::{api::deals::errors::EscrowError, types::arbitrator::ArbitratorProfile};

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
