# Deal flows

Visual reference for each end-to-end flow the canister supports. Optimised for "what does this look like in practice" rather than line-by-line API reference (for that, see [`src/escrow/README.md`](../../src/escrow/README.md)).

> **Universal principle: commit-at-first-action.** Every party deposits everything they're on the hook for as part of their first money-moving call (creator → `create_deal`, counterparty → `consent_deal`). There is no separate `fund_deal` step. Bound-deal creators additionally pay a small `creation_fee` that goes to the canister's treasury subaccount and is never refunded — anti-spam deterrent for serious flows.

| Flow                                 | Who creates | Counterparty bound?              | Settlement trigger                        | Doc                                            |
| ------------------------------------ | ----------- | -------------------------------- | ----------------------------------------- | ---------------------------------------------- |
| **Tip**                              | Payer       | No (open recipient + claim code) | Recipient claims with `accept_deal(code)` | [tip.md](./tip.md)                             |
| **Payer-creator deal (3a)**          | Payer       | Yes                              | Two-signature tally (`Yes` / `No`)        | [payer-creator.md](./payer-creator.md)         |
| **Recipient-creator deal (3b)**      | Recipient   | Yes                              | Two-signature tally (`Yes` / `No`)        | [recipient-creator.md](./recipient-creator.md) |
| **Dispute** (overlay on bound deals) | —           | —                                | Arbitrator panel majority OR withdraw     | [dispute.md](./dispute.md)                     |

## What the bound flows have in common

After funding, every bound deal goes through the same two-signature tally. Each party calls `sign_yes` or `sign_no`; the canister tallies on each call.

```mermaid
flowchart TD
    F[Funded] --> P{tally<br/>(payer_sig, recipient_sig)}
    P -->|Yes + Yes| S[Settled — release to recipient]
    P -->|No + No| A[Aborted — refund payer]
    P -->|mixed Yes / No| D[Disputed — auto-open arbitration]
    P -->|any Empty| F2[stays Funded — wait for the other party]
    F2 -.expiry.-> EXP[auto-YES rule:<br/>any Empty → Yes,<br/>signed votes preserved]
    EXP --> P
```

Tips bypass the tally entirely — there's no bound counterparty to sign for. `sign_yes` / `sign_no` reject tip deals, and disputes are unavailable for the same reason.

## Glossary

| Term           | Meaning                                                                                                                                                          |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EF`           | Escrow service fee. Operator's share, retained in the deal subaccount on every terminal that involves a fund movement.                                           |
| `DC/2`         | Per-party dispute reserve. Each party deposits this; refunded on happy paths, consumed by the panel on dispute.                                                  |
| `creation_fee` | Anti-spam deterrent paid by the creator at create time on bound deals. Routed to the canister treasury and never refunded. Tip flows skip it.                    |
| `LF`           | Live ICRC-1 ledger fee, queried at runtime (not snapshotted).                                                                                                    |
| Treasury       | A canister-owned subaccount (`escrow-treasury` domain prefix) where every bound deal's `creation_fee` accumulates. Drainable only via `admin_treasury_withdraw`. |
| Auto-YES rule  | At expiry on a bound deal, any `Empty` signature is treated as `Yes`. Explicit votes are preserved.                                                              |
| Tally          | The function `tally_signatures(payer_sig, recipient_sig)` that decides Settled / Aborted / Disputed / Pending.                                                   |

## Long-form companion

[`TIPS.md`](../../TIPS.md) at the repo root has the long-form security model + frontend integration notes for the tip flow specifically.
