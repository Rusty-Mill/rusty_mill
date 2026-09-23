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
- The `dm-log-writes` runbook could not run here: the session's guest
  kernel has no device-mapper (`CONFIG_BLK_DEV_DM` absent). On the
  owner's "Power", a second runbook was written and run:
  `scripts/power_loss_trial_loop.sh`, the crash-prefix snapshot — ext4
  on a `--direct-io=on` loop device, the backing file copied at the
  `SIGKILL` instant before dirty-page expiry, mounted with ext4's own
  journal replay and reopened. Three repeats per mode: `flushed` 500 of
  500 updates present; `unflushed` 0 of 500; `torn-write` the flushed
  seed intact and the torn slot absent. Recorded in `RESULTS.md`. One
  prefix per run, no block reordering: weaker than the replay, stronger
  than the process-kill gate.
- Declined: a VM power-off loop (one non-deterministic sample per
  boot) and a userspace fault layer (cannot see writeback order).

## Consequences

- Positive: the claim is stated precisely, the method is the standard
  one, and the runbook is one command on a disposable Linux host.
- Negative / tradeoffs: the fuller `dm-log-writes` replay is still
  unrun; the crash-prefix trial covers one prefix per run and no
  device-side reordering. The readiness verdict does not change on one
  session's trial, but the claim is now measured at the device, not
  only at the page cache.
- Named, not hidden: `STORAGE-021` remains the only automated proof;
  a self-hosted root runner could make the trial nightly — an open
  question, not a plan.

## Acceptance and implementation

- 2026-09-23: on `claude/pr-276-multimodal-db-growth-4lkjx8`, in one PR with `ADR-0115`–`ADR-0118`. `scripts/power_loss_trial.sh` refuses without root and without the tools, by inspection; `crash_writer reopen-check` builds under `research`. The `dm-log-writes` trial not run (no device-mapper in this kernel); the crash-prefix trial (`scripts/power_loss_trial_loop.sh`) run here, 9 of 9 as expected, recorded in `RESULTS.md`. Builder: Claude.
