# Escrow Engine Canister

An Internet Computer escrow canister MVP implementing a **tip flow**: a payer funds a deal-specific ledger subaccount via ICRC-2, a recipient claims before expiry, and otherwise the payer is refunded.

## Public API

### Update methods

| Method                          | Description                                                                                                          |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `create_deal(CreateDealArgs)`   | Create a new tip deal. Caller becomes the payer.                                                                     |
| `fund_deal(FundDealArgs)`       | Move tokens from payer to escrow subaccount via ICRC-2 `transfer_from`. Payer must have approved the canister first. |
| `accept_deal(AcceptDealArgs)`   | Recipient claims a funded tip before expiry. Binds the recipient if unset.                                           |
| `reclaim_deal(ReclaimDealArgs)` | Payer reclaims funds from an expired, unclaimed deal.                                                                |
| `cancel_deal(CancelDealArgs)`   | Payer cancels an unfunded (Created) deal.                                                                            |
| `process_expired_deals(limit)`  | Batch-refund up to `limit` expired funded deals. Idempotent.                                                         |

### Query methods

| Method                           | Description                                                                                                |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `get_deal(deal_id)`              | Full deal view. Caller must be payer or recipient.                                                         |
| `list_my_deals(ListMyDealsArgs)` | Paginated deals where caller is payer or recipient.                                                        |
| `get_claimable_deal(deal_id)`    | Reduced public view for claim/share-link pages. Any authenticated caller may query (no participant check). |
| `get_escrow_account(deal_id)`    | Returns the escrow `Account` (canister principal + deal subaccount). Caller must be payer or recipient.    |

> **Note:** Every endpoint rejects anonymous callers (`caller_is_not_anonymous` guard).

### Admin methods

| Method                  | Description                                      |
| ----------------------- | ------------------------------------------------ |
| `config()`              | Read canister configuration (controller-only).   |
| `update_config(Config)` | Update canister configuration (controller-only). |

## Deal lifecycle

```
Created ──fund──▶ Funded ──accept──▶ Completed
  │                 │
  │ cancel          │ reclaim (after expiry)
  ▼                 ▼
Cancelled        Refunded
```

`Completed`, `Refunded`, and `Cancelled` are terminal states.

## Module structure

| Module                  | Responsibility                                                                  |
| ----------------------- | ------------------------------------------------------------------------------- |
| `types/deal.rs`         | Internal `Deal`, `DealStatus`, `DealMetadata` types                             |
| `types/ledger_types.rs` | ICRC-1/ICRC-2 Account and transfer types                                        |
| `types/state.rs`        | Config, StableState for persistence                                             |
| `api/deals/api.rs`      | Thin deal endpoint layer (delegates to services)                                |
| `api/deals/params.rs`   | Public argument structs (`CreateDealArgs`, `FundDealArgs`, …)                   |
| `api/deals/results.rs`  | Public view structs (`DealView`, `ClaimableDealView`)                           |
| `api/deals/errors.rs`   | Typed `EscrowError` enum                                                        |
| `api/admin/api.rs`      | Controller-only admin endpoints                                                 |
| `services/deals.rs`     | Core deal business logic (create, fund, accept, reclaim, cancel, queries)       |
| `services/expiry.rs`    | Batch expired-deal refund processing                                            |
| `memory.rs`             | Thread-local storage, atomic deal-ID allocation, save/restore, processing locks |
| `ledger.rs`             | ICRC inter-canister call helpers (transfer, transfer_from)                      |
| `subaccounts.rs`        | Deterministic deal subaccount derivation                                        |
| `validation.rs`         | State transition and input validation                                           |
| `guards.rs`             | Caller authentication/authorization guards                                      |

## Future expansion

The design is structured to accommodate the following without breaking changes:

### Disputes and resolution

Add `Disputed` / `Resolved` variants to `DealStatus`. The explicit state machine in `validation.rs` already gates every transition — new edges (e.g. `Funded -> Disputed`, `Disputed -> Resolved`) can be added alongside the existing ones. The `Deal` struct can gain optional fields (`dispute_evidence`, `resolver`, `resolution_outcome`) wrapped in `Option` for backward-compatible deserialization.

### YES / NO / EMPTY signatures

Add `payer_signature` and `recipient_signature` fields to `Deal` (each an `Option<SignatureChoice>` enum). Settlement logic in `services/deals.rs` can branch on the combination of signatures once they exist, while the current tip flow simply hard-codes both as implicit YES.

### Multi-asset / multi-ledger

The `token_ledger` field is already per-deal. `ledger.rs` provides a small abstraction layer around ICRC calls; extending it to multiple ledgers requires no structural change. Fee handling can be added to `TransferArg` construction.

### Scheduled / instalment payments

Add a `parent_deal_id: Option<DealId>` to `Deal` for parent ↔ child relationships. A future `schedule.rs` module can generate child deals from a parent template and release them on a timer, using the same subaccount and ledger helpers.

### Pluggable resolvers

Introduce a `Resolver` trait or enum (`Admin`, `Oracle(Principal)`, `DaoVote { ... }`) stored on the deal. Resolution logic can dispatch on this field, keeping the core state machine unchanged.

### Storage at scale

The current `BTreeMap + stable_save` approach works for MVP volumes. For production scale, migrate the deal map to `ic-stable-structures` `StableBTreeMap` for O(1) upgrade cost. The `memory.rs` accessor API (`get_deal`, `with_deal`, `with_deals`) isolates callers from the storage backend, making this migration transparent.

### Deal ledger — deals as ICRC-7 NFTs

To further improve long-term scalability and avoid canister memory exhaustion, each deal could be represented as a non-fungible token on a dedicated **ICRC-7 deal ledger canister**.

**How it maps:**

| Deal concept                                            | ICRC-7 equivalent                                            |
| ------------------------------------------------------- | ------------------------------------------------------------ |
| `create_deal`                                           | `icrc7_mint` — mint a new NFT with deal metadata             |
| `accept_deal`                                           | `icrc7_transfer` — transfer the NFT from payer to recipient  |
| Terminal states (`Completed`, `Refunded`, `Cancelled`)  | `icrc7_burn` or metadata update marking the token as settled |
| Deal details (amount, status, expiry, payer, recipient) | Token metadata fields (ICRC-16 value map)                    |
| Deal history / audit trail                              | ICRC-3 transaction log (built into compliant ledgers)        |

**Benefits:**

- **Scalability** — deal storage moves out of the escrow canister into a purpose-built ledger canister (or canister group) with its own memory, effectively removing the single-canister memory ceiling.
- **Composability** — deals become first-class IC assets queryable via standard ICRC-7 methods; wallets, explorers, and other canisters can display and interact with deals without custom integration.
- **Auditability** — ICRC-3 provides an immutable, append-only transaction log of every mint, transfer, and burn at no extra development cost.
- **Transferability** — representing a deal as an NFT opens the door to secondary-market scenarios (e.g. selling or assigning a claim before settlement).

**Open challenges:**

- ICRC-7 does not standardize metadata _updates_ after minting; status transitions would require a custom extension method on the deal ledger (e.g. `update_deal_status`).
- Every deal lifecycle action becomes an inter-canister call (escrow canister -> deal ledger), adding ~1-2 seconds of latency per step.
- The deal ledger canister itself must be upgradeable and its cycles balance managed.

**Migration path:** the escrow canister already isolates storage behind the `memory.rs` accessor API. A phased rollout could (1) deploy the ICRC-7 deal ledger, (2) dual-write new deals to both local memory and the ledger, (3) backfill existing deals, and (4) remove the local deal map once the ledger is the source of truth.
