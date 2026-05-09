# Rust Patterns (Claude quick-reference)

> **Authoritative source:** [`docs/ai/backend/patterns.md`](../../docs/ai/backend/patterns.md).
> This card is a Claude-only summary.

## Error handling

- Every fallible operation: `Result<T, EscrowError>` and `?`.
- Map external errors at the boundary:

  ```rust
  ic_cdk::call::Call::unbounded_wait(/* … */)
      .await
      .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?
  ```

- No `unwrap()` / `expect()` / `panic!()` outside tests. For genuinely
  unreachable cases, model in `EscrowError` (the catch-all is
  `InvalidState { expected, actual }`).

## Async + inter-canister calls

- All ICRC ledger calls go through helpers in `ledger.rs`. Don't inline
  `Call::unbounded_wait` in a service.
- The wait variant is `unbounded_wait` (no timeout) for the existing
  helpers. Match that.
- Decode with `.candid_tuple()` and tuple-pattern destructuring:

  ```rust
  let (inner_result,): (Result<Nat, TransferError>,) = response.candid_tuple()?;
  ```

## Storage idioms

- `memory::insert_new_deal(|id| Deal { id, … })` — atomic ID + insert.
- `memory::with_deal(id, |deal| { deal.status = …; })` — in-place
  mutate; returns `Option<R>`.
- `memory::get_deal(id) -> Option<Deal>` — clone-out for read-only
  flows.
- Always update `updated_at_ns` + `updated_by` when mutating.

## Match patterns

- Always exhaustive on `DealStatus` / `Consent` / `EscrowError`. Use
  `match`, never `if let` chains for variant dispatch.
- Idempotency match:

  ```rust
  match deal.status {
      DealStatus::Settled => return Ok(true), // already-done short-circuit
      DealStatus::Funded => {}
      _ => return Err(EscrowError::InvalidState { /* … */ }),
  }
  ```

## Module boundaries

- `api/` → `services/` → `{validation, memory, ledger, subaccounts}.rs`.
- `services/` never calls another `services/` module. Extract shared
  logic to `validation.rs` or `ledger.rs`.
- `validation.rs` is **pure**: no `ic_cdk::*`, no I/O. Borrowed input
  → `Result<bool, EscrowError>`.

## Lints / clippy

- Don't `#[allow(...)]` a clippy lint — fix the code. The repo's
  `clippy.toml` is calibrated.
- `allow_attributes = deny` is enforced (see PR #24 for context). Use
  `#[expect(...)]` over `#[allow(...)]` when a suppression is genuinely
  required.

## Tests

- Unit tests inline in modules: `#[cfg(test)] mod tests { … }`.
- Integration tests under `src/escrow/tests/` driven by `pocket-ic`.
  Naming: `test_<flow>_<scenario>`.
- Every state-changing flow needs: happy path + idempotency (call
  twice) + per-error-variant negative test.
