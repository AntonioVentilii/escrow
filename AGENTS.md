# AGENTS.md

Canonical entry point for **all** AI coding agents working in this repository
(Claude Code, Cursor, OpenAI Codex / GPT, Aider, GitHub Copilot, Continue,
opencode, …). If your tool reads `AGENTS.md` automatically, this is the right
file. If it doesn't, point it here.

> **Read this first. Always.** It is short on purpose. Everything deeper lives
> under [`docs/ai/`](./docs/ai/) and is linked below.

---

## 1. What this repo is

A standalone **Escrow engine** running as a single Rust canister on the
Internet Computer. Payers lock ICRC-1 / ICRC-2 tokens into deal-specific
subaccounts; recipients claim before a deadline or the funds are
auto-refunded. Every deal is also an **ICRC-7 NFT** so wallets and
explorers can index deals through the standard NFT interface.

The frontend lives in a separate repository
([`pandame`](https://github.com/RetroPandaClub/pandame), typically
checked out at `../pandame/`). It pulls this canister's `escrow.did`
via `npm run did` and never modifies anything in this repo.

| Stack       | Path                                     | Language                   | Status                                  |
| ----------- | ---------------------------------------- | -------------------------- | --------------------------------------- |
| Canister    | `src/escrow/src/`                        | Rust (`ic-cdk` + `candid`) | **AI-active**                           |
| Candid IDL  | `src/escrow/escrow.did`                  | Candid                     | **AI-active** (regen via `npm run did`) |
| Tests       | `src/escrow/tests/`                      | Rust (`pocket-ic`)         | **AI-active**                           |
| Build / ops | `scripts/`                               | Bash + `dfx`               | AI-assisted                             |
| dfx wiring  | `dfx.json`, `canister_ids.json`          | JSON                       | Restricted boundary                     |
| CI / infra  | `.github/workflows/`, `.github/actions/` | YAML                       | Restricted                              |
| Release     | `package.json`, `Cargo.toml`             | npm + cargo                | Restricted (use `npm run release`)      |

The canister is published as `v0.0.3` on staging
(`umxj5-niaaa-aaaae-af2sq-cai`).

---

## 2. Core rules (read before every change)

1. **Always idiomatic.** Match the conventions of the surrounding Rust —
   `Result<T, EscrowError>` for fallible work, `with_deal` / `with_deals`
   accessors for storage, `caller_is_not_anonymous` guards on every
   public endpoint. Don't import patterns from other Rust projects.
2. **Always atomic.** One logical change per PR. No drive-by refactors.
   No "while I'm here" edits.
3. **Always small.** Prefer 5 small PRs over 1 big PR. Recent merged
   history is the model: `chore: bump version to 0.0.3`,
   `fix(clippy): restore allow_attributes = deny`,
   `ci(release): add Release workflow`.
4. **Always reusable.** Before adding a helper, search `services/`,
   `validation.rs`, `ledger.rs`, `subaccounts.rs`, `guards.rs`. Extend
   what's there.
5. **Always typed.** No `unwrap()` in service code (only in tests).
   Wrap external errors as `EscrowError` variants (see
   `api/deals/errors.rs`). Never let an `ic_cdk::call` return value
   leak as `String` past the api layer.
6. **Always idempotent at the canister boundary.** All funding /
   settlement / refund operations must be safe to replay. The expiry
   sweep relies on this.
7. **Respect the structure.** New code goes in the folder that already
   owns that concern (`api/{deals,icrc7,admin,reliability}/`,
   `services/`, `types/`, `validation.rs`, `guards.rs`, `memory.rs`,
   `ledger.rs`, `subaccounts.rs`). The taxonomy is closed — see
   [`docs/ai/backend/structure.md`](./docs/ai/backend/structure.md).
8. **Respect the state machine.** Every status transition goes through
   the validators in `validation.rs`. Don't mutate `Deal.status`
   anywhere else. Adding a new status means adding a new transition
   table entry, not bypassing the existing ones.
9. **Respect the Candid contract.** Public types in `escrow.did` are an
   external interface — adding a variant is backward-compatible,
   removing one is not. Run `npm run did` after any `.did` change so
   pandame's regen stays consistent.
10. **Respect CI.** Run the local gates from
    [`docs/ai/pr-and-ci.md`](./docs/ai/pr-and-ci.md#4-local-quality-gates)
    before opening a PR.
11. **Don't overengineer.** A 10x engineer ships the smallest correct
    change. **No new dependencies without explicit user approval** —
    `Cargo.toml` is a protected boundary.

---

## 3. Where to look (canister)

| You're about to…                           | Read first                                                                                         |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| Open any PR                                | [`docs/ai/pr-and-ci.md`](./docs/ai/pr-and-ci.md)                                                   |
| Touch any backend file                     | [`docs/ai/backend/README.md`](./docs/ai/backend/README.md)                                         |
| Add or move a file                         | [`docs/ai/backend/structure.md`](./docs/ai/backend/structure.md)                                   |
| Write a new endpoint / service / validator | [`docs/ai/backend/patterns.md`](./docs/ai/backend/patterns.md)                                     |
| Add an ICRC ledger call                    | [`docs/ai/backend/patterns.md#icrc-ledger-calls`](./docs/ai/backend/patterns.md#icrc-ledger-calls) |
| Add a new `EscrowError` variant            | [`docs/ai/backend/patterns.md#errors`](./docs/ai/backend/patterns.md#errors)                       |
| Add an integration test                    | [`docs/ai/backend/patterns.md#integration-tests`](./docs/ai/backend/patterns.md#integration-tests) |
| Regen Candid bindings                      | After every `.did` change: `npm run did`                                                           |
| Deploy locally / staging                   | [`.agents/workflows/deploy.md`](./.agents/workflows/deploy.md)                                     |
| Cut a release                              | `npm run release` — see `scripts/release.sh`                                                       |

---

## 4. Where to look (frontend — `../pandame/`)

The PandaMe SvelteKit app consumes this canister's Candid interface.
**Do not** modify pandame from this repo. If a backend change requires
a coordinated frontend update, land the backend change here, then open
a follow-up PR in `../pandame/` that runs `npm run did` against the
new tag and adapts the call sites.

| You're about to…                                     | Read                                                                           |
| ---------------------------------------------------- | ------------------------------------------------------------------------------ |
| Understand how PandaMe talks to this canister        | [`../pandame/AGENTS.md`](../pandame/AGENTS.md)                                 |
| See which endpoints PandaMe currently uses           | [`../pandame/src/lib/api/escrow.api.ts`](../pandame/src/lib/api/escrow.api.ts) |
| Check the PandaMe Figma reference for product intent | Ask the user — Figma URLs aren't checked in.                                   |

---

## 5. Governance & meta

- **Truth hierarchy.** When two sources disagree, the higher one wins
  — full ladder lives in
  [`docs/ai/governance.md#truth-hierarchy`](./docs/ai/governance.md#truth-hierarchy).
  Briefly: code > CI workflows > CODEOWNERS > `escrow.did` >
  `docs/ai/**` (this scaffold) > active RFCs > `AGENTS.md` >
  tool-specific layers (`CLAUDE.md`, `.claude/rules/`, …).
- **RFCs** for substantive design decisions. Process documented in
  [`docs/ai/governance.md#rfc-workflow`](./docs/ai/governance.md#rfc-workflow);
  filed RFCs live under [`docs/rfcs/`](./docs/rfcs/). The dispute /
  arbitration flow is the first one — tracked separately from this
  scaffold.
- **Auto-adapting docs.** When a PR introduces a new pattern, convention,
  shared helper, error variant, or workflow, the agent **MUST** update
  the relevant `docs/ai/**` file in the same PR. See
  [`docs/ai/governance.md#meta-update-rule`](./docs/ai/governance.md#meta-update-rule).

---

## 6. Tool-specific entry points

These are thin layers on top of this file. They never contradict it.

- **Claude Code / Anthropic:** [`CLAUDE.md`](./CLAUDE.md) (+ legacy
  [`.claude/rules/`](./.claude/rules/) cards).
- **Cursor:** drop a rule under `.cursor/rules/` that points here.
- **GitHub Copilot:** drop `.github/copilot-instructions.md` that points
  here.
- **OpenAI Codex / Aider / opencode / Continue / …:** read this file
  (`AGENTS.md`).

If you add a new agent / tool, add a tiny pointer file (≤ 30 lines) here
that references this `AGENTS.md` — do **not** duplicate the rules.
