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
