# Milestone 2 implementation validation — 2026-10-18

Implementation base: `25dd95173a1fee869411565092e63c2eacf01f40` (PR #783), verified
against current `main` at task start. Toolchain: Rust/Cargo 1.98.1; Python 3.12.
This records actual results; it does not claim green CI or a passing sandboxed
maintenance workflow while that acceptance case is unresolved.

## Executed in the original repository checkout

| Check | Observed result |
| --- | --- |
| `cargo test --locked --offline -p rusty_orch -p orch-store` | Passed: original command composition, store, unit/integration and subprocess-resumption tests; opt-in sandbox workflow remains ignored here |
| `cargo clippy --locked --offline -p rusty_orch --all-targets -- -D warnings` | Passed |
| Original core, dispatcher, BBP, Codex, Claude and Ollama adapter suites | 116 passed, 15 opt-in cases ignored |
| Real Cargo/Git maintenance integration | Dry-run makes no writes; current PR conflict prevents task creation |
| Real linked Git worktrees | Both recover the same private journal; the shared lock excludes a concurrent cycle |
| Real Git draft-publication fixture | Creates private worktree, verifies candidate tree, commits/pushes to a local bare origin; repeat adopts same commit/PR through a GitHub CLI seam |
| Public report boundary | Frozen profiles, launcher configuration and environment remain private |
| Python scope gate against staged Git candidates | 5 passed: layer repair and exact hoist accepted; unrelated/version/feature/deletion/wrong-layer edits refused; machine/user Git configuration excluded |
| Formatting, fragments, whitespace | Passed |

Additional focused checks: 12 BBP durable-store/human-fence tests passed. The
existing real CLI uncertain-open test passed across every injected lost reply
and froze the original provider pool. Earlier offline checks used a copied
first-party dependency scope; the original command and store were subsequently
built/tested directly after a scoped dependency-fetch retry succeeded. No
production implementation or store was stubbed in the repository.

The unchanged `orch-cli::timeout_kills_the_whole_group_and_joins_within_the_bound`
failed in the initial local regression run after killing its processes: `ps`
showed only PID-1-owned zombie `sleep` processes. The test explicitly delegates
orphan reaping to init. Its failure was retained, and no process-runner behavior
or assertion was weakened. CI's original workspace test lanes passed.

## Real BBP acceptance and remaining limitations

The initial PR CI run built the actual host and original Orch command and passed
workspace build/test/clippy, formatting, policy and workflow tests. Its failures
were the generated workspace map (fixed for the new atomic-file dependency) and
the new maintenance acceptance fixture. Existing confined lifecycle,
pause/adoption, Rust/LLD, timeout, cancellation and kill acceptance modes passed.

The maintenance fixture reached BBP's no-progress protection after a rejected
provider turn. Diagnostic agent/reporter logs were added; the diagnostic run was
queued behind broad workspace jobs. Machine Git configuration is now excluded
from scope checks, and the fixture uses the system Python/Git path plus the
explicit Rust toolchain. These changes do not relax BBP approval, health or
sandbox gates. A passing exact-head `BBP_ACCEPTANCE PASS maintenance` marker is
still required before calling the milestone fully validated.

Local execution of that same production-sandbox workflow reaches the plan gate,
recovers after moderator death, receives a separately issued human approval and
opens its validation run. The host then refuses execution because Landlock is
`NotEnforced` in this environment. The same refusal occurs outside the outer
tool sandbox. No unconfined result is counted as acceptance evidence. The
frozen scope/fmt/test/clippy commands pass separately in our trusted generated
fixture; that is diagnostic evidence, not a sandboxed end-to-end pass.

The Codex CLI is installed and reports a ChatGPT login. Automatic approval
rejected the proposed repository-content live smoke because it would transmit
repository instructions to the external provider. The safer retry used only a
synthetic fixture and nonce, and was approved; it failed before a model reply
because the CLI could not initialize its app-server state on a read-only
filesystem. Claude and Ollama CLIs are absent. Live-provider behavior therefore
remains unverified; passing adapter contracts is not a live-provider pass.

## Reproduction

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

The existing required Linux `orch-bbp-acceptance` lane runs that exact case,
requires one executed test and its completion marker, and uploads per-mode logs.
No maintenance candidate was merged. No independent approval or post-green
self-review is claimed; exact-head CI and human review remain required.
