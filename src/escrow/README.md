# Escrow Engine Canister

An Internet Computer escrow canister implementing a **tip flow**: a payer funds a deal-specific ledger subaccount via ICRC-2, a recipient claims before expiry, and otherwise the payer is refunded. Each deal is also exposed as an **ICRC-7 non-fungible token**, making deals queryable via standard NFT interfaces.

## Public API

### Escrow update methods

| Method                          | Description                                                                                                          |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `create_deal(CreateDealArgs)`   | Create a new tip deal. Caller becomes the payer.                                                                     |
| `fund_deal(FundDealArgs)`       | Move tokens from payer to escrow subaccount via ICRC-2 `transfer_from`. Payer must have approved the canister first. |
| `accept_deal(AcceptDealArgs)`   | Recipient claims a funded tip before expiry. Binds the recipient if unset.                                           |
| `reclaim_deal(ReclaimDealArgs)` | Payer reclaims funds from an expired, unclaimed deal.                                                                |
| `cancel_deal(CancelDealArgs)`   | Payer cancels an unfunded (Created) deal.                                                                            |
| `process_expired_deals(limit)`  | Batch-refund up to `limit` expired funded deals. Idempotent.                                                         |

### Escrow query methods

| Method                           | Description                                                                                                |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `get_deal(deal_id)`              | Full deal view. Caller must be payer or recipient.                                                         |
| `list_my_deals(ListMyDealsArgs)` | Paginated deals where caller is payer or recipient.                                                        |
| `get_claimable_deal(deal_id)`    | Reduced public view for claim/share-link pages. Any authenticated caller may query (no participant check). |
| `get_escrow_account(deal_id)`    | Returns the escrow `Account` (canister principal + deal subaccount). Caller must be payer or recipient.    |

> **Note:** All escrow endpoints reject anonymous callers (`caller_is_not_anonymous` guard).

### ICRC-7 NFT queries (open, no auth required)

Every deal is a non-fungible token. Ownership follows deal lifecycle: the payer owns the token until `Completed`, at which point the recipient becomes the owner.

| Method                                 | Description                                                   |
| -------------------------------------- | ------------------------------------------------------------- |
| `icrc7_name()`                         | Collection name: `"Escrow Deals"`.                            |
| `icrc7_symbol()`                       | Collection symbol: `"ESCROW"`.                                |
| `icrc7_description()`                  | Human-readable description of the collection.                 |
| `icrc7_logo()`                         | Collection logo (currently `None`).                           |
| `icrc7_total_supply()`                 | Total number of deal NFTs minted (one per deal).              |
| `icrc7_supply_cap()`                   | Maximum supply cap (`None` = unlimited).                      |
| `icrc7_collection_metadata()`          | Collection-level ICRC-16 metadata map.                        |
| `icrc7_token_metadata(token_ids)`      | Per-token metadata (deal details as ICRC-16 key-value pairs). |
| `icrc7_owner_of(token_ids)`            | Owner account for each token ID.                              |
| `icrc7_balance_of(accounts)`           | Number of deal NFTs owned by each account.                    |
| `icrc7_tokens(prev, take)`             | Paginated token IDs in ascending order.                       |
| `icrc7_tokens_of(account, prev, take)` | Paginated token IDs owned by a specific account.              |
| `icrc7_max_query_batch_size()`         | Max batch size for metadata/owner queries (100).              |
| `icrc7_max_update_batch_size()`        | Max batch size for transfer calls (`None`).                   |
| `icrc7_default_take_value()`           | Default page size for token listing (50).                     |
| `icrc7_max_take_value()`               | Max page size for token listing (500).                        |
| `icrc7_max_memo_size()`                | Max memo size (`None`).                                       |
| `icrc7_atomic_batch_transfers()`       | Whether batch transfers are atomic (`None`).                  |
| `icrc7_tx_window()`                    | Transaction dedup window (`None`).                            |
| `icrc7_permitted_drift()`              | Permitted time drift (`None`).                                |

### ICRC-7 transfer (always rejected)

| Method                 | Description                                                                             |
| ---------------------- | --------------------------------------------------------------------------------------- |
| `icrc7_transfer(args)` | Always returns `GenericError` — ownership is managed through escrow operations instead. |

### ICRC-10 supported standards

| Method                         | Description                     |
| ------------------------------ | ------------------------------- |
| `icrc10_supported_standards()` | Returns `ICRC-7` and `ICRC-10`. |

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

| Module                  | Responsibility                                                                   |
| ----------------------- | -------------------------------------------------------------------------------- |
| `types/deal.rs`         | Internal `Deal`, `DealStatus`, `DealMetadata` types                              |
| `types/ledger_types.rs` | ICRC-1/ICRC-2 Account and transfer types                                         |
| `types/icrc7.rs`        | ICRC-7/ICRC-16 `Value` type, ownership helpers, metadata builders                |
| `types/state.rs`        | Config, StableState for persistence                                              |
| `api/deals/api.rs`      | Thin deal endpoint layer (delegates to services)                                 |
| `api/deals/params.rs`   | Public argument structs (`CreateDealArgs`, `FundDealArgs`, …)                    |
| `api/deals/results.rs`  | Public view structs (`DealView`, `ClaimableDealView`)                            |
| `api/deals/errors.rs`   | Typed `EscrowError` enum                                                         |
| `api/icrc7/api.rs`      | ICRC-7 NFT standard query/update endpoints + ICRC-10 supported standards         |
| `api/admin/api.rs`      | Controller-only admin endpoints                                                  |
| `services/deals.rs`     | Core deal business logic (create, fund, accept, reclaim, cancel, queries)        |
| `services/expiry.rs`    | Batch expired-deal refund processing                                             |
| `services/icrc7.rs`     | ICRC-7 service logic (token metadata, ownership, pagination, transfer rejection) |
| `memory.rs`             | Thread-local storage, atomic deal-ID allocation, save/restore, processing locks  |
| `ledger.rs`             | ICRC inter-canister call helpers (transfer, transfer_from)                       |
| `subaccounts.rs`        | Deterministic deal subaccount derivation                                         |
| `validation.rs`         | State transition and input validation                                            |
| `guards.rs`             | Caller authentication/authorization guards                                       |

## Scalability & limitations

### Current storage model

The canister stores all deals in a heap-allocated `BTreeMap` that is serialized to stable memory on each upgrade (`stable_save` / `stable_restore`). This is simple and correct for MVP volumes, but it has two hard ceilings:

| Constraint       | Limit          | Bottleneck                                                            |
| ---------------- | -------------- | --------------------------------------------------------------------- |
| Wasm heap memory | ~2–3 GB usable | Serialization of the full `BTreeMap` on every upgrade                 |
| Upgrade cost     | O(n) per deal  | Every upgrade must serialize and then deserialize the entire deal map |

With each `Deal` struct weighing roughly 300–500 bytes, the heap approach supports approximately **4–8 million deals** before memory pressure becomes a concern.

### ICRC-7 does not shard token state

The ICRC-7 standard defines a **query interface** for non-fungible tokens, but it has **no built-in mechanism for sharding or archiving token state** across multiple canisters. All token ownership and metadata lives in the single canister that implements the standard.

ICRC-3 (the transaction log standard used by ICRC-1 and ICRC-7 ledgers) only archives the **transaction history** — the append-only log of mints, transfers, and burns — not the live token data. So even with ICRC-3, the ownership map and metadata for every token must fit inside one canister.

This is a known limitation of the current IC NFT ecosystem. There is no standardized cross-canister NFT sharding protocol.

### Scaling roadmap

The following steps are listed in order of increasing effort and capacity:

| Phase | Change                                  | Capacity                      | Effort |
| ----- | --------------------------------------- | ----------------------------- | ------ |
| 0     | Current (`BTreeMap + stable_save`)      | ~4–8 M deals                  | —      |
| 1     | `ic-stable-structures` `StableBTreeMap` | ~100 M+ deals (stable memory) | Low    |
| 2     | Separate ICRC-7 deal ledger canister    | Same, but isolates concerns   | Medium |
| 3     | Sharded deal ledger (router + shards)   | Effectively unbounded         | High   |

**Phase 1** is the highest-impact change. `StableBTreeMap` stores deals directly in stable memory, eliminating the serialize-everything-on-upgrade bottleneck and giving access to the full stable memory budget (up to hundreds of GB on some subnets). The `memory.rs` accessor API (`get_deal`, `with_deal`, `with_deals`) already isolates callers from the storage backend, making this migration transparent.

**Phase 2** extracts the deal ledger into a standalone canister with its own memory and cycles budget. The escrow canister would make inter-canister calls to the deal ledger for every deal operation. This adds ~1–2 s of latency per call but cleanly separates escrow logic from token storage.

**Phase 3** introduces manual sharding: a thin router canister maps token-ID ranges to multiple shard canisters, each implementing ICRC-7 independently. Aggregation queries (`balance_of`, `tokens_of`) fan out across shards and merge results. This is complex and not standardized, but it removes the single-canister ceiling entirely.

> **Bottom line:** with Phase 1 alone, the canister can comfortably support hundreds of millions of deals. Phases 2–3 are only needed at truly massive scale or when operational isolation is desired.

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

See the [Scalability & limitations](#scalability--limitations) section for the full analysis and phased roadmap. The `memory.rs` accessor API makes the migration from `BTreeMap` to `StableBTreeMap` transparent to all callers.

### Deal ledger — deals as ICRC-7 NFTs ✅ (implemented)

Each deal is exposed as a non-fungible token via the **ICRC-7** standard query interface. The canister natively implements all ICRC-7 query methods plus ICRC-10 supported-standards discovery. Ownership follows deal lifecycle: the payer owns the token until completion, at which point the recipient becomes the owner.

Direct `icrc7_transfer` calls are rejected — ownership transitions are managed exclusively through escrow operations (`accept_deal`, `reclaim_deal`, etc.).

| Deal concept                                            | ICRC-7 equivalent                                            |
| ------------------------------------------------------- | ------------------------------------------------------------ |
| `create_deal`                                           | `icrc7_mint` — mint a new NFT with deal metadata             |
| `accept_deal`                                           | `icrc7_transfer` — transfer the NFT from payer to recipient  |
| Terminal states (`Completed`, `Refunded`, `Cancelled`)  | `icrc7_burn` or metadata update marking the token as settled |
| Deal details (amount, status, expiry, payer, recipient) | Token metadata fields (ICRC-16 value map)                    |
| Deal history / audit trail                              | ICRC-3 transaction log (built into compliant ledgers)        |

**Next steps:** extracting the deal ledger into a dedicated canister and adding ICRC-3 transaction logging. See Phase 2 and Phase 3 in the [scaling roadmap](#scaling-roadmap).

**Open challenges:**

- ICRC-7 does not standardize metadata _updates_ after minting; status transitions would require a custom extension method on the deal ledger (e.g. `update_deal_status`).
- The deal ledger canister itself must be upgradeable and its cycles balance managed.
- There is no standardized ICRC protocol for NFT sharding across multiple canisters.
