# RFC-001 — Dispute resolution + arbitrators

| Field   | Value                                                                                                                                         |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Status  | **Draft** — open for comment                                                                                                                  |
| Author  | @antonioventilii                                                                                                                              |
| Created | 2026-05-09                                                                                                                                    |
| Targets | escrow `v0.1.x` (next minor — adds Candid types but no on-canister data migration; existing `Deal` records deserialise with `dispute = None`) |
| Related | `Pluggable resolvers` sketch in `src/escrow/README.md`                                                                                        |

> **What is this RFC?** A design proposal that has to be agreed before
> any implementation lands. Each "Open question" in the document is a
> decision the implementation depends on. Comments on this PR are the
> conversation surface — questions become decisions in the same PR.
> Once accepted, the implementation is split across multiple atomic
> PRs that each reference RFC-001 in their commit body.

---

## Table of contents

1. [Problem statement](#problem-statement)
2. [Goals & non-goals](#goals--non-goals)
3. [Proposed state machine](#proposed-state-machine)
4. [Proposed types](#proposed-types)
5. [Proposed Candid surface](#proposed-candid-surface)
6. [Open questions](#open-questions)
7. [Implementation plan](#implementation-plan)
8. [Alternatives considered](#alternatives-considered)
9. [Out of scope](#out-of-scope)

---

## Problem statement

The current escrow only resolves a `Funded` deal one of two ways:

- **Recipient claims** before expiry → `Settled`, funds released to
  recipient.
- **Recipient does not claim** before expiry → `Refunded`, funds
  returned to payer (auto-sweep + manual fallback).

This is sufficient for the **tip flow** (small amounts, no real
counterparty negotiation) but inadequate for the **two-party deal
flow**: a real-life asset exchange where one party believes the
counterparty isn't going to deliver. Examples: payer funded a deal
expecting a physical good and the recipient stalls; recipient is
about to claim but the payer disputes the delivery beforehand;
recipient claimed funds and the payer believes the asset wasn't
delivered (a post-claim challenge). Today both parties have only
one lever: refuse to consent / refuse to claim. There is no way to
**arbitrate** a contested delivery.

The v1 scope this RFC pins down is the first of those — disputes
opened **while the deal is `Funded`** (before the recipient claims).
Whether to also support post-claim challenges (a "settlement
challenge window") is open — see [Q2](#q2-who-can-open-a-dispute).

Adding dispute resolution unlocks the v1 product spec
(`old_escrow/Product.docx`'s "Evaluation" section) and is the first
concrete instance of the [`Pluggable resolvers` sketch](../../src/escrow/README.md#future-expansion)
in the canister README.

## Goals & non-goals

### In scope (v1 of dispute resolution)

- Either party of a **Funded** deal with a known recipient can open a
  dispute before expiry.
- Both parties can submit evidence within a fixed evidence window.
  Evidence is a free-form text note plus a reference to an
  off-canister artefact (URL + SHA-256 hash); binary blobs are
  **not** stored on-canister in v1 — see
  [Q8](#q8-evidence-storage).
- A randomly-selected panel of arbitrators reviews the evidence and
  votes for one of two outcomes: **CC** (Concluded Correctly →
  release to recipient) or **IC** (Incorrectly Concluded → refund to
  payer).
- A simple-majority vote (50% + 1) decides the outcome. The deal
  transitions to `ArbitratedSettled` or `ArbitratedRefunded`
  accordingly.
- Arbitrators are paid a fee on resolution, sourced from a deal-level
  arbitration fee deducted from the disputed amount.
- The reliability score (`api/reliability/`) gains a separate
  arbitrator-side score so a future reputation-weighted vote model
  can be plugged in without a schema migration.

### In scope, but the v1-vs-v2 split is open

- **Reputation-weighted voting** — `Product.docx` flags it as
  _"(TO BE DISCUSSED)"_. See [Q13](#q13-weighted-voting-now-or-v2).
- **Competency / subject-matter tags** — `Product.docx`'s alternative
  weight (_"history of correct decision … or the competency"_). See
  [Q14](#q14-competency--subject-matter-tags).

### Out of scope (deferred to later RFCs or future work)

- Tip flow (open-recipient deals): can't dispute since there's no
  bound counterparty until claim.
- Multi-round arbitration / appeals.
- Cross-chain / fiat evidence.
- Insurance / indemnity for slashed arbitrators.
- Disputes on `Created` deals (no funds at risk yet — use `cancel` /
  `reject` instead).

## Proposed state machine

```
Created ──[both consent]──▶ Created ──fund──▶ Funded ──accept──▶ Settled
  │                           │                 │ │
  │ reject                    │ cancel          │ │ open_dispute
  ▼                           ▼                 │ ▼
Rejected                  Cancelled             │ Disputed
                                                │   ├─[majority CC]─▶ ArbitratedSettled
                                                │   ├─[majority IC]─▶ ArbitratedRefunded
                                                │   └─[no quorum]──▶ ??? (see Q9)
                                                │
                                                │ reclaim (after expiry, if not Disputed)
                                                ▼
                                            Refunded
```

New variants on `DealStatus`:

- `Disputed` — dispute open, evidence and voting in progress.
- `ArbitratedSettled` — distinct from `Settled` so call sites can
  treat "settled by arbitration" differently in UX (e.g. show vote
  tally).
- `ArbitratedRefunded` — distinct from `Refunded` for the same
  reason.

`ArbitratedSettled`, `ArbitratedRefunded` are terminal.

> **Note on `Settled` / `Refunded` distinction.** An alternative is to
> reuse `Settled` / `Refunded` and add a separate `resolution: Option<Resolution>`
> field. Trade-off discussed in [Q1](#q1-distinct-statuses-vs-resolution-field).

## Proposed types

In `src/escrow/src/types/dispute.rs` (new module):

```rust
use candid::{CandidType, Deserialize, Principal};
use crate::types::deal::DealId;

/// New ID type, mirrors `DealId`. Allocated via `memory::insert_new_dispute`.
pub type DisputeId = u64;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DisputePhase {
    /// Evidence submission window. Both parties + (optionally) arbitrators
    /// can post evidence. Voting is closed.
    Evidence,
    /// Voting window. Evidence is frozen, arbitrators cast their votes.
    Voting,
    /// Tally finalised. Outcome propagated to the parent Deal. Terminal.
    Resolved,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Vote {
    /// "Concluded Correctly" — release funds to recipient.
    ConcludedCorrectly,
    /// "Incorrectly Concluded" — refund payer.
    IncorrectlyConcluded,
    /// Arbitrator abstained; counts toward the "no-vote" tally.
    Abstain,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Evidence {
    pub submitter: Principal,
    pub submitted_at_ns: u64,
    /// Free-form note (max 4 KiB — `EscrowError::EvidenceTooLarge` if exceeded).
    pub note: Option<String>,
    /// Off-canister artefact URL. On-canister blob storage is out of
    /// scope for v1 (see Q8).
    pub artefact_url: Option<String>,
    /// SHA-256 of the off-canister artefact. Always exactly 32 bytes
    /// when `Some`; the validator rejects other lengths
    /// (`EscrowError::ValidationError("artefact_sha256 must be 32 bytes")`).
    /// We use `Vec<u8>` rather than `[u8; 32]` because Candid emits
    /// fixed-size arrays as records of indexed fields, which is
    /// awkward in TS / didc — the length invariant is enforced at the
    /// canister boundary instead.
    pub artefact_sha256: Option<Vec<u8>>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Dispute {
    pub id: DisputeId,
    pub deal_id: DealId,
    pub opened_by: Principal,
    pub opened_at_ns: u64,
    pub phase: DisputePhase,
    /// Phase deadlines, set on phase transition.
    pub evidence_deadline_ns: u64,
    pub voting_deadline_ns: u64,
    /// Arbitrators selected for this dispute (from the registered pool).
    pub arbitrators: Vec<Principal>,
    /// Votes keyed by arbitrator principal. Missing keys = not yet voted.
    pub votes: std::collections::BTreeMap<Principal, Vote>,
    /// Evidence submissions, in submission order.
    pub evidence: Vec<Evidence>,
    /// Arbitration fee in the deal's token, deducted from the disputed
    /// amount on resolution. Split equally among arbitrators who voted
    /// (excluding `Abstain`).
    pub arbitration_fee: u128,
    /// Tally + outcome, set on resolution. None until `Resolved`.
    pub outcome: Option<DisputeOutcome>,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DisputeOutcome {
    /// Majority voted CC (Concluded Correctly) — funds released to recipient.
    Settled { cc: u32, ic: u32, abstain: u32 },
    /// Majority voted IC (Incorrectly Concluded) — funds refunded to payer.
    Refunded { cc: u32, ic: u32, abstain: u32 },
    /// Voting deadline reached without enough non-abstain votes. See Q9
    /// for the fallback behaviour.
    NoQuorum { cc: u32, ic: u32, abstain: u32 },
}
```

In `src/escrow/src/types/arbitrator.rs` (new module):

```rust
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct ArbitratorProfile {
    pub principal: Principal,
    pub registered_at_ns: u64,
    /// Plain-text introduction (max 1 KiB).
    pub bio: Option<String>,
    /// Total disputes the arbitrator was selected for.
    pub disputes_assigned: u32,
    /// Disputes the arbitrator submitted a non-abstain vote on.
    pub disputes_voted: u32,
    /// Disputes the arbitrator voted with the eventual majority.
    pub disputes_with_majority: u32,
    /// 0–100 reliability score. None until `disputes_voted >= MIN_VOTES_FOR_SCORE`.
    pub score: Option<u32>,
    pub status: ArbitratorStatus,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ArbitratorStatus {
    Active,
    Suspended,        // can't be selected; can finish current disputes
    Deregistered,     // permanent
}
```

Extension to `Deal` (`src/escrow/src/types/deal.rs`):

```rust
pub struct Deal {
    // … existing fields …

    /// `Some(dispute_id)` while the dispute is open or after resolution.
    /// `None` for deals that never went into dispute.
    pub dispute: Option<DisputeId>,
}
```

> **Backward-compat note.** `Option<DisputeId>` is a new field on
> `Deal`. Pre-RFC deals deserialise with `dispute = None` (Candid's
> `Option` semantics). No data migration is required.

## Proposed Candid surface

New endpoints in `api/disputes/api.rs`:

```candid
// Open a new dispute on a Funded deal. Caller must be payer or
// recipient. Funds remain escrowed. Deal transitions Funded → Disputed.
open_dispute : (OpenDisputeArgs) -> (OpenDisputeResult);

// Submit a new piece of evidence. Caller must be a party to the deal
// or an arbitrator on the dispute. Allowed only during the Evidence
// phase.
submit_evidence : (SubmitEvidenceArgs) -> (SubmitEvidenceResult);

// Cast a vote. Caller must be one of the dispute's selected arbitrators.
// Allowed only during the Voting phase.
cast_vote : (CastVoteArgs) -> (CastVoteResult);

// Force-finalise a dispute whose voting_deadline_ns has passed.
// Anyone can call (anonymous gate aside) — idempotent. Triggers tally
// + outcome propagation + ledger transfers.
finalize_dispute : (FinalizeDisputeArgs) -> (FinalizeDisputeResult);

// Full dispute view. Caller must be a party to the parent deal or an
// arbitrator assigned to this dispute (mirrors the auth model of
// `get_deal`, which is restricted to payer/recipient). Returns party
// principals + evidence URLs.
get_dispute : (DisputeId) -> (GetDisputeResult) query;

// Reduced public view for status pages (no party principals, no
// evidence URLs — just status, phase, deadlines, vote tally).
// Mirrors the `get_claimable_deal` pattern from the deal flow. Any
// authenticated caller may query.
get_public_dispute : (DisputeId) -> (GetPublicDisputeResult) query;

// List disputes the caller is assigned to as arbitrator (or all open
// ones if the caller is a controller). Pagination.
list_my_disputes : (ListMyDisputesArgs) -> (vec DisputeView) query;

// (Conditional on Q12 acceptance) Both parties opt out of arbitration
// and agree on an outcome. Each must call with the same `agreed_outcome`;
// on match, the dispute resolves with the agreed outcome and a reduced
// arbitrator fee.
withdraw_dispute : (WithdrawDisputeArgs) -> (WithdrawDisputeResult);
```

New endpoints in `api/arbitrators/api.rs`:

```candid
// Self-register as an arbitrator. Idempotent. Sets status = Active.
register_arbitrator : (RegisterArbitratorArgs) -> (RegisterArbitratorResult);

// Self-deregister. Status moves to Deregistered. In-flight disputes
// continue (the arbitrator is asked to finish; non-vote counts as
// Abstain at finalize).
deregister_arbitrator : () -> (DeregisterArbitratorResult);

// Read a single arbitrator profile. Public.
get_arbitrator : (principal) -> (GetArbitratorResult) query;

// List arbitrators with simple filters (status, min score).
list_arbitrators : (ListArbitratorsArgs) -> (vec ArbitratorProfile) query;
```

Admin (controller-only) endpoints in `api/admin/api.rs`:

```candid
// Suspend / reactivate an arbitrator (e.g. for misconduct).
admin_set_arbitrator_status : (principal, ArbitratorStatus) -> (Result);

// Update dispute config: panel size, evidence/voting windows, fee bps,
// min arbitrator score, etc.
update_dispute_config : (DisputeConfig) -> ();
```

New `EscrowError` variants:

```rust
pub enum EscrowError {
    // … existing variants …

    /// Caller is not a party to the deal (only payer / recipient can
    /// open a dispute).
    NotAParty,
    /// A dispute is already open / resolved on this deal.
    DisputeAlreadyExists,
    /// The requested dispute does not exist.
    DisputeNotFound,
    /// The action requires the dispute to be in a different phase.
    InvalidDisputePhase { expected: String, actual: String },
    /// The arbitrator is not assigned to this dispute.
    NotAssignedArbitrator,
    /// The arbitrator pool is too small to fill the panel.
    InsufficientArbitrators { need: u32, have: u32 },
    /// The arbitrator is suspended or deregistered.
    ArbitratorNotActive,
    /// Evidence note exceeds the maximum size.
    EvidenceTooLarge { max: u32 },
}
```

## Open questions

> Each subsection lists a current proposal, alternatives considered,
> and an explicit "decision" line that's empty until the discussion
> concludes. Implementation cannot start until every decision is filled
> in.

### Q1. Distinct statuses vs `resolution` field

**Current proposal.** Add `Disputed`, `ArbitratedSettled`,
`ArbitratedRefunded` to `DealStatus`. Distinct from `Settled` /
`Refunded` so call sites (especially the FE) can render the
arbitration outcome differently.

**Alternative.** Reuse `Settled` / `Refunded` + add a
`resolution: Option<Resolution>` field on `Deal` that captures
arbitrated vs unilateral.

**Trade-off.** Distinct statuses are easier to filter / index
(PandaMe's `History Filter CC / IC / JCC` screens already imply this
naming). Single-status + field is cleaner schema-wise but loses the
ICRC-7 token-status signal.

**Decision:** Distinct statuses — add `Disputed`,
`ArbitratedSettled`, `ArbitratedRefunded` to `DealStatus`. Mirrors
the existing `Cancelled` vs `Rejected` convention (distinct
terminals when the _reason_ differs); preserves exhaustive `match`
safety in `validate_can_*`; gives ICRC-7 a single-field status
signal. The `Deal.dispute: Option<DisputeId>` field carries the
audit-trail link. Future external resolvers (oracle, DAO) can
introduce a generic `Resolved` + `resolution: Option<…>` field at
_that_ point — Q1 doesn't preclude it.

### Q2. Who can open a dispute?

**Current proposal.** Either party of a **bound** deal (both `payer`
and `recipient` set), in `Funded` state, before `expires_at_ns`.

**Alternatives.**
(a) Recipient-only (since they're the one who can claim).
(b) Either party in `Settled` state too, within a "settlement
challenge window" (e.g. 24h post-claim).

**Trade-off.** (a) leaves the recipient with all the leverage —
payer can't dispute a settled-but-undelivered deal. (b) opens the
door to claw-back, which is operationally harder (already-released
funds need to be reversible).

**Decision:** Either _bound_ party (both `payer` and `recipient`
set), in `Funded` state, before `expires_at_ns`. Symmetrical with
the spec's two-signature DISPUTED model; the escrow has no
enforcement leverage post-`Settled` (no claw-back without a new
bond mechanism, which is out of scope for v1). Implementation
contract: `services::expiry` sweep must skip `Disputed` deals.
Schema rider: drop the proposed `EscrowError::NotAParty` variant
(reuse existing `NotAuthorised`, matching the convention in
`validate_can_cancel`).

### Q3. Open-recipient (tip-flow) deals

**Current proposal.** **Cannot** be disputed. Tips are bearer-token
flows; there's no bound counterparty until `accept_deal`. Once
accepted, see Q2.

**Alternative.** Allow disputes on tip flows post-claim with a much
shorter challenge window.

**Decision:** Open-recipient (tip-flow) deals cannot be disputed.
No bound counterparty exists in canister state to dispute against;
tip claim codes are bearer tokens, so post-claim there is no
principled way to identify "the right party" to refund.
`validate_can_open_dispute` requires `deal.recipient.is_some()`.
Schema rider: add `EscrowError::DisputeRequiresBoundRecipient` for
this gate (distinct from `NotAuthorised`, matching the granularity
of existing variants like `MissingClaimCode` and `RecipientMismatch`).

### Q4. Arbitrator pool — permissioned vs permissionless

**Current proposal.** **Permissionless.** Anyone non-anonymous can
call `register_arbitrator` and become eligible. The per-arbitrator
reliability score is the long-term signal; admin controllers can
suspend bad actors.

**Alternative.** **Permissioned.** Admin controller curates the
pool; users apply, controller approves. Lower abuse surface but
adds a centralised gatekeeper to a "trustless escrow" claim.

**Decision:** Permissionless self-registration via
`register_arbitrator` (idempotent — re-registering returns the
existing profile, no error). Admin retains emergency suspension
via `admin_set_arbitrator_status`. Future
`DisputeConfig::min_arbitrator_score: Option<u32>` knob (default
`None` for bootstrap) provides a tunable Sybil filter without
touching code. Permissioned alt rejected because it puts a
centralised gatekeeper at the most consequential layer.

### Q5. Arbitrator selection algorithm

**Current proposal.** Random selection from active arbitrators,
**weighted by `score`** (higher score → higher selection
probability), with a hard exclusion of `payer` and `recipient`.
Randomness via `raw_rand()` (already used for claim codes).

**Alternatives.**
(a) Pure uniform random.
(b) First-N self-select after dispute is opened (Kleros-style).
(c) Stake-weighted (requires a stake mechanic — out of scope for v1).

**Trade-off.** Score-weighted gives newer arbitrators a chance
(non-zero base probability) while rewarding consistently-correct ones.
Pure random is simpler but harder to bootstrap reputation. Self-select
adds latency to dispute resolution.

**Decision:** Score-weighted random selection at `open_dispute`
time, **base weight = 1** for unscored (`score = None`)
arbitrators, hard-exclude `payer` and `recipient` of the disputed
deal. Randomness via the existing `ledger::raw_rand` wrapper (same
primitive used by `services::deals::create_deal` for claim codes).
Selection is committed once to `Dispute.panel` — no re-selection
on tally, keeping the dispute deterministic from the moment it's
opened. Insufficient pool returns
`EscrowError::InsufficientArbitrators { need, have }` with the
deal still `Funded` (no partial state).

### Q6. Panel size

**Current proposal.** Fixed at **3** for v1. Scales to 5 / 7 / 9 in
v2 based on disputed amount.

**Alternative.** Scale immediately: `min(3 + log2(amount_in_usd),
9)`. Adds complexity (and a USD oracle) for marginal benefit at MVP.

**Decision:** Default `panel_size = 3`, admin-tunable via
`DisputeConfig::panel_size: u32` on the existing `Config` struct.
Validator: `panel_size >= 3 && panel_size % 2 == 1` (odd-only is
mathematically required by Q7's tie semantics). Bigger panels and
amount-scaling deferred until a price oracle exists on the canister.

### Q7. Quorum + tie-breaking

**Current proposal.** Quorum and majority share a single formula. Let
`P` be the panel size (Q6, currently 3), and let `cc`, `ic`, `abstain`
be the per-outcome vote counts at `voting_deadline_ns`. Define
`non_abstain = cc + ic`.

- **Quorum reached** ⇔ `non_abstain >= floor(P / 2) + 1`. For
  panel of 3 that's **≥ 2 non-abstain votes**.
- **If quorum reached:** majority = whichever of `cc` / `ic` is
  greater. (Ties are impossible while panel size stays odd — see
  below.)
- **If quorum not reached:** dispute resolves to `NoQuorum`; the
  fallback (refund payer / re-arbitrate / etc.) is decided in [Q9](#q9-evidence--voting-windows).

**Examples (panel = 3):**

| `cc` | `ic` | `abstain` | Outcome                                                                                                                                                        |
| ---- | ---- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2    | 0    | 1         | `Settled`                                                                                                                                                      |
| 2    | 1    | 0         | `Settled`                                                                                                                                                      |
| 0    | 2    | 1         | `Refunded`                                                                                                                                                     |
| 1    | 1    | 1         | `NoQuorum` (single non-abstain CC vs single non-abstain IC — `non_abstain = 2`, but the outcome is split, so this case is **resolved by tie rule**: see below) |
| 1    | 0    | 2         | `NoQuorum` (`non_abstain = 1` < 2)                                                                                                                             |
| 0    | 0    | 3         | `NoQuorum`                                                                                                                                                     |

**Ties.** A tie requires equal `cc` and `ic` with `non_abstain ≥
quorum`, which is only possible when both are non-zero AND equal.
With odd panel sizes this only happens when one arbitrator
abstains. The current proposal **treats ties as `NoQuorum`** —
mathematically a tie means there's no majority, and the dispute
flow's fallback rule (Q9) takes over. This keeps the formula
simple. (The original wording "≥ 50% + 1 of non-abstain" was
ambiguous on this case — fixed here.)

**Tie-avoidance.** Mandate odd panel sizes via
`DisputeConfig::panel_size: u32` validator (`panel_size % 2 == 1`).

**Alternative.** Resolve ties by `opened_by` (i.e., the disputer
loses on a tie because they failed to convince the panel). Keeps
strictly-decisive arbitration but adds an extra rule.

**Decision:** Quorum = `floor(P/2) + 1` non-abstain votes; majority
= greater of `cc` / `ic`; ties resolve as `NoQuorum` (fall through
to Q9's fallback). Odd-panel invariant enforced via Q6's validator.
Tiebreak-by-disputer rejected: would chill legitimate disputes
(asymmetric penalty on whoever exercised the dispute lever, which
is exactly the lever the escrow needs to make freely available to
either bound party).

### Q8. Evidence storage

**Current proposal.** **Off-canister.** `Evidence` records hold an
`artefact_url` (e.g. an IPFS / IC-asset-canister / arbitrary HTTPS
URL) and a SHA-256 hash. The canister stores only the URL +
hash + a short note (≤ 4 KiB). Tamper-evidence comes from the hash;
availability is on the submitter.

**Alternatives.**
(a) On-canister blob storage. Maximum tamper-resistance + zero
external dependencies, but disputed deals would balloon canister
storage; ICRC-7 ownership map starts competing with evidence blobs
for memory.
(b) Hybrid: small blobs on-canister (≤ 64 KiB), large blobs off.

**Decision:** Off-canister artefacts (URL + SHA-256 hash),
on-canister metadata only. Validation rules at the canister
boundary: at least one of `note` / `(artefact_url +
artefact_sha256)` must be present (empty evidence is rejected);
URL and hash are paired (one without the other is rejected); `note`
≤ 4 KiB → `EscrowError::EvidenceTooLarge { max: 4096 }`;
`artefact_url` ≤ 2 KiB and `artefact_sha256` exactly 32 bytes
both reuse `EscrowError::ValidationError(String)` per the existing
length-check convention. Privacy bonus: keeping evidence off-canister
means dispute artefacts are never in raw subnet state visible to node
operators.

### Q9. Evidence + voting windows

**Current proposal.**

- **Evidence:** **3 days** from `open_dispute`. Both parties +
  arbitrators may submit during this window.
- **Voting:** **2 days** from end of evidence phase.
- **No-quorum fallback:** if [Q7's quorum condition](#q7-quorum--tie-breaking)
  is not met by `voting_deadline_ns`, the deal **defaults to refund**
  (payer wins). Rationale: the burden of proof is on the recipient
  (who would receive the funds); silence shouldn't enrich them.

**Alternatives for no-quorum.**
(a) Default to settle (recipient wins).
(b) Re-arbitrate with a fresh panel (max 1 retry).
(c) Escalate to controller / DAO.

**Decision:** Evidence window 3 days default; voting window 2 days
default; both admin-tunable via `DisputeConfig::evidence_window_ns:
u64` / `voting_window_ns: u64`. No-quorum fallback: refund payer
(deal moves to `DealStatus::ArbitratedRefunded` with
`DisputeOutcome::NoQuorum { … }`). Burden of proof is on the
recipient (the party who would receive the funds); the escrow's
status quo ante is "funds with payer", and an indecisive panel
shouldn't enrich the recipient. Default-to-settle creates a
perverse incentive to engineer abstentions; re-arbitrate adds a
phase + timer that's not justified for v1; controller escalation
breaks the trust model.

### Q10. Arbitration fee — model + sourcing

**Current proposal.**

- **Per-deal arbitration fee** in the deal's token, computed at
  `open_dispute` time as `max(MIN_FEE, amount * FEE_BPS / 10_000)`.
  `FEE_BPS` and `MIN_FEE` are admin-configurable (`DisputeConfig`).
- **Sourced from the disputed amount.** Recipient's payout (or
  payer's refund) is reduced by the fee on resolution. Equally:
  losing party pays via reduced outcome.
- **Distributed equally** among arbitrators who cast a non-abstain
  vote. Abstainers get nothing. Non-voters get nothing.
- **No-quorum case:** fee returns to the deal (no arbitrator earns
  anything; deal is refunded in full to payer).

**Alternatives.**
(a) Fee paid by the dispute opener at open time (front-loaded).
Risk: weaponises disputes by the wealthier party.
(b) Loser-pays only (winner gets full payout). Cleaner UX but
requires either party to top up the deal post-funding to cover the
fee.
(c) Fixed fee in a separate canister token (if/when one exists).

**Decision:** Per-deal fee = `max(MIN_FEE, amount * FEE_BPS /
10_000)`, computed at `open_dispute`. Both `FEE_BPS`
(`DisputeConfig::arbitration_fee_bps: u32`) and `MIN_FEE`
(`DisputeConfig::arbitration_min_fee: u128`) are admin-tunable.
Sourced from the disputed amount in the escrow subaccount.
Distributed equally among non-abstain voters via direct
per-arbitrator transfer at finalize. NoQuorum: full refund, no fees
paid. Front-loaded rejected (weaponises wealth); loser-pays
rejected (requires post-fund top-ups the canister doesn't support);
fixed-fee-in-separate-token rejected (no such token).

**Refinements (load-bearing for the impl PRs):**

1. **Per-arbitrator ledger fees are absorbed by the prevailing
   party**: `prevailing_party_payout = amount - non_abstain_count
   * fee_per_arbitrator - sum_of_arbitrator_ledger_fees`. Same
   pattern as today's settle/refund flows where the prevailing party
   absorbs the single ledger fee.
2. **Integer-division remainder of the arbitration fee rolls back
   to the prevailing party** — every arbitrator gets exactly the
   same share; remainder is implicit in the subtraction above.
3. **Schema change to support per-arbitrator payout idempotency**:
   replace the RFC's `Dispute.arbitrators: Vec<Principal>` +
   `Dispute.votes: BTreeMap<Principal, Vote>` with a single
   `Dispute.panel: Vec<PanelMember>` where `PanelMember { principal:
   Principal, vote: Option<Vote>, paid_at_ns: Option<u64>,
   payout_tx: Option<u128> }`. Mirrors the existing
   `Deal.funded_at_ns` / `funding_tx` / `settled_at_ns` /
   `payout_tx` pattern. `cast_vote` mutates the matching member's
   `vote`. `finalize_dispute` walks the panel for tally and for
   replay-safe fan-out payouts (skip members where
   `paid_at_ns.is_some()`).

**Schema rider:** new `EscrowError::AmountTooSmallForArbitration {
min: u128 }` emitted at `open_dispute` when the deal's amount
cannot cover `MIN_FEE + N * estimated_ledger_fee`. New
`DisputeConfig` fields: `arbitration_fee_bps: u32`,
`arbitration_min_fee: u128`. `services::disputes::finalize` uses
the per-deal `PROCESSING` lock in `memory.rs` to serialise
concurrent finalize calls.

### Q11. Reliability score — arbitrator side

**Current proposal.** Add `disputes_assigned`, `disputes_voted`,
`disputes_with_majority` to `ArbitratorProfile`. Compute `score`
as `(disputes_with_majority * 100) / disputes_voted` with a
`MIN_VOTES_FOR_SCORE = 5` threshold below which `score = None`.

The existing `api/reliability/` module is **payer/recipient-side**
reliability. The arbitrator score is a separate concept; do not
conflate them in the same struct.

**Decision:** Schema as proposed (`disputes_assigned`,
`disputes_voted`, `disputes_with_majority`, `score: Option<u32>`,
`MIN_VOTES_FOR_SCORE = 5` constant). **Refinement:** NoQuorum
disputes only update `disputes_assigned` — they do **not** update
`disputes_voted` or `disputes_with_majority` — to avoid a perverse
incentive to abstain on hard-to-quorum disputes. Update rules:

| Outcome            | Voter type             | `assigned` | `voted` | `with_majority` |
| ------------------ | ---------------------- | ---------- | ------- | --------------- |
| Settled / Refunded | non-abstain w/ majority | +1         | +1      | +1              |
| Settled / Refunded | non-abstain vs majority | +1         | +1      | +0              |
| Settled / Refunded | abstain                | +1         | +0      | +0              |
| **NoQuorum**       | **any (incl. non-abstain)** | **+1**     | **+0**  | **+0**          |

### Q12. Out-of-band settlement during a dispute

**Current proposal.** **Allowed.** Both parties can call
`withdraw_dispute` (new endpoint) during the Evidence phase to
unilaterally agree on an outcome. Both must call it with the same
`agreed_outcome` argument (CC or IC). On match, the dispute moves
to `Resolved` with the agreed outcome and arbitrators get a
**reduced** fee (e.g. 25% of `arbitration_fee`) to compensate for
their time.

**Alternatives.**
(a) No out-of-band settlement; once disputed, always arbitrated.
Cleaner protocol but worse UX for parties who want to make peace.
(b) Allow only before any evidence is submitted.

**Decision:** _<TBD>_

### Q13. Weighted voting now or v2?

Direct from `Product.docx`: _"the majority, in future releases, can
be even **weighted**, considering as **weight the history of correct
decision** of each node or the **competency** (TO BE DISCUSSED)."_
This question covers the **history-based** half; competency is
[Q14](#q14-competency--subject-matter-tags).

**Current proposal.** **Simple majority in v1; weighted voting in
v2.** Q11's reliability score is computed and stored from day 1, so
the v2 switch is a service-level change with no schema migration.

**Alternatives.**
(a) **Simple majority always.** Each selected arbitrator counts as
1; ≥ 50% + 1 of non-abstain votes decides. Easy to reason about,
easy to explain to users.
(b) **Linear-weighted in v1.** Each arbitrator's vote weight is
`max(MIN_WEIGHT, score)` (e.g. clamp to `[1, 100]`). New / no-score
arbitrators contribute 1; established ones up to 100. Outcome
decided by greater weighted sum.
(c) **Square-root or log-weighted.** Same idea, but
`vote_weight = sqrt(score)` to soften reputation concentration.
Compromise between (a) and (b) — high-score arbitrators matter
more than newbies, but no single arbitrator can dominate.
(d) **Defer to v2** as currently proposed. v1 ships simple majority;
revisit after observing real disputes.

**Trade-off.**

- (a) is the safest MVP: easiest to explain ("3 arbitrators, 2 votes
  win") and removes a whole class of fairness arguments
  ("my vote weighed less because…").
- (b) and (c) are honest about reputation differences, but punish
  low-score arbitrators who were **selected by the canister** —
  they didn't ask to be there. Feels unfair unless arbitrators
  opt in per dispute (which makes selection even harder).
- (b)/(c) also create a Sybil-resistance question: an attacker who
  earns one high-score account can outweigh several honest newbies.
- (d) lets us collect data before committing. Cheap to defer because
  the schema doesn't change.

**Decision:** _<TBD>_

### Q14. Competency / subject-matter tags

Direct from `Product.docx`: weight the vote by _"the history of
correct decision of each node **or the competency**"_. Competency
is the alternative-or-additional weighting dimension to history.

**Current proposal.** **Schema in v1, behaviour in v2.** Add a
`tags: Vec<String>` field to `ArbitratorProfile` (self-declared —
free-form for v1 to avoid taxonomy bikeshedding) and a
`category: Option<String>` field on `Deal` (set at create time, can
be `None` for backward-compat). v1 selection and voting **ignore**
the tags. v2 can bias selection toward arbitrators whose `tags`
include the deal's `category`, with a fallback to the general pool
when no tag-match arbitrator is available.

**Alternatives.**
(a) **Skip entirely.** Competency without a verification mechanic
(certifications, KYC, etc.) is self-claimed and easily abused.
Lean on history-based reputation only.
(b) **Predefined enum.** Hard-code a closed set of categories
(`PHYSICAL_GOODS`, `DIGITAL_GOODS`, `SERVICES`, `REAL_ESTATE`, …).
Less flexible but enforceable.
(c) **Schema only as proposed.** Free-form string tags; no
selection bias yet.
(d) **Full v1.** Tags + category + selection bias from day 1.
Adds a category taxonomy decision and a fallback algorithm.

**Trade-off.**

- (a) keeps the canister narrow but drops half of `Product.docx`'s
  weight-source proposal.
- (b) is enforceable but inflexible — any new category needs a
  Candid update.
- (c) is forward-compatible: v1 doesn't bias on tags, but the
  schema is ready, so v2 can flip the switch without a data
  migration.
- (d) is the most product-complete v1 but adds a UX surface
  (category picker on deal creation, tag editor on arbitrator
  registration) that may not be worth it before the rest of the
  flow is battle-tested.

**Decision:** _<TBD>_

## Implementation plan

After this RFC is accepted, the implementation lands in **separate
atomic PRs**, each referencing `RFC-001` in the commit body:

1. `feat(types): add Dispute, Arbitrator, vote types` — types only,
   no endpoints, no behaviour change. Add `Disputed` /
   `ArbitratedSettled` / `ArbitratedRefunded` to `DealStatus`. Add
   `dispute: Option<DisputeId>` to `Deal`. Run `npm run did`.
2. `feat(memory): add dispute + arbitrator stable storage` —
   `BTreeMap<DisputeId, Dispute>`, `BTreeMap<Principal, ArbitratorProfile>`,
   atomic ID allocators, save/restore.
3. `feat(arbitrators): register / deregister / list endpoints` —
   no dispute integration yet.
4. `feat(disputes): open_dispute endpoint + Funded → Disputed
transition` — selection algorithm + Evidence phase. Doesn't
   support voting yet.
5. `feat(disputes): submit_evidence endpoint`.
6. `feat(disputes): cast_vote endpoint + Voting phase transition`.
7. `feat(disputes): finalize_dispute + Disputed →
ArbitratedSettled / ArbitratedRefunded transition` — ledger
   transfers + arbitrator fee distribution + reliability-score
   updates.
8. `feat(disputes): housekeeping timer for auto-finalize after
voting_deadline_ns` — reuses the expiry-sweep pattern.
9. `feat(disputes): withdraw_dispute (out-of-band settlement)` —
   only if Q12 is accepted.
10. `docs: update README + future-expansion section to mark dispute
flow as implemented`.

Each PR is independently mergeable behind feature flags is _not_
proposed — instead, the `Disputed` status simply isn't reachable
until step 4 lands, and `cast_vote` rejects until step 6 lands.
This keeps `main` releasable at every step without a feature-flag
mechanism.

A coordinated PR opens in `../pandame/` after step 1 (types) lands
to regenerate the bindings. Pandame's stubbed Dispute button stays
non-functional until `open_dispute` (step 4) lands; then it can wire
to a real flow. The Arbitrator profile + dispute screens already in
the Figma file go live as steps 3–7 land.

## Alternatives considered

### Off-the-shelf — Kleros / SAJE / etc.

Existing decentralised dispute systems (Kleros on Ethereum, etc.)
are mature. We don't reuse them because:

- The escrow lives on IC; bridging would add a foreign-chain
  dependency for an MVP feature.
- The product spec is small enough that a native impl is faster and
  keeps the canister self-contained.
- Permissioned-by-default arbitrator pool gives us a trust gradient
  the user can dial up over time; a Kleros-style anonymous juror
  pool is the v2 endgame.

### Single-arbiter (admin-resolved)

The simplest possible model: any dispute is resolved by a canister
controller. **Not chosen** because it contradicts the "trustless"
positioning of the project. Acceptable as an _emergency_ override
(`admin_force_resolve`) for arbitrator-pool failures, but not as
the default.

### Stake-based (Kleros-lite)

Arbitrators stake the canister token to vote; majority voters earn
the stake of minority voters. **Not in v1** because it requires a
canister-side token, which doesn't exist. Compatible with the
proposed schema (stake field can be added later as `Option<u128>`).

## Out of scope

- Tip-flow disputes (Q3).
- Multi-round arbitration / appeals (Q9 picks one no-quorum
  fallback; appeals would add a layer).
- Cross-chain evidence (everything off-canister is just a URL +
  hash for v1; bridging proofs is a separate problem).
- Fiat-side enforcement (real-life asset delivery is unverifiable
  on-chain — arbitrators rule on the evidence; off-chain
  enforcement is not the canister's problem).
- Arbitrator slashing for misconduct beyond the existing reliability
  score (Q11). Real slashing requires the stake mechanic.

> **Note:** reputation-weighted voting and competency weighting are
> **not** out of scope — they're explicit `Product.docx`
> requirements captured as [Q13](#q13-weighted-voting-now-or-v2) and
> [Q14](#q14-competency--subject-matter-tags). The "open" question is
> only whether they ship in v1 (active behaviour) or v2 (schema-only
> in v1, behaviour later).

---

## Decision log

> Filled in as questions are resolved. Each entry should reference
> the comment thread / message that resolved it.

| Q   | Resolved | Resolution | Source  |
| --- | -------- | ---------- | ------- |
| Q1  | 2026-05-10 | Distinct statuses (`Disputed`, `ArbitratedSettled`, `ArbitratedRefunded`) on `DealStatus`; `Deal.dispute: Option<DisputeId>` carries the audit-trail link. | RFC-001 design review (2026-05-10) |
| Q2  | 2026-05-10 | Either bound party (payer or recipient), `Funded` state, before expiry; expiry sweep skips `Disputed`. Drop `NotAParty` variant (reuse `NotAuthorised`). | RFC-001 design review (2026-05-10) |
| Q3  | 2026-05-10 | Open-recipient (tip-flow) deals cannot be disputed; gate via new `DisputeRequiresBoundRecipient` variant. | RFC-001 design review (2026-05-10) |
| Q4  | 2026-05-10 | Permissionless self-registration (idempotent); admin keeps emergency suspension; future `min_arbitrator_score` knob in `DisputeConfig`. | RFC-001 design review (2026-05-10) |
| Q5  | 2026-05-10 | Score-weighted random selection at `open_dispute` time, base weight = 1 for unscored arbitrators, hard-exclude payer + recipient, randomness via `ledger::raw_rand`. | RFC-001 design review (2026-05-10) |
| Q6  | 2026-05-10 | `panel_size = 3` default, admin-tunable via `DisputeConfig::panel_size: u32`; validator enforces odd and `>= 3`. | RFC-001 design review (2026-05-10) |
| Q7  | 2026-05-10 | Quorum = `floor(P/2) + 1` non-abstain; majority = greater of `cc`/`ic`; ties resolve as `NoQuorum`. | RFC-001 design review (2026-05-10) |
| Q8  | 2026-05-10 | Off-canister artefacts (URL + SHA-256), on-canister metadata only; URL/hash paired; `note` ≤ 4 KiB; `artefact_url` ≤ 2 KiB; `artefact_sha256` exactly 32 bytes. | RFC-001 design review (2026-05-10) |
| Q9  | 2026-05-10 | Evidence window 3 days, voting window 2 days, both admin-tunable; no-quorum fallback refunds payer (`ArbitratedRefunded`). | RFC-001 design review (2026-05-10) |
| Q10 | 2026-05-10 | Per-deal fee from disputed amount; `bps + min` admin-tunable; equally split among non-abstain voters; NoQuorum pays no fee. Schema refinement: `Dispute.panel: Vec<PanelMember>` replaces `arbitrators` + `votes` for per-arbitrator payout idempotency. | RFC-001 design review (2026-05-10) |
| Q11 | 2026-05-10 | Schema as proposed; refinement: NoQuorum disputes only update `disputes_assigned` (not `voted` / `with_majority`). | RFC-001 design review (2026-05-10) |
| Q12 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q13 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q14 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
