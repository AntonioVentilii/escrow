use crate::types::deal::DealId;

const DOMAIN_SEPARATOR: &[u8] = b"escrow-deal";
const TREASURY_DOMAIN: &[u8] = b"escrow-treasury";

/// Canonical 32-byte subaccount for the canister's anti-spam
/// treasury. All `creation_fee` deposits land here at `create_deal`
/// time. Owned by the canister principal; only drainable via the
/// controller-only `admin_treasury_withdraw` endpoint.
///
/// Layout: `[treasury_domain (15 bytes) | zeros (17 bytes)]` — shares the deal-subaccount
/// construction style (domain prefix + zero padding) so it's visually distinguishable from any
/// derived deal subaccount in ledger explorers.
#[must_use]
pub fn treasury_subaccount() -> Vec<u8> {
    let mut subaccount = vec![0_u8; 32];
    let prefix_len = TREASURY_DOMAIN.len().min(32);
    subaccount[..prefix_len].copy_from_slice(&TREASURY_DOMAIN[..prefix_len]);
    subaccount
}

/// Derives a deterministic, unique 32-byte subaccount for a given deal.
///
/// Layout: `[domain_separator (11 bytes) | zeros (13 bytes) | deal_id BE (8 bytes)]`
///
/// This is stable across upgrades and reproducible from the deal id alone.
#[must_use]
pub fn derive_deal_subaccount(deal_id: DealId) -> Vec<u8> {
    let mut subaccount = vec![0_u8; 32];
    let prefix_len = DOMAIN_SEPARATOR.len().min(24);
    subaccount[..prefix_len].copy_from_slice(&DOMAIN_SEPARATOR[..prefix_len]);
    subaccount[24..32].copy_from_slice(&deal_id.to_be_bytes());
    subaccount
}

#[cfg(test)]
mod tests {
    use super::{derive_deal_subaccount, treasury_subaccount, DOMAIN_SEPARATOR, TREASURY_DOMAIN};
    use crate::types::deal::DealId;

    #[test]
    fn treasury_is_deterministic_and_distinct_from_deals() {
        let t = treasury_subaccount();
        assert_eq!(t.len(), 32);
        assert_eq!(&t[..TREASURY_DOMAIN.len()], TREASURY_DOMAIN);
        // Treasury must not collide with any deal subaccount —
        // the prefixes differ (`escrow-treasury` vs `escrow-deal`),
        // which is enough to guarantee distinctness even before the
        // deal_id-encoded suffix matters.
        for deal_id in [0_u64, 1, 42, u64::MAX] {
            assert_ne!(t, derive_deal_subaccount(deal_id));
        }
    }

    #[test]
    fn deterministic() {
        let a = derive_deal_subaccount(1);
        let b = derive_deal_subaccount(1);
        assert_eq!(a, b);
    }

    #[test]
    fn unique_per_deal() {
        let a = derive_deal_subaccount(1);
        let b = derive_deal_subaccount(2);
        assert_ne!(a, b);
    }

    #[test]
    fn length_is_32() {
        let sub = derive_deal_subaccount(42);
        assert_eq!(sub.len(), 32);
    }

    #[test]
    fn starts_with_domain_separator() {
        let sub = derive_deal_subaccount(1);
        assert_eq!(&sub[..DOMAIN_SEPARATOR.len()], DOMAIN_SEPARATOR);
    }

    #[test]
    fn encodes_deal_id_in_last_8_bytes() {
        let deal_id: DealId = 0x0102_0304_0506_0708;
        let sub = derive_deal_subaccount(deal_id);
        assert_eq!(&sub[24..32], &deal_id.to_be_bytes());
    }
}
