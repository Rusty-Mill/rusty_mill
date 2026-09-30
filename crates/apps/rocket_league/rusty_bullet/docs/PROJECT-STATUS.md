# Project Status

Current state only. Per-change history lives in
[RELEASE_NOTES.md](../RELEASE_NOTES.md) and [CHANGELOG.md](../CHANGELOG.md);
per-unit status in [ROADMAP.md](./roadmap/ROADMAP.md),
[SPEC-REGISTRY.md](./specifications/SPEC-REGISTRY.md), and
[TRACEABILITY.md](./traceability/TRACEABILITY.md). The long-form
"Completed" log this file used to carry is in git history (last full
version: `a245d35`).

- Last verified main commit: `a245d35` (merge of [#165](https://github.com/baileyrd/rusty_bullet/pull/165))
- Verified at: 2026-09-30
- Current milestone: `PHASE-1-PHYSICS-CORE` (In Progress) — mechanics are
  broad (box cars, multi-contact solve, full standard arena with fillets,
  goals and nets, driving, boost, handbrake, jumps, dodges, wall jumps,
  air control); real-data fidelity is not yet established (see
  Validation). `RB-PHYSICS-001-FR-005` (real-data constant calibration)
  has not started.
- Health: green — workspace builds, `fmt`/`clippy`/`test` all pass on `main`

## Phases

| Phase | Status |
|---|---|
| `PHASE-0-BOOTSTRAP` | Done |
| `PHASE-0-REPLAY-INGEST` | Done |
| `PHASE-0-CAPTURE-INGEST` | Done |
| `PHASE-0-EXIT` | Done |
| `PHASE-1-PHYSICS-CORE-V0` | Done (v0 scope) |
| `PHASE-1-PHYSICS-CORE` | In Progress |
| `PHASE-2-DETERMINISM` | Not Started |
| `PHASE-3-NETCODE` | Not Started |
| `PHASE-4-POLISH` | Not Started |

## Recently completed

- [#165](https://github.com/baileyrd/rusty_bullet/pull/165) — per-car drive
  state grouped into `drive::DriveState`; refactor, no behavior change.
- [#164](https://github.com/baileyrd/rusty_bullet/pull/164) — `drive.rs`
  split into `drive/{ground,air,jump,boost}`; `AGENTS.md` crate map fixed.
- [#163](https://github.com/baileyrd/rusty_bullet/pull/163) —
  `RB-VERIFY-003-FR-004`, divergence-growth diagnostic
  (`rb-verify --self-growth`); sanity-checked on the synthetic fixture only.
- [#162](https://github.com/baileyrd/rusty_bullet/pull/162) —
  `RB-PHYSICS-001-FR-077`'s first real-capture fidelity number recorded
  (see Validation).
- [#161](https://github.com/baileyrd/rusty_bullet/pull/161) —
  `RB-PHYSICS-001-FR-078`, car hitbox tests calibrated to
  `CAR_HALF_EXTENTS`.

## In progress

- None.

## Blocked

- `RB-RESEARCH-O002` (binary reverse engineering of the shipped Rocket
  League client) — blocked on two things: (1) explicit owner sign-off after
  a legal/practical review, and (2) practically, this sandboxed environment
  has no access to the Rocket League client binary at all, so any actual RE
  work would have to happen on the owner's own machine. See
  `docs/research/RESEARCH-BACKLOG.md`.
- `RB-VERIFY-001`'s stricter manual single-timestamp cross-check (one ball
  position pinned against a remembered/verified instant, e.g. via in-game
  footage or BakkesMod) — the local `corpus_check` gate (40/40 real owner
  replays, see Validation) already closes the "runs correctly on real owner
  data at scale" half of this criterion; this narrower, precision-focused
  half is still open and needs the owner to do the manual cross-check
  locally, since this sandbox has no way to verify an exact remembered
  timestamp.
- `RB-VERIFY-002`'s manual BakkesMod-overlay single-timestamp cross-check
  (one physics value pinned against what BakkesMod's own overlay/logging
  reports for that same instant) — same shape as, and still open for the
  same reason as, `RB-VERIFY-001`'s equivalent item above: needs the owner
  to do it locally, since this sandbox has no way to verify an exact
  remembered timestamp.

## Next

1. Running the now-implemented `RB-VERIFY-003-FR-004` divergence-growth
   diagnostic (`rb-verify --self-growth`) against `FR-077`'s own real
   capture (`test2.jsonl`) on the owner's machine — the run that would
   actually show whether that run's divergence grew gradually (many
   small modeling errors compounding, pointing at broad constant
   calibration) or abruptly (one specific early mechanic mismatch
   derailing the whole run, pointing at a targeted fix instead). Only
   sanity-checked so far against the synthetic capture fixture (see
   Validation); recommended before `RB-PHYSICS-001-FR-005` (real-data
   constant calibration) starts, since blind curve-fitting against a
   fully-decorrelated trajectory isn't sound.
2. (Optional, owner-side, non-blocking) The manual BakkesMod-overlay
   single-timestamp cross-checks for `RB-VERIFY-001`/`RB-VERIFY-002` (see
   Blocked).

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (397 tests: 27 `rb_domain`, 337
  `rb_physics_bullet`, 14 `rb_replay_ingest` (incl. real-fixture
  integration test), 10 `rb_capture_ingest`, 9 `rb_verify_cli`)
- `cargo run -p rb_replay_ingest --bin corpus_check` (local only, not CI):
  40/40 real owner replays parsed cleanly, 2026-08-28
- `cargo run -p rb_verify_cli --bin rb-verify -- <replay> <capture>`
  (manual, 2026-08-28, default 0.02s timestamp tolerance): `frames
  compared: 6, mean ball distance: 0.25 uu, max ball distance: 0.25 uu,
  car pairs compared: 6, mean car position/rotation/velocity distance:
  2816.42 uu / 2.36 rad / 1307.87 uu/s` against the real replay fixture +
  (now time-aligned) synthetic capture fixture.
- `cargo run -p rb_verify_cli --bin rb-verify -- --self test2.jsonl`
  (manual, owner's machine, 2026-09-04, default 0.02s timestamp
  tolerance, `RB-PHYSICS-001-FR-077`'s own real-capture run): `frames
  compared: 2818, mean ball distance: 2206.08 uu, max ball distance:
  5673.98 uu, car pairs compared: 2818, mean car position/rotation/
  velocity distance: 4508.71 uu / 2.12 rad / 1421.73 uu/s, max car
  position/rotation/velocity distance: 8798.56 uu / 3.14 rad / 3643.64
  uu/s` — this project's first genuine fidelity number (candidate
  actually simulated from the real capture's own recorded input, not an
  unrelated match); see FR-077's entry in `RELEASE_NOTES.md` and
  `RB-PHYSICS-001`'s Interpretation note for what this large a divergence
  does and doesn't establish.
- `cargo run -p rb_verify_cli --bin rb-verify -- --self-growth
  crates/rb_capture_ingest/fixtures/example.capture.jsonl` (manual,
  2026-09-04, default `window_secs = 1.0`, `RB-VERIFY-003-FR-004`): `t=
  11.78s frames= 5 ball mean/max= 0.75/ 2.17 uu car mean pos/rot/vel=
  58.75 uu / 0.05 rad / 600.40 uu/s` — a single window, since the
  fixture's own 5 frames all fall within one second; confirms the new
  `--self-growth` CLI mode runs end-to-end. Not the diagnostic's real
  purpose — running it against `FR-077`'s own real capture is still
  pending the owner's own machine (see Next).

## Risks and decisions needed

- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
