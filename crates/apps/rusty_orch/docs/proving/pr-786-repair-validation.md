# PR #786 repair validation — 2026-10-19

Repair base and remote branch: `3522c7ded733b07fe88f353cf5601eafb680c3b1`,
`codex/autonomous-maintenance`. Rechecked against GitHub and `git ls-remote`
before committing. Work used an isolated clone on Beast; the existing
`C:\dev\rusty_mill` checkout and its unrelated untracked work were preserved.

Reviewed issue comments 6256548834, 6257055600 and 6257125519 and the three
Windows job logs from run 39177445244. All four reported Windows failures
reproduced locally on the original head with normal Windows process access.
The new linked-task and deleted-terminal-checkout tests also failed against
the original logic before the fixes.

## Behavior and regression coverage

- Publication freezes the actual push destination, GitHub repository and default
  branch in the private bundle, fetches that destination's branch, and requires
  the inspected base to be in its history before creating a worktree or writing
  remotely. Real local-Git regression: main A, feature B with unrelated source
  content, and an exact candidate tree on B is refused before worktree/push/PR.
  Recovery still adopts one commit/PR and retains the frozen destination after
  local `origin` changes. GitHub calls use an explicit repository.
- Maintenance accepts the shipped Codex author's built-in tools. The existing
  configuration validator still prohibits final-reviewer tools and requires an
  independent reviewer. The maintenance acceptance fixture uses the shipped
  roster, vendors and assignment policy with local provider scripts. Real
  launcher-stub tests use that roster and check workspace-write in an isolated
  clone, no author push remote, source/index isolation, and restricted review.
- Active tasks and journals use canonical shared Git-directory identity, rather
  than checkout paths or different Windows extended-path spellings. Coverage
  includes real linked worktrees, one shared store/journal, and lock exclusion.
- Terminal tasks are filtered before resolving their checkout. Persisted
  cancelled tasks with deleted checkouts no longer prevent future intake.
  Linux entry-point coverage proves refusal before opening a second task and
  that the terminal/deleted case advances past inventory without any provider.
- Discovery emits portable slash-separated manifest paths. The Windows dry-run
  failure was a real selection defect: native backslashes failed maintenance's
  relative-manifest eligibility check. Coverage asserts exact allowed paths.
- Windows continues to refuse unsupported execution before configuration reads;
  the integration test checks that ordering. The real-Git publication fixture
  sets its own `core.autocrlf=false`, so its literal LF tree/patch contract is
  independent of user configuration. Production scope/tree checks are unchanged.

## Commands and outcomes

Toolchains: Rust/Cargo 1.98.1 on Windows and FedoraLinux-43 WSL (Linux x86_64),
Python 3.14. No live or paid provider calls were made. Cargo used cached
dependencies. All GitHub responses inside maintenance tests are stubs; pushes
inside publication tests target temporary local bare repositories.

```sh
cargo test --locked --offline -p rusty_orch -p orch-core -p orch-dispatch -p orch-cli -p orch-bbp -p orch-store -p orch-codex -p orch-claude -p orch-ollama --all-features
cargo test --locked --offline -p rusty_orch --lib maintenance
cargo test --locked --offline -p rusty_orch --test maintenance
cargo clippy --locked --offline -p rusty_orch -p orch-bbp -p rusty_bbp_host --all-targets --all-features -- -D warnings
python3 -m unittest discover -s .github/scripts -p test_repository_intelligence.py -v
python3 -m unittest discover -s .github/scripts -p test_maintenance_validation.py -v
python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v
cargo fmt --all -- --check
python3 .github/scripts/changes.py check
git diff --check
```

- Windows affected Orch suite: 214 passed, five opt-in tests ignored. Final
  focused rerun: 16 maintenance unit tests and two integration tests passed.
  The three publication tests passed again on Windows and Linux after their
  fixture was strengthened to use a scoped layer-metadata repair.
- Windows clippy: passed with warnings denied. Intelligence: 14 passed; frozen
  scope validation: five passed. Fragment gate: 145 fragments valid.
- Full Python gate suite on Linux: 228 passed, one Windows-only test skipped.
  The Windows run exposed 21 failures and two errors in Linux shell/process
  assumptions outside this repair; it was not counted as a passing gate.
- Windows full-workspace fmt exceeds the OS command-line limit. The equivalent
  full-workspace check runs on Linux; scoped Windows formatting passes.
- Linux formatting and affected-scope clippy: passed.
- Linux affected Orch suite: 227 passed, 16 opt-in tests ignored. This includes
  the shipped-policy linked-store entry-point regression.
- All seven confined acceptance modes passed: lifecycle, pause/adoption,
  Rust/LLD, maintenance, timeout, cancellation and kill. Every mode executed one
  test and emitted its required `BBP_ACCEPTANCE PASS` marker. The maintenance
  case used the shipped Codex-author/Claude-reviewer policy with stub processes.

Linux build/test output uses `CARGO_TARGET_DIR=../linux-target`, separate from
the Windows target directory. Real launcher checks:

```sh
cargo build --locked --offline -p rusty_bbp_host -p rusty_orch
cargo test --locked --offline -p rusty_bbp_host --test provider_launchers
# All six launcher-stub tests passed.
export CARGO_NET_OFFLINE=true
export BBP_TEST_BIN=/absolute/path/to/linux-target/debug/bbp
export ORCH_TEST_BIN=/absolute/path/to/linux-target/debug/rusty_orch
python3 .github/scripts/orch_bbp_acceptance.py --output ../acceptance-results
```

No confinement, approval, credential or security settings were changed. This
record does not claim full-workspace Rust CI or macOS validation. Hosted CI for
the correction commit and independent review remain required before merge.
No replacement PR was created and no PR was merged.
