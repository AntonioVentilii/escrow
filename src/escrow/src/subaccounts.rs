use crate::types::deal::DealId;

const DOMAIN_SEPARATOR: &[u8] = b"escrow-deal";

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
    use super::{derive_deal_subaccount, DOMAIN_SEPARATOR};
    use crate::types::deal::DealId;

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
