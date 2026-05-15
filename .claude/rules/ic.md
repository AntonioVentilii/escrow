# Internet Computer + Candid (Claude quick-reference)

> **Authoritative sources:**
>
> - Backend patterns: [`docs/ai/backend/patterns.md`](../../docs/ai/backend/patterns.md)
> - Folder taxonomy: [`docs/ai/backend/structure.md`](../../docs/ai/backend/structure.md)
>
> This card is a Claude-only summary.

## Canister identity

- **Staging:** `umxj5-niaaa-aaaae-af2sq-cai` (latest tag `v0.0.3`).
- **Local:** ephemeral, allocated by `dfx deploy`.

## Public Candid interface

`src/escrow/escrow.did` is **generated** by `npm run did` from the
Rust types via `ic-cdk`'s `export_candid!` macro at the bottom of
`lib.rs`.

- Hand-editing the `.did` is forbidden.
- Run `npm run did` after touching any `#[ic_cdk::*]` annotation, any
  `#[derive(CandidType)]` type, or any function signature reachable
  from the canister boundary.
- Commit the regenerated `.did` together with the source change in
  the same PR.

## Standards compliance

- **ICRC-1** — token transfers (settlement, refund). Helpers in
  `ledger.rs::transfer`.
- **ICRC-2** — token approvals + `transfer_from` (deal funding).
  Helpers in `ledger.rs::transfer_from`.
- **ICRC-7** — every deal is an NFT. Implementation in
  `services/icrc7.rs`. `icrc7_transfer` always returns `Err` — deal
  ownership is managed via escrow operations, not direct NFT
  transfers.
- **ICRC-10** — supported-standards discovery. Returns `ICRC-7` and
  `ICRC-10`.
- **ICRC-3** — transaction log. **Not yet implemented**; tracked in
  `src/escrow/README.md#future-expansion`.

## Authentication

- `caller_is_not_anonymous` guard on every public update endpoint.
- `caller_is_controller` guard for admin / config endpoints.
- Per-record authorisation (payer-only, recipient-only) is **not** a
  guard — it's a check inside the relevant `validate_can_*` function
  in `validation.rs`.

## Time

- `ic_cdk::api::time()` returns nanoseconds since epoch.
- Every timestamp field uses the `_ns` suffix.
- The expiry sweep runs every 5 minutes via
  `ic_cdk_timers::set_timer_interval` (see `services/housekeeping.rs`).
  It uses a thread-local `SWEEP_RUNNING: Cell<bool>` flag + RAII
  `SweepGuard` so concurrent sweeps don't double-refund.
- The per-deal lock `PROCESSING: BTreeSet<DealId>` in `memory.rs`
  (`try_acquire_lock` / `release_lock`) is a different mechanism —
  it serialises concurrent async flows on the **same deal**. Use it
  in deal services, not in timers.

## Randomness

- The IC management canister's `raw_rand` returns 32 bytes. Use the
  `ledger::raw_rand()` wrapper.
- The 128-bit claim code is derived from the first 16 bytes:

  ```rust
  let (random_bytes,) = ledger::raw_rand().await?;
  let claim_code = hex::encode(&random_bytes[0..16]);
  ```

## Subaccounts

- Each deal has a deterministic 32-byte subaccount derived from its
  `DealId`. Helper: `subaccounts::derive_for_deal(deal_id)`.
- The escrow account for deal funds is
  `Account { owner: <canister id>, subaccount: Some(subaccount) }`.

## Upgrade safety

- `pre_upgrade` writes the entire state via
  `memory::save_state()` to stable memory.
- `post_upgrade` restores via `memory::restore_state()` and **must**
  re-arm the housekeeping timer (`services::housekeeping::start_expiry_sweep()`).
  Forgetting this is a silent bug — the timer dies on upgrade.
- `Deal` is `Deserialize` from a fixed-shape Candid record. While the
  canister is in active development (pre-v0.1.0) the staging instance
  is reinstalled from scratch on every deploy and carries no
  production-meaningful state, so new fields land as required (not
  `Option<T>`-wrapped) and any pre-existing stable snapshot that
  lacks them fails to deserialise on `post_upgrade` by design.
  Once the canister reaches v1.0 and accumulates real deals, this
  stance flips: subsequent additions go in as `Option<T>` for
  backward-compatible deserialisation. See RFC-002 § Migration.

## Local development

- `npm run deploy` — local dfx deploy with `--upgrade-unchanged`.
- `juno dev start` is **not** used here — escrow is a plain dfx
  canister, not a Juno satellite.
- Emulator port: dfx default (`4943`) — see `dfx.json` networks.

> [!IMPORTANT]
> Do **NOT** run `npm run deploy:staging` or `npm run deploy:prod` or
> `npm run release` without an explicit user prompt — they touch real
> canisters and cut public releases.
