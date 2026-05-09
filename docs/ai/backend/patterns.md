# Backend Patterns

Idiomatic patterns for the Rust escrow canister as it lives in this
repo. If a pattern here disagrees with code in `src/`, the code wins
(truth hierarchy in [`../governance.md`](../governance.md)). Update
this page in the same PR — that's the
[meta-update rule](../governance.md#meta-update-rule).

## The 4-layer call shape

Every public update flow looks like this:

```rust
// 1. api/deals/api.rs — public Candid signature.
#[ic_cdk::update(guard = "caller_is_not_anonymous")]
async fn accept_deal(args: AcceptDealArgs) -> AcceptDealResult {
    services::deals::accept_deal(msg_caller(), args).await.into()
}

// 2. services/deals.rs — business logic, returns Result<T, EscrowError>.
pub async fn accept_deal(
    caller: Principal,
    args: AcceptDealArgs,
) -> Result<DealView, EscrowError> {
    let deal = memory::get_deal(args.deal_id).ok_or(EscrowError::NotFound)?;
    validation::validate_can_accept(&deal, caller, &args.claim_code)?;

    // …mutate via memory::with_deals_mut, then issue the ledger call.
}

// 3. validation.rs — gates the state-machine transition.
pub fn validate_can_accept(
    deal: &Deal,
    caller: Principal,
    claim_code: &Option<String>,
) -> Result<bool, EscrowError> {
    match deal.status {
        DealStatus::Settled => return Ok(true), // idempotent
        DealStatus::Funded => {}
        _ => return Err(EscrowError::InvalidState { … }),
    }
    // …recipient binding + claim-code checks…
}

// 4. ledger.rs — single helper per ICRC call shape.
pub async fn transfer(/* … */) -> Result<u128, EscrowError> { … }
```

**Strict rules:**

- `api/*` functions are thin: parse + guard + delegate + return.
- Services never call `ic_cdk::api::msg_caller()` directly — receive
  the principal from the api wrapper. This is what makes services
  unit-testable.
- Validators are **pure** (no I/O, no `ic_cdk::api::*`). They take
  borrowed input and return `Result<bool, EscrowError>`.
- Storage mutations only via `memory::with_deals_mut`. Never reach
  into the thread-local `STATE` from anywhere else.

## Errors

`EscrowError` (in `api/deals/errors.rs`) is the canonical error type.
All public flows return `Result<T, EscrowError>` (or a `*Result` enum
that inlines this shape for Candid).

### Adding a variant

1. Add it to the enum in `api/deals/errors.rs` with a doc comment that
   explains exactly when it's emitted.
2. Map external errors at the boundary — never let `String` from
   `ic_cdk::call::Call` leak past the api layer:

   ```rust
   .await
   .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?
   ```

3. Run `npm run did` so `escrow.did` reflects the new variant. Commit
   the regenerated `.did` together with the source.

### Don't do this

- ❌ `panic!()` / `unwrap()` / `expect()` in service / api / validation
  code. Use `Result` + `?`.
- ❌ Returning a `String` error type from anything reachable by an
  `ic_cdk::*` annotated function. Always use `EscrowError`.
- ❌ Removing or renaming an existing variant — it's a Candid breaking
  change. Add a new variant and migrate call sites if you need a
  rename.

## Storage — the `with_deal` accessor pattern

`memory.rs` is the single legal owner of the `BTreeMap<DealId, Deal>`.
Every read/write goes through one of these:

| Function                       | Use                                                                                |
| ------------------------------ | ---------------------------------------------------------------------------------- |
| `insert_new_deal(build)`       | Allocate a new ID and insert. **Only** way to create a deal.                       |
| `get_deal(id) -> Option<Deal>` | Cheap clone-out for reads.                                                         |
| `with_deal(id, f)`             | Run `f(&mut Deal)` in-place. Returns `Option<R>` (None if the deal doesn't exist). |
| `with_deals(f)`                | Read-only iteration over all deals.                                                |
| `with_deals_mut(f)`            | Mutable iteration (rare; expiry-sweep uses it).                                    |

**Always update `updated_at_ns` and `updated_by` when mutating a
deal**, otherwise the audit metadata drifts. Idiom:

```rust
memory::with_deal(deal_id, |deal| {
    deal.status = DealStatus::Settled;
    deal.settled_at_ns = Some(now_ns);
    deal.updated_at_ns = Some(now_ns);
    deal.updated_by = Some(caller);
})
.ok_or(EscrowError::NotFound)?;
```

## Idempotency — a hard contract

All funding / settlement / refund flows must be replay-safe. The expiry
sweep (`services::housekeeping`) relies on this: it scans every 5
minutes, and if a deal happens to be reclaimed by the payer between
two sweeps, the next sweep must no-op rather than double-refund.

The pattern is **early-return-Ok on already-terminal status**:

```rust
match deal.status {
    DealStatus::Refunded => return Ok(true), // idempotent
    DealStatus::Funded => {}
    _ => return Err(EscrowError::InvalidState { … }),
}
```

The validator returns `Ok(true)` to mean "the operation is already
done; no further work needed". The service then short-circuits.

When you write a new state-changing flow, add an integration test
that calls it **twice** and asserts the second call is a no-op (same
status, same `_at_ns` fields).

## ICRC ledger calls

`ledger.rs` is the single owner of `ic_cdk::call::Call`. The pattern:

```rust
pub async fn transfer_from(
    ledger: Principal,
    from: Account,
    to: Account,
    amount: u128,
) -> Result<u128, EscrowError> {
    let args = TransferFromArgs { /* … */ };

    let response = Call::unbounded_wait(ledger, "icrc2_transfer_from")
        .with_args(&(args,))
        .await
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    let (inner_result,): (Result<Nat, TransferFromError>,) = response
        .candid_tuple()
        .map_err(|e| EscrowError::LedgerError(format!("{e:?}")))?;

    match inner_result {
        Ok(block_index) => nat_to_u128(&block_index),
        Err(e) => Err(EscrowError::TransferFailed(format!("{e:?}"))),
    }
}
```

**Strict rules:**

- Two-step error mapping: `LedgerError` for transport / decoding
  failures, `TransferFailed` for ledger-accepted-but-failed responses.
- `Nat` always decoded via `nat_to_u128` (not direct `as u128`).
- New ICRC method? Add a new helper in `ledger.rs`. Don't inline `Call`
  construction in a service.

## Caller guards

`guards.rs` provides:

- `caller_is_not_anonymous` — applied to **every** public update
  endpoint via `#[ic_cdk::update(guard = "…")]`. Public read queries
  may also use it; ICRC-7 NFT queries don't (they're public).
- `caller_is_controller` — admin endpoints.

Authorisation **beyond** non-anonymous (e.g. payer-only, recipient-
only, payer-or-recipient) is **not** a guard — it's a check inside the
relevant `validate_can_*` function in `validation.rs`. Reason: those
checks need access to the deal record, which the guard signature
doesn't allow.

## Subaccounts

Each deal gets a deterministic 32-byte subaccount derived from its
`DealId` — see `subaccounts.rs`. Always use the helper; don't
hand-roll a derivation.

```rust
let subaccount = subaccounts::derive_for_deal(deal_id);
```

The escrow's funded subaccount is `Account { owner: <canister id>,
subaccount: Some(subaccount) }`. The canister can `transfer` from this
subaccount on settlement / refund (using `from_subaccount`).

## Random data (`raw_rand`)

The IC management canister's `raw_rand` is used to generate the 128-bit
claim code at deal creation. The wrapper is `ledger::raw_rand` (yes,
it lives in `ledger.rs` because it's an inter-canister `Call`).

```rust
let (random_bytes,) = ledger::raw_rand().await?;
let claim_code = hex::encode(&random_bytes[0..16]);
```

Never call `Call::unbounded_wait(Principal::management_canister(), …)`
inline — go through the helper.

## State machine

Every `DealStatus` transition goes through a `validate_can_*` function.
The current map (in `validation.rs`):

| Function               | From               | To                                                    |
| ---------------------- | ------------------ | ----------------------------------------------------- |
| `validate_can_fund`    | `Created`          | `Funded`                                              |
| `validate_can_accept`  | `Funded`           | `Settled` (returns `Ok(true)` if already `Settled`)   |
| `validate_can_reclaim` | `Funded` + expired | `Refunded` (returns `Ok(true)` if already `Refunded`) |
| `validate_can_cancel`  | `Created`          | `Cancelled`                                           |
| `validate_can_consent` | `Created`          | `Created` (consent flag flip; status unchanged)       |
| `validate_can_reject`  | `Created`          | `Rejected`                                            |

**Adding a new transition** requires an [RFC](../governance.md#rfc-workflow).
The accepted RFC dictates the transition table; the implementation PR
adds it.

## Integration tests

Live under `src/escrow/tests/`. Driven by `pocket-ic`. Required to be
green on every PR.

The shape:

```rust
#[tokio::test]
async fn test_accept_funded_deal_settles() {
    let pic = setup_pic().await;
    let escrow = install_escrow(&pic).await;
    let ledger = install_test_ledger(&pic).await;

    let payer = test_principal("payer");
    let recipient = test_principal("recipient");

    // … create + fund + accept …

    let deal = call_get_deal(&pic, escrow, payer, deal_id).await;
    assert_eq!(deal.status, DealStatus::Settled);
}
```

**For every state-changing flow you add, write at least:**

1. The happy-path test.
2. The idempotency test (call twice, assert no-op).
3. The negative tests (wrong caller, wrong state, expired, etc.) for
   each `EscrowError` variant the flow can emit.

Helpers (`setup_pic`, `install_escrow`, etc.) live in the test crate —
extend them rather than duplicating setup boilerplate.

## Anti-patterns (do not do these)

- `unwrap()` / `expect()` outside test code.
- `panic!` for unreachable cases — model them in `EscrowError`
  (`InvalidState { expected, actual }` is the catch-all).
- Mutating `Deal.status` outside a `validate_can_*` check.
- Reaching into `STATE` directly from a service / api function.
- Service-to-service calls — extract shared logic into `validation.rs`
  or `ledger.rs` instead.
- Inline `Call::unbounded_wait` in a service — go through `ledger.rs`.
- Hand-editing `escrow.did` — regenerate via `npm run did`.
- Removing or renaming a Candid variant / field / method without a
  user prompt asking for the breaking change.
- Adding a new `EscrowError` variant without a corresponding
  `npm run did` regen + a doc comment + a test that triggers it.
- Catching an error and silently swallowing it — propagate via `?` or
  map to a typed variant.
- A timer / `set_timer_interval` that doesn't have a re-entrancy
  guard. The existing one in `services/housekeeping.rs` uses a
  thread-local `SWEEP_RUNNING: Cell<bool>` flag plus an RAII
  `SweepGuard` that resets the flag on drop — follow that pattern.
  (Note: the per-deal lock `PROCESSING: BTreeSet<DealId>` in
  `memory.rs` is a different mechanism — it serialises concurrent
  async flows on the **same deal** via `try_acquire_lock` /
  `release_lock`, used by the deal services, not by the sweep.)
