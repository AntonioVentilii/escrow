# PR & CI

Everything an agent needs to open a green PR.

## 1. PR title

The repo follows [Conventional Commits](https://www.conventionalcommits.org/).
Recent merged history (from `git log` on `main`) — use as templates:

- `chore: bump version to 0.0.3`
- `chore(ci): bump setup-node and add release helper (#25)`
- `fix(clippy): restore allow_attributes = deny (main still has allow) (#24)`
- `chore: bump version to 0.0.2`
- `ci(release): add Release workflow (#23)`

Pattern: `verb(scope)?: description` — scope optional but encouraged.

### Verbs

| verb       | when                                                               |
| ---------- | ------------------------------------------------------------------ |
| `feat`     | new endpoint, new lifecycle state, new public capability           |
| `fix`      | bug fix                                                            |
| `refactor` | internal change with no behaviour change                           |
| `perf`     | performance improvement                                            |
| `docs`     | docs only (incl. `docs/ai/**`, `docs/rfcs/**`, READMEs)            |
| `test`     | unit / integration tests only                                      |
| `chore`    | misc maintenance (dependency bumps, version, release housekeeping) |
| `build`    | build system / packaging                                           |
| `ci`       | CI workflows / actions                                             |

### Scope

Single word or comma-separated list of affected areas. Use the existing
vocabulary so it shows up grouped in changelogs:

- `deals`, `icrc7`, `admin`, `reliability`, `validation`, `ledger`,
  `subaccounts`, `housekeeping`, `cargo-deps`, `cargo-deps-dev`,
  `github-actions`, `clippy`, `rfc`, `ai` (for `docs/ai/**` updates).

If you introduce a new scope, keep it short and lowercase, no spaces.

### Breaking changes

If your change breaks the public Candid interface (`escrow.did`),
mark the title with `!`:

```
feat(deals)!: rename DealView field `metadata` to `details`
```

…and add a `BREAKING CHANGE:` block in the body explaining what
callers (currently: PandaMe) need to do.

The Candid interface is contractual — adding variants / fields is
backward-compatible, removing or renaming is not. Anything in the
"breaking" category requires the user to opt in explicitly.

## 2. PR body — template

Honor [`.github/pull_request_template.md`](../../.github/pull_request_template.md):

```markdown
# Motivation

<!-- Describe the motivation that lead to the PR -->

# Changes

<!-- List the changes that have been developed -->

# Tests

<!-- Please provide any information or screenshots about the tests that have been done -->
```

Rules:

- **All three sections are required.** Don't leave them empty. Even tiny PRs benefit from one bullet per section.
- **Use the exact section headings** (`# Motivation`, `# Changes`, `# Tests`).
- **Do not hard-wrap lines.** Write one line per paragraph or list item and let the GitHub renderer wrap. Hard-wrapping at ~80 columns (a default many models fall back to) breaks rendering inside lists, blockquotes, and tables, and makes later edits in the GitHub UI ugly. This applies to the PR body only — source files still follow `rustfmt.toml`.
- **Atomicity statement** if the PR touches more than one logical thing — add a one-liner explaining why they belong together. If you can't, split.
- **Mention RFC numbers** in `# Motivation` whenever the PR implements a piece of an accepted RFC (e.g. "Implements step 2 of RFC-001 — dispute resolution.").
- **Mention `docs/ai/` updates** under `# Changes` whenever the [meta-update rule](./governance.md#meta-update-rule) fired.
- **For Candid-breaking changes**, include a `BREAKING CHANGE:` block in `# Changes` listing the field / variant that changed and what callers must do.
- **For changes that need a coordinated frontend update**, call out the matching PandaMe PR in `# Motivation` so a reviewer can sequence the deploy.

## 3. Atomicity

One logical change per PR. If you catch yourself writing
"and also" / "while I was here" / "I noticed that" in the body, split.

| Anti-pattern                              | Do this instead                                                                           |
| ----------------------------------------- | ----------------------------------------------------------------------------------------- |
| "Add Disputed status and arbitrator pool" | PR 1: `feat(deals): add Disputed status + transition`. PR 2: `feat: arbitrator registry`. |
| "Fix bug Y and update unrelated docs"     | Two PRs.                                                                                  |
| "Refactor 5 services into shared `Foo`"   | PR 1: introduce `Foo` + migrate 1 caller. PR 2..N: migrate the others.                    |
| "New endpoint + run `npm run did`"        | One PR — the regenerated `escrow.did` belongs together with the source.                   |

## 4. Local quality gates

From the repo root, before opening the PR:

```bash
# Rust + workspace
npm run format               # cargo fmt + scripts/format.sh
npm run lint                 # prettier --check + scripts/lint.sh (clippy + did + sh + yaml)
npm run quality              # = format && lint, in one shot

# Tests
npm run test                 # unit tests
npm run test:integration     # pocket-ic suite (slow; required before push)

# Candid
npm run did                  # regenerate escrow.did + format + lint
                             # commit the regenerated .did with the source change

# Build
npm run build                # cargo build --release via scripts/build.sh
```

The CI workflow [`.github/workflows/checks.yml`](../../.github/workflows/checks.yml)
runs `format` and `lint`. The
[`.github/workflows/tests.yml`](../../.github/workflows/tests.yml)
workflow runs unit + integration. The `format` job currently fails
loudly when formatting changes are needed (no auto-commit bot
configured); always run `npm run format` locally first.

## 5. CI jobs you must keep green

| Workflow      | Job(s)        | What it runs                                                |
| ------------- | ------------- | ----------------------------------------------------------- |
| `checks.yml`  | `format`      | `npm run format`. Fails if formatting changes are needed.   |
| `checks.yml`  | `lint`        | `npm run lint` (prettier `--check` + clippy + did/sh/yaml). |
| `checks.yml`  | `checks-pass` | Aggregator — must be green to merge.                        |
| `tests.yml`   | `unit`        | `npm run test:unit`.                                        |
| `tests.yml`   | `integration` | `npm run test:integration` (`pocket-ic`).                   |
| `tests.yml`   | `tests-pass`  | Aggregator — must be green to merge.                        |
| `release.yml` | release       | Triggered on tag push from `npm run release`.               |

If your change is doc-only, the `format` and `lint` jobs still run
because they cover the whole repo. The `tests` workflow runs too,
but a doc-only change can't break it (no Rust source touched), so it
passes trivially.

## 6. After CI fails

- **`format` failed** → run `npm run format` locally and push the
  formatting changes. Don't bypass with `cargo fmt -- --check` overrides.
- **`lint` failed** → run `npm run lint` locally. Common catches:
  - `clippy::*` warnings — fix the code, don't `#[allow(...)]` the lint.
  - Stale `escrow.did` — run `npm run did` and commit the regenerated
    file alongside the source change.
  - Shell script style — `scripts/lint.sh` runs on `scripts/**/*.sh`.
- **`unit` / `integration` failed** → reproduce locally with
  `npm run test` / `npm run test:integration`. Don't `#[ignore]` to
  go green; either fix the code or fix the test.
- **`checks-pass` / `tests-pass` red but children green** → the
  aggregator has a stale cache; push an empty no-op commit or rerun.

## 7. Updating an existing PR

- **Add commits.** Never `git push --force` to a PR branch. Don't
  `git commit --amend` after pushing. Don't rebase a PR onto `main` to
  "tidy history".
- **What counts as "the user explicitly asks":** the user names the
  operation directly — any unambiguous phrasing works. Examples that
  count: "force-push", "force push", "push --force", "push -f",
  "amend", "amend the commit", "git commit --amend", "rebase",
  "git rebase", "rewrite history", "rewrite the history". Selecting
  a multi-choice option whose label itself contains one of those
  phrases also counts. Anything else **DOES NOT** count, including:
  - "do what you think is best",
  - "do what's most correct" / "do it the idiomatic way",
  - "do it your way" / "use your judgement",
  - approval of a stacked-PR plan,
  - approval of a refactor that would "look cleaner" in the originating PR.

  If in doubt, **add a new commit** even if the result looks messier.
  Squash-merge tidies history at merge time; force-push destroys it.

- When the agent is offering choices, the **default** must always be
  the no-force-push option. Do not put a force-push option first, and
  do not pick a force-push option in response to delegated decisions.
- Typical legitimate reasons a user might ask for a force-push include
  removing an accidentally-committed secret (rotate the secret afterwards
  too) or recovering from a catastrophic mistake. These are illustrative,
  not an exhaustive whitelist.
- If you need to drop a commit, push a new revert commit instead.

## 8. CODEOWNERS auto-routing

[`.github/CODEOWNERS`](../../.github/CODEOWNERS) routes reviews. Agents
do not assign reviewers — the file does it.

## 9. Cross-repo coordination (PandaMe)

[`../pandame/`](../../pandame/) is the SvelteKit frontend. It pulls
this canister's `escrow.did` via its own `npm run did` script.
Coordinate when:

1. **Adding a new public endpoint** to `escrow.did`. PandaMe's call
   sites won't see it until they rerun `npm run did`. Land the escrow
   change first; open a follow-up PR in `../pandame/`.
2. **Renaming or removing a Candid type / field / variant**. This
   breaks PandaMe's TypeScript bindings. Land the rename in escrow,
   then immediately open the matching `../pandame/` PR.
3. **Adding a new error variant**. PandaMe's `escrow.canister.ts`
   `unwrap()` helper just renders whatever shape it gets, so this
   never breaks the build — but consumers may want to handle the new
   variant explicitly.

If your escrow PR requires a PandaMe PR, name it in
`# Motivation` so a reviewer can sequence the merges.
