# Escrow

A generic escrow engine on the [Internet Computer](https://internetcomputer.org/), built as a Rust canister.

Payers lock tokens into deal-specific subaccounts; recipients claim them before a deadline or the funds are automatically refunded. The engine is designed to support multiple use cases through the same core deal lifecycle.

## Use cases

| Use case | Description                                                                                                     | Details            |
| -------- | --------------------------------------------------------------------------------------------------------------- | ------------------ |
| **Tips** | Send a tip via QR code or link — the recipient signs up and claims it, or the payer gets a refund after expiry. | [TIPS.md](TIPS.md) |

More use cases (disputes, instalment payments, multi-party escrow, ...) are planned — see the [future expansion](src/escrow/README.md#future-expansion) section.

## Technical documentation

For the full API reference, deal lifecycle, and module structure, see the [escrow canister README](src/escrow/README.md).
