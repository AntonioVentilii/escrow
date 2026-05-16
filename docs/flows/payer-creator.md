# Payer-creator deal (3a)

Payer creates a bound deal with a known recipient and **deposits everything they owe at create time** — `amount + DC/2` to the deal subaccount and `creation_fee` to the canister-owned treasury subaccount. Recipient consents (which deposits _their_ `DC/2` and auto-flips status to `Funded`). Both parties then sign at settlement time.

There is no separate `fund_deal` step.

## Sequence

```mermaid
sequenceDiagram
    autonumber
    participant P as Payer (creator)
    participant E as Escrow
    participant L as ICRC Ledger
    participant T as Treasury subaccount
    participant R as Recipient

    %% --- Create (commit-at-first-action) ---
    P->>L: icrc2_approve(E, amount + DC/2 + creation_fee + 2*LF)
    P->>E: create_deal({ payer: P, recipient: R, amount, expiry })
    E->>L: transfer_from(P → escrow subaccount, amount + DC/2)
    E->>L: transfer_from(P → treasury subaccount, creation_fee)
    Note over T: treasury += creation_fee (forfeited, never refunded)
    E-->>P: deal_id (P consent = Accepted, R consent = Pending)
    Note over E: Created — payer has deposited everything they owe

    %% --- Recipient consent (deposits DC/2 + auto-flips to Funded) ---
    R->>L: icrc2_approve(E, DC/2 + LF)
    R->>E: consent_deal(deal_id)
    E->>L: transfer_from(R → escrow subaccount, DC/2)
    Note over E: both consents Accepted → status auto-flips to Funded

    %% --- Two-signature tally (happy path: both Yes) ---
    R->>E: sign_yes(deal_id)
    Note over E: recipient_signature = Yes, tally Pending → stays Funded
    Note over E: (R can also call accept_deal — routes to sign_yes for bound deals)
    P->>E: sign_yes(deal_id)
    Note over E: BothYes tally → settle
    E->>L: transfer(escrow → R, amount − EF + DC/2 − LF)
    E->>L: transfer(escrow → P, DC/2 − LF)
    Note over E: Settled
```

## Status path

```mermaid
stateDiagram-v2
    [*] --> Created: create_deal (P deposits amount + DC/2 + creation_fee)
    Created --> Cancelled: cancel_deal (either)
    Created --> Rejected: reject_deal (either)
    Created --> Funded: consent_deal (R deposits DC/2 → both Accepted)
    Funded --> Settled: BothYes tally
    Funded --> Aborted: BothNo tally
    Funded --> Disputed: Mixed tally (auto-open) OR explicit open_dispute
    Funded --> Settled: expiry + auto-YES (both Empty / one Yes one Empty)
    Funded --> Disputed: expiry + auto-YES (one No, one Empty)
    Funded --> Aborted: expiry + auto-YES (both No)
```

Note that `Funded` is now reached **at consent time** for bound deals (when the counterparty deposits and both consents are Accepted), not at a separate `fund_deal` step.

## Endpoints

| Step                          | Endpoint                                                                                   |
| ----------------------------- | ------------------------------------------------------------------------------------------ |
| Create + payer deposit        | `create_deal({ recipient: Some(R), … })` (pulls `amount + DC/2 + creation_fee` via ICRC-2) |
| Recipient consent + reserve   | `consent_deal(deal_id)` (pulls `DC/2` via ICRC-2; auto-flips to Funded)                    |
| Sign Yes                      | `sign_yes(deal_id)` (or `accept_deal` for the recipient)                                   |
| Sign No                       | `sign_no(deal_id)`                                                                         |
| Open dispute manually         | `open_dispute(deal_id)`                                                                    |
| Reclaim after expiry (P only) | `reclaim_deal(deal_id)` — routes through the same auto-YES tally                           |

## Tally outcomes

| `(payer_sig, recipient_sig)` | Status     | Money flow                                                 |
| ---------------------------- | ---------- | ---------------------------------------------------------- |
| `Yes` + `Yes`                | `Settled`  | R: `amount − EF + DC/2 − LF`; P: `DC/2 − LF`; sub keeps EF |
| `No` + `No`                  | `Aborted`  | P: `amount − EF + DC/2 − LF`; R: `DC/2 − LF`; sub keeps EF |
| `Yes` + `No` (or vice versa) | `Disputed` | Funds locked pending arbitration                           |
| `Empty` + anything           | `Funded`   | No movement; deal waits                                    |

`creation_fee` was deposited at create and lives in the treasury subaccount throughout the deal's lifetime — every terminal leaves it untouched.

`Aborted` and `Refunded` use **identical** fee math — the difference is the audit trail (mutual `No` vs expiry on a tip).

## Cancel / reject before consent

If the payer cancels (or the recipient rejects) BEFORE the recipient consents, the payer's create-time deposit (`amount + DC/2`) is refunded back to them minus one outgoing ledger fee. The `creation_fee` is already in the treasury and stays there (forfeited by design — it's the cost of having created a deal that consumed system resources).

## At expiry

The 5-min housekeeping sweep (or a manual `reclaim_deal`) applies the auto-YES rule, then re-runs the tally:

| Before expiry           | After auto-YES | Outcome    |
| ----------------------- | -------------- | ---------- |
| both `Empty`            | `Yes` + `Yes`  | `Settled`  |
| one `Yes` + one `Empty` | `Yes` + `Yes`  | `Settled`  |
| one `No` + one `Empty`  | `No` + `Yes`   | `Disputed` |
| both `No`               | (unchanged)    | `Aborted`  |
