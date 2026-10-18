# Milestone 2 implementation validation — 2026-10-18

Base: `25dd95173a1fee869411565092e63c2eacf01f40` (PR #783), verified against
GitHub's current `main` at implementation start. Toolchain: Rust/Cargo 1.98.1;
Python 3.12. This is an honest local checkpoint, **not a claim of green CI or a
completed sandboxed/live-provider proving run**.

## Executed results

| Check | Observed result |
| --- | --- |
| Maintenance module, recovery/identity/approval/publication tests, existing argument and inspection unit tests | 19 passed; sandboxed end-to-end case ignored pending host prerequisites |
| Real Cargo/Git maintenance integration | 2 passed: deterministic read-only discovery; open-PR conflict refuses before configuration/task creation |
| Actual Git draft-publication path | Included above: private worktree, exact candidate tree, commit and push to local bare origin, GitHub CLI seam, second invocation adopts the same commit/PR |
| Frozen Python scope gate against real staged Git patches | 4 passed: beneficial layer repair and identical hoist accepted; unrelated/version/feature/deletion/wrong-layer changes refused |
| BBP durable store and fenced human actions | 12 passed, including reopened task inventory, interrupted log recovery, competing writers and approval/receipt replay |
| Orch BBP/Claude/CLI regression invocation | 34 passed, 1 failed, 11 ignored; process-group failure described below |
| Remaining Orch core/dispatch/Codex/Ollama regression invocation | 92 passed, 4 ignored |
| Clippy on the maintenance library/integration target | Passed with `-D warnings` |
| Repository metadata, without resolving dependencies | `cargo metadata --no-deps --format-version 1 --offline` passed on the actual full workspace |
| Repository intelligence Python suite | 12 passed, 1 platform-specific skip |
| Existing layer/dependency policy Python suites | 15 and 17 passed |
| Acceptance collector tests / workflow contract tests | 3 and 60 passed |
| Formatting / change fragments / whitespace | Passed |

The Rust results above use a separate offline validation checkout containing the
first-party dependency scope. Its Cargo workspace excludes unrelated
members, removes uncached external *dev* oracles, and excludes `orch-store` and its
uncached embedded-engine dependencies from the command library's composition.
The actual maintenance/inspection/argument modules, BBP adapter/protocol/store,
and tested core/provider adapters are copied unchanged. No production store or
adapter was stubbed in the repository. This checks the new code and existing
adapter contracts; it does **not** establish that the original complete binary,
store composition or full workspace compiled. See commands below and the CI
acceptance lane for the required complete-environment verification.

The unchanged `orch-cli::timeout_kills_the_whole_group_and_joins_within_the_bound`
test timed out/killed its process group correctly, then failed its final liveness
assertion. `ps` showed both remaining `sleep` processes in zombie state with
PPID 1 (PGID 6674), rather than executing children. This container's PID 1 had not
reaped them. The failure is retained as a failure; the test was not weakened or
silently counted as passing.

## Blocked or unexecuted

- Full workspace tests fail dependency resolution: cached `boxcars` is missing.
- Complete focused Orch composition requires uncached registry dependencies
  (`proptest` at resolution, then the embedded engine's serde/bincode/etc.).
- Building the real BBP host in the offline scope requires uncached `libc`.
- Git fetch and package fetch cannot reach the configured proxy. A requested
  network-enabled build was not executed: automatic approval review failed to
  initialize its session (`thread-store` metadata recovery deadline). This was
  a review infrastructure failure, not a determination that the command was
  unsafe. No network-policy bypass or sandbox weakening was used.
- The sandboxed end-to-end maintenance case is implemented, typechecked and
  registered in the existing unconditional Linux BBP acceptance lane. It has
  **not run locally**. The lane installs fmt/clippy, builds the real host and
  runs the exact case, requiring one executed test and a completion marker;
  its logs/results are uploaded by the existing acceptance collector.
- Codex, Claude and Ollama live-provider tests remain opt-in and unexecuted.
  Provider contract tests passing does not verify subscription login or live
  provider functionality. No credentials were configured in this environment.
- Remote draft-publication behavior uses a GitHub CLI seam in local tests. Real
  Git branching, tree verification, committing, local push and retry adoption
  execute; no maintenance candidate was published or merged remotely.

## Reproduction

In a complete checkout with dependencies cached or network access restored:

```sh
cargo fmt -p rusty_orch -p rusty_bbp -- --check
cargo clippy --locked -p rusty_orch -p rusty_bbp --all-targets -- -D warnings
cargo test --locked -p rusty_orch -p orch-core -p orch-dispatch -p orch-cli \
  -p orch-bbp -p orch-store -p orch-codex -p orch-claude -p orch-ollama
cargo test --locked -p rusty_bbp --test durable --test human_fence
python3 -m unittest discover -s .github/scripts -p test_maintenance_validation.py
cargo build --locked -p rusty_bbp_host --bin bbp
BBP_TEST_BIN="$PWD/target/debug/bbp" cargo test --locked -p rusty_orch --lib \
  maintenance::e2e::sandboxed_maintenance_recovers_and_prepares_reviewed_pr_without_merging \
  -- --ignored --exact --nocapture
```

The local offline run used `--manifest-path
/workspace/maintenance-validation/Cargo.toml --offline` with the Rust target
flags above. Detailed local logs are retained in that checkout as
`maintenance-tests.log`, `bbp-durable.log`, `orch-regression.log`,
`orch-remaining-regression.log` and `ci-workflow.log`. Merge readiness still
requires green checks for the actual PR head and human review.
