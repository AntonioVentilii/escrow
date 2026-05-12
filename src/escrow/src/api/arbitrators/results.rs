use candid::{CandidType, Deserialize};

use crate::{api::deals::errors::EscrowError, types::arbitrator::ArbitratorProfile};

#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum DeregisterArbitratorResult {
    Ok(Box<ArbitratorProfile>),
    Err(EscrowError),
}

impl From<Result<ArbitratorProfile, EscrowError>> for DeregisterArbitratorResult {
    fn from(result: Result<ArbitratorProfile, EscrowError>) -> Self {
        match result {
            Ok(v) => Self::Ok(Box::new(v)),
            Err(e) => Self::Err(e),
        }
    }
}
