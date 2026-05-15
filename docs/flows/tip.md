# Tip flow

Payer locks tokens for an unknown recipient. Anyone with the bearer claim code can claim before expiry; otherwise the funds refund to the payer.

## Sequence

```mermaid
sequenceDiagram
    autonumber
    participant P as Payer
    participant E as Escrow
    participant L as ICRC Ledger
    participant R as Recipient (anyone with code)

    P->>L: icrc2_approve(E, amount + DC/2 + LF)
    P->>E: create_deal({ recipient: None, amount, expiry })
    Note over E: generate claim_code (raw_rand)
    E-->>P: deal_id + claim_code (in DealView)

    P->>E: fund_deal(deal_id)
    E->>L: transfer_from(P → escrow subaccount)
    Note over E: Funded

    Note over P,R: Payer shares QR / link (deal_id + claim_code)

    R->>E: accept_deal(deal_id, claim_code)
    Note over E: bind R as recipient, recipient_consent = Accepted
    E->>L: transfer(escrow → R, amount − EF − LF)
    Note over E: Settled
```

## Status path

```mermaid
stateDiagram-v2
    [*] --> Created: create_deal
    Created --> Funded: fund_deal
    Funded --> Settled: accept_deal(claim_code)
    Funded --> Refunded: expiry sweep OR manual reclaim_deal(P)
```

## Endpoints

| Step                  | Endpoint                                                  |
| --------------------- | --------------------------------------------------------- |
| Create                | `create_deal({ recipient: None, … })`                     |
| Fund                  | `fund_deal(deal_id)`                                      |
| Claim                 | `accept_deal(deal_id, claim_code)`                        |
| Refund (manual)       | `reclaim_deal(deal_id)` (payer only, after expiry)        |
| Refund (auto)         | `process_expired_deals(limit)` (or 5-min housekeeping)    |
| Public preview for QR | `get_claimable_deal(deal_id)` (no auth, hides claim code) |

## Notes

- **No signature tally.** `sign_yes` / `sign_no` reject tip deals with `DisputeRequiresBoundRecipient`.
- **Disputes unavailable.** Same reason — no bound counterparty in canister state.
- **Expiry default = refund payer.** This is the _only_ flow where silence at expiry refunds the payer; for bound deals silence flips to release-to-recipient.
- **Long-form guide:** [`TIPS.md`](../../TIPS.md) covers the bearer-token security model, node-provider visibility caveats, and frontend integration.
