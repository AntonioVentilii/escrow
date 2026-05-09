# AI Agents Documentation

This is the long-form documentation that backs the agent entry points
([`AGENTS.md`](../../AGENTS.md), [`CLAUDE.md`](../../CLAUDE.md), and the
legacy [`.claude/rules/`](../../.claude/rules/) cards).

If you are an agent: do **not** read everything. Read the entry point first
(`AGENTS.md`), then jump to the specific page you need.

## Map

```
docs/ai/
├── README.md                    ← you are here
├── governance.md                Truth hierarchy, boundaries, capabilities, meta-update rule
├── pr-and-ci.md                 PR title regex, body template, CI cheatsheet, local gates
└── backend/
    ├── README.md                Backend (Rust canister) bootstrap — start here for any change
    ├── structure.md             Folder taxonomy under src/escrow/src/, naming, module rules
    └── patterns.md              Rust + IC + ICRC + state-machine + Candid + idempotency idioms
```

Active RFCs live under [`docs/rfcs/`](../rfcs/) (created when the first
RFC is filed).

## Audience

- **AI agents** (Claude Code, Cursor, Copilot, Codex, Aider, opencode, …).
- **Humans** giving instructions to those agents — including non-engineers
  describing product requirements that need to be turned into Rust + Candid.

## Maintenance — auto-adapting

These docs **must auto-adapt**. When you (agent or human) introduce a
new pattern, naming convention, shared helper, error variant, or
workflow, update the relevant page in the **same PR** as the code
change. See the [meta-update rule](./governance.md#meta-update-rule).

## Relationship to the legacy `.claude/rules/`

The repository also keeps Claude-specific guidance under
[`.claude/rules/`](../../.claude/rules/) and operational runbooks
under [`.agents/workflows/`](../../.agents/workflows/):

- `.claude/rules/*.md` — short Claude-only quick-reference cards.
  Defer to the matching `docs/ai/**` page; only Claude-specific tweaks
  live there.
- `.agents/workflows/*.md` — operational runbooks (e.g. local /
  staging / prod deploy).

If two pages disagree, the page under `docs/ai/` wins (see the
[truth hierarchy](./governance.md#truth-hierarchy)).
