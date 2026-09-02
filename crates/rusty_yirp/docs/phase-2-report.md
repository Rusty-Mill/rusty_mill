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
| Windows path-length mitigation | **Done, with a caveat** — manifest wired into the build and verified embedded; a lower, git-for-windows-internal ceiling found. See [`phase-2-windows-verification.md`](phase-2-windows-verification.md). |
| Windows Defender smoke test | **Done** — see [`phase-2-windows-verification.md`](phase-2-windows-verification.md) |
| **Full suite on real Windows** | **Done** — 105/105, see below |

105 tests pass in total, **on Linux and on real Windows alike**.
`cargo clippy --workspace --all-targets` is clean and the workspace
type-checks for `x86_64-pc-windows-msvc`.

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

## Windows work that was outstanding here — now closed out

All three items below were run against a real Windows desktop and are
recorded in full, including the one that did not come back clean, in
[`phase-2-windows-verification.md`](phase-2-windows-verification.md) rather
than repeated here:

- The `longPathAware` manifest is now embedded by `build.rs` and verified
  present in the built binary.
- The Windows Defender smoke test (PLAN.md risk 7) has been run: 24
  concurrent worktree sessions under real-time protection, no failures.
- Trying it against a genuinely deep repository surfaced a real limitation
  neither the manifest nor `LongPathsEnabled` nor `core.longpaths` fully
  covers, internal to Git for Windows' `worktree add` — measured, not
  assumed, with the threshold recorded.

That same pass also root-caused and fixed issue #2 (the daemon hang) while
investigating this ground, since the two shared a root cause.

## Slightly ahead of scope, deliberately

`sessionmgr-git` implements `changed_files` and `diff` as well as the worktree
lifecycle. Those are consumed by the Phase 4 diff pane, not by anything Phase 2
ships. They are about twenty lines with their own parser tests (porcelain
status codes, renames, quoted paths), and writing them now avoided a second
editing pass over the same file for the sake of phase purity. No CLI command
exposes them yet.

## The Windows verification pass

Run on the Windows dev box against `x86_64-pc-windows-msvc`, 2026-08-16.
Final result: **105 passed, 0 failed**, including all 10 worktree lifecycle
tests, all 3 supervisor-restart-recovery tests, and all 4 worker-crash tests.

It was worth doing. 56 of the first 62 tests passed immediately — the daemon,
detached workers, Windows `AF_UNIX` sockets, worktree creation and teardown all
worked first time — but the run surfaced **two genuine product bugs that
neither the Linux suite nor review had found**, plus two defects in the test
harness itself.

### Product bug 1 — bind before recover, an unbounded wait (was: an infinite hang)

`supervisor_restart_recovery` hung indefinitely. Three `sessionmgr` processes
alive during the hang — worker, replacement daemon, and the client — showed the
*client* was wedged rather than blocked on an inherited pipe.

Root cause: **no socket read had a timeout.** `Connection::request` blocked
forever, which made `wait_ready`'s 20-second deadline decorative, since the
deadline was only checked *between* probes and one probe could block forever.

The window that triggered it: `supervisor::run` bound the listener, *then* ran
`reconcile_all()`, *then* started accepting. A client connecting in between
connects successfully — straight into the listen backlog — and waits for an
answer nobody is accepting yet. Recovery probes one pid per session, so the
window widens with the number of sessions.

Three fixes: recovery now runs **before** the socket is bound (with no socket, a
client fails to connect and retries, which its readiness loop already handles,
and no client can observe the registry mid-recovery); each readiness probe gets
a 2-second timeout so the caller's deadline is real; and one-shot client
commands get a 60-second backstop that names `daemon.log` instead of hanging.

**This was never a Windows bug.** The ordering window was always there. Linux
just lost the race less often.

### Product bug 2 — `terminate()` failed on a pid that had already exited

`terminate()`'s contract says a pid that is already gone is not an error: the
caller's goal is "not running", which is already true. The Windows arm did not
honour it. `TerminateProcess` returns `ERROR_ACCESS_DENIED` on a process that
has **already exited**, and that was propagated as `Err`.

Not an edge case: a process object outlives the process itself whenever anyone
still holds a handle to it, so the failing call is the ordinary one. Since
teardown terminates a recorded pid pair, closing a session whose child had
already exited on its own would have failed on Windows — and a finished session
is the common case for closing.

Fixed by opening with `PROCESS_QUERY_LIMITED_INFORMATION` as well and, on a
failed `TerminateProcess`, asking `GetExitCodeProcess` whether the process has
in fact ended.

### Harness defects (not product code)

- **`cmd.exe` has no single quotes.** `commit_a_file` built a shell one-liner
  using `-m 'add {name}'`; `cmd` split that into `-m`, `'add`, and `{name}'`, so
  git took `'add` as the message and the filename as a pathspec. Six of the ten
  worktree tests failed this way. Fixed by removing every quote: the git
  identity now lives in the repository config (linked worktrees share it, and a
  session's shell inherits none of the test process's environment anyway), and
  the commit message contains no spaces.
- **A stale socket could stop the daemon restarting.** `clear_socket` returned
  `Err` on any delete failure other than "not found", and `Listener::bind`
  propagated it — so on Windows, where deleting an `AF_UNIX` socket left by a
  killed process does not reliably succeed, one unclean kill could leave the
  daemon permanently unable to bind. `clear_socket` is now infallible and `bind`
  retries once. Not confirmed as the cause of any observed failure, but a real
  latent defect in a tool whose premise is surviving unclean exits.
