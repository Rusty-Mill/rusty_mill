---
category: Fixed
changelog: The rb_tape_bot batch runner and scoring tools find rb-verify.exe in the monorepo's workspace target directory instead of assuming a standalone checkout.
---
## 2026-10-15 - rb_tape_bot: find rb-verify.exe in the monorepo layout

- **Fixed:** `run_batch.ps1` built `rb_verify_cli` but then looked for `rb-verify.exe` under
  `rusty_bullet/target/release/`, where a workspace build never puts it, so scoring failed with "not recognized
  as a name of a cmdlet" after a successful build. It now asks `cargo metadata` for the workspace target directory,
  falls back to the old path when that already holds the binary, and sets `RB_VERIFY` in its own process.
  Failed builds and invalid metadata or missing binaries now stop before scoring.
- **Fixed:** the 12 Python tools that hard-coded the same path (`score_dir.py`, `fuzz_report.py`, `regime_bias.py`,
  `kstep_regimes.py`, `duel_report.py`, `spread_report.py`, `ball_events.py`, `onestep_episodes.py`,
  `touchdown_errors.py`, `two_wheel_errors.py`, `wheel_facets.py`, `wheel_kstep.py`) read `RB_VERIFY` first and
  keep the old path as the default, so a standalone checkout behaves as before.
- **Verified:** deterministic runtime coverage exercises all 12 Python modules with unset, empty, and spaced
  verifier paths; PowerShell discovery tests cover native failures, stale binaries, target overrides, and invalid metadata.
  Hosted Windows CI runs `-ScoreOnly` against the existing matching `probe_reverse` scenario/capture, without launching a game.
- **Known limitations:** run the Python tools directly with `RB_VERIFY` explicitly set to the workspace binary.
  `run_batch.ps1` does not invoke the Python tools; `powershell -File` cannot export environment changes to its parent.
  The capture side (`rb_run_tapes` building the bot into `tools/rb_tape_bot/target`)
  already worked in the monorepo; only scoring was broken.
