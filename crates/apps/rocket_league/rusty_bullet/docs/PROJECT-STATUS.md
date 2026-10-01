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

- [#178](https://github.com/baileyrd/rusty_bullet/pull/178) —
  `RB-PHYSICS-001-FR-086`, steering by per-wheel side impulses (ADR-0016).
- [#177](https://github.com/baileyrd/rusty_bullet/pull/177) — spin in
  `--self-trace` (`RB-VERIFY-003` 0.12.0) and `RB-PHYSICS-001-FR-085`,
  flip torque on the press tick.
- [#176](https://github.com/baileyrd/rusty_bullet/pull/176) —
  `RB-PHYSICS-001-FR-084`, RocketSim air control (ADR-0015).
- [#175](https://github.com/baileyrd/rusty_bullet/pull/175) —
  `RB-PHYSICS-001-FR-083`, the flip as RocketSim has it (ADR-0014); flip
  vz stall matches the recording.
- [#174](https://github.com/baileyrd/rusty_bullet/pull/174) —
  `RB-PHYSICS-001-FR-082`, RocketSim stick signs and dodge impulse
  (ADR-0013); real-capture position error at 5.0 s 1,590 to 79 uu.
- [#173](https://github.com/baileyrd/rusty_bullet/pull/173) —
  `RB-PHYSICS-001-FR-081`, per-axis tire grip (ADR-0012); real-capture
  velocity error at 4.0 s ~250 to 9.9 uu/s.
- [#172](https://github.com/baileyrd/rusty_bullet/pull/172) —
  `RB-PHYSICS-001-FR-080`, curve-based steering (ADR-0011); real-capture
  heading error at 4.0 s 0.46 to 0.06 rad.
- [#171](https://github.com/baileyrd/rusty_bullet/pull/171) —
  `RB-PHYSICS-001-FR-079`, restitution threshold in uu; ended the
  driving-car hop.
- [#170](https://github.com/baileyrd/rusty_bullet/pull/170) —
  `RB-VERIFY-003-FR-005`, per-frame trace (`rb-verify --self-trace`).
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

- `RB-PHYSICS-001-FR-087` (car spin clamped after the transform) and
  `RB-VERIFY-003` 0.13.0 (`q-rate`): implemented; awaiting the owner's
  re-trace.

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

1. Re-run `rb-verify --self-trace <test2.jsonl> 4.3 5.6` and
   `--self-growth` to confirm `RB-PHYSICS-001-FR-087`: the simulated
   `q-rate` during the flip should reach ~7.5 rad/s like the recording, and
   orientation error through 4.3-4.95 s should stay small. Smaller leftovers:
   steady yaw rate ~5% low (2.28 vs 2.40 rad/s near 4.0 s), and a ~3 deg
   flip-axis offset (orientation error 0.04 to 0.58 rad over 4.32-4.6 s). Known small
   gap: for ~0.05 s after the 4.142 s jump the recorded car's horizontal
   velocity keeps turning (wheels likely still touch via suspension).
   Later: the real hitbox sits
   20.755 uu above the car origin (`hitboxPosOffset`), ~18 uu above the
   floor at rest, while the port's box rests on the floor.
2. (Optional, owner-side, non-blocking) The manual BakkesMod-overlay
   single-timestamp cross-checks for `RB-VERIFY-001`/`RB-VERIFY-002` (see
   Blocked).

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass
- `cargo test --workspace`: pass (419 tests: 27 `rb_domain`, 354
  `rb_physics_bullet`, 14 `rb_replay_ingest` (incl. real-fixture
  integration test), 10 `rb_capture_ingest`, 14 `rb_verify_cli`)
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
- `rb-verify --self-growth test2.jsonl` (owner's machine, 2026-10-01,
  `RB-VERIFY-003-FR-004`'s real run, 1 s windows): car error ~2 uu /
  0.01 rad for 0-3 s; 34 uu at 3 s; 1,315 uu / 1.37 rad / 2,887 uu/s at
  4 s; ball within 0.05 uu until 5 s, when a car reaches it in one run
  but not the other. Abrupt, car-only derailment: a targeted mechanic
  fix, not broad `RB-PHYSICS-001-FR-005` calibration, comes first.
- `rb-verify --self-growth test2.jsonl` after `RB-PHYSICS-001-FR-079`
  (owner's machine, 2026-10-01): 3-4 s window 10 uu / 67 uu/s (was 34 uu
  / 164 uu/s); 4-5 s window 751 uu / 1.21 rad / 1,902 uu/s (was 1,315 uu
  / 1.37 rad / 2,887 uu/s); ball 0.01 uu (was 0.05) until a car reaches
  it. `--self-trace 4.10 4.20`: the jump fires as a jump (sim vz 286 vs
  recorded 296), no sideways dodge; remaining error is heading (~0.45 rad)
  and ~10% speed.
- `rb-verify --self-trace test2.jsonl 3.7 4.3` after
  `RB-PHYSICS-001-FR-080` (owner's machine, 2026-10-01): orientation error
  at 4.0 s 0.06 rad (was 0.46), at most 0.09 rad through 4.13 s; velocity
  error at 4.10 s 364 uu/s (was 563); jump still correct (vz 286 vs 296).
  Remaining: velocity direction ~53 deg vs 72 deg recorded, speed ~10% low.
- `rb-verify --self-trace test2.jsonl 3.7 4.3` after
  `RB-PHYSICS-001-FR-081` (owner's machine, 2026-10-01): velocity error at
  4.0 s 9.9 uu/s (was ~250), simulated (325, 968) vs recorded (318, 961).
  `--self-growth`: 3-4 s window 2.8 uu / 0.03 rad / 5.7 uu/s (was 10 uu /
  67 uu/s); 4-5 s 532 uu / 1.15 rad / 1,574 uu/s (was 751 / 1.21 /
  1,902). With the zero-steer yaw fix: velocity error at 4.10 s 22 uu/s
  (was 96), at 4.30 s 38 uu/s (was 132); position error at most 6 uu
  through 3.7-4.3 s.
- `rb-verify --self-trace test2.jsonl 4.3 5.0` after
  `RB-PHYSICS-001-FR-083` (owner's machine, 2026-10-01): flip vz stall
  -15.5 uu/s from 4.55 s, matching the recording (was falling to -211);
  vz at 5.0 s -23 (recorded -4, was -211); position error at 5.0 s 79 uu;
  orientation error at 5.0 s 1.41 rad (was 1.63). Damping starts one tick
  early, likely because the trace's per-pair dt differs from RocketSim's
  fixed 1/120.
- After `RB-PHYSICS-001-FR-084` (owner's machine, 2026-10-01):
  `--self-growth` 4-5 s window 30 uu / 0.55 rad / 95 uu/s (was 1,315 uu /
  1.37 rad / 2,887 uu/s at the start of the day), 5-6 s 219 uu (was
  2,660); ball diverges from 5 s. `--self-trace 4.3 5.0`: orientation
  error at 5.0 s 1.36 rad (was 1.41), at 4.30 s 0.21 (was 0.14).
- After `RB-PHYSICS-001-FR-086` (owner's machine, 2026-10-01):
  `--self-growth` 3-4 s 2.5 uu / 0.01 rad / 5.2 uu/s; 4-5 s 16 uu / 0.48
  rad / 35 uu/s (was 30 / 0.55 / 95); 5-6 s 157 uu (was 219).
  `--self-trace 3.7 4.6`: yaw spin ramps with the recording (0.27 vs
  0.31, 0.92 vs 1.06, 1.61 vs 1.80 rad/s over the first 9 ticks after
  3.742 s); orientation error 0.04 rad at the dodge (was 0.22); spin error
  just before it 0.10 rad/s (was 0.71); velocity error after it 36 uu/s
  (was 128).
- `rb-verify --self-trace` against the synthetic capture fixture
  (2026-10-01, `RB-VERIFY-003-FR-005`): runs end-to-end; shows a
  recorded ground jump (t=0.15 s) firing as a dodge in the candidate
  (see Next).

## Risks and decisions needed

- `RB-RESEARCH-O002` (binary reverse engineering) — needs explicit owner
  sign-off after legal/practical review before any work starts, and needs
  the owner's own machine/game install since this sandbox has neither.
  Owner: baileyrd.
