# Coding Standards (Claude quick-reference)

> **Authoritative sources:**
>
> - Folder taxonomy + naming: [`docs/ai/backend/structure.md`](../../docs/ai/backend/structure.md)
> - Patterns: [`docs/ai/backend/patterns.md`](../../docs/ai/backend/patterns.md)
> - 10 commandments: [`AGENTS.md`](../../AGENTS.md#2-the-10-commandments-read-before-every-change)
>
> This card is a Claude-only summary. If it disagrees with the docs
> above, the docs above win.

## Code philosophy

- **Idiomatic Rust + IC.** Match the surrounding code's style. Use
  `Result<T, EscrowError>`, `?`, `with_deal` accessors, `match` on
  `DealStatus`. Don't bring patterns from non-IC Rust projects.
- **DRY at the helper layer, not service-to-service.** When two
  services need the same thing, extract to `validation.rs`,
  `ledger.rs`, `memory.rs`, `subaccounts.rs`, or `guards.rs`.
- **Single-responsibility modules.** `api/` parses, `services/`
  orchestrates, `validation.rs` checks, `ledger.rs` calls ledgers,
  `memory.rs` owns storage. Don't blur the boundaries.
- **No `unwrap()` / `expect()` / `panic!()` outside tests.**
- **No `String` errors past the api layer.** Map at the boundary to
  `EscrowError::LedgerError` / `EscrowError::TransferFailed` /
  `EscrowError::ValidationError`.

## File naming & namespacing

- **Functions:** `snake_case`. `validate_can_*` for state-machine
  validators. `with_*` for accessor closures.
- **Types:** `PascalCase`. `DealStatus`, `EscrowError`, `DealView`.
- **Modules:** `snake_case`. `deals`, `subaccounts`, `housekeeping`.
- **Constants:** `SCREAMING_SNAKE`. `MAX_ACTIVE_DEALS`,
  `DEFAULT_EXPIRY_NS`.
- **Files:** `<area>.rs` per concern. Public-Candid types live in
  `api/<area>/{params,results,errors}.rs`; internal types in
  `types/<area>.rs`.

## Time variables — `_ns`

Time at the canister boundary is **always** nanoseconds. Suffix every
`u64` ns timestamp with `_ns`: `created_at_ns`, `expires_at_ns`,
`funded_at_ns`. Use `ic_cdk::api::time()` to read it. No `_ms` inside
the canister; that's a frontend concept.

## State machine

Every `DealStatus` transition goes through a `validate_can_*` function
in `validation.rs`. **Never** mutate `Deal.status` outside one. Always
update `updated_at_ns` + `updated_by` when mutating.

Adding a new state or edge requires an
[RFC](../../docs/ai/governance.md#rfc-workflow). The accepted RFC
dictates the transition table; the implementation PR adds it.

## Idempotency

Funding / settlement / refund flows must be safe to replay. The
expiry sweep relies on it. The pattern:

```rust
match deal.status {
    DealStatus::Refunded => return Ok(true), // idempotent no-op
    DealStatus::Funded => {}
    _ => return Err(EscrowError::InvalidState { … }),
}
```

For every state-changing flow you add, write a "call twice" test that
asserts the second call is a no-op.

## Compliance

- Run `npm run quality` before completing tasks.
- Run `npm run test` (unit) + `npm run test:integration` (`pocket-ic`)
  before pushing.
- Run `npm run did` after touching any IDL-bearing type / endpoint;
  commit the regenerated `escrow.did`.
- Don't `#[allow(...)]` a clippy lint — fix the code.
- Don't `#[ignore]` a test on `main` — either fix it or remove it.

## Identity & guards

- `caller_is_not_anonymous` is the default guard on every public
  update endpoint.
- `caller_is_controller` for admin / config endpoints.
- Per-deal authorisation (payer-only, recipient-only, …) lives in
  `validate_can_*` functions, not in guards. Reason: guards can't see
  the deal record.

## Storage

- Never reach into `STATE` directly from a service / api / validation
  function. Use `memory::get_deal`, `memory::with_deal`,
  `memory::with_deals`, `memory::with_deals_mut`.
- The atomic ID allocator is `memory::insert_new_deal(build)` — the
  **only** public way to create a deal.

## Candid contract

`escrow.did` is the public interface to PandaMe. **Generated** via
`npm run did`. Don't hand-edit.

- Adding a variant / field / method → backward-compatible. Fine.
- Removing or renaming a variant / field / method → breaking. Requires
  an explicit user prompt + a `feat!` PR title + a `BREAKING CHANGE:`
  block + a coordinated PandaMe PR.
