# ADR-0071: Pad sidecar log from the batch runner, and a spread report

- Status: Accepted
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `docs/roadmap/PARITY-PLAN.md` (workstreams C and F), ADR-0064, `RB-RESEARCH-O021`, `RB-RESEARCH-O020`
- Supersedes/Superseded by: none

## Context

The port has no boost pads, and the capture format has no pad state, so a
pickup can only be inferred from a jump in the boost value. Workstream C needs
the exact tick a pad is taken and its timer. Workstream F needs the game's own
run-to-run spread per tape, which no tool reports.

## Decision drivers

- No change to the capture format, the plugin or the bot: each is a release and
  a risk to the 62-fixture gate and the unattended runs.
- `rb_run_tapes` already receives every game packet and `FieldInfo` from core
  while it waits for the tape to end.

## Considered options

1. Add pad state to the BakkesMod capture (plugin release, format change).
2. Log from the tape bot (the bot is per car and sees the packet, but the
   hivemind and single-car bots would both need it).
3. Log from the runner, as a sidecar next to each capture.

## Decision

Option 3. For each run `rb_run_tapes` writes `<capture>.pads.jsonl`: one
`field` line (every pad's position and whether it is a full pad, in the game's
order, by y then x) and a `pads` line for the first packet after the start
state and for every later packet in which a pad's `is_active` flipped:
`{"frame", "car":[x,y,z,boost], "pads":[[index,is_active,timer],...]}`.
`tools/rb_tape_bot/spread_report.py <batch>` compares the runs of each tape
with each other (the game's spread) and with the port (best run).

## Consequences

### Positive

- Pad data accumulates on every batch for free, aligned by physics frame.
- Parity of a tape that the game does not repeat has a definition.

### Negative / tradeoffs

- The sidecar is keyed by RLBot physics frame, the capture by plugin
  timestamps; joining them uses the runner's start frame, which is not stored
  yet (the pad fit will add it if needed).

## Validation and revisit triggers

- Unit tests for the log (first packet, changes only, empty field skipped).
- Revisit when the plugin records pads itself, or when the pad fit shows the
  frame join is ambiguous.
