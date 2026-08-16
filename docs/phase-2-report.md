# Phase 2 report — worktree isolation and all three session kinds

Phase 2's stated scope:

> worktree isolation, all three `SessionKind`s, worktree lifecycle tests,
> Windows path-length/Defender smoke tests. Still Claude Code only, still
> CLI-only (no TUI).

## Status

| Item | Outcome |
|---|---|
| Worktree isolation | **Done** |
| All three `SessionKind`s | **Done** |
| Worktree lifecycle tests | **Done** — 10 black-box tests against real repositories |
| Windows path-length mitigation | **Partly** — budget asserted in tests, manifest written but not wired into the build |
| Windows Defender smoke test | **Not run** — needs a Windows machine |

105 tests pass in total. `cargo clippy --workspace --all-targets` is clean and
the workspace type-checks for `x86_64-pc-windows-msvc`.

## What was built

A `sessionmgr-git` adapter that shells out to the system `git`, a
`workspace` module in the domain crate holding the naming policy, and the
three session kinds wired end to end:

| Kind | Runs in | Owns a branch | Isolated |
|---|---|---|---|
| `worktree` | `<repo>/.sessionmgr-worktrees/<id>` | `sessionmgr/<id>` | yes |
| `same-dir` | the repository's own working copy | no | **no** |
| `terminal` | nowhere in particular | no | n/a (no repo) |

Teardown gained dispositions: `close` stops the processes and leaves the work
alone, `close --merge` fast-forwards the branch back, `close --discard` deletes
the worktree and branch.

### The isolation property is tested directly

Not just that the commands return zero. `a_worktree_sessions_work_does_not_touch_the_main_working_copy`
runs a session that commits a file and then asserts the file exists in the
worktree, does **not** exist in the repository's working copy, and its commit
is **not** on the main branch. `two_worktree_sessions_on_one_repo_are_independent`
asserts neither session can see the other's files, then merges one without
disturbing the other — the "many agents, one repo, no conflicts" property.

## Decisions worth recording

**A bare `close` does not throw work away.** `--merge` and `--discard` are
opt-in; a `close` with neither stops the processes and leaves the worktree and
branch untouched. Discarding work is not something to infer from an ambiguous
instruction. Passing both at once is refused rather than resolved by
precedence: they mean opposite things about the user's work, and one of them is
irreversible.

**A refused merge keeps everything.** `--ff-only` means a diverged branch fails
loudly. When it does, the worktree and branch are left in place and the error
says what the user can do next — because removing the worktree anyway would
destroy exactly the work that could not be merged. The session stays closeable
afterwards, so a refusal never wedges it.

**Teardown order is processes, then files.** A worktree cannot be removed while
something holds a file open inside it, and on Windows that is not advisory.
Close terminates the recorded pid pair first and only then touches git.

**`--discard` on a same-directory session cannot delete the user's repository.**
A session that owns no branch always ends `Closed`, whatever disposition was
passed. There is a test for this specifically, because the alternative failure
is catastrophic and silent.

**Same-directory sessions get no collision protection**, exactly as the plan
states. Concurrent same-directory sessions share a working copy and an index
and can collide. No lock, no `.git/index.lock` mitigation. This is documented
in the CLI's own help text (`NOT isolated; concurrent same-dir sessions can
collide with each other`) rather than only in a design document, since the user
choosing it is the one who needs to know.

## Bugs the tests caught

1. **A bare `close` recorded the session as `Discarded`.** It correctly left the
   worktree on disk, but told the user their work had been thrown away when it
   had not — a lie in the more alarming direction. `teardown_status` now takes
   `Option<Disposition>` so "no disposition" is representable rather than being
   collapsed into a default. Covered by a domain unit test as well as the
   black-box one that found it.
2. **A test polluted this repository.** The obsolete Phase 1 test asserting
   `--kind worktree` was unsupported started *succeeding* once the kind
   existed — creating two real worktrees and two real branches in this
   project's own checkout, because it ran with the repo as its working
   directory. Removed and replaced; `common::TempRepo` now gives every test a
   throwaway repository, and the reasoning is recorded on the type so it does
   not recur.

## Windows work that is declared but not finished

- **`longPathAware` manifest**: written to
  `crates/sessionmgr-daemon/sessionmgr.exe.manifest`, including `activeCodePage`
  UTF-8, but **not wired into the build**. Embedding it needs a `build.rs`
  driving a resource compiler, which only runs on a Windows toolchain. It is
  checked in so the requirement lives next to the code rather than in someone's
  memory. Wiring it up is a task for the Windows verification pass.
- **Path-length exposure** is asserted at the unit level from both directions:
  `workspace::worktree_dir` proves this tool adds at most ~40 characters to a
  path it does not control, and `paths` proves socket paths fit the 107-byte
  `AF_UNIX` budget for a realistic Windows state root. Neither is a substitute
  for trying it against a genuinely deep repository on Windows.
- **Windows Defender smoke test** (PLAN.md risk 7 — heavy concurrent worktree
  file I/O at Defender's default settings) has not been run. It cannot be
  simulated here. This remains an open, genuinely Windows-native risk.

## Slightly ahead of scope, deliberately

`sessionmgr-git` implements `changed_files` and `diff` as well as the worktree
lifecycle. Those are consumed by the Phase 4 diff pane, not by anything Phase 2
ships. They are about twenty lines with their own parser tests (porcelain
status codes, renames, quoted paths), and writing them now avoided a second
editing pass over the same file for the sake of phase purity. No CLI command
exposes them yet.
