# Governance

This page defines what agents may, should, and must not do. It applies
to every agent (Claude Code, Cursor, Copilot, Codex, Aider, opencode, …).

## Truth hierarchy

When two sources disagree, the higher one wins:

1. **The code** (`src/**`, `scripts/**`) — current state of reality.
2. **CI workflows** ([`.github/workflows/**`](../../.github/workflows/)) —
   non-negotiable gates that must be green to merge.
3. **CODEOWNERS** ([`.github/CODEOWNERS`](../../.github/CODEOWNERS)) —
   defines who must approve each path.
4. **`escrow.did`** — the public Candid interface. Removing or renaming
   a variant / method is a breaking change; adding one is not.
5. **This file + sibling pages under `docs/ai/`** — policy.
6. **Active RFCs** under [`docs/rfcs/`](../rfcs/) — design decisions
   that have been accepted but not yet fully implemented.
7. [`AGENTS.md`](../../AGENTS.md) — universal entry, points at the
   above.
8. **Tool-specific layers** ([`CLAUDE.md`](../../CLAUDE.md),
   [`.claude/rules/`](../../.claude/rules/), `.cursor/rules/`,
   `.github/copilot-instructions.md`). These never contradict 1–7.

If an agent finds a contradiction, the agent **stops and asks** instead
of guessing.

## Roles

You can play any of these roles in a single session — most non-trivial
PRs involve all three, in order. Keep them mentally separated.

### Planner

Decompose the task. Surface trade-offs in plain text _before_ editing
files. For anything > ~50 changed lines or touching > 3 files, write a
short bullet plan and confirm scope with the human. For substantive
design changes (new states, new endpoints, new fee model, etc.), write
an RFC under `docs/rfcs/` instead — see
[RFC workflow](#rfc-workflow) below.

### Implementer

Execute the plan with **targeted edits**. Strictly follow:

- [`backend/structure.md`](./backend/structure.md) for placement.
- [`backend/patterns.md`](./backend/patterns.md) for Rust + IC + state-
  machine + Candid + idempotency idioms.

Run the local quality gates from [`pr-and-ci.md`](./pr-and-ci.md)
before declaring done.

### Reviewer

Before opening / merging, self-review against:

- [core rules](../../AGENTS.md#2-core-rules-read-before-every-change)
- [PR conventions](./pr-and-ci.md)
- [`backend/patterns.md`](./backend/patterns.md) — flag unguarded state
  mutations, missing `Result` returns, missing idempotency, missing
  validators.
- The relevant active RFC, if any.

## Boundaries

These paths are **protected**. Agents may read them, but must not
modify them without an explicit ask in the user prompt.

| Path                                                 | Reason                                                                         | Owner            |
| ---------------------------------------------------- | ------------------------------------------------------------------------------ | ---------------- |
| `.github/workflows/**`                               | CI integrity. Only `ci(...)` PRs touch these.                                  | repo maintainers |
| `.github/CODEOWNERS`, `.github/actions/**`           | Review routing & action policy.                                                | repo maintainers |
| `.github/dependabot.yml`                             | Dependency update policy.                                                      | repo maintainers |
| `Cargo.toml`, `Cargo.lock`                           | Workspace dependency set. Bumps require explicit approval (or via dependabot). | repo maintainers |
| `package.json`, `package-lock.json`                  | npm helpers + version. The version field drives `npm run release`.             | repo maintainers |
| `dfx.json`, `canister_ids.json`                      | Canister wiring. Schema drift breaks deploys.                                  | repo maintainers |
| `src/escrow/escrow.did`                              | **Generated** by `npm run did` from the Rust types. Don't hand-edit.           | —                |
| `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml` | Lint / format / toolchain policy.                                              | repo maintainers |
| `target/`, `.dfx/`, `node_modules/`                  | Build output. Never commit.                                                    | —                |
| `../pandame/**`                                      | External repo. Open it as a separate workspace; obey its `AGENTS.md`.          | pandame team     |

If a change must touch a protected path, call it out explicitly in the
PR description.

## Capabilities — quick reference

### Allowed without prompting

- Edit any `.rs` file under `src/escrow/src/{api,services,types,validation,guards,memory,ledger,subaccounts}.rs`.
- Add files inside the existing folder taxonomy
  ([backend structure](./backend/structure.md)).
- Add new `*.rs` test files under `src/escrow/tests/`.
- Run `npm run format`, `npm run lint`, `npm run quality`,
  `npm run test`, `npm run test:integration`, `npm run did`,
  `npm run build`, `npm run deploy` (local).
- Update these `docs/ai/**` pages when the meta-update rule fires.

### Forbidden without explicit prompt

- Add a new dependency or upgrade one (`Cargo.toml` workspace deps).
- Add a new top-level folder under `src/escrow/src/`.
- Edit any path in the [Boundaries](#boundaries) table.
- Remove or rename a variant / method on the public Candid interface
  (`escrow.did` — backward-compat is contractual). Adding is fine.
- Add a new `EscrowError` variant — it's part of the public interface.
- Add a new `DealStatus` variant or state-machine edge — see
  [RFC workflow](#rfc-workflow).
- Disable a clippy lint, suppress a `cargo check` warning, or use
  `unwrap()` / `expect()` outside test code.
- Mutate `Deal.status` outside a `validate_can_*` function.
- Skip / `#[ignore]` an existing test on `main`.
- Run `npm run deploy:staging` / `npm run deploy:prod` /
  `npm run release` — these touch real canisters.
- `git push --force`, amend a pushed commit, rebase to "tidy
  history", or rewrite shared history. "Explicit prompt" means the
  user names the operation directly — any unambiguous phrasing works
  (e.g. "force-push", "force push", "push --force", "push -f",
  "amend", "git commit --amend", "rebase", "git rebase",
  "rewrite history"). Task-level delegation like "do what you think is
  best" or "do what's most correct" does **NOT** count. See
  [`pr-and-ci.md#updating-an-existing-pr`](./pr-and-ci.md#updating-an-existing-pr).
- Commit secrets, `.env*` (other than examples), or large binaries.

## RFC workflow

Substantive design changes (new lifecycle states, new endpoint
families, new roles, new fee models, new on-chain mechanisms) go
through an **RFC** before any code lands.

1. **Write the RFC** under `docs/rfcs/<NNNN>-<slug>.md` where `NNNN`
   is the next zero-padded number.
2. **Open as a draft PR** titled `docs(rfc): <slug> (RFC-<NNNN>)`.
3. **Iterate on the design** in PR comments. The RFC's "Open
   questions" section is the conversation surface — questions become
   decisions in the same PR.
4. **Mark accepted** by removing the "Status: Draft" header. Merge the
   draft PR.
5. **Implementation** lands in subsequent PRs that reference the RFC
   number in their commit body. Each implementation PR is still
   atomic — RFCs are usually multiple implementation PRs.

Existing RFCs:

- [`docs/rfcs/0001-dispute-resolution.md`](../rfcs/0001-dispute-resolution.md) — dispute resolution + arbitrator pool + voting. **Accepted** 2026-05-10; full implementation landed in PR #29. Two post-implementation revisits captured in the Decision Log: Q4 (permissionless self-registration → admin-curated, same PR #29) and Q6 (canister-wide-only `panel_size` → per-deal override bounded by `min_panel_size` / `max_panel_size`, follow-up PR 2026-05-13).

## Meta-update rule

> If a PR introduces a new pattern, new shared helper, new error
> variant, new naming convention, new policy, or a new workflow, the
> PR **MUST** also update the relevant page under `docs/ai/**` so the
> next agent picks it up.

How:

1. Identify which page describes the area you changed:
   - Folder taxonomy → `backend/structure.md`.
   - New shared helper / module-level pattern → `backend/patterns.md`.
   - New `EscrowError` variant → `backend/patterns.md#errors` (and
     update `escrow.did` via `npm run did`).
   - New CI gate / PR rule → `pr-and-ci.md`.
   - New policy / boundary → this file.
   - New design decision (new state, new role, new fee model) → an
     RFC under `docs/rfcs/`, then update the relevant `backend/`
     pages once the RFC is accepted.
2. Edit that page in the same PR with the smallest possible delta — add
   a bullet, add a row, swap a code sample.
3. Mention the doc update in the PR body under `# Changes`.
4. If the change is structural enough that the existing taxonomy stops
   making sense, open a separate **doc-only** PR first:
   `docs(ai): <reshape>` — and land it before the code PR.

This is what makes the docs _auto-adapting_: every PR is a small
opportunity to keep them honest. Reviewers should reject code PRs
that introduce a new pattern without the matching doc update.
