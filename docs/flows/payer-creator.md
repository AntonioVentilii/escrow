# Payer-creator deal (3a)

Payer creates a bound deal with a known recipient. Recipient consents (depositing their dispute reserve), payer funds, then both parties sign at settlement time.

## Sequence

```mermaid
sequenceDiagram
    autonumber
    participant P as Payer (creator)
    participant E as Escrow
    participant L as ICRC Ledger
    participant R as Recipient

    %% --- Create ---
    P->>E: create_deal({ payer: P, recipient: R, amount, expiry })
    E-->>P: deal_id (P consent = Accepted, R consent = Pending)
    Note over E: Created

    %% --- Recipient consent (deposits DC/2) ---
    R->>L: icrc2_approve(E, DC/2 + LF)
    R->>E: consent_deal(deal_id)
    E->>L: transfer_from(R → escrow subaccount)
    Note over E: Still Created — R consent Accepted, R reserve in subaccount

    %% --- Payer fund ---
    P->>L: icrc2_approve(E, amount + DC/2 + LF)
    P->>E: fund_deal(deal_id)
    E->>L: transfer_from(P → escrow subaccount)
    Note over E: Funded — subaccount holds amount + DC, both signatures Empty

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
    [*] --> Created: create_deal
    Created --> Created: consent_deal (R deposits DC/2)
    Created --> Cancelled: cancel_deal (either)
    Created --> Rejected: reject_deal (either)
    Created --> Funded: fund_deal (P deposits amount + DC/2)
    Funded --> Settled: BothYes tally
    Funded --> Aborted: BothNo tally
    Funded --> Disputed: Mixed tally (auto-open) OR explicit open_dispute
    Funded --> Settled: expiry + auto-YES (both Empty / one Yes one Empty)
    Funded --> Disputed: expiry + auto-YES (one No, one Empty)
    Funded --> Aborted: expiry + auto-YES (both No)
```

## Endpoints

| Step                          | Endpoint                                                         |
| ----------------------------- | ---------------------------------------------------------------- |
| Create                        | `create_deal({ recipient: Some(R), … })`                         |
| Recipient consent + reserve   | `consent_deal(deal_id)` (pulls `DC/2` via ICRC-2)                |
| Fund                          | `fund_deal(deal_id)` (pulls `amount + DC/2` via ICRC-2)          |
| Sign Yes                      | `sign_yes(deal_id)` (or `accept_deal` for the recipient)         |
| Sign No                       | `sign_no(deal_id)`                                               |
| Open dispute manually         | `open_dispute(deal_id)`                                          |
| Reclaim after expiry (P only) | `reclaim_deal(deal_id)` — routes through the same auto-YES tally |

## Tally outcomes

| `(payer_sig, recipient_sig)` | Status     | Money flow                                                 |
| ---------------------------- | ---------- | ---------------------------------------------------------- |
| `Yes` + `Yes`                | `Settled`  | R: `amount − EF + DC/2 − LF`; P: `DC/2 − LF`; sub keeps EF |
| `No` + `No`                  | `Aborted`  | P: `amount − EF + DC/2 − LF`; R: `DC/2 − LF`; sub keeps EF |
| `Yes` + `No` (or vice versa) | `Disputed` | Funds locked pending arbitration                           |
| `Empty` + anything           | `Funded`   | No movement; deal waits                                    |

`Aborted` and `Refunded` use **identical** fee math — the difference is the audit trail (mutual `No` vs expiry on a tip).

## At expiry

The 5-min housekeeping sweep (or a manual `reclaim_deal`) applies the auto-YES rule, then re-runs the tally:

| Before expiry           | After auto-YES | Outcome    |
| ----------------------- | -------------- | ---------- |
| both `Empty`            | `Yes` + `Yes`  | `Settled`  |
| one `Yes` + one `Empty` | `Yes` + `Yes`  | `Settled`  |
| one `No` + one `Empty`  | `No` + `Yes`   | `Disputed` |
| both `No`               | (unchanged)    | `Aborted`  |
