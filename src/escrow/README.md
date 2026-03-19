# Escrow Engine Canister

An Internet Computer escrow canister implementing **tip and deal flows**: a payer funds a deal-specific ledger subaccount via ICRC-2, a recipient claims before expiry, and otherwise the payer is refunded. Each deal is also exposed as an **ICRC-7 non-fungible token**, making deals queryable via standard NFT interfaces.

## Security model

### Claim codes

Every deal is assigned a **cryptographically random 128-bit claim code** (32-character hex string) generated via the IC management canister's `raw_rand`. This code is the bearer secret that authorizes fund release for open (unbound-recipient) deals.

- The QR code / share link encodes `deal_id + claim_code`, not just `deal_id`.
- `deal_id` remains a sequential `u64` used for ICRC-7 token IDs and subaccount derivation — knowing the ID alone is not sufficient to claim funds.
- Recipient-bound deals do not require the claim code (the caller's principal is the auth).
- The claim code is returned to the creator in `DealView` but is **never** exposed via `get_claimable_deal`.

### Consent

Both parties to a deal must consent before funds can move:

- The creator's consent is automatically set to `Accepted` at creation time.
- For deals with a known counterparty, the counterparty must call `consent_deal` before funding can proceed.
- For tips (unknown recipient), the recipient's consent is implicitly granted when they claim.
- Either party can call `reject_deal` to permanently refuse, transitioning the deal to `Rejected`.

### Node provider visibility

> **Important:** On the Internet Computer, node providers have access to the raw state of canisters running on their subnet. This means that any unencrypted data stored in a canister — including claim codes, deal amounts, and participant principals — is technically readable by node operators.
>
> **Implications:**
>
> - A malicious node operator could read a claim code from canister state and front-run a legitimate recipient's claim.
> - This is an **inherent property of the IC's current architecture**, not specific to this canister.
> - For the **tip flow** (payer → unknown recipient via QR code), the claim code is a bearer token. This makes tips fundamentally vulnerable to node-level snooping, though the attack window is limited by expiry deadlines and the small amounts typical of tips.
> - For **deals between known parties** (both payer and recipient specified), the claim code is not required — authentication is based on the caller's principal, which is cryptographically verified. These deals are **not vulnerable** to node-level claim code extraction.
>
> **Mitigations (defense in depth):**
>
> | Mitigation                                                                                                                    | Effect                                                                   |
> | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
> | Short expiry windows                                                                                                          | Reduces the attack window                                                |
> | Recipient binding when possible                                                                                               | Eliminates bearer-token risk entirely                                    |
> | Amount caps on open deals                                                                                                     | Limits exposure per tip                                                  |
> | [VetKeys](https://internetcomputer.org/docs/building-apps/network-features/encryption/vetkeys) (threshold encryption, future) | Would allow encrypting claim codes so node providers see only ciphertext |
>
> **Bottom line:** for anonymous tips to strangers, the claim code is a bearer token with an inherent node-level risk bounded by short expiry and small amounts. For deals between known parties, principal-based authentication eliminates this risk entirely.

## Public API

### Escrow update methods

| Method                          | Description                                                                                                                                                       |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_deal(CreateDealArgs)`   | Create a new deal. Caller is assigned as payer or recipient based on the supplied args. Returns a `DealView` with the claim code.                                 |
| `fund_deal(FundDealArgs)`       | Move tokens from payer to escrow subaccount via ICRC-2 `transfer_from`. Payer must have approved the canister first. Implicitly sets payer consent to `Accepted`. |
| `accept_deal(AcceptDealArgs)`   | Recipient claims a funded deal before expiry. Requires `claim_code` for open deals. Binds the recipient if unset. Sets recipient consent to `Accepted`.           |
| `reclaim_deal(ReclaimDealArgs)` | Payer reclaims funds from an expired, unclaimed deal.                                                                                                             |
| `cancel_deal(CancelDealArgs)`   | Either party cancels an unfunded (Created) deal.                                                                                                                  |
| `consent_deal(ConsentDealArgs)` | Explicitly consent to a deal's terms. Required for the counterparty before the payer can fund a deal with a known recipient.                                      |
| `reject_deal(RejectDealArgs)`   | Reject a deal's terms. The deal transitions to `Rejected` (terminal).                                                                                             |
| `process_expired_deals(limit)`  | Batch-refund up to `limit` expired funded deals. Idempotent.                                                                                                      |

### Escrow query methods

| Method                           | Description                                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `get_deal(deal_id)`              | Full deal view including claim code. Caller must be payer or recipient.                                      |
| `list_my_deals(ListMyDealsArgs)` | Paginated deals where caller is payer or recipient.                                                          |
| `get_claimable_deal(deal_id)`    | Reduced public view for claim/share-link pages. No claim code, no payer. Any authenticated caller may query. |
| `get_escrow_account(deal_id)`    | Returns the escrow `Account` (canister principal + deal subaccount). Caller must be payer or recipient.      |

> **Note:** All escrow endpoints reject anonymous callers (`caller_is_not_anonymous` guard).

### ICRC-7 NFT queries (open, no auth required)

Every deal is a non-fungible token. Ownership follows deal lifecycle: the payer owns the token until `Settled`, at which point the recipient becomes the owner.

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

### Status

| Status      | Description                                            |
| ----------- | ------------------------------------------------------ |
| `Created`   | Deal exists, possibly waiting for counterparty consent |
| `Funded`    | Tokens locked in escrow                                |
| `Settled`   | Funds released to recipient (happy path)               |
| `Refunded`  | Funds returned to payer (expiry)                       |
| `Cancelled` | Creator or counterparty cancelled before funding       |
| `Rejected`  | Counterparty refused the deal terms                    |

### Consent

| Consent    | Description                    |
| ---------- | ------------------------------ |
| `Pending`  | Party hasn't responded yet     |
| `Accepted` | Party agrees to the deal terms |
| `Rejected` | Party refuses the deal terms   |

### State machine

```
Created ──[both consent]──▶ Created ──fund──▶ Funded ──accept──▶ Settled
  │                           │                 │
  │ reject                    │ cancel          │ reclaim (after expiry)
  ▼                           ▼                 ▼
Rejected                  Cancelled          Refunded
```

`Settled`, `Refunded`, `Cancelled`, and `Rejected` are terminal states.

> **Automatic refund:** A repeating timer (every 5 minutes) sweeps expired funded deals and refunds them automatically. The `reclaim_deal` endpoint serves as a manual fallback. Both paths are idempotent.

### Flows

**Tip flow** (payer → unknown recipient):

1. Payer creates deal → `payer_consent = Accepted`
2. Payer funds → `Funded`
3. QR / link shared (contains `deal_id + claim_code`)
4. Recipient claims with `claim_code` → `recipient_consent = Accepted`, `Settled`

**Two-party deal** (both parties known):

1. Creator creates deal → creator's consent `Accepted`, counterparty's `Pending`
2. Counterparty calls `consent_deal` → both consents `Accepted`
3. Payer funds → `Funded`
4. Recipient accepts → `Settled`

**Invoice flow** (recipient creates, payer pays):

1. Recipient creates deal with `payer` specified → `recipient_consent = Accepted`
2. Payer calls `consent_deal` → `payer_consent = Accepted`
3. Payer funds → `Funded`
4. Recipient accepts → `Settled`

## Module structure

| Module                     | Responsibility                                                                    |
| -------------------------- | --------------------------------------------------------------------------------- |
| `types/deal.rs`            | Internal `Deal`, `DealStatus`, `Consent`, `DealMetadata` types                    |
| `types/ledger_types.rs`    | ICRC-1/ICRC-2 Account and transfer types                                          |
| `types/icrc7.rs`           | ICRC-7/ICRC-16 `Value` type, ownership helpers, metadata builders                 |
| `types/state.rs`           | Config, StableState for persistence                                               |
| `api/deals/api.rs`         | Thin deal endpoint layer (delegates to services)                                  |
| `api/deals/params.rs`      | Public argument structs (`CreateDealArgs`, `AcceptDealArgs`, …)                   |
| `api/deals/results.rs`     | Public view structs (`DealView`, `ClaimableDealView`)                             |
| `api/deals/errors.rs`      | Typed `EscrowError` enum                                                          |
| `api/icrc7/api.rs`         | ICRC-7 NFT standard query/update endpoints + ICRC-10 supported standards          |
| `api/admin/api.rs`         | Controller-only admin endpoints                                                   |
| `services/deals.rs`        | Core deal business logic (create, fund, accept, reclaim, cancel, consent, reject) |
| `services/expiry.rs`       | Batch expired-deal refund processing                                              |
| `services/housekeeping.rs` | Repeating timer that auto-refunds expired deals every 5 minutes                   |
| `services/icrc7.rs`        | ICRC-7 service logic (token metadata, ownership, pagination, transfer rejection)  |
| `memory.rs`                | Thread-local storage, atomic deal-ID allocation, save/restore, processing locks   |
| `ledger.rs`                | ICRC inter-canister call helpers (transfer, transfer_from, raw_rand)              |
| `subaccounts.rs`           | Deterministic deal subaccount derivation                                          |
| `validation.rs`            | State transition, consent, and input validation                                   |
| `guards.rs`                | Caller authentication/authorization guards                                        |

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

### State history / audit trail via ICRC-3

The `Deal` struct intentionally stores only the **current state**, not a history of transitions. A full audit trail (who changed what, when, and from which state) is deferred to the ICRC-3 transaction log standard, which is purpose-built for this:

- **ICRC-3** defines an append-only, tamper-evident transaction log for ICRC-7 tokens. Each state transition (create, fund, consent, settle, refund, reject) would be recorded as a transaction entry with the caller's principal, timestamp, and previous/new status.
- This aligns with Phase 2 of the [scaling roadmap](#scaling-roadmap) — when the deal ledger is extracted into a dedicated canister, ICRC-3 logging comes naturally as part of its standard interface.
- Keeping history **out** of the `Deal` struct has deliberate benefits:
  - **Privacy**: the audit trail can have separate access control from the deal itself (e.g. visible only to arbiters during disputes, not to both parties by default).
  - **Scalability**: the `Deal` struct stays fixed-size, which is critical for the `StableBTreeMap` migration (Phase 1). An inline `Vec<Transition>` would grow unboundedly with dispute back-and-forth.
  - **Standards compliance**: no custom storage format — the ICRC-3 log is interoperable with any IC tooling that supports the standard.

Until ICRC-3 is implemented, the existing `updated_at_ns` / `updated_by` fields on `Deal` provide a lightweight last-modified record, and the `DealStatus` + `Consent` fields fully describe the current state. A temporary `BTreeMap<DealId, Vec<StateTransition>>` audit log can be added as an intermediate step if needed before the full ICRC-3 integration.

### Disputes and resolution

Add `Disputed` / `ArbitratedSettled` / `ArbitratedRefunded` variants to `DealStatus`. The explicit state machine in `validation.rs` already gates every transition — new edges (e.g. `Funded -> Disputed`, `Disputed -> ArbitratedSettled`) can be added alongside the existing ones. The `Deal` struct can gain optional fields (`dispute_evidence`, `resolver`, `resolution_outcome`) wrapped in `Option` for backward-compatible deserialization.

### Multi-asset / multi-ledger

The `token_ledger` field is already per-deal. `ledger.rs` provides a small abstraction layer around ICRC calls; extending it to multiple ledgers requires no structural change. Fee handling can be added to `TransferArg` construction.

### Scheduled / instalment payments

Add a `parent_deal_id: Option<DealId>` to `Deal` for parent ↔ child relationships. A future `schedule.rs` module can generate child deals from a parent template and release them on a timer, using the same subaccount and ledger helpers.

### Pluggable resolvers

Introduce a `Resolver` trait or enum (`Admin`, `Oracle(Principal)`, `DaoVote { ... }`) stored on the deal. Resolution logic can dispatch on this field, keeping the core state machine unchanged.

### Storage at scale

See the [Scalability & limitations](#scalability--limitations) section for the full analysis and phased roadmap. The `memory.rs` accessor API makes the migration from `BTreeMap` to `StableBTreeMap` transparent to all callers.

### Deal ledger — deals as ICRC-7 NFTs (implemented)

Each deal is exposed as a non-fungible token via the **ICRC-7** standard query interface. The canister natively implements all ICRC-7 query methods plus ICRC-10 supported-standards discovery. Ownership follows deal lifecycle: the payer owns the token until settlement, at which point the recipient becomes the owner.

Direct `icrc7_transfer` calls are rejected — ownership transitions are managed exclusively through escrow operations (`accept_deal`, `reclaim_deal`, etc.).

| Deal concept                                            | ICRC-7 equivalent                                            |
| ------------------------------------------------------- | ------------------------------------------------------------ |
| `create_deal`                                           | `icrc7_mint` — mint a new NFT with deal metadata             |
| `accept_deal`                                           | `icrc7_transfer` — transfer the NFT from payer to recipient  |
| Terminal states (`Settled`, `Refunded`, `Cancelled`)    | `icrc7_burn` or metadata update marking the token as settled |
| Deal details (amount, status, expiry, payer, recipient) | Token metadata fields (ICRC-16 value map)                    |
| Deal history / audit trail                              | ICRC-3 transaction log (built into compliant ledgers)        |

**Next steps:** extracting the deal ledger into a dedicated canister and adding ICRC-3 transaction logging. See Phase 2 and Phase 3 in the [scaling roadmap](#scaling-roadmap).

**Open challenges:**

- ICRC-7 does not standardize metadata _updates_ after minting; status transitions would require a custom extension method on the deal ledger (e.g. `update_deal_status`).
- The deal ledger canister itself must be upgradeable and its cycles balance managed.
- There is no standardized ICRC protocol for NFT sharding across multiple canisters.
