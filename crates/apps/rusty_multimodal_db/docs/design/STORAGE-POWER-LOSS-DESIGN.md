# Storage Power-Loss Proof: What Would Prove It, and the Runbook (Proposed)

- Status: **Proposed, design and runbook only; the owner asked for
  the item** (2026-09-23, "1-5", item 5, `ADR-0119`). No code under
  `src/` changes; no CI job is added.
- Date: 2026-09-23
- Related: `docs/design/STORAGE-CRASH-SAFETY-GATE-DESIGN.md` and
  `STORAGE-021` (the process-kill gate this extends in scope but not
  in mechanism), `src/bin/crash_safety_harness.rs` (whose module docs
  state the gap exactly: `SIGKILL` leaves the page cache intact),
  `ADR-0092` (`sync_parent_dir`), `ADR-0097`/`ADR-0099` (`msync` before
  an in-place update's acknowledgement), `ADR-0025` (the journal's
  `fsync` before the acknowledgement), `STORAGE-017` (the `COMMITTED`
  marker).
- Supersedes/Superseded by: none.

## Purpose and scope

State what a power-loss proof for this crate's storage would have to
show, why no test that runs in this repository's CI can show it, the
three ways it can be shown, and a runbook an operator with a Linux
host and root can run today. The claim under test: **every write this
crate acknowledged as durable is present after the machine loses
power at any instant, and no torn write is ever read as committed.**

Scope, exactly: the definition of the claim (`PLP-FR-001`); the
options and why `dm-log-writes` is the one to run (`PLP-FR-002`); the
runbook script (`PLP-FR-003`); what the script does not prove
(`PLP-FR-004`); nothing under `src/` changed (`PLP-FR-005`).

## Non-goals

- **Running the trial in CI.** GitHub-hosted runners have no root
  device-mapper access and no way to cut power; the `STORAGE-021` gate
  stays the CI proof, of a process crash only.
- **Changing the writer or the harness.** `crash_writer`'s modes are
  the workloads the runbook replays; they stay as they are.
- **Proving the filesystem.** The trial assumes `ext4` or `xfs` with
  `data=ordered` semantics; a filesystem that lies about `fsync` fails
  everything above it and is out of scope.

## Context and terminology

What `SIGKILL` cannot show: the page cache belongs to the file, not to
the process, so a killed writer's dirty pages are still read back by
the reopening process. A power loss additionally loses every dirty
page that had not reached the device, and — on a device without a
write barrier honoured — may lose or reorder blocks the kernel had
already submitted. The crate's durability points are three `fsync`
families: `sync_data` on the insert log and the journal before an
acknowledgement, `msync` on the mapped slot file before an in-place
update's acknowledgement (`ADR-0097`), and `sync_parent_dir` after a
file is created or renamed (`ADR-0092`). The claim is that the set of
bytes on the device after any prefix of those syncs reopens to a
store holding every acknowledged write and no torn one.

Terms: a **sync point** is a completed `fsync`/`fdatasync`/`msync`; a
**crash prefix** is the device state after some set of submitted
writes completed and the rest did not; **replay to a mark** is
`dm-log-writes`' reconstruction of the device at a given sync point.

## Requirements

- `PLP-FR-001` **The claim.** For every `crash_writer` mode, at every
  sync point the writer reports, the device state replayed to that
  point reopens (through the real `GenericMmapStore::open`) to a store
  that holds every write acknowledged before the point and reads no
  torn slot as committed.
- `PLP-FR-002` **The method.** `dm-log-writes` (Linux ≥ 4.1): a
  device-mapper target that records every bio and every flush/FUA
  mark; `replay-log` (xfstests) reconstructs the device at any mark.
  Chosen over a VM with forced power-off (non-deterministic, one
  crash per boot) and over a userspace shim (cannot see the page
  cache's writeback order, which is the thing under test).
- `PLP-FR-003` **The runbook.** `scripts/power_loss_trial.sh`: builds
  the writer, creates a loop device and a `log-writes` target over it,
  runs one writer mode on a filesystem mounted there, then replays to
  each mark the writer's sync points left and reopens the store at
  each, reporting per mark: acknowledged writes present, torn slots
  read as committed. Root required; refuses to run otherwise; every
  device it creates is torn down on exit, success or failure.
- `PLP-FR-004` **What it does not prove.** Device firmware that
  acknowledges a flush before the platter or NAND has it; a
  filesystem's own bugs; anything about Windows.
- `PLP-FR-005` **Nothing under `src/` changed.**

## Considered options

- **(a) `dm-log-writes` runbook, hand-run — proposed.** Deterministic,
  replays every mark, needs root on a Linux host; the tool xfstests
  uses for exactly this claim.
- **(b) A VM with forced power-off in a loop.** Real power loss
  semantics for the guest, but one sample per boot, minutes each, and
  the hypervisor's disk cache policy decides what "power loss" means.
- **(c) A userspace fault-injecting file layer** under the writer.
  Cannot observe writeback ordering of a memory-mapped file; would
  prove the wrong thing.
- **(d) Decline.** Keep `STORAGE-021` as the only proof and say so.
- **(e) The crash-prefix snapshot — run, alongside (a).** ext4 on a
  loop device opened `--direct-io=on`; at the sync point the writer is
  killed and the backing file copied at once, before dirty-page expiry;
  the copy is mounted (ext4 journal replay) and reopened. Needs only
  loop devices and root, so it runs where (a) cannot. One prefix per
  run, no device-side reordering. `scripts/power_loss_trial_loop.sh`.

The owner's shorthand: **(a)** the runbook; **(b)** the VM loop
instead; **(d)** decline and keep the caveat.

## Proposed shape

`scripts/power_loss_trial.sh` (new, Linux, root); this document;
`ADR-0119`. No Cargo target.

## Data/state and invariants

- The trial's loop device is a fresh sparse file; nothing on the host
  is touched.
- A mark is named by the writer's own stdout line (`FLUSHED`,
  `WROTE <i>`, `MARKER_WRITTEN`), so a finding names the sync point
  it belongs to.

## Errors, failure, recovery, and observability

A replay that fails to reopen, or reopens without an acknowledged
write, is reported with the mark and the mode; the script exits
non-zero. A missing `dm-log-writes` module or `replay-log` binary is
a named refusal before any device is created.

## Security, privacy, and compatibility

Root on a disposable host only. The script never touches a path it
did not create.

## Acceptance criteria

1. The script refuses without root and without the tools, naming each.
2. On a host with both, the `flushed-updates` mode reopens with every
   update at the `FLUSHED` mark.
3. The `torn-write` mode reopens at the `ID_WRITTEN` and
   `VALUE_WRITTEN` marks with the new slot absent and the reseed value
   present.

## Verification plan

Hand-run on a Linux host with root; the results, when obtained, go in
`RESULTS.md` under a power-loss heading and this document's change
history. Not part of `cargo test`.

## Traceability

- Roadmap: `STORAGE-POWER-LOSS-PROOF`.
- Decision: `ADR-0119`.
- Requirements: `PLP-FR-001`–`005`.

## Open questions

- Whether a self-hosted runner with root could run the trial nightly;
  not decided here.

## Change history

- 2026-09-23: proposed with the runbook (`ADR-0119`).
- 2026-09-23: option (e) added and run in the session's guest (no
  device-mapper there): `flushed` 500/500, `unflushed` 0/500,
  `torn-write` seed intact and torn slot absent, three repeats each —
  `RESULTS.md`.
