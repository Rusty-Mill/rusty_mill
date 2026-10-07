# ADR-0066: Golden captures in CI

- Status: Accepted (2026-10-07)
- Date: 2026-10-07
- Deciders: baileyrd
- Related: `RB-VERIFY-003-FR-016`, `RB-VERIFY-003-FR-017`, ADR-0059, ADR-0064,
  ADR-0065, `docs/research/BOT-RUN-SHEET.md` (session 3)
- Supersedes/Superseded by: none

## Context

The unattended tape runs (ADR-0064) give real Rocket League ground truth on
demand, and replaying the recording's own input in the port
(`--recorded-inputs`, FR-016) turns each into a pure physics comparison. On
2026-10-07 that took `speed_flip` from 87 uu to 0.8 uu and showed most
mechanics agreeing to well under 1 uu. Nothing in CI protected those gains:
a later change could undo them and every workspace test would still pass,
because the existing tests check the port against itself and against
hand-written expectations. The ground truth lived in `replays/`
(gitignored, personal data for human recordings).

## Decision drivers

- A regression against the real game must fail a test, not wait for a
  manual re-scoring.
- No new dependency; small repository growth.
- The captures must hold no personal data.

## Considered options

1. Keep scoring by hand (`run_batch.ps1`) after physics changes.
2. Check trimmed captures into the repository and gate on them in
   `cargo test`.
3. Compress and store the full captures elsewhere.

## Decision

Option 2. `tools/rb_tape_bot/fixtures/<scenario>.jsonl` holds each recording
trimmed to its tape (`make_fixture.py`, 70 to 360 KB each, 30 files, 5.1 MB).
`crates/rb_verify_cli/tests/golden_captures.rs` replays each fixture's
recorded input in the port and asserts car (and, where the scenario moves it,
ball) mean and max position error stay within a bound: the 2026-10-07
measurement plus a quarter. A second test shows a deliberately shifted
recording exceeds the bound, a third that every fixture has a case. The
captures are of a scripted bot, so they carry no personal data (unlike the
owner's replays, which stay gitignored).

## Consequences

### Positive

- Undoing FR-139 fails the gate (`speed_flip` 89.7 uu, `jumpgap_H3_R3` 77.5
  uu over bounds of about 1), checked by reverting it.
- Improving the port means lowering a bound, so the gain is kept.

### Negative / tradeoffs

- 5.1 MB of fixtures, and about 3 s of test time (one thread per case).
- Bounds are a quarter over one measurement of a deterministic recording;
  a legitimate model change that trades error between scenarios must move
  several bounds in the same PR and say why.
- The fixtures were recorded with one game build; a game update can change
  the physics, and the right response is to re-record, not to loosen a bound.

## Validation and revisit triggers

- Revisit if fixtures grow past about 10 MB (trim further or move to Git
  LFS), or if a game update makes recordings disagree with the port in
  scenarios that agreed.
