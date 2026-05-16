# Backend AI Guide

If you are about to touch anything under `src/escrow/src/`,
`src/escrow/tests/`, or `scripts/**`, this is your starting point.
Read it once per session.

> Higher up the chain: [`AGENTS.md`](../../../AGENTS.md) → [`docs/ai/`](../README.md).

## Pre-flight checklist (every change)

- [ ] I read [`AGENTS.md`](../../../AGENTS.md) and the
      [core rules](../../../AGENTS.md#2-core-rules-read-before-every-change).
- [ ] I know which folder my code belongs in — see
      [`structure.md`](./structure.md).
- [ ] My code follows [`patterns.md`](./patterns.md): `Result<T, EscrowError>` returns,
      `caller_is_not_anonymous` guards on every public endpoint, state mutations only via a
      `validate_can_*` function + a `with_deals_mut` accessor, idempotent flows.
- [ ] If I added a new public endpoint or changed a Candid-bearing
      type, I ran `npm run did` and committed the regenerated
      `escrow.did`.
- [ ] If I added a new `EscrowError` variant, I documented it in the
      Rust enum **and** noted the call site that produces it.
- [ ] I added or updated a `pocket-ic` integration test for any new
      lifecycle path. Idempotency tests (call the function twice, expect
      the second call to be a no-op) for any state-changing flow.
- [ ] Local quality gates pass —
      [`../pr-and-ci.md`](../pr-and-ci.md#4-local-quality-gates).
- [ ] PR title + body match conventions — [`../pr-and-ci.md`](../pr-and-ci.md).
- [ ] If I introduced a new pattern, I updated `docs/ai/**` in the
      same PR ([meta-update rule](../governance.md#meta-update-rule)).
- [ ] If I implemented part of an accepted RFC, I referenced its
      number in the commit body.

## Stack at a glance

- **Rust + `ic-cdk` + `candid`.** Single canister, single workspace.
  Toolchain pinned via `rust-toolchain.toml`.
- **Storage:** `BTreeMap<DealId, Deal>` in a thread-local `RefCell`,
  serialised to stable memory on `pre_upgrade` / `post_upgrade`. The
  `memory.rs` accessor API (`with_deal`, `with_deals`,
  `with_deals_mut`) is the single legal way to touch storage.
- **Standards compliance:** ICRC-1 / ICRC-2 (token transfers via the
  ledger), ICRC-7 (every deal is an NFT), ICRC-10 (supported-standards
  discovery). See [`src/escrow/README.md`](../../../src/escrow/README.md).
- **Tests:** unit tests inline in modules; integration tests under
  `src/escrow/tests/` driven by `pocket-ic`. The integration suite is
  required to be green on every PR.
- **Frontend:** [`../pandame/`](../../../../pandame/) consumes the
  Candid interface. Coordinate breaking changes — see
  [`../pr-and-ci.md#9-cross-repo-coordination-pandame`](../pr-and-ci.md#9-cross-repo-coordination-pandame).

## Where things go (one-liner)

```
src/escrow/
├── escrow.did              Generated Candid IDL (DO NOT hand-edit; npm run did)
├── src/
│   ├── lib.rs              Canister entry — init / pre_upgrade / post_upgrade,
│   │                       export_candid! macro at the bottom.
│   ├── api/
│   │   ├── deals/          Public deal endpoints (params / results / errors / api).
│   │   ├── icrc7/          ICRC-7 NFT query / transfer (always-Err) endpoints.
│   │   ├── reliability/    Per-principal reliability score.
│   │   └── admin/          Controller-gated config + treasury endpoints, plus the public `get_fees` snapshot.
│   ├── services/
│   │   ├── deals.rs        Core deal business logic (create / fund / accept / …).
│   │   ├── expiry.rs       Batch expired-deal refund processing.
│   │   ├── housekeeping.rs Repeating timer that auto-refunds expired deals.
│   │   ├── icrc7.rs        ICRC-7 service logic.
│   │   └── reliability.rs  Reliability score computation.
│   ├── types/
│   │   ├── deal.rs         Internal `Deal`, `DealStatus`, `Consent`, `DealMetadata`.
│   │   ├── icrc7.rs        ICRC-7 / ICRC-16 `Value` type and helpers.
│   │   ├── ledger_types.rs ICRC-1 / -2 `Account` and transfer types.
│   │   └── state.rs        Canister `Config` + `StableState` for persistence.
│   ├── memory.rs           Thread-local storage + atomic ID + save/restore.
│   ├── ledger.rs           ICRC-1 / -2 inter-canister call helpers + `raw_rand`.
│   ├── subaccounts.rs      Deterministic per-deal subaccount derivation.
│   ├── validation.rs       State-machine + consent + input validators.
│   └── guards.rs           Caller guards (`caller_is_not_anonymous`).
└── tests/                  pocket-ic integration suite.
```

Full taxonomy and naming conventions: [`structure.md`](./structure.md).

## What "good" looks like in this repo

A 10x change is small, focused, and reuses what's there. Recent merged
PRs to learn from (from `git log` on `main`):

- `chore: bump version to 0.0.3` — single-purpose release commit.
- `fix(clippy): restore allow_attributes = deny (main still has allow)` (#24)
  — single lint nit, contained scope, mentions the upstream context.
- `ci(release): add Release workflow` (#23) — single piece of
  infrastructure, doesn't touch any code.

If your PR doesn't look like one of those (single verb, single
concern, small diff), reconsider scope before continuing. Substantive
design changes (new states, new endpoints, new mechanisms) go through
an [RFC](../governance.md#rfc-workflow) first.
