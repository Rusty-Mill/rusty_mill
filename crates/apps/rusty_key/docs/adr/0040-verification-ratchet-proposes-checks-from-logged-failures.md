# ADR-0040: The verification ratchet proposes checks only from logged failures

- Status: Accepted (implemented — `compose::RatchetLog` + `/ratchet`)
- Date: 2026-05-28
- Tags: compose, verification, feedback, ratchet, roadmap

## Context

The harness has a strong *feedback* signal — every failed turn is attributed to a
fixed `FailureType` plus a `(category, layer)` pair (ADR-0021, `compose`
verifier) — but nothing closes the loop back into *deterministic* verification.
Recurring failures are re-discovered turn after turn rather than being pinned by
a `checks.toml` check that would catch them. The harness-assessment review
(`docs/assessment/RECOMMENDATIONS.md`, P3) names this the feedback half of the
feedforward/feedback pair, to ship alongside the guides (ADR-0037).

A self-improving harness that *writes its own rules* risks "aspirational rules":
checks asserting things that sound good but never correspond to an observed
failure, which accrete and rot. The benchmarked Claude Code writeups treat
human attention as the scarce resource — the harness should propose, the human
disposes.

## Decision

Add a **`RatchetLog` in `compose`** (append-only `.rustykeys/ratchet.jsonl`,
mirroring the existing JSONL logs). On a failed turn, `app::Session` records each
`Attribution`. `/ratchet` aggregates by `(failure_type, category)` and, for pairs
that recur at least `RATCHET_MIN_OCCURRENCES` (2) times, **proposes** a
`checks.toml` `[[check]]` stanza (with a `REPLACE_ME` command for the human to
fill in).

**Zero aspirational rules, enforced in code.** `propose_checks` derives proposals
*solely* from `RatchetLog::aggregate` output, and there is no other path to a
proposal — an empty log yields zero proposals. A check can only be proposed from
an attribution that actually happened.

**Propose, never apply.** `/ratchet` prints stanzas; the harness never writes
`checks.toml`. The human reviews, edits the placeholder command, and commits —
keeping verification authority human-owned and the rule set honest.

The ratchet reuses the `FailureType` matrix (ADR-0021) and the `CheckRegistry`
file format (PRD 05); it lives in `compose` beside both. The kernel dependency
path is untouched.

## Consequences

- Closes the feedback loop: recurring failures become candidate deterministic
  checks, so verification tightens (ratchets) over time instead of re-litigating
  the same failure each turn.
- The "zero aspirational rules" invariant is a code guarantee, not a convention,
  so the proposed rule set cannot drift from observed reality.
- Human attention stays the gate: proposals are advisory, never auto-applied.
- Pairs with ADR-0037 (guides) as the two halves of feedforward/feedback.
- Adds one append-only log (`ratchet.jsonl`); torn-line tolerant like the others.
