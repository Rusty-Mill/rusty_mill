# ADR-0119: A Power-Loss Proof — the Method, the Runbook, and Why Not CI

- Status: **Proposed, design and runbook; the owner chose the item**
  (2026-09-23, "1-5", item 5). No code under `src/` changes; one
  diagnostic mode on `crash_writer`; no CI job.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/design/STORAGE-POWER-LOSS-DESIGN.md` (this decision's
  design), `ADR-0095` / `STORAGE-021` (the process-kill gate),
  `src/bin/crash_safety_harness.rs` (the caveat this answers),
  `ADR-0092`, `ADR-0097`, `ADR-0099` (the sync points under test).
- Supersedes/Superseded by: none. Additive:
  `scripts/power_loss_trial.sh`, `crash_writer reopen-check`.

## Context

The crash-safety gate proves survival of a process crash. Its own
docs say what it cannot prove: `SIGKILL` leaves the page cache intact,
so a power loss — which loses every dirty page not yet on the device,
and may reorder what was submitted — is untested. The growth document
lists it under hardening as "still a process-kill proof, not a
power-loss one". A CI runner cannot run the proof: it needs a
device-mapper target and root, or a machine whose power can be cut.

## Decision

- `PLP-FR-001`–`005` as the design states: the claim (every
  acknowledged write present, no torn write read as committed, at
  every sync point); the method, `dm-log-writes` with `replay-log`,
  which records every block write and flush mark and replays the
  device to any mark — deterministic, every mark, the tool xfstests
  uses for this claim; the runbook, `scripts/power_loss_trial.sh`,
  root and Linux, which refuses without the tools, builds
  `crash_writer`, runs one mode on a `log-writes` device, marks each
  sync point the writer reports, and replays to each mark and reopens
  through `crash_writer reopen-check`, tearing every device down on
  exit; what it does not prove (firmware that lies about a flush, the
  filesystem's own bugs, Windows); nothing under `src/`.
- `crash_writer reopen-check <path>`: reopen through the real
  `GenericMmapStore::open`, print the record count and how many carry
  an update, exit `0`, or print the error and exit `1`.
- Not run here: this container has no root device-mapper access. The
  trial's results, when an operator runs it, go to `RESULTS.md`.
- Declined: a VM power-off loop (one non-deterministic sample per
  boot) and a userspace fault layer (cannot see writeback order).

## Consequences

- Positive: the claim is stated precisely, the method is the standard
  one, and the runbook is one command on a disposable Linux host.
- Negative / tradeoffs: unrun. Until someone runs it the crate's
  durability claim is still the process-kill one; this ADR does not
  change the readiness verdict.
- Named, not hidden: `STORAGE-021` remains the only automated proof;
  a self-hosted root runner could make the trial nightly — an open
  question, not a plan.

## Acceptance and implementation

- 2026-09-23: on `claude/pr-276-multimodal-db-growth-4lkjx8`, in one PR with `ADR-0115`–`ADR-0118`. `scripts/power_loss_trial.sh` refuses without root and without the tools, by inspection; `crash_writer reopen-check` builds under `research`. Trial not run (no root device-mapper access here). Builder: Claude.
