# Claude Code prompt: scenario-from-capture and seed-first-frame

Paste everything below the line into a new Claude Code session in this repository.

---

You are working in the `baileyrd/rusty_bullet` repository (Rust workspace; a port of Rocket League's physics, scored against the owner's BakkesMod recordings). Read `AGENTS.md`, `WORKFLOW.md` and `docs/research/BOT-CAPTURE-PLAN.md` first, then `docs/adr/0056-*.md` and `docs/adr/0057-*.md`.

## Context

`rb-verify --scenario <file>` runs a scripted scenario (initial state plus a run-length input tape, crate `rb_scenario`) through `rb_physics_bullet`; `rb-verify --scenario <file> --against <capture>` scores a recording of it. `tools/rb_tape_bot` (standalone package outside the workspace, RLBot v5) replays a scenario in the real game while the BakkesMod plugin records it. Captures are JSON Lines (ADR-0005), read by `rb_capture_ingest::CaptureFileSource`. The owner's own captures are in the gitignored `replays/` (for example `replays/test2.jsonl`); never commit them, and never copy personal data out of them.

## Task 1: `rb-verify --scenario-from <capture> <from-secs> <to-secs> [name]`

Write a scenario file to stdout from a window of a capture, so a human performance can be replayed exactly by the tape bot.

- The scenario's `car` state is the first car's state at the first frame in the window (location, rotation, velocity, angular velocity, boost). Rotation must be converted from the capture quaternion to RLBot `[pitch, yaw, roll]`. Add the inverse of `rb_scenario::rotator_to_quat` to `rb_scenario` (for example `quat_to_rotator`) and test it by round trip against `rotator_to_quat` over several rotations including pitch near ±90°, and against the recorded quaternion in the existing test `a_rotator_round_trips_a_recorded_quaternion`. Roll sign must match `rotator_to_quat`.
- The ball state is the ball's state at the same frame.
- `steps` is the run-length encoding of the recorded input per frame over the window (merge consecutive frames whose inputs are equal within 0.01). Frames without an input (replay-derived captures) are an error.
- `settle_ticks` is 0.
- Frames are 120 Hz in the game; check the capture's mean frame interval and fail with a clear message if it is not within 5% of 1/120 s.
- Add `Scenario` serialisation to `rb_scenario` (add `Serialize` derives; keep `from_json` unchanged) and a `to_json` that round trips with `from_json` (test it).
- Put the pure logic (window to scenario) in `rb_verify_cli`'s library (a new function, unit tested with a synthetic capture) and keep `main.rs` as argument parsing and printing only.
- Acceptance: generating a scenario from the window `test2.jsonl` 8.95 to 9.2 s and running `rb-verify --scenario <it> --against replays/test2.jsonl` reports position error under 5 uu for the first 20 ticks and zero recorded-input mismatches. Record the numbers you actually get in the spec, whatever they are.

## Task 2: `rb-verify --seed-first-frame`

`seed()` in `crates/rb_verify_cli/src/lib.rs` starts the simulation from the first grounded, neutral frame and errors if none exists, so a capture that starts airborne (state set by the tape bot) cannot be scored with `--self`, `--self-growth`, `--self-onestep` or `--self-kstep`. Add a global flag, accepted before the subcommand's capture argument or as the first argument (choose one, document it in `usage()`), that makes `seed()` use the capture's first frame instead. Keep the default behaviour unchanged. Test with a synthetic capture whose first frame is airborne: without the flag the existing error, with it a successful run.

## Rules (from AGENTS.md; treat a violation as a defect)

- No `unwrap`/`expect`/`panic` outside tests; `Result` and `?` with context.
- No new third-party dependency beyond what the workspace already uses; if you think you need one, stop and say why.
- Tests for all non-trivial logic: happy path plus a boundary or failure case.
- Docs: add a spec FR to `docs/specifications/verification/RB-VERIFY-003-divergence-scoring.md` (next numbers after FR-011, bump its version and change history), update `docs/specifications/SPEC-REGISTRY.md`, `docs/traceability/TRACEABILITY.md`, `CHANGELOG.md`, and write one ADR (`docs/adr/`, next number after the latest, template `docs/adr/TEMPLATE.md`) for the cycle. Update `tools/rb_tape_bot/README.md` and `docs/research/BOT-CAPTURE-PLAN.md` to mention both options.
- Before you commit, run exactly these and report their real output, with colour off so errors are not hidden by escape codes:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --color never -- -D warnings`
  - `cargo test --workspace`
  - `cargo doc --workspace --no-deps --color never`
  - `cd tools/rb_tape_bot && cargo clippy --all-targets --color never -- -D warnings && cargo test` (the tape bot is its own package and must keep building)
- Work on a branch named for the task; commit in small steps with clear messages. Do not open a pull request unless asked; if asked, use the matching template under `.github/PULL_REQUEST_TEMPLATE/` and justify any new dependency.

## Do not

- Do not change `rb_physics_bullet` behaviour; these tasks only add tooling. Run `rb-verify --self-kstep replays/test2.jsonl 30` before and after and confirm the printed car velocity error is unchanged (if `replays/` is absent, say so and skip).
- Do not run or modify RLBot, BakkesMod or the game; they are not available here.
- Do not commit anything from `replays/`.

## Report

End with: what you built, the acceptance numbers from Task 1 as measured, the command outputs above, anything you could not verify, and open questions.
