# Backend Structure & Naming

The folder taxonomy is **closed**: do not add new top-level folders
under `src/escrow/src/` without explicit user approval. Place new code
in the folder that already owns the concern.

## Top level (`src/escrow/src/`)

```
src/escrow/src/
├── lib.rs                  Canister entry: init / pre_upgrade / post_upgrade + export_candid!
│
├── api/                    Public endpoints — thin wrappers that delegate to services.
│   ├── mod.rs
│   ├── deals/              Deal lifecycle endpoints (create, fund, accept, …).
│   │   ├── mod.rs
│   │   ├── api.rs          The actual #[ic_cdk::update] / #[ic_cdk::query] functions.
│   │   ├── params.rs       Public Candid argument structs (`CreateDealArgs`, …).
│   │   ├── results.rs      Public Candid view structs and `*Result` enums.
│   │   └── errors.rs       The `EscrowError` enum (the canonical error type).
│   ├── icrc7/              ICRC-7 NFT query/transfer endpoints + ICRC-10 supported-standards.
│   ├── reliability/        Reliability-score read endpoint.
│   └── admin/              Controller-only config endpoints.
│
├── services/               Core business logic. Pure of `ic_cdk::api` for testability.
│   ├── mod.rs
│   ├── deals.rs            create / fund / accept / reclaim / cancel / consent / reject.
│   ├── expiry.rs           Batch expired-deal refund processing.
│   ├── housekeeping.rs     Repeating timer that calls expiry.rs every 5 minutes.
│   ├── icrc7.rs            ICRC-7 service logic (token metadata, ownership, transfer rejection).
│   └── reliability.rs      Reliability-score computation.
│
├── types/                  Internal + public Candid types. Distinguished by visibility.
│   ├── mod.rs
│   ├── arbitrator.rs       Arbitrator profile + status (curated registry).
│   ├── asset.rs            `Asset` enum — settlement-currency abstraction (today: `Icrc(Principal)` only).
│   ├── deal.rs             Internal `Deal`, `DealStatus`, `Consent`, `DealMetadata`, `DealFees`.
│   ├── dispute.rs          `Dispute`, `DisputeConfig`, `DisputePhase`, `DisputeOutcome`, `Vote`, `PanelMember`, `Evidence`.
│   ├── icrc7.rs            ICRC-7 / ICRC-16 `Value`, ownership helpers, metadata builders.
│   ├── ledger_types.rs     ICRC-1 / -2 `Account` + transfer types (re-exported to api).
│   └── state.rs            Canister `Config` + `StableState` for persistence.
│
├── memory.rs               Thread-local `STATE` cell, atomic ID alloc,
│                           save/restore on upgrade, processing locks.
├── ledger.rs               ICRC-1 / -2 inter-canister call helpers; `raw_rand` wrapper.
├── subaccounts.rs          Deterministic per-deal subaccount derivation.
├── validation.rs           State-machine + consent + input validation.
└── guards.rs               Caller authentication guards (`caller_is_not_anonymous`).
```

## The four-layer rule

```
api/         (public Candid signature)
  ↓
services/    (orchestration, business logic)
  ↓
{validation, ledger, memory, subaccounts}.rs  (single-concern helpers)
```

- **`api/*` functions** are thin: parse args, call `caller()`, hand off
  to a `services::*` function, return the typed `*Result`.
- **`services/*` functions** orchestrate: validate, compute, mutate
  storage via `memory.rs`, schedule ledger calls via `ledger.rs`.
- **No service module ever calls another service module.** Cross-cutting
  helpers live in `validation.rs`, `ledger.rs`, `memory.rs`,
  `subaccounts.rs`, or `guards.rs`. If two services need the same
  thing, extract it into one of those — never service-to-service.

## Naming conventions

These are **strict**. Rust + clippy + rustfmt enforce most of them; the
rest are on you.

### File suffixes

| Where             | Convention              | Example                                          |
| ----------------- | ----------------------- | ------------------------------------------------ |
| `api/<area>/`     | One module per concern  | `api.rs`, `params.rs`, `results.rs`, `errors.rs` |
| `services/`       | `<area>.rs` per concern | `deals.rs`, `expiry.rs`, `icrc7.rs`              |
| `types/`          | `<area>.rs` per concern | `deal.rs`, `state.rs`, `ledger_types.rs`         |
| Top-level helpers | `<concern>.rs`          | `validation.rs`, `guards.rs`, `memory.rs`        |

### Symbols

| Thing                | Style             | Example                                 |
| -------------------- | ----------------- | --------------------------------------- |
| Public function      | `snake_case`      | `validate_can_accept`, `with_deals_mut` |
| Type / enum / struct | `PascalCase`      | `Deal`, `DealStatus`, `EscrowError`     |
| Enum variant         | `PascalCase`      | `Funded`, `RecipientMismatch`           |
| Module               | `snake_case`      | `deals`, `subaccounts`                  |
| Constant             | `SCREAMING_SNAKE` | `MAX_ACTIVE_DEALS`, `DEFAULT_EXPIRY_NS` |
| Test function        | `test_<thing>`    | `test_accept_funded_deal_settles`       |

### Time variables — `_ns`

Time at the canister boundary is **always** nanoseconds:

- **`_ns`** suffix on every nanosecond field: `created_at_ns`,
  `expires_at_ns`, `funded_at_ns`, `settled_at_ns`, …
- **`_ms`** is reserved for FE-side conversions (in `../pandame/`); the
  canister never sees ms.
- The IC native time source is `ic_cdk::api::time()` — already ns.

### `Option<T>` and `[] | [T]` Candid mapping

Every Rust `Option<T>` becomes a `[] | [T]` in the generated Candid.
On the FE side, `@dfinity/utils`'s `fromNullable` / `toNullable` is the
canonical wrapper — always use those, never `value[0]`.

## Errors

`EscrowError` is the **canonical** error type. Every public endpoint
returns `Result<T, EscrowError>` (or `*Result` enums that inline this
shape for Candid compatibility).

Adding a new variant:

1. Add the variant to `api/deals/errors.rs` with a doc comment
   explaining when it's emitted.
2. Run `npm run did` so `escrow.did` reflects the new variant.
3. Document the trigger condition + which call site emits it.
4. **Do not remove or rename existing variants** — that's a Candid
   breaking change.

See [`patterns.md#errors`](./patterns.md#errors) for the full taxonomy.

## State machine

`DealStatus` transitions are gated by the `validate_can_*` family in
`validation.rs`. The current valid edges (RFC-001 added the `Disputed`,
`ArbitratedSettled`, `ArbitratedRefunded` triple):

```
Created ──[both consent]──▶ Created ──fund──▶ Funded ──accept──▶ Settled
  │                           │                 │  │
  │ reject                    │ cancel          │  │ open_dispute
  ▼                           ▼                 │  ▼
Rejected                  Cancelled             │  Disputed
                                                │    ├─[majority CC]──▶ ArbitratedSettled
                                                │    ├─[majority IC]──▶ ArbitratedRefunded
                                                │    ├─[no quorum]────▶ ArbitratedRefunded (Q9)
                                                │    └─[Q12 withdrawn]▶ ArbitratedSettled / ArbitratedRefunded
                                                │ reclaim (after expiry, if not Disputed)
                                                ▼
                                            Refunded
```

`Settled`, `Refunded`, `Cancelled`, `Rejected`, `ArbitratedSettled`,
`ArbitratedRefunded` are terminal. `Disputed` is non-terminal — funds
remain in the escrow subaccount until the dispute resolves.

**Adding a new state or edge requires an [RFC](../governance.md#rfc-workflow).**
The accepted RFC dictates the new transition table; the implementation
PR adds it.

## Where to put new files (decision tree)

1. **Is it a new public endpoint?** → `api/<area>/api.rs` (existing
   area) or, if it doesn't fit any existing area, surface a question
   (rare — almost everything is a deal endpoint).
2. **Is it a new business-logic function?** → `services/<area>.rs`
   (existing area).
3. **Is it a new validator / state-machine check?** → `validation.rs`
   (single file by design).
4. **Is it a new ICRC ledger call helper?** → `ledger.rs` (single
   file by design).
5. **Is it a new caller guard?** → `guards.rs` (single file by
   design).
6. **Is it a new type that crosses the Candid boundary?** → either
   `api/<area>/{params,results,errors}.rs` (public) or
   `types/<area>.rs` (re-exported by api).
7. **Is it a new internal type?** → `types/<area>.rs`.
8. **Is it a new integration test?** → `tests/<flow>.rs` under
   `src/escrow/tests/` — see [`patterns.md#integration-tests`](./patterns.md#integration-tests).
9. **Is it generated?** → don't create it by hand. Run the generator
   (`npm run did` for `escrow.did`).
10. **None of the above?** → ask. Don't invent a folder.
