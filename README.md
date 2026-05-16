# Escrow

A generic escrow engine on the [Internet Computer](https://internetcomputer.org/), built as a Rust canister.

Two flows on top of one engine, both following the **commit-at-first-action** principle (every party deposits everything they owe on their first money-moving call — there is no separate "fund" step). **Bound deals** between two known parties: creator's `create_deal` deposits their share, counterparty's `consent_deal` deposits theirs and auto-flips the deal to `Funded`; settlement is then driven by a two-party signature tally — each side records `Yes` or `No`, and the result drives `Settled` (both `Yes`), `Aborted` (both `No`), or auto-`Disputed` (mixed) — with a panel of curated arbitrators to resolve disputes. Bound deals also pay a small `creation_fee` at create that lands in the canister's controller-controlled treasury subaccount (anti-spam deterrent). **Tips** to an unknown recipient: payer's `create_deal` deposits the full amount and the deal goes straight to `Funded`; anyone with the bearer claim code can claim before expiry, otherwise the payer is refunded. Every deal is also exposed as an **ICRC-7 non-fungible token**, enabling standard wallets, explorers, and other canisters to discover and display deals without custom integration.

## Standards compliance

| Standard | Description                                        |
| -------- | -------------------------------------------------- |
| ICRC-1   | Fungible token transfers (settlement & refund)     |
| ICRC-2   | Token approval (`transfer_from` for funding deals) |
| ICRC-7   | Non-fungible token queries (deals as NFTs)         |
| ICRC-10  | Supported-standards discovery                      |

## Use cases

Each row links to a visual flow doc with sequence + state diagrams.

| Use case                             | Description                                                                                                                                                         | Visual flow                                                          |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| **Tip**                              | Payer locks tokens for an unknown recipient; anyone with the bearer claim code can claim before expiry, otherwise the payer gets a refund.                          | [`docs/flows/tip.md`](docs/flows/tip.md)                             |
| **Payer-creator deal (3a)**          | Payer creates a bound deal with a known recipient. Recipient consents (deposits dispute reserve), payer funds, both parties sign at settlement (`Yes` / `No`).      | [`docs/flows/payer-creator.md`](docs/flows/payer-creator.md)         |
| **Recipient-creator deal (3b)**      | Recipient creates an invoice for a known payer (deposits the dispute reserve atomically). Payer consents and funds; both parties sign at settlement.                | [`docs/flows/recipient-creator.md`](docs/flows/recipient-creator.md) |
| **Dispute** (overlay on bound deals) | Either bound party can open a dispute (manually, via mixed sign-tally, or via expiry's auto-YES rule). Random arbitrator panel votes; majority decides the outcome. | [`docs/flows/dispute.md`](docs/flows/dispute.md)                     |

Index + glossary: [`docs/flows/README.md`](docs/flows/README.md). Long-form security model + frontend integration for the tip flow: [`TIPS.md`](TIPS.md). Future use cases (instalment payments, multi-party escrow, ...) are tracked in the [future expansion](src/escrow/README.md#future-expansion) section.

## Scalability

The canister currently stores all deals in heap memory (~4–8 M deals). ICRC-7 has no built-in token-state sharding, so all deal NFTs live in one canister. A phased migration to `ic-stable-structures` (hundreds of millions of deals) and optional canister sharding (unbounded) is documented in the [scalability & limitations](src/escrow/README.md#scalability--limitations) section.

## Technical documentation

For the full API reference, deal lifecycle, module structure, and ICRC-7 NFT interface, see the [escrow canister README](src/escrow/README.md).

## 📖 Project Guidelines & AI Agent Docs

This project follows strict development patterns. AI agents (Claude Code, Cursor, Copilot, Codex, …) and humans should start at the canonical entry point:

- **[AGENTS.md](./AGENTS.md)** — universal entry for every AI agent.
- **[CLAUDE.md](./CLAUDE.md)** — Claude-specific runtime layer (defers to AGENTS.md).
- **[docs/ai/](./docs/ai/)** — long-form documentation:
  - [`docs/ai/governance.md`](./docs/ai/governance.md) — truth hierarchy, boundaries, capabilities, RFC workflow, meta-update rule.
  - [`docs/ai/pr-and-ci.md`](./docs/ai/pr-and-ci.md) — PR conventions, CI gates, local quality gates.
  - [`docs/ai/backend/`](./docs/ai/backend/) — Rust + IC + ICRC + state-machine + Candid conventions, structure, patterns.
- **[docs/rfcs/](./docs/rfcs/)** — substantive design RFCs (created when the first one lands).
- **[.agents/workflows/](./.agents/workflows/)** — operational runbooks (deploy, …).
- **[.claude/rules/](./.claude/rules/)** — Claude-only quick-reference cards that defer to `docs/ai/`.
