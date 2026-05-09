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

candid_result!(RegisterArbitratorResult, ArbitratorProfile);
candid_result!(DeregisterArbitratorResult, ArbitratorProfile);
