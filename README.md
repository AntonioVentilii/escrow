# Escrow

A generic escrow engine on the [Internet Computer](https://internetcomputer.org/), built as a Rust canister.

Payers lock tokens into deal-specific subaccounts; recipients claim them before a deadline or the funds are automatically refunded. Every deal is also exposed as an **ICRC-7 non-fungible token**, enabling standard wallets, explorers, and other canisters to discover and display deals without custom integration.

## Standards compliance

| Standard | Description                                        |
| -------- | -------------------------------------------------- |
| ICRC-1   | Fungible token transfers (settlement & refund)     |
| ICRC-2   | Token approval (`transfer_from` for funding deals) |
| ICRC-7   | Non-fungible token queries (deals as NFTs)         |
| ICRC-10  | Supported-standards discovery                      |

## Use cases

| Use case | Description                                                                                                     | Details            |
| -------- | --------------------------------------------------------------------------------------------------------------- | ------------------ |
| **Tips** | Send a tip via QR code or link — the recipient signs up and claims it, or the payer gets a refund after expiry. | [TIPS.md](TIPS.md) |

More use cases (disputes, instalment payments, multi-party escrow, ...) are planned — see the [future expansion](src/escrow/README.md#future-expansion) section.

## Scalability

The canister currently stores all deals in heap memory (~4–8 M deals). ICRC-7 has no built-in token-state sharding, so all deal NFTs live in one canister. A phased migration to `ic-stable-structures` (hundreds of millions of deals) and optional canister sharding (unbounded) is documented in the [scalability & limitations](src/escrow/README.md#scalability--limitations) section.

## Technical documentation

For the full API reference, deal lifecycle, module structure, and ICRC-7 NFT interface, see the [escrow canister README](src/escrow/README.md).
