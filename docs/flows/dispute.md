# Dispute flow

A dispute opens on a `Funded` bound deal — manually via `open_dispute`, automatically when a sign-tally produces a mixed outcome, or automatically when expiry's auto-YES rule produces a mixed tally on an expired deal. A randomly-weighted panel of curated arbitrators votes; majority decides the outcome. Either party can also resolve out-of-band via matched `withdraw_dispute` proposals.

## How a dispute opens

```mermaid
flowchart LR
    F[Funded deal] --> Q{How?}
    Q -->|direct call by P or R| C[open_dispute]
    Q -->|sign mismatch pre-expiry| D[sign_yes × sign_no = Mixed<br/>→ services::deals::sign auto-opens]
    Q -->|expiry sweep, Mixed after auto-YES| X[expiry → open_post_expiry<br/>opener = the explicit No-signer]
    C --> R[Disputed<br/>opener's signature stamped to No if Empty]
    D --> R
    X --> R
```

## Lifecycle (panel-resolved)

```mermaid
sequenceDiagram
    autonumber
    participant D as Disputer (P or R)
    participant E as Escrow
    participant Panel as Arbitrator panel (N members, raw_rand-weighted)
    participant CP as Counterparty
    participant L as ICRC Ledger

    D->>E: open_dispute(deal_id)
    E->>E: select N arbitrators (eligible × score-weighted, raw_rand)
    Note over E: Disputed — Evidence phase
    Note over E: opener's signature stamped to No if Empty

    %% --- Evidence window (default 3 days) ---
    par Both sides may submit evidence
        D->>E: submit_evidence({ note, artefact_url, sha256 })
    and
        CP->>E: submit_evidence(...)
    and
        Panel->>E: submit_evidence(...)
    end

    %% --- Voting window (default 2 days, opens at evidence_deadline) ---
    Panel->>E: cast_vote(ConcludedCorrectly | IncorrectlyConcluded | Abstain)
    Note over E: Voting (latest-wins per arbitrator)

    %% --- Finalize: anyone (non-anon) after voting deadline; sweep does it automatically ---
    Note over E: voting_deadline_ns reached
    E->>E: tally_votes (quorum = floor(N/2)+1 non-abstain)

    alt Majority CC
        E->>L: split arbitration fee across non-abstain voters
        E->>L: transfer(remainder → R)
        Note over E: ArbitratedSettled
    else Majority IC
        E->>L: split arbitration fee across non-abstain voters
        E->>L: transfer(remainder → P)
        Note over E: ArbitratedRefunded
    else No quorum / tie
        E->>L: transfer(remainder → P) [arbitrators NOT paid]
        Note over E: ArbitratedRefunded
    end
```

## Out-of-band withdrawal

Both parties can independently propose an outcome during the Evidence phase. When both proposals match, the dispute resolves immediately and the panel receives a reduced fee (`withdraw_fee_pct`, default 25%) for the work already done.

```mermaid
sequenceDiagram
    autonumber
    participant P as Payer
    participant E as Escrow
    participant R as Recipient
    participant L as ICRC Ledger

    Note over E: Disputed (Evidence phase only)

    P->>E: withdraw_dispute({ proposal: Some(ConcludedCorrectly) })
    Note over E: payer_withdraw_proposal recorded
    R->>E: withdraw_dispute({ proposal: Some(ConcludedCorrectly) })
    Note over E: both proposals match → resolve

    E->>L: split (arbitration_fee × withdraw_fee_pct / 100) across panel
    E->>L: transfer(remainder → R) [matches CC outcome]
    Note over E: ArbitratedSettled (Withdrawn)
```

`withdraw_dispute({ proposal: None })` retracts a prior proposal. Mismatched proposals stay recorded silently — either side can amend until both align.

## Status path

```mermaid
stateDiagram-v2
    Funded --> Disputed: open_dispute / Mixed sign-tally / expiry Mixed
    Disputed --> ArbitratedSettled: majority CC OR Withdrawn(CC)
    Disputed --> ArbitratedRefunded: majority IC OR NoQuorum OR Withdrawn(IC)
```

## Endpoints

| Step                  | Endpoint                                                                              |
| --------------------- | ------------------------------------------------------------------------------------- | ------------------------------ |
| Open (manual)         | `open_dispute(deal_id)` — caller-as-`No` recorded                                     |
| Submit evidence       | `submit_evidence({ note?, artefact_url?, artefact_sha256? })` (party or panel member) |
| Vote (panel only)     | `cast_vote({ vote })` during the voting window; latest-wins                           |
| Force-finalize        | `finalize_dispute(dispute_id)` — anyone after `voting_deadline_ns`; sweep auto-runs   |
| Out-of-band agreement | `withdraw_dispute({ proposal: Some(vote)                                              | None })` — Evidence phase only |
| Public read           | `get_public_dispute(dispute_id)` — no party / panel principals, no evidence URLs      |

## Notes

- **Tip flows can't be disputed** — no bound counterparty. Returns `DisputeRequiresBoundRecipient`.
- **Per-deal panel size** can be locked at `create_deal` time via `panel_size: Some(n)`, bounded by `[DisputeConfig.min_panel_size, DisputeConfig.max_panel_size]`. The locked value survives subsequent `update_config` changes.
- **No-quorum ≠ tie**: both fall back to `ArbitratedRefunded` (status quo for the payer); arbitrators are NOT paid in either case.
- **Reliability scoring**: arbitrators in the majority on a CC/IC outcome get `+1 disputes_voted` and `+1 disputes_with_majority`. `NoQuorum` and `Withdrawn` outcomes only bump `disputes_assigned`.
- **Auto-finalize timer** runs every 5 minutes; per-dispute errors are swallowed so a single ledger blip doesn't block the whole sweep.
- **Design history**: see [RFC-001](../rfcs/0001-dispute-resolution.md) for the dispute-resolution design rationale.
