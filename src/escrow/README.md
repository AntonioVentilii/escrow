# Escrow Engine Canister

An Internet Computer escrow canister implementing **tip and deal flows**: a payer funds a deal-specific ledger subaccount via ICRC-2; for **two-party deals** both sides record a settlement signature (`Yes` / `No`) and the tally drives the outcome; for **tips** (unknown recipient) a claim code lets anyone claim before expiry, and otherwise the payer is refunded. Each deal is also exposed as an **ICRC-7 non-fungible token**, making deals queryable via standard NFT interfaces.

## Security model

### Claim codes

Every deal is assigned a **cryptographically random 128-bit claim code** (32-character hex string) generated via the IC management canister's `raw_rand`. This code is the bearer secret that authorizes fund release for open (unbound-recipient) deals.

- The QR code / share link encodes `deal_id + claim_code`, not just `deal_id`.
- `deal_id` remains a sequential `u64` used for ICRC-7 token IDs and subaccount derivation — knowing the ID alone is not sufficient to claim funds.
- Recipient-bound deals do not require the claim code (the caller's principal is the auth).
- The claim code is returned to the creator in `DealView` but is **never** exposed via `get_claimable_deal`.

### Consent and settlement signatures

Two distinct mechanisms gate funds across the lifecycle:

**Pre-funding consent** (`Consent` enum: `Pending` / `Accepted` / `Rejected`) — gates whether the deal can be funded at all:

- The creator's consent is automatically set to `Accepted` at creation time.
- For deals with a known counterparty, the counterparty must call `consent_deal` before funding can proceed.
- For tips (unknown recipient), the recipient's consent is implicitly granted when they claim.
- Either party can call `reject_deal` to permanently refuse, transitioning the deal to `Rejected`.

**Post-funding settlement signatures** (`Signature` enum: `Empty` / `Yes` / `No`, bound deals only) — drive the terminal outcome of a `Funded` deal via a two-party tally:

- Each bound party calls `sign_yes` or `sign_no` (latest-wins while `Funded`).
- The tally fires on each call: both `Yes` → `Settled`; both `No` → `Aborted`; mixed → auto-`Disputed`; one still `Empty` → deal stays `Funded`.
- At expiry the auto-YES rule fires (any `Empty` signature → `Yes`; explicit votes preserved), then the same tally runs.
- `accept_deal` for bound deals routes through `sign_yes` for the recipient — calling it records the recipient's `Yes` and only settles when the payer also signs `Yes` (or expiry's auto-YES fires).
- Tips have no signatures: they're claimed unilaterally with `accept_deal` + `claim_code`.

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

| Method                          | Description                                                                                                                                                                                                                                                               |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_deal(CreateDealArgs)`   | Create a new deal. Caller is assigned as payer or recipient based on the supplied args. Returns a `DealView` with the claim code.                                                                                                                                         |
| `fund_deal(FundDealArgs)`       | Move tokens from payer to escrow subaccount via ICRC-2 `transfer_from`. Payer must have approved the canister first. Implicitly sets payer consent to `Accepted`.                                                                                                         |
| `accept_deal(AcceptDealArgs)`   | Tip: claim with `claim_code`, bind recipient, settle immediately. Bound deal: routes to `sign_yes` for the recipient — only settles when the payer also signs `Yes` (or expiry's auto-YES fires).                                                                         |
| `sign_yes(FundDealArgs)`        | Bound payer or recipient records a `Yes` settlement signature. Triggers the tally: BothYes → `Settled`; one party still `Empty` → deal stays `Funded`; payer-`Yes` × recipient-`No` → auto-`Disputed`. Latest-wins while `Funded`. Rejects tip flows + post-expiry calls. |
| `sign_no(FundDealArgs)`         | Bound payer or recipient records a `No` settlement signature. BothNo → `Aborted` (refund payer); one party still `Empty` → deal stays `Funded`; payer-`No` × recipient-`Yes` → auto-`Disputed`. Same caller / tip / expiry semantics as `sign_yes`.                       |
| `reclaim_deal(ReclaimDealArgs)` | Tip: payer refund after expiry (legacy behaviour). Bound deal: routes through the expiry auto-YES tally — typically settles to recipient on silence, NOT refunds the payer.                                                                                               |
| `cancel_deal(CancelDealArgs)`   | Either party cancels an unfunded (Created) deal.                                                                                                                                                                                                                          |
| `consent_deal(ConsentDealArgs)` | Explicitly consent to a deal's terms. Required for the counterparty before the payer can fund a deal with a known recipient.                                                                                                                                              |
| `reject_deal(RejectDealArgs)`   | Reject a deal's terms. The deal transitions to `Rejected` (terminal).                                                                                                                                                                                                     |
| `process_expired_deals(limit)`  | Batch-dispatch up to `limit` expired funded deals. Tips refund to payer; bound deals run the auto-YES tally and settle / abort / open-dispute accordingly. Idempotent. Skips deals in `Disputed` state.                                                                   |

### Dispute & arbitrator methods

| Method                                  | Description                                                                                                                                                                                          |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `open_dispute(OpenDisputeArgs)`         | Either bound party of a `Funded` deal opens a dispute. Selects a randomly-weighted arbitrator panel via `raw_rand`. Deal transitions `Funded → Disputed`.                                            |
| `submit_evidence(SubmitEvidenceArgs)`   | Party of the deal or arbitrator on the panel submits evidence (note + off-canister URL + SHA-256 hash) during the Evidence phase.                                                                    |
| `cast_vote(CastVoteArgs)`               | Active arbitrator on the panel votes `ConcludedCorrectly` / `IncorrectlyConcluded` / `Abstain`. Allowed during the open voting window. Latest-wins.                                                  |
| `finalize_dispute(FinalizeDisputeArgs)` | Anyone (non-anonymous) can trigger after `voting_deadline_ns`. Tallies, fans out per-arbitrator fees, transfers prevailing-party payout, flips deal to `ArbitratedSettled` / `ArbitratedRefunded`.   |
| `withdraw_dispute(WithdrawDisputeArgs)` | Out-of-band settlement during the Evidence phase. Either party proposes an outcome; resolution fires when both proposals match. Arbitrators receive a reduced fee (`withdraw_fee_pct`, default 25%). |
| `deregister_arbitrator()`               | Self-deregister (opt-out). In-flight assignments still honoured (non-vote counts as `Abstain` at finalize). To re-enter the pool requires admin re-registration.                                     |

### Dispute & arbitrator queries

| Method                                  | Description                                                                                                                                                           |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `get_dispute(dispute_id)`               | Full dispute view. Caller must be a party of the parent deal or an arbitrator on the panel.                                                                           |
| `get_public_dispute(dispute_id)`        | Reduced public view — no party / panel principals, no evidence URLs. Tally + outcome are revealed only after the dispute reaches `Resolved` (phase-gated disclosure). |
| `list_my_disputes(ListMyDisputesArgs)`  | Paginated reverse-chronological list of disputes the caller is involved with (party or arbitrator on the panel). Optional `phase` filter.                             |
| `get_arbitrator(principal)`             | Returns the arbitrator profile for `principal`, or `None` if unregistered. Public read.                                                                               |
| `list_arbitrators(ListArbitratorsArgs)` | Paginated arbitrator list with optional `status` and `min_score` filters.                                                                                             |

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

All controller-only.

| Method                              | Description                                                                                                                                                                                          |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config()`                          | Read canister configuration.                                                                                                                                                                         |
| `update_config(Config)`             | Update canister configuration.                                                                                                                                                                       |
| `admin_register_arbitrator(args)`   | Register `args.principal` as an arbitrator (curated registration). Idempotent; reactivates `Suspended` / `Deregistered` profiles. Validators reject anonymous + the canister's own principal.        |
| `admin_set_arbitrator_status(args)` | Set an arbitrator's status (`Active` ↔ `Suspended` ↔ `Deregistered`). All transitions allowed; self-transitions are no-op success. `Deregistered → Active` reactivates a previously-removed profile. |

## Deal lifecycle

### Status

| Status               | Description                                                                                                                                                                          |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Created`            | Deal exists, possibly waiting for counterparty consent.                                                                                                                              |
| `Funded`             | Tokens locked in escrow. For bound deals, awaiting both parties' settlement signatures (or expiry's auto-YES tally).                                                                 |
| `Settled`            | Funds released to recipient. Reached when both parties signed `Yes` (manually or via expiry's auto-YES rule), or when a tip was claimed via `accept_deal`. Terminal.                 |
| `Refunded`           | Funds returned to payer. Reached on tip expiry (no claimer), or via the legacy manual `reclaim_deal` path on a tip. Terminal.                                                        |
| `Aborted`            | Both parties signed `No` on a `Funded` bound deal — mutual agreement that the off-chain part didn't happen. Funds refunded to payer using the same fee math as `Refunded`. Terminal. |
| `Cancelled`          | Creator or counterparty cancelled before funding. Terminal.                                                                                                                          |
| `Rejected`           | Counterparty refused the deal terms before funding. Terminal.                                                                                                                        |
| `Disputed`           | A dispute is open on the deal. Funds remain in the escrow subaccount; expiry sweep skips it.                                                                                         |
| `ArbitratedSettled`  | Dispute resolved with a `ConcludedCorrectly` outcome (panel majority CC, or out-of-band withdrawal agreement). Funds released to recipient. Terminal.                                |
| `ArbitratedRefunded` | Dispute resolved with an `IncorrectlyConcluded` outcome or no-quorum fallback. Funds refunded to payer. Terminal.                                                                    |

`Aborted`, `Refunded`, and `ArbitratedRefunded` all return funds to
the payer and share the same fee math, but the audit trail records
WHY the deal didn't settle: mutual `No` signatures, expiry on a tip
with no claimer, or a dispute that resolved against the recipient
respectively.

### Consent

| Consent    | Description                    |
| ---------- | ------------------------------ |
| `Pending`  | Party hasn't responded yet     |
| `Accepted` | Party agrees to the deal terms |
| `Rejected` | Party refuses the deal terms   |

### State machine

```
Created ──[both consent]──▶ Created ──fund──▶ Funded ──[two-sig tally]──▶ Settled         (BothYes)
  │                           │                 │  │                    ▶ Aborted         (BothNo)
  │ reject                    │ cancel          │  │                    ▶ Disputed        (Mixed → auto-open)
  ▼                           ▼                 │  │
Rejected                  Cancelled             │  │ open_dispute       ▶ Disputed
                                                │  │
                                                │  ▼
                                                │  Disputed
                                                │    ├─[majority CC]────▶ ArbitratedSettled
                                                │    ├─[majority IC]────▶ ArbitratedRefunded
                                                │    ├─[no quorum]──────▶ ArbitratedRefunded
                                                │    └─[withdrawn]──────▶ ArbitratedSettled / ArbitratedRefunded
                                                │
                                                │ EXPIRY (bound deal): auto-YES rule applied,
                                                │   then re-tally → Settled / Aborted / Disputed
                                                │
                                                │ EXPIRY (tip, recipient = None) or manual
                                                │ reclaim on tip:
                                                ▼
                                            Refunded
```

`Settled`, `Refunded`, `Aborted`, `Cancelled`, `Rejected`, `ArbitratedSettled`, and `ArbitratedRefunded` are terminal states. `Disputed` is non-terminal — funds stay in escrow until the dispute resolves.

> **Two-signature tally:** For bound deals, settlement is driven by per-party signatures (`Yes` / `No`) recorded via `sign_yes` / `sign_no` (or via `accept_deal` for the recipient, which routes through `sign_yes`). The tally fires on each call: both `Yes` → `Settled`; both `No` → `Aborted`; mixed → auto-`Disputed`; one party still `Empty` → deal stays `Funded`.

> **Auto-YES at expiry:** A repeating timer (every 5 minutes) sweeps expired funded deals via `process_expired_deals`. For tips it refunds the payer (legacy behaviour — silence on a tip means no taker). For bound deals it applies the auto-YES rule (any `Empty` signature → `Yes`; explicit votes preserved) and runs the same tally + dispatch as `sign_*`, so silence by default means "release to recipient". The `reclaim_deal` endpoint on bound deals routes through the same dispatcher, so a payer calling `reclaim_deal` after expiry on an unsigned bound deal will typically end up settling to the recipient, not refunding themselves. Both paths are idempotent. The sweep skips deals in `Disputed` state.

> **Automatic dispute finalize:** A second repeating timer (every 5 minutes) auto-finalises disputes whose `voting_deadline_ns` has passed. Per-dispute errors are swallowed so a single failure (e.g. ledger temporarily unreachable) doesn't block the sweep — it gets retried on the next cycle. The two sweeps share the re-entrancy-guard pattern but use independent flags so they can interleave.

### Flows

**Tip flow** (payer → unknown recipient): no signatures, claim-code based unilateral settle.

1. Payer creates deal → `payer_consent = Accepted`
2. Payer funds → `Funded`
3. QR / link shared (contains `deal_id + claim_code`)
4. Recipient claims with `accept_deal(claim_code)` → `recipient_consent = Accepted`, `Settled`
5. (Or: expiry hits with no claimer → `Refunded`.)

**Two-party deal** (both parties known): two-signature tally drives the outcome.

1. Creator creates deal → creator's consent `Accepted`, counterparty's `Pending`
2. Counterparty calls `consent_deal` → both consents `Accepted`
3. Payer funds → `Funded` (both signatures `Empty`)
4. Each party records a settlement signature via `sign_yes` / `sign_no` (or recipient via `accept_deal`, which routes to `sign_yes`).
5. The tally on each call decides the outcome:
   - both `Yes` → `Settled` (recipient paid)
   - both `No` → `Aborted` (payer refunded)
   - mixed → auto-`Disputed` (panel arbitration)
   - one still `Empty` → deal stays `Funded`; wait for the other party or expiry
6. At expiry the auto-YES rule fires: `Empty` → `Yes`, explicit votes preserved, then re-tally.

**Invoice flow** (recipient creates, payer pays): same as two-party deal, just with the recipient as creator.

1. Recipient creates deal with `payer` specified → `recipient_consent = Accepted`
2. Payer calls `consent_deal` → `payer_consent = Accepted`
3. Payer funds → `Funded`
4. Same two-signature tally as above.

### Fee accounting

Every deal locks a [`DealFees`](./src/types/deal.rs) snapshot at `create_deal` time so subsequent admin `update_config` calls cannot retroactively change the agreed economics. The snapshot carries the escrow service fee, the per-party dispute reserve, the withdraw-fee percentage, and the create-time ledger fee (the last is record-only — every actual transfer re-queries the live `icrc1_fee`).

On every terminal-state transition where funds leave the escrow subaccount, the recipient receives `amount − escrow_fee − ledger_fee`. The `escrow_fee` stays in the per-deal subaccount as the operator's share (a sweeper / treasury is out of scope for now). The `ledger_fee` is debited by the ledger itself.

| Field on `DealFees`         | What it locks                                                                                                                                     |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `escrow_fee`                | Service fee charged on every terminal state. Default `20_000` e8s (= `2 × ICP_LEDGER_FEE`). Admin-tunable via `Config.escrow_fee`.                |
| `dispute_reserve_per_party` | Each party's half of the full dispute cost. Snapshot of `compute_arbitration_fee(amount, DisputeConfig) / 2`. Used by the two-sided reserve flow. |
| `withdraw_fee_pct`          | Reduced arbitrator fee on out-of-band withdrawal resolution. Snapshot of `DisputeConfig.withdraw_fee_pct`.                                        |
| `ledger_fee_at_create`      | Record-only. The ledger's fee at the time of create — never used for arithmetic.                                                                  |

The min-amount check at `create_deal` rejects deals whose `amount` is too small to leave a positive remainder after all fee deductions — see `validation::validate_min_amount`.

## Module structure

| Module                       | Responsibility                                                                                                                                                   |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `types/deal.rs`              | Internal `Deal`, `DealStatus` (incl. `Disputed` / `ArbitratedSettled` / `ArbitratedRefunded` / `Aborted`), `Consent`, `Signature`, `DealMetadata`, `DealFees`    |
| `types/dispute.rs`           | Internal `Dispute`, `DisputeId`, `DisputePhase`, `Vote`, `Evidence`, `PanelMember`, `DisputeOutcome`, `DisputeConfig`                                            |
| `types/arbitrator.rs`        | Internal `ArbitratorProfile`, `ArbitratorStatus`, `MIN_VOTES_FOR_SCORE`, `compute_score` helper                                                                  |
| `types/ledger_types.rs`      | ICRC-1/ICRC-2 Account and transfer types                                                                                                                         |
| `types/icrc7.rs`             | ICRC-7/ICRC-16 `Value` type, ownership helpers, metadata builders                                                                                                |
| `types/state.rs`             | `Config` (incl. `dispute_config: DisputeConfig` + `escrow_fee: u128`), `StableState` for persistence                                                             |
| `api/deals/api.rs`           | Thin deal endpoint layer (delegates to services)                                                                                                                 |
| `api/deals/params.rs`        | Public argument structs (`CreateDealArgs`, `AcceptDealArgs`, …)                                                                                                  |
| `api/deals/results.rs`       | Public view structs (`DealView` (incl. `dispute: Option<DisputeId>`), `ClaimableDealView`)                                                                       |
| `api/deals/errors.rs`        | Typed `EscrowError` enum (incl. dispute / arbitrator variants)                                                                                                   |
| `api/disputes/api.rs`        | Dispute endpoints (`open_dispute`, `submit_evidence`, `cast_vote`, `finalize_dispute`, `withdraw_dispute`, `get_*`, `list_my_*`)                                 |
| `api/disputes/params.rs`     | Public dispute argument structs                                                                                                                                  |
| `api/disputes/results.rs`    | Public dispute view structs (`DisputeView`, `PublicDisputeView`, `DisputeTally`)                                                                                 |
| `api/arbitrators/api.rs`     | Public + self-service arbitrator endpoints (`deregister_arbitrator`, `get_arbitrator`, `list_arbitrators`). Admin-side registration lives in `api/admin/api.rs`. |
| `api/arbitrators/params.rs`  | Public arbitrator argument structs                                                                                                                               |
| `api/arbitrators/results.rs` | Public arbitrator result types                                                                                                                                   |
| `api/icrc7/api.rs`           | ICRC-7 NFT standard query/update endpoints + ICRC-10 supported standards                                                                                         |
| `api/admin/api.rs`           | Controller-only admin endpoints (`config`, `update_config`, `admin_register_arbitrator`, `admin_set_arbitrator_status`)                                          |
| `services/deals.rs`          | Core deal business logic (create, fund, accept, reclaim, cancel, consent, reject, sign) + shared `record_signature_and_dispatch` helper                          |
| `services/disputes.rs`       | Dispute lifecycle (open, open_post_expiry, submit_evidence, cast_vote, finalize, withdraw, queries) + tally + panel selection + auto-finalize sweep              |
| `services/arbitrators.rs`    | Arbitrator registry service (register / deregister / get / list)                                                                                                 |
| `services/expiry.rs`         | Expired-deal dispatcher: tip → refund payer; bound → auto-YES tally → settle / abort / open-dispute. Skips `Disputed`.                                           |
| `services/housekeeping.rs`   | Two repeating timers: expiry auto-refund (every 5 min) + dispute auto-finalize (every 5 min) — independent re-entrancy guards                                    |
| `services/icrc7.rs`          | ICRC-7 service logic (token metadata, ownership, pagination, transfer rejection)                                                                                 |
| `memory.rs`                  | Thread-local storage (deals + disputes + arbitrators), atomic ID allocation, save/restore, processing locks                                                      |
| `ledger.rs`                  | ICRC inter-canister call helpers (transfer, transfer_from, fee, raw_rand)                                                                                        |
| `subaccounts.rs`             | Deterministic deal subaccount derivation                                                                                                                         |
| `validation.rs`              | State transition, consent, dispute, evidence, and input validation                                                                                               |
| `guards.rs`                  | Caller authentication/authorization guards                                                                                                                       |

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

Until ICRC-3 is implemented, the existing `updated_at_ns` / `updated_by` fields on `Deal`, plus the `payer_signature` / `recipient_signature` snapshot (which records each party's last vote), provide a lightweight last-modified record. The `DealStatus` + `Consent` + `Signature` fields together fully describe the current state. A temporary `BTreeMap<DealId, Vec<StateTransition>>` audit log can be added as an intermediate step if needed before the full ICRC-3 integration.

### Disputes and resolution (implemented)

Dispute resolution is fully implemented:

- `DealStatus` gained `Disputed`, `ArbitratedSettled`, `ArbitratedRefunded` variants.
- `Deal` gained `dispute: Option<DisputeId>` linking to the optional dispute record.
- A parallel arbitrator pool (`ArbitratorProfile` keyed by principal) with **admin-curated registration** (controllers add arbitrators via `admin_register_arbitrator`; arbitrators can self-opt-out via `deregister_arbitrator`), score-weighted random panel selection, and admin status moderation via `admin_set_arbitrator_status`. Admin-curated registration (rather than permissionless self-registration) prevents Sybil flooding during the bootstrap window. Each profile records `registered_by` for audit.
- Five new lifecycle endpoints: `open_dispute`, `submit_evidence`, `cast_vote`, `finalize_dispute`, `withdraw_dispute`.
- Out-of-band settlement (`withdraw_dispute`) lets parties resolve in the Evidence phase with a reduced arbitrator fee.
- A second housekeeping timer auto-finalises disputes whose voting deadline has passed.
- The expiry sweep skips `Disputed` deals so a deal mid-arbitration cannot be auto-refunded out from under the panel.
- `compute_reliability_for` and `count_active_deals_for` treat the new arbitrated terminals symmetrically with their unilateral counterparts.
- All knobs (`panel_size`, `min_panel_size`, `max_panel_size`, `evidence_window_ns`, `voting_window_ns`, `arbitration_fee_bps`, `arbitration_min_fee`, `withdraw_fee_pct`, `min_arbitrator_score`) live on `Config::dispute_config: DisputeConfig` and are admin-tunable via `update_config`.
- **Per-deal panel-size override:** the deal creator can pin `CreateDealArgs.panel_size: Option<u32>` to lock a specific panel size into the deal terms, bounded by `[DisputeConfig.min_panel_size, DisputeConfig.max_panel_size]`. `None` falls back to the canister default at `open_dispute` time. The locked value persists across subsequent `update_config` changes — admin can't retroactively grow or shrink panels for existing deals. Out-of-range or even values return `EscrowError::PanelSizeOutOfRange { min, max, got }` carrying the offending value alongside the active range.

### Multi-asset / multi-ledger

The per-deal settlement currency is captured by an `Asset` enum (`src/types/asset.rs`) — today carrying only `Asset::Icrc(Principal)` for ICRC-1/-2 ledgers, but with the variant shape ready to absorb future settlement domains (EVM ERC-20 / native EVM, Solana SPL, …) as backward-compatible Candid variant additions. `ledger.rs` provides a small abstraction layer around ICRC calls; service code calls `asset.as_icrc()?` to dispatch to it. Adding a new variant is a typed-error-returning extension at every existing site (`EscrowError::UnsupportedAsset`) plus a new ledger-helper module — no structural change to the storage / state-machine.

### Scheduled / instalment payments

Add a `parent_deal_id: Option<DealId>` to `Deal` for parent ↔ child relationships. A future `schedule.rs` module can generate child deals from a parent template and release them on a timer, using the same subaccount and ledger helpers.

### Pluggable resolvers

Introduce a `Resolver` trait or enum (`Admin`, `Oracle(Principal)`, `DaoVote { ... }`) stored on the deal. Resolution logic can dispatch on this field, keeping the core state machine unchanged. The arbitrator pool is the first concrete instance of this pattern.

### Storage at scale

See the [Scalability & limitations](#scalability--limitations) section for the full analysis and phased roadmap. The `memory.rs` accessor API makes the migration from `BTreeMap` to `StableBTreeMap` transparent to all callers.

### Deal ledger — deals as ICRC-7 NFTs (implemented)

Each deal is exposed as a non-fungible token via the **ICRC-7** standard query interface. The canister natively implements all ICRC-7 query methods plus ICRC-10 supported-standards discovery. Ownership follows deal lifecycle: the payer owns the token until settlement, at which point the recipient becomes the owner.

Direct `icrc7_transfer` calls are rejected — ownership transitions are managed exclusively through escrow operations (`accept_deal`, `sign_yes` / `sign_no`, `reclaim_deal`, etc.).

| Deal concept                                                       | ICRC-7 equivalent                                            |
| ------------------------------------------------------------------ | ------------------------------------------------------------ |
| `create_deal`                                                      | `icrc7_mint` — mint a new NFT with deal metadata             |
| `accept_deal` (tip) / `sign_yes` (bound deal both-yes tally)       | `icrc7_transfer` — transfer the NFT from payer to recipient  |
| Terminal states (`Settled`, `Refunded`, `Aborted`, `Cancelled`, …) | `icrc7_burn` or metadata update marking the token as settled |
| Deal details (amount, status, expiry, payer, recipient, sigs)      | Token metadata fields (ICRC-16 value map)                    |
| Deal history / audit trail                                         | ICRC-3 transaction log (built into compliant ledgers)        |

**Next steps:** extracting the deal ledger into a dedicated canister and adding ICRC-3 transaction logging. See Phase 2 and Phase 3 in the [scaling roadmap](#scaling-roadmap).

**Open challenges:**

- ICRC-7 does not standardize metadata _updates_ after minting; status transitions would require a custom extension method on the deal ledger (e.g. `update_deal_status`).
- The deal ledger canister itself must be upgradeable and its cycles balance managed.
- There is no standardized ICRC protocol for NFT sharding across multiple canisters.
