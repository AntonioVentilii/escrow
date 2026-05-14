# RFC-002 — Symmetric escrow fees + dispute reserve

| Field   | Value                                                                                                                                       |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Author  | @antonioventilii                                                                                                                            |
| Created | 2026-05-14                                                                                                                                  |
| Status  | Proposed                                                                                                                                    |
| Targets | escrow `v0.1.x` (next minor — adds Candid types + a `Config` field; backward-compatible deserialisation for existing `v0.0.5` stable state) |
| Related | [RFC-001](./0001-dispute-resolution.md) (dispute resolution + arbitrators)                                                                  |

> **What is this RFC?** A design proposal that has to be agreed before
> any implementation lands. Each "Open question" in the document is a
> decision the implementation depends on. Once accepted, the
> implementation is split across multiple atomic PRs — see
> [Implementation plan](#implementation-plan).

---

## Table of contents

1. [Problem statement](#problem-statement)
2. [Goals & non-goals](#goals--non-goals)
3. [Proposed types](#proposed-types)
4. [Proposed flow — bound deals](#proposed-flow--bound-deals)
5. [Proposed Candid surface](#proposed-candid-surface)
6. [Decision log](#decision-log)
7. [Implementation plan](#implementation-plan)
8. [Migration](#migration)
9. [Out of scope](#out-of-scope)

---

## Problem statement

The current canister has three fee-related defects that compound:

1. **Settle / refund ledger fee under-funding** _(latent bug)_.
   `execute_fund` calls `icrc2_transfer_from(payer → escrow_sub, amount = deal.amount)`,
   so the escrow subaccount ends with exactly `deal.amount`. Both
   `execute_accept` and `execute_reclaim` (and `expiry::try_refund_deal`) then call
   `icrc1_transfer(escrow_sub → recipient_or_payer, amount = deal.amount)`,
   which under ICRC-1 semantics debits the subaccount by `amount + ledger_fee`. The
   subaccount is `ledger_fee` short → the transfer fails with `InsufficientFunds`. The
   dispute finalize path already handles this correctly (subtracts `ledger_fee` from
   the prevailing-party payout in `services/disputes.rs::withdraw_finalize_locked`);
   the happy path does not.

2. **Asymmetric dispute cost**. Today the arbitration fee comes out
   of `deal.amount` only when `open_dispute` fires; whoever opens the
   dispute and the deal's nominal recipient are exposed to a haircut
   they didn't agree to upfront. There is no symmetry — the payer's
   `deal.amount` carries the entire dispute economics. The receiver
   only sees the cost if the dispute resolves in their favour and
   their payout is reduced.

3. **No operator revenue**. The escrow canister currently keeps `0`
   on every terminal state. There is no "service fee" knob — every
   ledger fee burned is also a real cost the operator absorbs in
   cycles + ledger fees they implicitly pay (canister-owned
   subaccount). For a production deployment we need a configurable
   service fee that is charged on every terminal state.

This RFC fixes all three with a single mechanism: **a per-deal fee
snapshot captured at `create_deal` time**, locking in:

- The escrow service fee (`EF`) the operator collects on any terminal state.
- The per-party dispute reserve (`DC/2`) both parties pre-commit and which is fully
  refunded on happy paths, or fans out to the arbitrator panel on disputes.
- The reduced-fee percentage on out-of-band withdrawals (`withdraw_fee_pct` snapshot).
- The ledger fee at create time (snapshot for audit only — arithmetic always re-queries
  the live `icrc1_fee`).

The snapshot pattern is identical to the precedent set by `Deal.panel_size`
in [RFC-001 Q6 revisit](./0001-dispute-resolution.md): the deal terms are
a contract at create time; subsequent admin `update_config` changes do not
retroactively alter in-flight deals.

## Goals & non-goals

### In scope

- A new `Deal.fees: Option<DealFees>` snapshot field carrying every fee
  the canister will charge over the deal's lifetime.
- A new `Config.escrow_fee: Option<u128>` config field, admin-tunable
  via `update_config`. Default = `2 × ICP_LEDGER_FEE` = `20_000` e8s.
- A `validate_create` minimum-amount check that rejects deals whose
  `amount` cannot cover `EF + ledger_fee × N + DC + ε`.
- Fix the settle/refund/expiry under-funding: every outgoing
  `icrc1_transfer` from the escrow subaccount must account for the
  ledger fee in the amount it sends, OR the escrow subaccount must be
  pre-funded with the headroom. We pick the latter (pre-fund) so the
  recipient receives exactly `amount − EF`.
- A two-sided dispute reserve: both payer and receiver commit `DC/2`
  before the deal can be `Funded`. On happy paths each gets their
  `DC/2 − ledger_fee` back; on disputes the full `DC` is consumed by
  the arbitrator panel.
- Snapshot every fee in `DealFees` so subsequent `update_config` calls
  cannot retroactively change the economics of an in-flight deal.

### Out of scope

- Per-token escrow-fee overrides (everything denominated in the deal's
  token; `Config.escrow_fee` is a single value used for any token —
  callers operating with non-ICP tokens may need to adjust).
- Frontend coordination (the PandaMe repo will be updated in a
  follow-up PR per the cross-repo coordination rule in
  [`docs/ai/pr-and-ci.md`](../ai/pr-and-ci.md)).
- A platform-wide treasury / sweeper — `EF` accumulates in the
  canister's main account (subaccount `None`) and is left there for
  controllers to sweep manually. A dedicated treasury mechanism is a
  separate concern.
- Per-outcome fee rates (settled vs rejected vs expired) — per
  [Q2](#q2-penalty-vs-escrow-fee), v1 uses a single `EF` for every
  terminal state. Differentiated rates can be added later by
  expanding `Config.escrow_fee` from `u128` to a struct.

## Proposed types

### `DealFees` (new, in `types/deal.rs`)

```rust
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct DealFees {
    /// Escrow service fee in the deal's token. Charged on EVERY
    /// terminal state (`Settled`, `Refunded`, `Cancelled`, `Rejected`,
    /// `ArbitratedSettled`, `ArbitratedRefunded`). Snapshot of
    /// `Config.escrow_fee` at `create_deal` time. Locked — subsequent
    /// `update_config` does not retroactively alter this.
    pub escrow_fee: u128,

    /// Per-party dispute reserve. Each party deposits this amount
    /// before the deal can be `Funded`:
    ///   - Payer: deposited inside `fund_deal` (together with `amount`)
    ///   - Receiver: deposited inside `consent_deal` (Q1 option A)
    /// Refunded `dispute_reserve_per_party − ledger_fee` to each party
    /// on happy-path terminal states; consumed (full `2 ×
    /// dispute_reserve_per_party = DC`) by the arbitrator panel on
    /// `Disputed → ArbitratedX`.
    /// Snapshot of `compute_arbitration_fee(amount, DisputeConfig) / 2`
    /// at create time.
    pub dispute_reserve_per_party: u128,

    /// Reduced-fee percentage the arbitrator panel receives when
    /// the parties resolve out-of-band via `withdraw_dispute`.
    /// Snapshot of `DisputeConfig.withdraw_fee_pct` at create time.
    pub withdraw_fee_pct: u32,

    /// Ledger `icrc1_fee` value at create time. RECORD ONLY — never
    /// used for arithmetic. Every actual transfer re-queries the
    /// live fee via `ledger::fee`. Snapshotted so the audit trail
    /// can answer "what was the user shown at create time?" even
    /// if the ledger later changes its fee. Operator absorbs any
    /// drift between create-time `icrc1_fee` and runtime
    /// `icrc1_fee` out of `escrow_fee`.
    pub ledger_fee_at_create: u128,
}
```

### `Config.escrow_fee` (additive on `types/state.rs`)

```rust
pub struct Config {
    pub dispute_config: Option<DisputeConfig>,
    /// Per-deal escrow service fee, in the deal's token. Charged on
    /// every terminal state. Defaults to `DEFAULT_ESCROW_FEE`
    /// (`20_000` e8s = `2 × ICP_LEDGER_FEE`) via the manual
    /// `Default for Config` impl. Snapshotted into each
    /// `Deal.fees.escrow_fee` at `create_deal` time; subsequent
    /// `update_config` changes do not retroactively alter
    /// in-flight deals.
    pub escrow_fee: u128,
}
```

### `Deal.fees` (additive on `types/deal.rs`)

```rust
pub struct Deal {
    // ... existing fields ...
    /// Fee snapshot taken at `create_deal` time.
    pub fees: DealFees,
}
```

Required (not `Option`-wrapped) per the [Migration](#migration)
section: dev-mode canister, reinstalled from scratch.

### New `EscrowError` variants (in `api/deals/errors.rs`)

```rust
/// `create_deal` was called with an `amount` too small to cover
/// the escrow fee + ledger fees + dispute reserve. The `min`
/// field surfaces the calculated floor so the caller can render
/// the rejection.
AmountBelowMinimum { min: u128 },

/// `consent_deal` was called by a receiver who has not approved
/// the canister to pull their `DC/2`, OR the actual
/// `transfer_from` failed. The deal stays `Created` so the
/// receiver can retry after approving.
DisputeReserveRequired,
```

## Proposed flow — bound deals

The flow below covers shape #3 from
[`src/escrow/README.md#deal-lifecycle`](../../src/escrow/README.md#deal-lifecycle)
(bound deal — both parties known) for both sub-cases. The numbers
referenced in the tables are in the symbols defined in
[Q3](#q3-minimum-viable-amount).

### Case 3a — Payer-creator

1. **Create** — payer calls `create_deal({ payer = self, recipient = R, amount = A })`.
   Canister snapshots `DealFees`, allocates a subaccount, status =
   `Created`, `payer_consent = Accepted`, `recipient_consent = Pending`.
   No ledger calls. No funds move.

2. **Consent** — receiver `R` calls `consent_deal(deal_id)`. Canister
   does `icrc2_transfer_from(R → escrow_sub, DC/2)`. Receiver must
   have approved `DC/2 + ledger_fee` beforehand. On success:
   `recipient_consent = Accepted`, status stays `Created`. On failure
   (`InsufficientAllowance` / `InsufficientFunds`): error is bubbled
   as `DisputeReserveRequired`, no state change.
   - Alternative: receiver calls `reject_deal(deal_id)` → status =
     `Rejected` (terminal), nothing moved, no fees.

3. **Fund** — payer calls `fund_deal(deal_id)`. Canister does
   `icrc2_transfer_from(payer → escrow_sub, A + DC/2 + ledger_fee)`
   so the subaccount ends with `A + DC + ledger_fee` (enough to
   cover the outgoing transfer on accept). Payer must have approved
   `A + DC/2 + 2 × ledger_fee` beforehand. Status → `Funded`.

4. **Accept** — receiver `R` calls `accept_deal(deal_id)`. Canister
   fans out 3 transfers from `escrow_sub`:
   - To recipient `R`: `A − EF` (single transfer combining settlement + DC/2 refund? — see [Q5](#q5-accept-transfer-fan-out))
   - To payer: `DC/2 − ledger_fee`
   - Refund of receiver's DC/2: bundled into the recipient transfer per Q5
     Status → `Settled`. Canister retains `EF` in its main account.

### Case 3b — Receiver-creator

1. **Create** — receiver calls `create_deal({ payer = P, recipient = self, amount = A })`.
   Same snapshot + subaccount, status = `Created`,
   `recipient_consent = Accepted`, `payer_consent = Pending`. **Plus**
   the canister does `icrc2_transfer_from(receiver → escrow_sub, DC/2)`
   in the same call. Receiver must have approved `DC/2 + ledger_fee`
   beforehand. If the transfer fails the deal is not created
   (atomic with the create).

2. **Consent** — payer `P` calls `consent_deal(deal_id)`. No money
   movement on this step in 3b — the payer's reserve is deposited
   together with the deal amount in step 3. Sets `payer_consent =
Accepted`. (Step is optional; `fund_deal` auto-sets the consent.)

3. **Fund** — payer calls `fund_deal(deal_id)`. Same as 3a step 3 —
   `icrc2_transfer_from(payer → escrow_sub, A + DC/2 + ledger_fee)`.

4. **Accept** — same as 3a step 4.

### Terminal states + fee accounting

| Terminal state                       | Recipient receives                                   | Payer receives                          | Receiver reserve refund                                          | Escrow operator keeps                     |
| ------------------------------------ | ---------------------------------------------------- | --------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------- |
| `Settled`                            | `A − EF − ledger_fee` (combined with reserve refund) | `DC/2 − ledger_fee`                     | included in recipient transfer                                   | `EF`                                      |
| `Refunded` (expiry / `reclaim_deal`) | —                                                    | `A + DC/2 − EF − ledger_fee` (combined) | `DC/2 − ledger_fee`                                              | `EF`                                      |
| `Cancelled` (before fund)            | —                                                    | — (nothing in escrow if no consent yet) | `DC/2 − ledger_fee` if receiver had consented                    | `EF` if receiver had consented, else `0`  |
| `Rejected`                           | —                                                    | (payer hadn't funded yet)               | `DC/2 − ledger_fee` if rejector is receiver and they'd consented | `EF` if any reserve was on hand, else `0` |
| `ArbitratedSettled`                  | `A − DC − EF − ledger_fee`                           | reserved for the dispute panel          | —                                                                | `EF` + arbitrator panel gets `DC`         |
| `ArbitratedRefunded`                 | reserved for the dispute panel                       | `A − DC − EF − ledger_fee`              | —                                                                | `EF` + arbitrator panel gets `DC`         |

> **Note:** in `Cancelled` / `Rejected` cases where the rejector
> already had their `DC/2` on deposit, they still get it back
> (minus `ledger_fee`). The "no penalty above EF" rule is
> per [Q2](#q2-penalty-vs-escrow-fee).

## Proposed Candid surface

All changes are **additive** — existing fields stay in place with
the same types. No client breakage.

```diff
 type Config = record {
   dispute_config : opt DisputeConfig;
+  escrow_fee : nat;
 };

+type DealFees = record {
+  escrow_fee : nat;
+  dispute_reserve_per_party : nat;
+  withdraw_fee_pct : nat32;
+  ledger_fee_at_create : nat;
+};

 type Deal = record {
   // ... existing fields ...
+  fees : DealFees;
 };

 type DealView = record {
   // ... existing fields ...
+  fees : DealFees;
 };

 type EscrowError = variant {
   // ... existing variants ...
+  AmountBelowMinimum : record { min : nat };
+  DisputeReserveRequired;
 };
```

`consent_deal` becomes an `async` update method that performs an
ICRC-2 `transfer_from` against the caller. The Candid signature
doesn't change — only the runtime behaviour.

## Decision log

### Q1 — When does the receiver deposit `DC/2`?

**Options considered:** at consent / at accept / new "engage" step.

**Decision: at `consent_deal`** (option A). `consent_deal` becomes
an `async` update method that performs `icrc2_transfer_from(receiver
→ escrow_sub, DC/2)`. Rationale:

- Consent stops being free — it becomes the receiver's
  capital commitment.
- The dispute window (between `Funded` and `Settled`) always has
  the full `DC` available, since both halves are committed before
  the deal can transition to `Funded`.
- Avoids inventing a new `Engaged` lifecycle state.

In case 3b (receiver-creator), the receiver's reserve is deposited
inside `create_deal` itself — same approval-then-transfer-from
pattern, atomic with creation.

### Q2 — Penalty vs escrow fee

**Options considered:** separate `PF` and `EF` / unified.

**Decision: unified.** Single `EF` charged on every terminal state.
The "penalty fee" framing is dropped entirely; an early rejection
costs the same as a successful settlement. Rationale:

- Cleaner economic model — one knob to tune, easier to explain
  to users ("you used the escrow, you pay").
- Removes the perverse incentive to keep the deal "open" purely
  to avoid penalty.
- v1-appropriate complexity. If rejection-abuse becomes a real
  problem we can expand `Config.escrow_fee: Option<u128>` into a
  struct with per-outcome rates without breaking the Candid
  surface (additive `Config` struct change).

### Q3 — Minimum viable amount

**Decision:** `validate_create` rejects deals where
`amount ≤ EF + 2 × ledger_fee + DC + ε`.

The actual check:

```
let DC = compute_arbitration_fee(amount, dispute_config);
let min = EF
    + 2 * ledger_fee_at_create   // settle/refund + dispute panel fee
    + DC                          // potential full panel consumption
    + 1;                          // > 0 leftover for recipient
if amount <= min {
    return Err(AmountBelowMinimum { min });
}
```

Mirrors the existing `AmountTooSmallForArbitration` check in
`services/disputes.rs::open_dispute_async` but lifts it to create
time, where it can also reject sub-`EF` deals before any
ledger interaction happens.

### Q4 — Expiry handling

**Options considered:** rejection-like / auto-refund-with-fee /
auto-settle.

**Decision: auto-refund minus `EF`.** Expiry is a passive timeout;
both parties walked away from the engagement. The escrow charges
its standard `EF` and refunds the rest, including both `DC/2`
halves to their respective owners. Rationale:

- Neither party actively defected → no asymmetric penalty.
- Operator still gets paid for the work + storage during the deal's
  lifetime.
- "Auto-settle on expiry" can be added later as an opt-in per-deal
  flag if a use-case demands it.

### Q5 — Accept transfer fan-out

How many ICRC-1 transfers does `execute_accept` issue?

**Options:**

- A: 3 transfers — recipient (`A − EF − ledger_fee`), payer-reserve-refund (`DC/2 − ledger_fee`), receiver-reserve-refund (`DC/2 − ledger_fee`). 3 ledger fees burned.
- B: 2 transfers — recipient receives `A − EF − ledger_fee + DC/2` combined, payer-reserve refunded separately. 2 ledger fees burned.

**Decision: B (combined).** Receiver receives one transfer of `A − EF + DC/2 − ledger_fee`
(their settlement + their own reserve refund, minus one ledger fee
for the actual transfer). Payer gets their reserve back in a separate
transfer of `DC/2 − ledger_fee`. Saves one ledger-fee burn per happy
path. Bookkeeping-wise the receiver "absorbs" two notional fees
(`EF` and the LF on the combined transfer), but only one real LF is
charged to the ledger.

### Q6 — Snapshot drift between create and runtime

If `Config.escrow_fee` changes after `create_deal` but before
terminal state, which value applies?

**Decision: the snapshot.** `Deal.fees.escrow_fee` is read at every
terminal site (`execute_accept`, `execute_reclaim`,
`expiry::try_refund_deal`, `services::disputes::finalize`,
`services::disputes::withdraw_finalize_locked`). Same rule as
RFC-001 Q6 for `panel_size`: terms are a contract at create.

Live ledger fee (`icrc1_fee` queried at transfer time) is the
**only** value that is NOT snapshot-locked, because the canister
cannot dictate the external ledger's fee. Operator absorbs any drift
out of `EF`.

## Implementation plan

This RFC accepts that the implementation lands across multiple PRs
referencing RFC-002 in their commit body. Recommended split:

| #               | Title                                                                   | Scope                                                                                                                                                                                                                                                                                                                                                      |
| --------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **1 (this PR)** | `feat(fees): RFC-002 foundation — escrow fee + DealFees snapshot`       | RFC document; `DealFees` + `Config.escrow_fee` types; `validate_create` min-amount check; snapshot on create; settle/refund/expiry fee accounting (the bug fix); legacy fallback for pre-snapshot deals; updated unit tests; Candid regen; docs. **Does not** change `consent_deal` to async or introduce the two-sided reserve flow — those gate on PR-2. |
| 2               | `feat(fees): two-sided dispute reserve via consent_deal (RFC-002)`      | `consent_deal` becomes async + ICRC-2 flow. `fund_deal` pulls `A + DC/2 + LF`. `accept_deal` fans out per Q5. Receiver-creator deposits DC/2 inside `create_deal`. New error variants. Integration tests with a real ledger.                                                                                                                               |
| 3               | `feat(disputes): consume deal.fees.dispute_reserve_per_party (RFC-002)` | Dispute open/finalize/withdraw read `DC` from snapshot. Removes the runtime `compute_arbitration_fee` call at open time.                                                                                                                                                                                                                                   |
| 4               | `feat(pandame): UI for symmetric escrow fees (RFC-002)`                 | In `../pandame/` — `createAndFundDeal` approves `A + DC/2 + 2*LF`. `consentDeal` does approve + canister call. `DealView.fees` rendered in deal cards.                                                                                                                                                                                                     |

## Migration

This canister is in active development (pre-v0.1.0). The staging
canister `umxj5-niaaa-aaaae-af2sq-cai` is reinstalled from scratch
on every deploy and holds no production-meaningful state, so this
RFC drops backward-compat for legacy snapshots entirely:

- `Deal.fees: DealFees` is required (not `Option`-wrapped). Any
  pre-snapshot stable state that lacks the field fails to
  deserialise on `post_upgrade` — by design.
- `Config.escrow_fee: u128` is required, with a `Default` impl
  that supplies `DEFAULT_ESCROW_FEE` on fresh deployments.

Once the canister reaches v1.0 and accumulates real deals, future
schema changes will need to add new fields as `Option<T>` for
backward-compat (the existing precedent on `StableState.deals` /
`StableState.disputes` and `Deal.dispute` already demonstrates this
pattern). The current required-field choice is a "dev-mode now,
add `Option` wrappers when there's anything to migrate" stance.

## Out of scope

- Per-token `escrow_fee` (one knob serves all tokens; non-ICP tokens
  may need a follow-up).
- A treasury / sweeper canister to collect `EF` into an external account.
- Per-outcome fee rates (e.g. higher `EF` on rejection); deferred per Q2.
- Frontend changes (`../pandame/`) — separate PR per the cross-repo
  coordination rule in [`docs/ai/pr-and-ci.md`](../ai/pr-and-ci.md).
- Auto-settle on expiry (per-deal opt-in flag); deferred per Q4.
