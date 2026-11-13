# Workspace pass: what is enforced, and where

Commit audited: `f80ee7fc5572de65928be3f7df019ae1f056c7e3`.
Sources read: root `Cargo.toml`, `deny.toml`, `rust-toolchain.toml`, `.config/nextest.toml`,
`.github/workflows/*.yml`, and the manifests of the crates in scope. No expert prompts, prior
reviews or audit reports were read.

## Crates in scope and their paths

| Name asked for | What it is in the workspace | Path(s) |
|---|---|---|
| `rusty_mill_board` | One package (lib + bin), plus a `runner` sub-package | `crates/apps/rusty_mill_board`, `crates/apps/rusty_mill_board/runner` |
| `rusty_mux` | No package of that name. A family of 11 packages: `rusty_mux_{client,core,ctl,gui,mcp,proto,script,server,status,transport,tui}` | `crates/apps/rusty_mux/crates/*` |
| `rusty_multimodal_db` | One package. `rusty_multimodal_db_engine` (`crates/libs/storage/`) is a separate package and is treated as a dependency, not audited | `crates/apps/rusty_multimodal_db` |
| `rusty_bullet` | No package of that name. A project: 12 `rb_*` packages plus one tool package | `crates/apps/rocket_league/crates/rb_*`, `crates/apps/rocket_league/rusty_bullet/tools/rb_tape_bot` |

## Config files that exist, and those that do not

| File | State |
|---|---|
| `[workspace.lints]` in root `Cargo.toml` | Exists, two lints (below). Opt-in only. |
| `clippy.toml` / `.clippy.toml` | **Absent.** No thresholds, no `disallowed-*` lists, no `allow-unwrap-in-tests`. |
| `rustfmt.toml` / `.rustfmt.toml` | **Absent.** rustfmt defaults. |
| `.cargo/config.toml` | **Absent.** No workspace `rustflags`. |
| `deny.toml` | Exists. |
| `rust-toolchain.toml` | Pins `1.98.1` with `rustfmt`, `clippy`. CI pins the same in `RUST_TOOLCHAIN`; `test_ci_workflow` fails if they differ. |
| `.config/nextest.toml` | `slow-timeout` 2m × 5, `retries = 2`, JUnit report. |

## Lints

### `[workspace.lints]` (root `Cargo.toml:1028-1032`)

| Lint | Level |
|---|---|
| `rust::unsafe_code` | warn |
| `clippy::undocumented_unsafe_blocks` | deny |

A crate gets these only with `[lints] workspace = true`. Eleven crates opt in (rustils' eight,
`rusty_conhost`, `rusty_atomic_file`, `rusty_confined_fs`, `coreutils`). **None of the crates in
scope opts in.** The comment above the table says so on purpose: "every other crate in this
workspace is unaffected".

### Per-crate `[lints]` tables in scope

| Crate(s) | `[lints.rust]` | `[lints.clippy]` |
|---|---|---|
| `rusty_mill_board`, `rusty_mill_board` runner | `missing_docs = warn`, `unsafe_code = forbid` | `todo = warn`, `unwrap_used = warn` |
| all 11 `rusty_mux_*` | none | none |
| `rusty_multimodal_db` | none | none |
| `rb_domain`, `rb_capture_ingest`, `rb_replay_ingest`, `rb_env`, `rb_physics_bullet`, `rb_scenario`, `rb_verify_cli` | `unsafe_code = deny` | `unwrap_used`, `expect_used`, `panic` = warn |
| `rb_rl_game_bridge`, `rb_rlbot_bridge`, `rb_rlbot_client`, `rb_rlbot_core`, `rb_rlbot_wire` | `unsafe_code = forbid` | `unwrap_used`, `expect_used`, `panic` = warn |
| `rb_tape_bot` | none | none |

CI runs clippy with `-D warnings`, so every `warn` above is an error in CI. A crate with no
`[lints]` table gets only clippy's default set (`clippy::all` at warn → error) and rustc's default
warnings.

## CI checks (`.github/workflows/ci.yml`)

`required-gate` is the one check branch protection requires; it fails if any job below failed or was
cancelled. A planned-out job is `skipped`, which passes. There is no `push` trigger: `pull_request`,
`merge_group`, a daily full sweep at 03:17 UTC, and manual dispatch.

### Workspace-wide (apply to every crate in scope)

| Check | Command | Scope |
|---|---|---|
| Format | `cargo fmt --all -- --check` | Whole workspace, every event |
| Clippy | `cargo clippy <-p affected…> --all-targets --all-features -- -D warnings` | Affected crates on a PR; whole workspace on the daily sweep. Linux and Windows on a PR; macOS added in the merge queue |
| Tests | `cargo nextest run <scope> --all-features --no-fail-fast --no-tests=warn` (retries 2, 10-minute cap per test) | Same scoping. Linux on a PR; Windows and macOS in the merge queue |
| Doctests | `cargo test --doc <scope> --all-features` | Lib crates in scope of the run |
| Advisories, licences, sources | `cargo deny --workspace check` | Whole workspace. `multiple-versions = allow`, `wildcards = allow`, unknown registry denied, unknown git warned, `unmaintained = "workspace"` |
| Unused dependencies | `cargo shear` | Whole workspace, with the ignore list in `[workspace.metadata.cargo-shear]` |
| Dependency sovereignty | `check_workspace_deps.py` | A workspace member may not also be pinned as a git dependency |
| Layer order | `check_workspace_layers.py` | `foundation → platform → libs → apps → tools`; no upward edges, no app edges across product families. Each manifest declares `[package.metadata.rusty_mill] layer` |
| MSRV honesty | `check_rust_versions.py`, then `check_msrv_compile.py` (`cargo check` at each crate's declared `rust-version`) | Crates whose own `Cargo.toml` changed; all on the full sweep |
| Workspace map | `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` | Whole workspace |
| Workflow lint | `actionlint` + `lint_workflows.py` | `.github/workflows/*.yml` |
| CI script tests | `python3 -m unittest discover -s .github/scripts` | CI planner and policy scripts |

Windows clippy and test legs exclude `rusty_journal`, `rusty_fedora_agent` and
**`rusty_multimodal_db`** (`ci.yml:528-541`): that crate has never been linted or tested on Windows.

### Crate-specific jobs that touch the scope

| Job | Crate | What it adds |
|---|---|---|
| `multimodal-db-feature-sets` | `rusty_multimodal_db` | Clippy `-D warnings` per feature set (default, `client`, `server`, `research`, `server,research`); `cargo test --features client`; `cargo doc --all-features --no-deps` with `RUSTDOCFLAGS=-D warnings` |
| `multimodal-db-msrv` | `rusty_multimodal_db` | `cargo check --all-features --all-targets` at the declared `rust-version` |
| `orch-bbp-acceptance` | `rusty_mill_board` | Builds the board binary and runs `--test inbox_real_bbp` against a real BBP host (Linux) |
| `rb-tape-verifier` | `rb_tape_bot`, `rb_verify_cli` | Python and PowerShell discovery regressions; a real `ScoreOnly` run on a recorded fixture (Windows) |
| `rl-live-test-safety` | rusty_bullet tooling | Mocked PowerShell orchestration tests (Windows) |
| `rleval-viewer-gl-smoke` | `rb_verify_cli` (as a tool) | `rb-verify --self <capture> --write-candidate` feeding the replay viewer |

No crate-specific job exists for any `rusty_mux_*` package: it gets fmt, default clippy, nextest and
doctests, nothing more. `.github/scripts/mux_egress_audit.sh` exists but no workflow calls it.

### Not enforced anywhere

- `clippy::unwrap_used` / `expect_used` / `panic` for `rusty_mux_*`, `rusty_multimodal_db`, `rb_tape_bot`.
- `clippy::expect_used` and `clippy::panic` for `rusty_mill_board`.
- `missing_docs` for anything except `rusty_mill_board`.
- `unsafe_code` for `rusty_mux_*`, `rusty_multimodal_db`, `rb_tape_bot`.
- `clippy::pedantic`, `clippy::cognitive_complexity`, `clippy::too_many_lines`, any nesting or size
  threshold (no `clippy.toml`).
- Rustdoc warnings, except for `rusty_multimodal_db`.
- Coverage, Miri, `cargo audit` (cargo-deny covers advisories), semver checks, `cargo udeps`.
- Banned duplicate versions or wildcard version requirements (`deny.toml` allows both).

## Conventions imposed from the workspace level

| Convention | Where it comes from | Effect on the crates in scope |
|---|---|---|
| First-party replacements for common third-party crates ("dependency sovereignty", ADR-0002) | `[workspace.dependencies]` path entries; `check_workspace_deps.py` | Scoped crates use `rusty_uuid`, `rusty_tokio`, `rusty_json`, `rusty_base64`, `rusty_rand`, `rusty_http`, `rusty_serve`, `rusty_request`, `rusty_tls`, `rusty_log`, `rusty_libc`, `rusty_wire`, `rusty_flatbuffers`, `rusty_atomic_file`, `rusty_aead`, `rusty_crypto_key`, `rusty_mcp_server`, `rusty_oauth*`, `rusty_sandbox`, `rusty_worktree` in place of the crates.io names. A fluent Rust author reaching for `tokio`, `uuid` or `base64` meets a different API. |
| Layered layout | ADR-0003; `check_workspace_layers.py` | All four are `apps`-layer. They may depend down, never on another app family. Shared code must move to `libs` or `foundation` first. |
| No shared error crate in use | `rusty_err` exists in `foundation` but no scoped crate depends on it | Each crate chooses its own error style. |
| No shared test-utility crate | `rusty_wiremock` exists in `libs`; no scoped crate uses it | Test helpers are per crate (`tests/common/mod.rs`, with `#![allow(dead_code)]`). |
| No shared runtime configuration | `rusty_config` exists; no scoped crate depends on it | Each binary reads its own environment variables and flags. |
| Literal `[package]` fields | Root `Cargo.toml:375-392` | Scoped crates do not inherit `[workspace.package]`; each states its own `edition`, `rust-version`, `license`. MSRV differs per crate (1.75 to 1.98.1). |
| Lints are per crate, not inherited | `[workspace.lints]` comment | Explains why four neighbouring projects have four different lint levels. |
| Tests must be process-isolated and tolerate retry | `.config/nextest.toml` | A flaky test passes on its third try; a real failure still fails. Tests cannot rely on in-process shared state. |
| Scoped CI | `plan` job (`ci_plan.py`, `affected_crates.py`) | A PR only lints and tests crates it touches and their dependents. Drift elsewhere shows up in the daily sweep, not on the PR. |
| Change fragments | `changes/README.md`, CONTRIBUTING | Every PR adds a fragment; not a Rust check. |
| Toolchain pin | `rust-toolchain.toml`, `RUST_TOOLCHAIN` | New default clippy lints arrive only with a deliberate bump. |
