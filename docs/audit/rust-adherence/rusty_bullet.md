# Rust adherence audit: rusty_bullet

| | |
|---|---|
| Crate | `rusty_bullet` (a project, not a package: 13 packages audited as one) |
| Paths | `crates/apps/rocket_league/crates/rb_{capture_ingest,domain,env,physics_bullet,replay_ingest,rl_game_bridge,rlbot_bridge,rlbot_client,rlbot_core,rlbot_wire,scenario,verify_cli}` and `crates/apps/rocket_league/rusty_bullet/tools/rb_tape_bot` |
| Commit audited | `f80ee7fc5572de65928be3f7df019ae1f056c7e3` |
| Path shorthand | Every `file:line` below is relative to `crates/apps/rocket_league/`. `crates/rb_x/...` is a package; `rusty_bullet/tools/rb_tape_bot/...` is the tool package. |

**Sizes** (`find <pkg> -name '*.rs' | xargs cat | wc -l`): rb_physics_bullet 24,898; rb_rl_game_bridge 7,887;
rb_verify_cli 4,935; rb_rlbot_core 3,400; rb_rlbot_wire 3,073; rb_env 1,854; rb_tape_bot 1,840; rb_rlbot_client 1,502;
rb_rlbot_bridge 1,028; rb_domain 995; rb_scenario 606; rb_replay_ingest 543; rb_capture_ingest 455. Total 53,016.

**Read in full (non-test source):** rb_domain, rb_capture_ingest, rb_replay_ingest, rb_scenario, rb_rlbot_wire
(`tables.rs` read to line 140 and 420-520, the rest is the same pattern), rb_rlbot_client, rb_rlbot_bridge, rb_env
(`lib.rs`, `flow.rs`), rb_verify_cli (`main.rs`, `lib.rs`, `scenario.rs`, `sweep.rs`), rb_tape_bot (all seven source files).
**Sampled:** rb_physics_bullet (`lib.rs`, the public surface and struct of `world.rs`, `world.rs:1221-1250` and
`1705-1879`, every assert and `#[allow]` site, plus grep and clippy output over all of it); rb_rl_game_bridge
(`ports.rs`, `launch.rs`, `link.rs:40-130` and `290-357`, `tick.rs:40-128`, spots in `game_wire.rs`); rb_rlbot_core
(`backend.rs`, `validation.rs`, `bin/rb_rlbot_core.rs`, `server.rs:1-418` and `487-600`, `sim.rs:300-330`). Test code
was counted, not read, except where a fix commit pointed at it.

**How counts were produced**

- "Library code" / "non-test" means files under `src/` (lib and bins), cut at the first `#[cfg(test)]` line of each file,
  and without `rb_physics_bullet/src/drive/tests.rs`. That is 24,964 lines. Excluded: `tests/`, `examples/`, in-source test
  modules. No package has `benches/`. Three files have a `#[cfg(test)]` helper before their test module
  (`rb_physics_bullet/src/arena.rs:550`, `drive/mod.rs:282`, `rb_rl_game_bridge/src/bin/rb_rl_demo_count.rs:137`), so
  a few lines of non-test code after those points are not counted.
  ```
  find <13 pkg dirs> -name '*.rs' -path '*/src/*' ! -name tests.rs | sort > srcfiles
  for f in $(cat srcfiles); do awk -v f=$f '/^\s*#\[cfg\(test\)\]/{exit} {print f":"NR":"$0}' $f; done > nontest.txt
  grep -cE '\.unwrap\(\)' nontest.txt        # and '\.expect\(', 'panic!\(', 'assert(_eq|_ne)?!\(' ...
  ```
- Lint counts are from clippy 1.98.1 on lib and bin targets only (no `--all-targets`, so test code is not compiled),
  de-duplicated by lint, file, line and column, restricted to files under `crates/apps/rocket_league/`:
  ```
  CARGO_TARGET_DIR=<scratch> cargo clippy --locked -p rb_domain -p rb_capture_ingest -p rb_replay_ingest -p rb_env \
    -p rb_physics_bullet -p rb_scenario -p rb_verify_cli -p rb_rl_game_bridge -p rb_rlbot_bridge -p rb_rlbot_client \
    -p rb_rlbot_core -p rb_rlbot_wire -p rb_tape_bot --message-format=json -- -W missing_docs -W clippy::indexing_slicing ...
  ```
  It built in under a minute. No `--fix`.
- Function length and nesting are from a brace-counting script over `nontest.txt` (893 functions found). Depth counts every
  `{` inside the body, including match arms and struct literals, so those numbers are marked "~".
- Test counts: `grep -rE '^\s*#\[test\]'` per package. History: `git log -- <13 paths>` (385 commits) and
  `git log -- crates/apps/rocket_league/rusty_bullet` (554 commits), commit bodies read for 45 fix-type commits.

---

## A. Adherence and drift per preference

### 1. Result and `?` over panics

Adheres in the twelve `rb_*` packages. Non-test code has 0 `.unwrap()`, 0 `.expect(`, 0 `panic!`, 0 `unreachable!`,
0 `todo!`. Every binary in those packages returns `ExitCode` or `Result` from `main`
(`crates/rb_rlbot_core/src/bin/rb_rlbot_core.rs:95`, `crates/rb_rlbot_bridge/src/bin/rb_chaser_bot.rs:16`,
`crates/rb_verify_cli/src/main.rs:598`).

Drift:

- rb_tape_bot: 13 `.expect(` in non-test code, all in binaries that an external process launches, so a missing
  environment variable is a panic with a backtrace instead of an exit message.
  `rusty_bullet/tools/rb_tape_bot/src/main.rs:25-27,99,109`, `src/bin/rb_tape_hive.rs:30-32,108,116`,
  `src/bin/rb_probe.rs:13,20,22`. The same package's `rb_run_tapes.rs:600` and `rb_match_log.rs:124` already use `Result`.
- rb_physics_bullet: 5 `assert!`/`assert_ne!` in non-test code. Three are argument checks in a public constructor
  (`crates/rb_physics_bullet/src/body.rs:238,243,244`), one in another (`src/net.rs:170`), one an internal invariant
  (`src/solver.rs:1037`). `clippy::panic` does not see `assert!`, so the crate's own lint table does not cover them.
- Panicking index, not flagged by any enabled lint: 283 `clippy::indexing_slicing` sites in non-test code
  (rb_physics_bullet 196, rb_verify_cli 43, rb_rlbot_core 22, rb_rl_game_bridge 11). Two that depend on a caller's
  argument: `crates/rb_physics_bullet/src/world.rs:924` (`self.car_inputs[index]`) and `:953`. Two that depend on the
  lengths of three vectors returned by another crate agreeing: `crates/rb_verify_cli/src/lib.rs:439-452`.
- `rusty_bullet/tools/rb_tape_bot/src/main.rs:82` and `src/bin/rb_tape_hive.rs:88` subtract two `u32` frame numbers
  from the game (`frame - start`). A frame counter that restarts panics in a debug build.
- `rusty_bullet/tools/rb_tape_bot/src/bin/rb_probe.rs:68` slices a `String` by byte count (`&name[..name.len().min(120)]`)
  where the text is a `Debug` print that includes player names. A multi-byte character at byte 120 panics.

### 2. Ownership and the type system; illegal states unrepresentable

Adheres:

- rb_rlbot_wire decodes every protocol enum through a checked conversion (`crates/rb_rlbot_wire/src/codec.rs:40-56`) and
  distinguishes absent from required fields (`codec.rs:62-88`).
- `rb_domain::ControllerInput` uses `Option<f32>` for axes a source cannot recover, with the reason written down
  (`crates/rb_domain/src/state.rs:247-272`).
- `SeedFrame` (`crates/rb_verify_cli/src/lib.rs:84`), `Pacing` (`crates/rb_rlbot_core/src/server.rs:49`), `Wait`
  (`crates/rb_rlbot_client/src/connection.rs:166`), `Phase`/`Transition` (`crates/rb_env/src/flow.rs:60,99`) are enums
  where a boolean would have done.
- `unsafe`: 0 occurrences.

Drift:

- `PhysicsWorld` keeps per-car state in 13 parallel vectors (`crates/rb_physics_bullet/src/world.rs:339-341,401-424,441`)
  that `with_car` must push in step (`world.rs:823-837`). `cars` is `pub` (`world.rs:339`) and the other twelve are
  private, so any caller can push a car and leave the rest short; `step` then indexes them (`world.rs:1223-1225`).
- Out-of-range car index is handled two ways in one type: silently ignored (`world.rs:842-856`, `:912`) and panic
  (`world.rs:924`, `:953`). `rb_env::Env::step` passes every input it is given straight to the panicking one
  (`crates/rb_env/src/lib.rs:273-279`).
- `MatchBackend::set_input` and `apply_desired` return `io::Result<bool>` (`crates/rb_rlbot_core/src/backend.rs:75,79`).
  The bool means "a car was unknown" in one (`crates/rb_rlbot_core/src/sim.rs:300-313`), and the server tells four
  outcomes apart by `io::ErrorKind` (`crates/rb_rlbot_core/src/server.rs:522-531,544-556`).
- A team is `u8` (`crates/rb_rl_game_bridge/src/link.rs:42`), `u32` (`crates/rb_rlbot_wire/src/types.rs:258`,
  `crates/rb_env/src/lib.rs:25`) and `usize` (`crates/rb_env/src/flow.rs:103`), compared with `!= 0`
  (`crates/rb_env/src/lib.rs:190`) and converted with `as` (`crates/rb_rlbot_bridge/src/bin/rb_chaser_bot.rs:34`).
- `ScenarioComparison` serves two modes with placeholder values: `recorded_ball_velocity: Vec3::ZERO` and
  `input_offset: None` in one (`crates/rb_verify_cli/src/scenario.rs:370-371,380`), `input_mismatches: 0` in the other
  (`scenario.rs:627`). A reader cannot tell "zero" from "not computed".
- `InterfaceMessage::Other(u8)` and `CoreMessage::Other(u8)` can be built by anyone and fail at encode time
  (`crates/rb_rlbot_wire/src/message.rs:53,79,143`).
- Connection lifecycle as three flags, with a helper to combine them (`crates/rb_rlbot_core/src/server.rs:121-128,136`).
- Invariants held only in prose: "every `Quat` in this codebase always is one [unit]" (`crates/rb_domain/src/state.rs:204`)
  with public fields; frames "chronologically ordered" (`crates/rb_domain/src/divergence.rs:60-61`) for a plain `Vec`.
- `IngestError::NotImplemented` (`crates/rb_domain/src/error.rs:12`) is never constructed in any of the 13 packages.
- `Scenario::from_json` rejects an empty tape (`crates/rb_scenario/src/lib.rs:126`), but all fields are public and
  `scenario_from_capture` builds one directly (`crates/rb_verify_cli/src/scenario.rs:685`).

### 3. Async only where there is real I/O concurrency

Adheres. No `async`, `.await` or runtime dependency anywhere. The client says why (`crates/rb_rlbot_client/src/lib.rs:8-9`).
The simulated core serves up to 64 clients from one thread with nonblocking sockets (`crates/rb_rlbot_core/src/server.rs:229-301`).
Cost of that choice: the server and client poll with short sleeps (`server.rs:284,295`, `crates/rb_rlbot_client/src/connection.rs:162,214`),
10 `sleep` calls in non-test code.

### 4. Explicit, flat, short functions

Adheres in the small packages: guard clauses and `let ... else` are the norm (`crates/rb_domain/src/divergence.rs:149`,
`crates/rb_verify_cli/src/scenario.rs:339-347`).

Drift (~ counts from the brace script, 893 functions):

- ~48 functions over 50 lines, ~14 over 100, 3 over 200. Longest: `PhysicsWorld::step` ~380 lines
  (`crates/rb_physics_bullet/src/world.rs:1221`), `main` ~343 (`crates/rb_verify_cli/src/main.rs:598`),
  `resolve_manifolds` ~206 (`crates/rb_physics_bullet/src/solver.rs:1499`), `compare_scenario_recorded` ~176
  (`crates/rb_verify_cli/src/scenario.rs:455`), `run_scenario_against` ~168 (`crates/rb_verify_cli/src/main.rs:303`).
- ~53 functions nest more than three brace levels (rb_rl_game_bridge 17, rb_physics_bullet 13, rb_rlbot_core 9,
  rb_verify_cli 6). Example with five control levels: `crates/rb_verify_cli/src/lib.rs:505-522`;
  `rusty_bullet/tools/rb_tape_bot/src/bin/rb_match_log.rs:124` is one ~99-line `main` with the whole loop inline.
- `crates/rb_verify_cli/src/main.rs:598-940` is a chain of `if first == "--mode"` blocks; the block
  `eprintln!(usage); return ExitCode::FAILURE` appears 25 times. An unknown flag falls through to the last branch and
  is opened as a replay file (`main.rs:915-927`).
- Hidden switches: four environment variables change what `rb-verify --scenario` prints (`main.rs:350,375,390,409`) and
  a fifth overrides an alignment result inside a library function, ignoring unparsable values
  (`crates/rb_verify_cli/src/scenario.rs:615-620`). None is in `usage()`.
- 24 function signatures take a `bool` (rb_physics_bullet 12, rb_rl_game_bridge 7, rb_env 4).
  `Env` has three on/off setters (`crates/rb_env/src/lib.rs:76,83,115`).
- The tick rate is written four ways: `TICK_SECS` (`crates/rb_env/src/lib.rs:18`), `SCENARIO_TICK_SECS`
  (`crates/rb_verify_cli/src/scenario.rs:10`), a literal `120.0` (`crates/rb_env/src/flow.rs:93`), and a nanosecond
  `TICK` (`crates/rb_rlbot_core/src/server.rs:23`).

### 5. Composition; no abstraction before two call sites

Adheres, and says so in the code. `PhysicsStateSource` has two implementers and the doc gives that as the reason it
exists (`crates/rb_domain/src/port.rs:1-4`). `MatchBackend` has the simulation and the real-game link. `Transport` exists
to script a clock in tests (`crates/rb_rlbot_client/src/connection.rs:175-180`). `Vec3` was not replaced by a maths
crate, on a stated rule (`crates/rb_domain/src/state.rs:7-15`). No trait objects for behaviour; `dyn` appears 12 times,
9 of them `Box<dyn Error>` in binaries.

Drift is the opposite failure, copy instead of extraction:

- The tape gate (phase filter, frame de-duplication, start-state push) is written twice:
  `rusty_bullet/tools/rb_tape_bot/src/main.rs:42-90` and `src/bin/rb_tape_hive.rs:55-99`. The tape loader too
  (`main.rs:24-39`, `rb_tape_hive.rs:29-52`).
- Start-frame search duplicated: `crates/rb_verify_cli/src/scenario.rs:269-288` and `:459-477`.
- `write_candidate` (`crates/rb_verify_cli/src/lib.rs:160`) is public and called only from tests.
- Nearly every module of rb_physics_bullet is `pub mod` (`crates/rb_physics_bullet/src/lib.rs:106-118`), so the solver's
  internals are public API.

### 6. Tests and docstrings

Tests adhere. 952 `#[test]` functions: rb_physics_bullet 520, rb_rl_game_bridge 122, rb_verify_cli 69, rb_rlbot_core 40,
rb_env 35, rb_rlbot_client 33, rb_domain 27, rb_rlbot_wire 26, rb_tape_bot 21, rb_rlbot_bridge 18, rb_replay_ingest 14,
rb_scenario 14, rb_capture_ingest 13. Two are `#[ignore]`d with reasons (`crates/rb_rlbot_core/tests/core.rs:938`,
`crates/rb_rlbot_wire/tests/verified.rs:343`). Failure cases are present where history demanded them
(`crates/rb_rlbot_wire/tests/required.rs`, `crates/rb_rl_game_bridge/tests/robustness.rs`,
`crates/rb_verify_cli/tests/write_candidate.rs`), there is a bit-for-bit determinism test
(`crates/rb_env/tests/determinism.rs`), and golden bytes from the reference implementation (`crates/rb_rlbot_wire/tests/golden.rs`).

Drift:

- `crates/rb_verify_cli/src/main.rs` (940 lines, all the argument parsing) has no unit test; only `--write-candidate`
  is covered from outside.
- Three tape-bot binaries have none: `rb_probe.rs`, `rb_tape_hive.rs`, `main.rs`.
- 22 `sleep` calls in test code, under a CI runner that retries a failing test twice.
- Docs: 497 `missing_docs` warnings on the public surface (rb_rlbot_wire 155, rb_rl_game_bridge 131,
  rb_physics_bullet 84, rb_domain 37, rb_rlbot_core 24, rb_scenario 22). `Vec3::dot` and the wire structs are obvious;
  `PhysicsStateSource::frames` (`crates/rb_domain/src/port.rs:17`) and `IngestError` (`crates/rb_domain/src/error.rs:8`) are not.
- 62 `clippy::missing_errors_doc`, 2 `missing_panics_doc`.
- Stale docs: `crates/rb_capture_ingest/src/lib.rs:11-16` and its `Cargo.toml:3` say the capture plugin "has not been
  built yet", while `rb_run_tapes` drives it (`rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:77-83`);
  `crates/rb_physics_bullet/Cargo.toml:3` says "scoped to sphere-vs-static-plane for v0"; `crates/rb_verify_cli/src/main.rs:5`
  says "Three modes" above a list of ten.

### 7. Never silently swallow errors

Adheres in the network code, where history forced it: accept failures are reported once and the loop continues
(`crates/rb_rlbot_core/src/server.rs:391-399`), a dropped client carries a reason (`server.rs:86-89`), pad-log write
failures are printed (`rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:520-522`), cleanup failure after a failed
run is printed (`rb_run_tapes.rs:480-482`), and a temp file that cannot be removed is reported
(`crates/rb_verify_cli/src/lib.rs:241-246`).

Drift:

- Bad option values become defaults without a word: `rusty_bullet/tools/rb_tape_bot/src/bin/rb_match_log.rs:126,127,147`,
  `:136-139` (any `--length` but `five` is unlimited), `src/bin/rb_probe.rs:32-35`,
  `crates/rb_verify_cli/src/scenario.rs:616`.
- `crates/rb_replay_ingest/src/bin/corpus_check.rs:35` drops unreadable directory entries with `filter_map(Result::ok)`.
- Errors lose their cause: `IngestError::Io(String)` (`crates/rb_domain/src/error.rs:17`) is built with `e.to_string()`
  and no path (`crates/rb_capture_ingest/src/lib.rs:36,41`, `crates/rb_replay_ingest/src/lib.rs:27`); it has no
  `source()`. 13 `clippy::map_err_ignore` sites. 15 function signatures return `Result<_, String>`.
- `IngestError::Malformed` is used for a bad argument (`crates/rb_verify_cli/src/lib.rs:185`) and `IngestError::Io`
  for "destination already exists" (`lib.rs:192`); the CLI then prints both as "ingestion failed" (`crates/rb_verify_cli/src/main.rs:932`).
- A score over zero matched frames is `0.0`, the same number as a perfect match
  (`crates/rb_domain/src/divergence.rs:170-174`); the caller must check `frames_compared`. A window with nothing to
  score returns zeros too (`crates/rb_verify_cli/src/sweep.rs:102-108`).
- `crates/rb_physics_bullet/src/collision.rs:422`: `let _ = on_box;` silences an unused computed value.

### 8. Dependencies, secrets, input validation

Dependencies adhere. Third-party crates across all 13 packages: `serde`, `serde_json`, `boxcars`, `subtr-actor`. Each
has a written justification in its manifest (`crates/rb_replay_ingest/Cargo.toml:21-35`,
`crates/rb_capture_ingest/Cargo.toml:21-26`). rb_domain and rb_physics_bullet have none beyond the workspace.

Secrets adhere. No credentials in source. Player names are hidden in probe output unless asked for, with a test
(`crates/rb_rl_game_bridge/tests/probe_privacy_cli.rs`); real replays are kept out of the repo
(`crates/rb_replay_ingest/src/bin/corpus_check.rs:4-6`).

Validation is uneven:

- Network input is validated: finite and ranged desired state (`crates/rb_rlbot_core/src/validation.rs:18-50`), clamped
  sticks (`validation.rs:53-69`), capped frames, strings, clients and queues (`crates/rb_rlbot_core/src/server.rs:27-45`).
- File input is not. A capture's timestamps become the physics step size unchecked
  (`crates/rb_physics_bullet/src/world.rs:1876`): a repeated or reordered line gives a zero or negative `dt`. No package
  checks capture timestamps for order, quaternions for length, or boost for range.
- Scenario and capture JSON accept unknown keys, and every scenario struct is `#[serde(default)]`
  (`crates/rb_scenario/src/lib.rs:14,28,37,55,69`), so a misspelt input name plays as neutral.
- `Scenario::total_ticks` sums `u64` tick counts from the file (`crates/rb_scenario/src/lib.rs:153-162,195`); two large
  values overflow.
- `window_secs` "must be positive" only in a doc comment (`crates/rb_domain/src/divergence.rs:236`); the CLI passes it
  through unchecked (`crates/rb_verify_cli/src/main.rs:844`). The tolerance check exists for `--self`
  (`main.rs:899`) and not for `--self-growth` (`main.rs:851`) or the replay mode (`main.rs:920`).
- `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:308-311` escapes `\` and `"` by hand to build JSON;
  control characters are not handled.

### 9. Explicit configuration, no magic globals, modular layout

Layout adheres: a dependency-free domain crate, adapters behind a port, one composition root
(`crates/rb_domain/src/lib.rs:1-8`). `Environment::from_lookup` takes the lookup as a parameter so it is testable
(`crates/rb_rlbot_client/src/env.rs:22`). Two statics only, both function-local
(`crates/rb_verify_cli/src/lib.rs:214`, `crates/rb_rl_game_bridge/src/bin/rb_rl_bridge.rs:171`).

Drift:

- Five undocumented environment switches (see 4), one read inside library code (`crates/rb_verify_cli/src/scenario.rs:615`).
- `rb_run_tapes` and `rb_match_log` hard-code the core address (`rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:33`,
  `rb_match_log.rs:31`) while the two bots in the same package read it from `Environment`. Port 23234 is written in five
  files.
- `Env` is configured by setters that take effect only at the next `reset` (`crates/rb_env/src/lib.rs:63-85,95-103`),
  and it writes a public field of the world directly (`lib.rs:97`).
- The verifier infers "unlimited boost" from the data and changes the simulation to match
  (`crates/rb_verify_cli/src/lib.rs:660-662,675-691`); nothing in the output says it did.

---

## B. Lints

Only what is not already enforced for these packages. Counts are non-test code (lib and bin targets).

### Enforced elsewhere in the project but not on rb_tape_bot

rb_tape_bot has no `[lints]` table (`rusty_bullet/tools/rb_tape_bot/Cargo.toml`).

| Lint | Hits | Examples |
|---|---|---|
| `clippy::expect_used` | 13 | `rusty_bullet/tools/rb_tape_bot/src/main.rs:25`, `src/bin/rb_tape_hive.rs:108`, `src/bin/rb_probe.rs:13` |
| `clippy::unwrap_used`, `clippy::panic` | 0 | not enforced; nothing to fix today |
| `unsafe_code` | 0 | not enforced; the other 12 deny or forbid it |

### Not enforced on any of the 13 packages

| Lint or check | Hits | Examples |
|---|---|---|
| `missing_docs` (rustc) | 497 | `crates/rb_domain/src/port.rs:17`, `crates/rb_domain/src/error.rs:8`, `crates/rb_domain/src/state.rs:18` |
| `clippy::arithmetic_side_effects` | 444 (313 in rb_physics_bullet) | `rusty_bullet/tools/rb_tape_bot/src/main.rs:82`, `src/bin/rb_tape_hive.rs:88`, `crates/rb_domain/src/divergence.rs:71` |
| `clippy::indexing_slicing` | 283 | `crates/rb_physics_bullet/src/world.rs:924`, `crates/rb_verify_cli/src/lib.rs:439`, `crates/rb_domain/src/divergence.rs:114` |
| `clippy::must_use_candidate` | 220 | `crates/rb_domain/src/divergence.rs:206`, `:238`, `crates/rb_domain/src/state.rs:30` |
| `clippy::print_stdout` / `clippy::print_stderr` | 93 / 53 (one in a library: `crates/rb_verify_cli/src/lib.rs:242`) | `crates/rb_rlbot_core/src/bin/rb_rlbot_core.rs:88`, `crates/rb_rl_game_bridge/src/bin/rb_rl_demo_count.rs:208` |
| `clippy::as_conversions` | 91 | `crates/rb_domain/src/divergence.rs:252`, `crates/rb_replay_ingest/src/convert.rs:156`, `crates/rb_rlbot_bridge/src/convert.rs:36` |
| of which `clippy::cast_precision_loss` | 32 | `crates/rb_domain/src/divergence.rs:173`, `:180`, `:261` |
| of which `clippy::cast_possible_truncation` | 25 | `crates/rb_domain/src/divergence.rs:252`, `crates/rb_physics_bullet/src/bvh.rs:56`, `:58` |
| of which `clippy::cast_sign_loss` / `cast_possible_wrap` | 5 / 3 | `crates/rb_physics_bullet/src/world.rs:1716`, `crates/rb_rlbot_core/src/sim.rs:123`, `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:89` |
| `clippy::missing_errors_doc` | 62 | `crates/rb_domain/src/port.rs:17`, `crates/rb_rlbot_wire/src/frame.rs:9`, `:19` |
| `clippy::missing_panics_doc` | 2 | `crates/rb_physics_bullet/src/body.rs:237`, `crates/rb_physics_bullet/src/net.rs:161` |
| `unreachable_pub` (rustc) | 27 (24 in rb_rl_game_bridge) | `crates/rb_rl_game_bridge/src/game_wire.rs:60`, `crates/rb_rlbot_wire/src/codec.rs:21`, `crates/rb_physics_bullet/src/drive/wheels.rs:247` |
| `clippy::map_err_ignore` | 13 | `crates/rb_rlbot_wire/src/frame.rs:10`, `crates/rb_rlbot_core/src/bin/rb_rlbot_core.rs:46`, `:70` |
| `clippy::too_many_lines` (default threshold 100) | 11 | `crates/rb_physics_bullet/src/collision.rs:309`, `:1505`, `crates/rb_physics_bullet/src/solver.rs:1225` |
| `clippy::cognitive_complexity` (default 25) | 1 | `crates/rb_physics_bullet/src/world.rs:1221` |
| `clippy::excessive_nesting` (needs `excessive-nesting-threshold` in a `clippy.toml`, which does not exist) | ~53 functions deeper than 3 by brace count | `crates/rb_verify_cli/src/lib.rs:505`, `crates/rb_rl_game_bridge/src/game_wire.rs:514`, `rusty_bullet/tools/rb_tape_bot/src/bin/rb_match_log.rs:124` |
| `clippy::wildcard_enum_match_arm` | 11 | `crates/rb_env/src/flow.rs:246`, `crates/rb_env/src/lib.rs:222`, `crates/rb_rl_game_bridge/src/game_wire.rs:559` |
| `clippy::wildcard_imports` | 6 | `crates/rb_rlbot_wire/src/tables.rs:12`, `crates/rb_rl_game_bridge/src/game_wire.rs:70`, `:427` |
| `clippy::partial_pub_fields` | 5 | `crates/rb_physics_bullet/src/body.rs:201`, `src/drive/wheels.rs:99`, `src/mesh.rs:219` |
| `clippy::struct_excessive_bools` / `fn_params_excessive_bools` | 4 / 1 | `crates/rb_rlbot_wire/src/types.rs:86`, `crates/rb_physics_bullet/src/drive/mod.rs:393`, `crates/rb_replay_ingest/src/convert.rs:71` |
| `clippy::allow_attributes_without_reason` | 4 in non-test code; all 62 `allow` attributes lack a reason | `crates/rb_physics_bullet/src/collision.rs:1382`, `:1504`, `:1709` |
| `clippy::let_underscore_must_use` | 3 | `crates/rb_rlbot_core/src/server.rs:322`, `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:405`, `:429` |
| `clippy::string_slice` | 1 | `rusty_bullet/tools/rb_tape_bot/src/bin/rb_probe.rs:68` |
| `clippy::disallowed_methods` for `std::env::var` / `var_os` outside `main` (needs a `clippy.toml` entry) | 1 in library code, 4 more in a binary's helper | `crates/rb_verify_cli/src/scenario.rs:615`, `crates/rb_verify_cli/src/main.rs:350`, `:375` |
| `clippy.toml` `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests` | would remove 54 of the 62 `allow` attributes below | |
| Rustdoc with `RUSTDOCFLAGS=-D warnings` (CI runs it for one other crate only) | not run; 306 `clippy::doc_markdown` hits suggest unlinked identifiers | `crates/rb_domain/src/divergence.rs:59`, `:64`, `crates/rb_domain/src/port.rs:12` |

Checked and clean, so no entry: `clippy::float_cmp`, `clippy::unwrap_in_result`, `clippy::panic_in_result_fn`,
`clippy::exit` in libraries, `clippy::unreachable`.

Two findings a lint finds but a human must judge: the `u32` frame subtractions (`arithmetic_side_effects`, A1), which
is the same class of bug commits `6adc03ca4` and `c431fbad2` fixed in `TickTracker`; and the three-vector indexing in
`crates/rb_verify_cli/src/lib.rs:439-452` (`indexing_slicing`).

### Every `#[allow(...)]` in the 13 packages (62)

None states a reason. "Enforced" means the package's own `[lints]` table warns on it, or it is a default clippy lint
that CI denies.

| Lint(s) switched off | Enforced? | Where | Reason given |
|---|---|---|---|
| `clippy::too_many_arguments` | yes (default clippy, `-D warnings`) | **non-test:** `crates/rb_physics_bullet/src/collision.rs:1382`, `:1504`, `:1709` | none |
| `clippy::type_complexity` | yes (default clippy) | **non-test:** `crates/rb_physics_bullet/src/solver.rs:748` | none |
| `clippy::too_many_arguments` | yes | test helpers: `crates/rb_physics_bullet/src/drive/tests.rs:81`, `:106`, `:133` | none |
| `clippy::unwrap_used` | yes (crate table) | test modules: `crates/rb_capture_ingest/src/wire.rs:230`; `crates/rb_domain/src/state.rs:300`, `src/divergence.rs:266`; `crates/rb_env/src/lib.rs:295`; `crates/rb_physics_bullet/src/drive/mod.rs:283`, `src/solver.rs:1707`, `src/mat3.rs:107`, `src/collision.rs:1956`; `crates/rb_replay_ingest/src/convert.rs:170`; `crates/rb_rl_game_bridge/src/link_backend.rs:281`, `src/ports.rs:126`, `src/rl_link.rs:773`, `src/link.rs:359`, `src/bin/rb_rl_game_probe.rs:431`, `src/bin/rb_rl_demo_count.rs:221`, `src/bin/rb_rl_bridge.rs:241`; `crates/rb_rlbot_client/src/connection.rs:263`; `crates/rb_rlbot_core/src/validation.rs:72`; `crates/rb_rlbot_wire/src/frame.rs:71`; `crates/rb_verify_cli/src/lib.rs:694` | none |
| `clippy::expect_used` | yes | test modules: `crates/rb_physics_bullet/src/mesh.rs:764`, `src/arena.rs:599`, `src/world.rs:1880`, `src/net.rs:391` | none |
| `clippy::unwrap_used, expect_used` | yes | test modules: `crates/rb_capture_ingest/src/lib.rs:67`, `crates/rb_replay_ingest/src/lib.rs:46` | none |
| `clippy::unwrap_used, panic` | yes | test module: `crates/rb_rlbot_core/src/sim.rs:505` | none |
| `clippy::unwrap_used, expect_used, panic` | yes | test modules: `crates/rb_rlbot_core/src/server.rs:802`, `crates/rb_scenario/src/lib.rs:343`, `crates/rb_verify_cli/src/scenario.rs:783` | none |
| `clippy::unwrap_used, expect_used, panic` (crate-level) | yes, except rb_tape_bot where none of the three is on | `crates/rb_rl_game_bridge/tests/census.rs:3`, `real_link.rs:2`, `bridge.rs:2`, `game_finding.rs:3`, `robustness.rs:3`, `real_core.rs:3`, `live_capture.rs:8`, `golden.rs:4`; `crates/rb_rlbot_bridge/tests/chaser_bot.rs:7`, `bridge.rs:2`; `crates/rb_rlbot_client/tests/connection.rs:3`, `agents.rs:3`; `crates/rb_rlbot_core/tests/sim.rs:2`, `core.rs:2`; `rusty_bullet/tools/rb_tape_bot/tests/processes.rs:6` | none |
| `dead_code, clippy::unwrap_used, expect_used, panic` | yes | `crates/rb_rlbot_client/tests/common/mod.rs:3` | none |
| `clippy::unwrap_used, panic` (crate-level) | yes | `crates/rb_env/tests/determinism.rs:4`, `crates/rb_rlbot_wire/tests/required.rs:7` | none |
| `clippy::unwrap_used` (crate- or fn-level) | yes | `crates/rb_rl_game_bridge/tests/probe_privacy_cli.rs:2`, `demo_count_cli.rs:2`; `crates/rb_rlbot_wire/tests/verified.rs:11`, `golden.rs:4`; `crates/rb_env/tests/full_match.rs:47` | none |
| `clippy::expect_used` (crate-level) | yes | `crates/rb_verify_cli/tests/write_candidate.rs:4` | none |
| `dead_code` | yes (rustc default) | `crates/rb_rlbot_wire/tests/fixtures/mod.rs:5` | none |

The four in non-test code are the ones to look at. History shows the alternative was known: commit `5680d6edd` removed
a `too_many_arguments` hit by grouping parameters into a struct; the three in `collision.rs` were silenced instead.

---

## C. General Rust rules

### Rules

1. When several collections are indexed by the same entity, store one collection of a per-entity struct. Do not keep
   parallel vectors that every insertion must remember to extend.
2. Do not make a field public when private fields of the same struct must stay in step with it.
3. Give one type one policy for an out-of-range index. Either every accessor returns `Option`/`Result`, or the index is
   checked once where it enters; never ignore it in some setters and panic in others.
4. When an operation has more than two outcomes, return an enum that names them. Do not carry domain outcomes in a
   bare `bool` inside `Result`, or in the `ErrorKind` of an I/O error that no I/O produced.
5. A public constructor that can reject its arguments returns `Result`. Keep `assert!` for conditions only a bug in
   the same module can break.
6. Keep the original error as `source()` and add what was being done and to what (operation, path). Do not flatten an
   error to a `String` at the first boundary it crosses.
7. Do not reuse an error variant for an unrelated failure because its payload is a string. Add a variant, and delete
   variants nothing constructs.
8. When a field is not computed in some mode, type it as `Option` or give each mode its own result type. Do not fill
   it with zero.
9. Check a precondition inside the function that depends on it, once. A check placed in one of several callers, or
   only in a doc comment, is not a check.
10. When deserialising a hand-edited file into structs whose fields all have defaults, reject unknown keys.
11. When a quantity has exactly two legal values, or one meaning across crates, give it one shared type. The same id
    as three integer widths produces `as` casts at every boundary.
12. Model a connection or session lifecycle as one enum of states, not as several booleans plus a helper that combines
    them.
13. If an enum variant exists only so decoding can report "something I do not model", keep it out of the type that
    encoding accepts.
14. Before writing a second copy of a state machine or parsing block for a second binary, move the first into the
    library both link.
15. When a lint objects to a signature, change the signature (group the parameters). If it is silenced instead, write
    the reason in the attribute.
16. Keep project status out of doc comments and manifest descriptions: "not built yet", "three modes", "v0 scope".
    They are not updated when the code is.
17. Define a physical constant once and derive its other forms from it.

### Evidence

| Rule | file:line or commit | What was seen |
|---|---|---|
| 1 | `crates/rb_physics_bullet/src/world.rs:339-341,401-424,441`, `:823-837` | 13 per-car vectors; `with_car` pushes to 9 of them by hand, the rest are sized elsewhere |
| 2 | `crates/rb_physics_bullet/src/world.rs:339` vs `:340-341`; `crates/rb_env/src/lib.rs:97` | `pub cars` beside private per-car vectors; another crate assigns `world.car_ball` directly |
| 3 | `crates/rb_physics_bullet/src/world.rs:842-856` vs `:924`, `:953`; `crates/rb_env/src/lib.rs:273-279` | two setters ignore a bad index, two panic; the caller passes unchecked input to a panicking one |
| 4 | `crates/rb_rlbot_core/src/backend.rs:75,79`; `crates/rb_rlbot_core/src/sim.rs:300-313`; `crates/rb_rlbot_core/src/server.rs:544-556` | `io::Result<bool>` where `true` means "unknown car", `InvalidInput` means rejected, `WouldBlock` means ignore |
| 5 | `crates/rb_physics_bullet/src/body.rs:237-248`; `crates/rb_physics_bullet/src/net.rs:161-173` | public constructors assert on mass, radius, grid size |
| 6 | `crates/rb_capture_ingest/src/lib.rs:36,41`; `crates/rb_domain/src/error.rs:17`; commits `0c03ee5f7`, `62f0e3e1c` | file open error printed with no path and no source; the client crate later had to add exactly this to find a Windows fault |
| 7 | `crates/rb_verify_cli/src/lib.rs:185,192`; `crates/rb_verify_cli/src/scenario.rs:270`; `crates/rb_domain/src/error.rs:12` | "tolerance must be finite" as `Malformed`, "destination exists" as `Io`, `NotImplemented` never built |
| 8 | `crates/rb_verify_cli/src/scenario.rs:370-371,380,627`; `crates/rb_domain/src/divergence.rs:170-174`; commit `c0285c38c` | zeros standing for "not computed"; tests later added to reject empty scores |
| 9 | `crates/rb_verify_cli/src/lib.rs:184` and `src/main.rs:899` vs `:851`, `:920`; `crates/rb_domain/src/divergence.rs:236`; `crates/rb_physics_bullet/src/world.rs:1876` | one check duplicated in two places and missing in two modes; "must be positive" only in prose; `dt` from file timestamps unchecked |
| 10 | `crates/rb_scenario/src/lib.rs:14,28,37,55,69`; `crates/rb_capture_ingest/src/wire.rs:73-78,193` | `#[serde(default)]` on every struct, no `deny_unknown_fields` |
| 11 | `crates/rb_rl_game_bridge/src/link.rs:42`; `crates/rb_rlbot_wire/src/types.rs:258`; `crates/rb_env/src/flow.rs:103`; `crates/rb_env/src/lib.rs:190`; `crates/rb_rlbot_bridge/src/bin/rb_chaser_bot.rs:34` | team as `u8`, `u32`, `usize`; `!= 0`; `as usize` |
| 12 | `crates/rb_rlbot_core/src/server.rs:121-128,136,511`; commit `8f917ca8e` | `agent_id`/`ready`/`awaiting_opening`; ordering bugs ("InitComplete before the connection settings") fixed by adding more flag tests |
| 13 | `crates/rb_rlbot_wire/src/message.rs:53,79,143` | `Other(u8)` is constructible and is a runtime error in `to_payload` |
| 14 | `rusty_bullet/tools/rb_tape_bot/src/main.rs:42-90` vs `src/bin/rb_tape_hive.rs:55-99`; `crates/rb_verify_cli/src/scenario.rs:269-288` vs `:459-477` | same gate and same search written twice |
| 15 | commit `5680d6edd`; `crates/rb_physics_bullet/src/collision.rs:1382,1504,1709` | fixed once by a parameter struct, silenced three times with no reason |
| 16 | `crates/rb_capture_ingest/src/lib.rs:11-16`; `crates/rb_physics_bullet/Cargo.toml:3`; `crates/rb_verify_cli/src/main.rs:5`; commits `3d48f5311`, `50142dc4f` | stale statements; two commits whose subject includes fixing doc drift |
| 17 | `crates/rb_env/src/lib.rs:18`; `crates/rb_verify_cli/src/scenario.rs:10`; `crates/rb_env/src/flow.rs:93`; `crates/rb_rlbot_core/src/server.rs:23` | tick rate as two constants, a literal and a nanosecond duration |

---

## D. Repo facts

| Fact | Proof |
|---|---|
| There is no `rusty_bullet` package. The project is 12 `rb_*` packages plus `rb_tape_bot`, which lives under the non-Rust tool tree and joined the workspace later. | `rusty_bullet/tools/rb_tape_bot/Cargo.toml:1-5`; commit `c0da4bf5d` ("rb_tape_bot excluded") |
| Lints are copied into each manifest, not inherited. rb_tape_bot was outside the workspace when that was done and never got the table. | commit `c0da4bf5d`; `rusty_bullet/tools/rb_tape_bot/Cargo.toml` |
| Two lint generations: the seven older packages `deny` unsafe code, the five RLBot packages `forbid` it. | `crates/rb_domain/Cargo.toml` vs `crates/rb_rlbot_wire/Cargo.toml` |
| `boxcars` must stay on 0.11.x and `subtr-actor` on `~1.2`; a fresh resolve gives two incompatible `boxcars::Replay` types. The lock file is what holds it. | `crates/rb_replay_ingest/Cargo.toml:31-35`; commit `c0da4bf5d` |
| Each package declares its own MSRV (1.75 to 1.88) and CI makes `clippy::incompatible_msrv` fatal, so one newer std call (`is_multiple_of`) raises a floor. | commit `0a9642e07`; `crates/rb_verify_cli/src/main.rs:507`, `Cargo.toml:6` |
| Lengths are Unreal units. Bullet constants are in metres at 50 uu per unit; a threshold copied without the factor made a driving car bounce on two ticks in three. | commit `cd5a0bec7` |
| `player_id` is assumed to equal the car's index in the world, although the replay adapter defines it only as "stable per replay". | `crates/rb_physics_bullet/src/world.rs:1871`; `crates/rb_verify_cli/src/scenario.rs:430`; `crates/rb_rlbot_core/src/sim.rs:312`; `crates/rb_replay_ingest/src/convert.rs:90-95` |
| The one RLBot rotator convention lives in the scenario-file crate, so the simulated core and the bot bridge depend on `rb_scenario` for two maths functions. | `crates/rb_rlbot_core/Cargo.toml:26-27`; `crates/rb_rlbot_bridge/Cargo.toml:25-26`; `crates/rb_scenario/src/lib.rs:228,282` |
| `rb_rlbot_core` depends on `rb_rlbot_bridge`, and `rb_rlbot_bridge` dev-depends on `rb_rlbot_core`: a deliberate dev-only cycle. | `crates/rb_rlbot_bridge/Cargo.toml:28-31` |
| Comments cite requirement and decision ids (`RB-PHYSICS-001-FR-nnn`, `ADR-00nn`) and commit subjects carry the same id. A physics change without one is out of pattern. | `crates/rb_domain/src/state.rs:219-220`; commits `f2bb268a9`, `b94447338` |
| A parameter sweep only nominates. A winner must pass the full-capture `--self-kstep 30` table before it touches the physics crate; one sweep winner cut one error from 51 to 8 and doubled another. | `crates/rb_verify_cli/src/sweep.rs:11-15`; `crates/rb_verify_cli/src/main.rs:546` |
| `--seed-first-frame` is only recognised as the first argument. | `crates/rb_verify_cli/src/main.rs:605` |
| The verifier always simulates with keyboard dodge behaviour and switches boost drain off when the capture looks like unlimited boost. | `crates/rb_verify_cli/src/lib.rs:659-662`; commits `b6ce04da0`, `7fce043d5` |
| Debug switches are environment variables: `RB_STATES`, `RB_BOOST`, `RB_BALL`, `RB_OTHERS`, `RB_FORCE_ALIGN`; the tape is passed as `RB_TAPE`. | `crates/rb_verify_cli/src/main.rs:350-409`; `src/scenario.rs:615`; `rusty_bullet/tools/rb_tape_bot/src/main.rs:25` |
| The tape tools are Windows-only in practice: `%APPDATA%`, `%LOCALAPPDATA%`, `.exe` run commands with backslashes. | `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:78,188,234` |
| RLBot core reports `Paused` in freeplay while physics runs, and counts the clock negative in overtime. Code must gate on the frame counter and on the overtime flag. | `rusty_bullet/tools/rb_tape_bot/src/main.rs:49-53`; commit `c6e357e8e` |
| The capture plugin drops a few ticks per capture, so anything that lines a capture up by row is wrong. | commit `14e13b39d` |
| Wire layouts are pinned to schema revision `c38374e` and checked against bytes from the reference implementation. | `crates/rb_rlbot_wire/src/lib.rs:26`; `crates/rb_rlbot_wire/tests/golden.rs` |
| Real replays are personal data and stay out of the repo; the corpus check is a no-op without a local `replays/`. Live recordings are committed with the player name replaced by a same-length placeholder. | `crates/rb_replay_ingest/src/bin/corpus_check.rs:4-10`; commit `4abfb3f02` |
| Tests run under a runner that retries twice, and 22 test-side sleeps exist. A timing test that fails one run in three still passes CI. | `docs/audit/rust-adherence/_workspace.md` (nextest `retries = 2`); commit `3d48f5311` |

---

## E. Non-Rust rules

### Networking

1. Do not use a socket receive timeout as a polling interval. Use a nonblocking read and your own deadline.
2. If an error code appears where a timeout was expected, do not map it to "timeout" until you have shown the
   connection is still usable afterwards.
3. On a socket whose reads are nonblocking, either switch to blocking before a write or queue the unsent bytes. A
   partial frame on the wire desynchronises everything after it.
4. A server bounds each client: queued bytes, bytes read per pass, accepts per pass, number of clients, time to
   introduce itself.
5. Keep reading from a peer while you wait for something else. A peer whose queue to you fills will drop you.
6. Handle everything already received before acting on a close or a read error.
7. Decide which connection is the one you are waiting for by what it sends, not by being first.
8. Name the failing call in every I/O error.

### Protocols and decoding

9. In a stream of incremental state, a message you cannot decode is fatal if it carries state. Skip only messages
   whose loss changes nothing later.
10. Decode required fields strictly and pin encoders to bytes produced by the reference implementation.
11. Do not trust the documented meaning of a field from an external system until it has been observed live. Gate on
    the field that was seen to behave.

### Numerics and determinism

12. When a constant is ported from another code base, convert its unit. Check every copied threshold for the unit it
    was written in.
13. Do not detect "time moved" from a floating-point seconds value in a long-running process. Use the integer
    counter, and decide in advance whether each counter wraps or saturates.
14. Compare two recordings by timestamp, not by row. Recorders drop samples.
15. Drive scripted playback from the authoritative frame counter, not from the number of messages received.
16. Test determinism directly: the same start and inputs in two instances, and in a reused instance after reset, must
    match bit for bit.
17. A parameter fitted on one window is a candidate. Accept it only after it holds on data it was not fitted on, and
    state a bound only as far as the data supports it.
18. Reject or clamp non-finite and out-of-range numbers at every boundary, files included, not only the network one.

### Files and storage

19. Before writing an output, prove it is not an input under another name (same path, symlink, hard link). Write
    beside the destination and publish only when complete, without overwriting.
20. Hand a file to another process by writing a temporary name and renaming.
21. A pattern that selects output files must not also match their sidecar files.
22. A path placed inside JSON or TOML needs that format's escaping. A Windows backslash is an escape character in both.

### CLI

23. An option value that does not parse is an error. Do not fall back to the default.
24. Validate every argument before the first side effect.
25. In a tool that stops something the user is running, do every step that can fail first.

### Testing

26. A test's stand-in peer must not close on an event that races the behaviour under test.
27. Every connection a test helper opens has a timeout, so cleanup cannot hang.
28. Check against the real system at least once, keep what it sent as fixtures, and remove personal names from them.

### Privacy

29. Diagnostic output hides other people's names unless the operator asks for them.

### Evidence

| Rule | Commit or file:line | What was seen |
|---|---|---|
| 1 | commit `87723e823`; `crates/rb_rlbot_client/src/connection.rs:187-190` | a 2 ms `SO_RCVTIMEO` on Windows left the connection in a documented-indeterminate state; replaced by nonblocking reads with 1 ms naps |
| 2 | commit `dfc54aec8`, reverted by `87723e823` | OS error 997 was mapped to a timeout, then the mapping was withdrawn as hiding a failure |
| 3 | commits `7317d5696`, `8f917ca8e`; `crates/rb_rlbot_client/src/connection.rs:54-58` | `write_all` failed with `WouldBlock` after part of a frame was sent; a partial write to the game socket desynchronised framing |
| 4 | commits `2480bf2ae`, `0a35b6044`; `crates/rb_rlbot_core/src/server.rs:27-45` | write queue cap, client cap, handshake timeout, per-pass accept and read budgets added after review |
| 5 | commit `47f84b219`; `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:445-453` | the runner stopped reading during a game launch and core dropped its session |
| 6 | commit `8f917ca8e` | a stop sent just before a client closed was lost with the client |
| 7 | commit `fc3a6b055`; `crates/rb_rl_game_bridge/src/link.rs:296-333` | the first connection was taken as the game; now the first message must decode as the game's |
| 8 | commits `0c03ee5f7`, `62f0e3e1c`; `crates/rb_rlbot_client/src/connection.rs:145-150` | a bare OS error could not be placed until the call was named |
| 9 | commit `2480bf2ae` reverted in part by `0a35b6044`; commits `e8262b6a8`, `e074ce40c` | skipping undecodable frames lost spawns, scores and pad state "for good"; made a fault |
| 10 | commit `c3534c9bc`; `crates/rb_rlbot_wire/tests/golden.rs`, `tests/required.rs` | 910 lines added for strict required fields and verified bytes |
| 11 | commit `c6e357e8e`; `rusty_bullet/tools/rb_tape_bot/src/main.rs:49-53`; commit `4abfb3f02` | overtime clock negative, freeplay reported as paused, untimed clock counts up |
| 12 | commit `cd5a0bec7` | 0.2 m/s copied as 0.2 uu/s; car airborne 24 ticks of 36 |
| 13 | commits `6adc03ca4`, `c431fbad2`; `crates/rb_rl_game_bridge/src/tick.rs:63-75` | two frames 20 hours in have equal `f32` seconds; counters overflowed at about 207 days |
| 14 | commit `14e13b39d`; `crates/rb_domain/src/divergence.rs:101-121` | a 2-tick hole read as a permanent 30 uu error |
| 15 | `rusty_bullet/tools/rb_tape_bot/src/main.rs:79-85` | tape indexed by physics frame so a dropped packet does not shift it |
| 16 | commits `2909bb821`, `fa937e047`; `crates/rb_env/tests/determinism.rs` | bit-for-bit comparison of two environments and of a reset one |
| 17 | `crates/rb_verify_cli/src/sweep.rs:11-15`; commits `3cff8fa54`, `fa1195657` | a sweep winner doubled another capture's error; a change "the real capture suggests" reverted; a claimed interval withdrawn |
| 18 | `crates/rb_rlbot_core/src/validation.rs:18-69` and commit `8f917ca8e` vs `crates/rb_physics_bullet/src/world.rs:1876` | network numbers checked after review; file timestamps still feed `dt` unchecked |
| 19 | commits `9c742edb3`, `80d9c55aa`; `crates/rb_verify_cli/src/lib.rs:190-248` | the candidate could be written over the capture it came from |
| 20 | `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:293-297` | `job.tmp` renamed to `job.json` so the plugin never reads half a file |
| 21 | commits `ab44cddc8`, `a3ddac82f` | `<name>_run*.jsonl` matched `.pads.jsonl` and aborted the batch |
| 22 | commits `281e924ab`, `42357bf21`; `rusty_bullet/tools/rb_tape_bot/src/bin/rb_run_tapes.rs:222,308-311` | a backslash path made a TOML file unparseable; JSON still escaped by hand |
| 23 | `rusty_bullet/tools/rb_tape_bot/src/bin/rb_match_log.rs:126,127,136-139,147`; `src/bin/rb_probe.rs:32-35`; `src/bin/rb_run_tapes.rs:121` | bad numbers become defaults; an unknown flag becomes a scenario name |
| 24 | commit `9c742edb3` ("validates every argument before writing anything"); `crates/rb_verify_cli/src/main.rs:880-902` | fixed for one mode |
| 25 | commit `cb21a7c3d` | the game was stopped before a build that could fail |
| 26 | commit `3d48f5311` | the stand-in closed after the bot's second packet; the bot sometimes saw two, not three |
| 27 | commit `7ca075b61` | a test client connected without a timeout and the cleanup join was unbounded |
| 28 | commit `4abfb3f02`; `crates/rb_rl_game_bridge/tests/live_capture.rs` | four behaviours found only against the live game; recordings committed with the name replaced |
| 29 | `crates/rb_rl_game_bridge/tests/probe_privacy_cli.rs`; commit `38ef62b8d` | names hidden by default, with a CLI test |
