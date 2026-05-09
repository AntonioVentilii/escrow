# RFC-001 — Dispute resolution + arbitrators

| Field   | Value                                                  |
| ------- | ------------------------------------------------------ |
| Status  | **Draft** — open for comment                           |
| Author  | @antonioventilii                                       |
| Created | 2026-05-09                                             |
| Targets | escrow `v0.1.x` (next minor — needs schema migration)  |
| Related | `Pluggable resolvers` sketch in `src/escrow/README.md` |

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
flow**: a real-life asset exchange where the recipient _has_ claimed
the funds but the payer believes the asset wasn't delivered (or vice
versa). Today, both parties have only one lever: refuse to consent /
refuse to claim. There is no way to **arbitrate** a contested
delivery.

Adding dispute resolution unlocks the v1 product spec
(`old_escrow/Product.docx`'s "Evaluation" section) and is the first
concrete instance of the [`Pluggable resolvers` sketch](../../src/escrow/README.md#future-expansion)
in the canister README.

## Goals & non-goals

### In scope (v1 of dispute resolution)

- Either party of a **Funded** deal with a known recipient can open a
  dispute before expiry.
- Both parties can submit text + binary evidence within a fixed
  evidence window.
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

### Out of scope (deferred to later RFCs or future work)

- Tip flow (open-recipient deals): can't dispute since there's no
  bound counterparty until claim.
- Reputation-weighted voting (mentioned in `Product.docx` as v2 —
  this RFC writes the schema fields but always uses simple majority).
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
    /// Free-form note (max 4 KiB, see Q8).
    pub note: Option<String>,
    /// Off-canister artefact pointer (URL + content hash). On-canister
    /// blob storage is out of scope for v1 (see Q8).
    pub artefact_url: Option<String>,
    pub artefact_sha256: Option<Vec<u8>>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Dispute {
    pub deal_id: u64,
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
    Settled { yes: u32, no: u32, abstain: u32 },
    Refunded { yes: u32, no: u32, abstain: u32 },
    /// Quorum not reached by `voting_deadline_ns`. See Q9 for the
    /// fallback behaviour.
    NoQuorum { yes: u32, no: u32, abstain: u32 },
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

// Read a single dispute. Public — anyone can see open disputes.
get_dispute : (DisputeId) -> (GetDisputeResult) query;

// List disputes the caller is assigned to as arbitrator (or all open
// ones if the caller is a controller). Pagination.
list_my_disputes : (ListMyDisputesArgs) -> (vec DisputeView) query;
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

**Decision:** _<TBD>_

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

**Decision:** _<TBD>_

### Q3. Open-recipient (tip-flow) deals

**Current proposal.** **Cannot** be disputed. Tips are bearer-token
flows; there's no bound counterparty until `accept_deal`. Once
accepted, see Q2.

**Alternative.** Allow disputes on tip flows post-claim with a much
shorter challenge window.

**Decision:** _<TBD>_

### Q4. Arbitrator pool — permissioned vs permissionless

**Current proposal.** **Permissionless.** Anyone non-anonymous can
call `register_arbitrator` and become eligible. The per-arbitrator
reliability score is the long-term signal; admin controllers can
suspend bad actors.

**Alternative.** **Permissioned.** Admin controller curates the
pool; users apply, controller approves. Lower abuse surface but
adds a centralised gatekeeper to a "trustless escrow" claim.

**Decision:** _<TBD>_

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

**Decision:** _<TBD>_

### Q6. Panel size

**Current proposal.** Fixed at **3** for v1. Scales to 5 / 7 / 9 in
v2 based on disputed amount.

**Alternative.** Scale immediately: `min(3 + log2(amount_in_usd),
9)`. Adds complexity (and a USD oracle) for marginal benefit at MVP.

**Decision:** _<TBD>_

### Q7. Quorum + tie-breaking

**Current proposal.** Simple majority (≥ 50% + 1 of non-abstain
votes) decides. With panel of 3: 2 votes decide, abstentions count
toward "no quorum" if too many abstain.

**Tie possible if panel is even (4, 6, 8, …).** Avoid by mandating
odd panel sizes.

**Decision:** _<TBD>_

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

**Decision:** _<TBD>_

### Q9. Evidence + voting windows

**Current proposal.**

- **Evidence:** **3 days** from `open_dispute`. Both parties +
  arbitrators may submit during this window.
- **Voting:** **2 days** from end of evidence phase.
- **No-quorum fallback:** if `< panel_size / 2 + 1` non-abstain
  votes by `voting_deadline_ns`, the deal **defaults to refund**
  (payer wins). Rationale: the burden of proof is on the recipient
  (who would receive the funds); silence shouldn't enrich them.

**Alternatives for no-quorum.**
(a) Default to settle (recipient wins).
(b) Re-arbitrate with a fresh panel (max 1 retry).
(c) Escalate to controller / DAO.

**Decision:** _<TBD>_

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

**Decision:** _<TBD>_

### Q11. Reliability score — arbitrator side

**Current proposal.** Add `disputes_assigned`, `disputes_voted`,
`disputes_with_majority` to `ArbitratorProfile`. Compute `score`
as `(disputes_with_majority * 100) / disputes_voted` with a
`MIN_VOTES_FOR_SCORE = 5` threshold below which `score = None`.

The existing `api/reliability/` module is **payer/recipient-side**
reliability. The arbitrator score is a separate concept; do not
conflate them in the same struct.

**Decision:** _<TBD>_

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
- Reputation-weighted voting (mentioned in Product.docx as v2; the
  schema is ready, the algorithm isn't).
- Multi-round arbitration / appeals (Q9 picks one no-quorum
  fallback; appeals would add a layer).
- Cross-chain evidence (everything off-canister is just a URL +
  hash for v1; bridging proofs is a separate problem).
- Fiat-side enforcement (real-life asset delivery is unverifiable
  on-chain — arbitrators rule on the evidence; off-chain
  enforcement is not the canister's problem).
- Arbitrator slashing for misconduct beyond the existing reliability
  score (Q11). Real slashing requires the stake mechanic.

---

## Decision log

> Filled in as questions are resolved. Each entry should reference
> the comment thread / message that resolved it.

| Q   | Resolved | Resolution | Source  |
| --- | -------- | ---------- | ------- |
| Q1  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q2  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q3  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q4  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q5  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q6  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q7  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q8  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q9  | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q10 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q11 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
| Q12 | _<TBD>_  | _<TBD>_    | _<TBD>_ |
