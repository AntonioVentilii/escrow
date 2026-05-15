//! Generic asset abstraction for the escrow's settlement currency.
//!
//! Today the canister only knows how to settle in **ICRC-1 / ICRC-2**
//! tokens on the Internet Computer, so [`Asset`] currently carries a
//! single variant — [`Asset::Icrc`] — that wraps the ledger
//! [`Principal`]. The enum exists to future-proof the public Candid
//! surface: adding a new on-chain settlement domain (EVM ERC-20s,
//! native EVM gas tokens, Solana SPL tokens, …) is a backward-
//! compatible Candid change (new variant) instead of a breaking field
//! swap on every public type.
//!
//! The shape mirrors patterns from sister projects we want this engine
//! to be able to talk to in the future:
//!
//! - <https://github.com/RetroPandaClub/icdc-core> (`shared::types::asset::Asset`).
//! - The Oisy wallet's `shared::types::token` / `network` layout.
//!
//! Service / api / validation code that needs the underlying ICRC
//! ledger principal goes through [`Asset::as_icrc`] (today infallible
//! because [`Asset::Icrc`] is the only variant; explicitly fallible so
//! adding a new variant later doesn't silently change behaviour at
//! every call site).

use core::fmt::{self, Display, Formatter};

use candid::{CandidType, Deserialize, Principal};
use serde::Serialize;

use crate::api::deals::errors::EscrowError;

/// A settlement asset accepted by the escrow canister.
///
/// Always interrogate via the typed accessors (e.g. [`Asset::as_icrc`])
/// rather than pattern-matching directly — this keeps service code
/// resilient to future variant additions.
#[derive(
    CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum Asset {
    /// An ICRC-1 / ICRC-2 token identified by its ledger
    /// [`Principal`] on the Internet Computer.
    Icrc(Principal),
}

impl Asset {
    /// Convenience constructor for ICRC ledgers.
    #[must_use]
    pub const fn icrc(ledger_id: Principal) -> Self {
        Self::Icrc(ledger_id)
    }

    /// Returns the underlying ICRC ledger principal, or
    /// [`EscrowError::UnsupportedAsset`] when the asset is anything
    /// else.
    ///
    /// Today this is infallible — [`Asset::Icrc`] is the only
    /// variant — but the explicit `Result` shape is the contract:
    /// when a future variant lands (EVM ERC-20, native EVM,
    /// Solana SPL, …), service code that expected an ICRC ledger
    /// will surface a typed error instead of silently mis-dispatching.
    pub fn as_icrc(&self) -> Result<Principal, EscrowError> {
        match self {
            Self::Icrc(ledger) => Ok(*ledger),
        }
    }

    /// Returns a stable short identifier for the asset's domain
    /// (e.g. `"Icrc"`). Used in ICRC-7 token metadata so off-chain
    /// indexers can dispatch on the asset family without parsing
    /// the full Candid value.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Icrc(_) => "Icrc",
        }
    }
}

impl Display for Asset {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Icrc(p) => write!(f, "ICRC-{}", p.to_text()),
        }
    }
}

#[cfg(test)]
mod tests {
    use candid::Principal;

    use super::Asset;

    fn principal(id: u8) -> Principal {
        Principal::from_slice(&[id])
    }

    #[test]
    fn icrc_constructor_round_trips() {
        let p = principal(7);
        let asset = Asset::icrc(p);
        assert_eq!(asset, Asset::Icrc(p));
    }

    #[test]
    fn as_icrc_returns_ledger_principal_for_icrc_variant() {
        let p = principal(13);
        let asset = Asset::Icrc(p);
        assert_eq!(asset.as_icrc().unwrap(), p);
    }

    #[test]
    fn kind_for_icrc_is_stable_string() {
        let asset = Asset::Icrc(principal(1));
        assert_eq!(asset.kind(), "Icrc");
    }

    #[test]
    fn display_for_icrc_uses_canonical_prefix() {
        let p = principal(2);
        let s = format!("{}", Asset::Icrc(p));
        assert!(s.starts_with("ICRC-"), "unexpected display: {s}");
        assert!(s.contains(&p.to_text()));
    }
}
