# CLAUDE.md

Claude-specific runtime layer. Anything not contradicted here defers to
[`AGENTS.md`](./AGENTS.md), which is the canonical entry for **all** agents.

> **Mandatory first step:** read [`AGENTS.md`](./AGENTS.md). Then read the
> matching area README before touching code:
>
> - Backend (Rust canister) → [`docs/ai/backend/README.md`](./docs/ai/backend/README.md)

---

## Project memory (quick reference)

- **What this is:** standalone Rust escrow canister on the Internet
  Computer. Implements ICRC-1/-2 (token transfers + approvals via the
  ledger) + ICRC-7 (every deal is an NFT) + ICRC-10 (supported standards
  discovery).
- **Canister ID:** `umxj5-niaaa-aaaae-af2sq-cai` (staging, mainnet
  subnet). Latest tag: `v0.0.3`.
- **Frontend:** [`../pandame/`](../pandame/) — separate repo. Do not
  modify it from here.
- **Essential commands:**
  - `npm run build` — `cargo build --release` + dfx build wrapper.
  - `npm run test` — unit tests.
  - `npm run test:integration` — `pocket-ic` integration suite.
  - `npm run quality` — `format` + `lint` (rust + workspace + did + GH
    Actions + shell).
  - `npm run did` — regenerate `escrow.did` from the Rust types via
    `ic-cdk`'s `export_candid!`. Always run after touching any
    `#[candid_method]` / `#[ic_cdk::*]` annotation.
  - `npm run deploy` — local dfx deploy. `:staging` / `:prod` for
    network overrides.
  - `npm run release` — version bump + tag + GH release helper.
- **State machine source of truth:**
  [`src/escrow/src/validation.rs`](./src/escrow/src/validation.rs).
  Every `DealStatus` transition goes through a `validate_can_*`
  function before any storage mutation.
- **Storage accessor:**
  [`src/escrow/src/memory.rs`](./src/escrow/src/memory.rs). Use
  `with_deal` / `with_deals` / `with_deals_mut`. Never reach into
  `STATE` directly from a service.
- **Identity / auth:** every public endpoint uses
  `caller_is_not_anonymous` from
  [`src/escrow/src/guards.rs`](./src/escrow/src/guards.rs). Authorisation
  beyond non-anonymous (e.g. payer-only, recipient-only) lives in the
  `validate_*` functions.

---

## Reasoning preferences

- **Plan briefly, then act.** For non-trivial work (>1 file or >50
  lines), lay out a 3–6 step plan in plain text before editing files.
  Keep it tight.
- **Targeted edits.** Use `StrReplace`-style precise edits. Never
  rewrite an entire file when 5 lines change.
- **Explore in parallel.** Batch independent reads / greps / globs in a
  single tool turn. Don't serialize what can be parallel.
- **Stop and ask** if a request is ambiguous about scope, atomicity, or
  policy — better one extra question than a sprawling PR. Especially
  before:
  - Adding a new dependency (`Cargo.toml` workspace deps).
  - Adding a new `EscrowError` variant.
  - Adding a new `DealStatus` variant or state-machine edge.
  - Modifying `dfx.json` / `canister_ids.json` / `package.json`
    `version`.
  - Touching anything under `.github/workflows/**` or
    `.github/actions/**`.
  - Modifying `src/escrow/escrow.did` by hand (regenerate via
    `npm run did` instead).

---

## Coding rules (Claude-specific addenda)

These are on top of the [core rules](./AGENTS.md#2-core-rules-read-before-every-change):

- **Read before edit.** Always read a file (or its relevant range) at
  least once before modifying it. The `Read` / `Grep` tools are cheap.
- **Run quality gates.** Before declaring done:

  ```bash
  npm run format         # prettier + cargo fmt + scripts/format.sh
  npm run lint           # prettier --check + clippy + did/sh/yaml linters
  npm run test           # unit tests
  npm run test:integration   # pocket-ic suite (slow; run before push)
  npm run did            # regen escrow.did if you touched any IDL-bearing types
  npm run build          # cargo build --release via scripts/build.sh
  ```

  In one shot: `npm run quality` (format + lint).

  If you regenerated `escrow.did` (`npm run did`), commit the
  regenerated file together with the source change.

- **Reuse over rebuild.** Before creating a new service / validator /
  ledger helper, search the existing modules. The canister has been
  carefully factored — extending a function is almost always better
  than creating a parallel one.
- **No new dependencies** without explicit user approval (`Cargo.toml`
  workspace deps). Bumps from dependabot count as approval.
- **No new top-level folders** under `src/escrow/src/`. The taxonomy in
  [`docs/ai/backend/structure.md`](./docs/ai/backend/structure.md) is
  closed; surface a question instead of inventing a bucket.
- **Comments are for _why_, not _what_.** No narrating comments
  (`// fetch the deal`). Only write a comment if it captures intent,
  trade-off, or an invariant the code can't express.
- **No `unwrap()` / `expect()` outside tests.** Every fallible operation
  in service / api / validation code returns `Result<T, EscrowError>`.
- **No silent state mutations.** If you mutate `Deal.status`, you must
  go through a validator + a `with_deals_mut` accessor + an `updated_at_ns`
  / `updated_by` update.
- **Idempotency is a hard contract.** Settlement, refund, and accept
  flows must be replay-safe. The expiry sweep depends on this. If
  you're not sure, write the integration test that calls the function
  twice and verifies the second call is a no-op.
- **Never push force / amend pushed commits / rewrite shared history.**
  Add a new commit instead. Approval of a broader task ("do what you
  think is best", "make the most correct one") is **NOT** approval to
  force-push — the user must name the operation directly using any
  unambiguous phrasing (e.g. "force-push", "force push", "push --force",
  "push -f", "amend", "git commit --amend", "rebase", "git rebase",
  "rewrite history"), or pick a multi-choice option whose label contains
  one of those phrases. When in doubt, add a new commit. See
  [`docs/ai/pr-and-ci.md#updating-an-existing-pr`](./docs/ai/pr-and-ci.md#updating-an-existing-pr).

---

## Tool-use cheatsheet

| Goal                          | Use                                         |
| ----------------------------- | ------------------------------------------- |
| Find files by name            | `Glob`                                      |
| Find code by exact text/regex | `Grep` (prefer over shell `rg`)             |
| Find code by meaning          | `SemanticSearch`                            |
| Read a file                   | `Read` (NOT `cat` / `head` / `tail`)        |
| Edit a file                   | `StrReplace` (NOT `sed` / `awk` / heredocs) |
| Run a one-shot command        | `Shell`                                     |
| Multi-step exploration        | `Task` with `subagent_type: "explore"`      |

---

## Personalize & evolve

> [!IMPORTANT]
> If you (the AI agent) recognize a change in project behavior, patterns,
> or requirements that differs from these instructions, you **MUST**
> proactively update the relevant doc — usually a page under
> [`docs/ai/`](./docs/ai/) — in the same PR. See the
> [meta-update rule](./docs/ai/governance.md#meta-update-rule).
> Use the legacy [`.claude/rules/`](./.claude/rules/) cards only for
> very small, Claude-only quick-references; everything substantive
> lives in `docs/ai/`.

---

## Coordination

- For role-based collaboration (planner / implementer / reviewer),
  follow [`docs/ai/governance.md`](./docs/ai/governance.md).
- For PR title, scope, body and CI gates, follow
  [`docs/ai/pr-and-ci.md`](./docs/ai/pr-and-ci.md).
- For meta-updates (changing the rules themselves), follow the
  [meta-update rule](./docs/ai/governance.md#meta-update-rule).
