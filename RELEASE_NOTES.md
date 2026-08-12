# Release Notes

One entry per merged PR against `main`, reverse chronological, each linking to its
PR and (where one exists) to the doc that covers the change in full detail.

---

## PR #9 — parity-gap: async job-control wait (wait_job/try_wait_job), closes #6
**2026-08-12** · [#9](https://github.com/baileyrd/rustils_async/pull/9)

- **Added:** `AsyncChild::wait_job`/`try_wait_job`, Unix job control
  (`WUNTRACED`/`WCONTINUED` — stop/continue, not just terminate).
  `try_wait_job` stays synchronous (already non-blocking, `WNOHANG`).
  `wait_job` is a disclosed one-shot background thread running the real
  blocking `waitpid` — a pidfd does **not** become readable on
  stop/continue (confirmed against real behavior, not assumed from the
  termination case), so the `EpollReactor` this workspace otherwise
  builds everything on cannot multiplex this the way it does plain
  termination.
- **Design note, not a shortcut:** `AsyncLinuxChild` now owns a single
  authoritative reap-state cache (`reaped: Mutex<Option<ExitStatus>>`)
  consulted by every reaping path (`wait`, `try_wait`, `ready`,
  `try_wait_job`, `wait_job`), mirroring rustils' own
  `LinuxChild::reaped` field. Without this, a caller mixing job-control
  and plain-wait calls on the same child could hit `ECHILD` from
  re-`waitpid`-ing an already-reaped pid — confirmed by a real test
  (`wait_job_observes_stop_then_continue_then_terminate`) that calls
  `wait()` *after* `wait_job()` already reaped the child and checks the
  stashed status comes back instead of an error.
- Real end-to-end test: a spawned child stopped (`SIGSTOP`), observed as
  `Stopped` through `wait_job`, resumed (`SIGCONT`), observed as
  `Continued`, then killed and observed as `Signaled` — not a scripted
  mock, an actual OS process transitioning through real job-control
  states.
- Closes parity-gap #6, from the `parity-loop` run against `rustils`
  (`gap-analysis.md`). This closes the gap list from that run — all
  three identified gaps (`wait_any`, `take_stdin`/`take_stdout`/
  `take_stderr`, `wait_job`/`try_wait_job`) are now merged.

## PR #8 — parity-gap: async pipe handle retrieval (take_stdin/take_stdout/take_stderr), closes #5
**2026-08-12** · [#8](https://github.com/baileyrd/rustils_async/pull/8)

- **Added:** `AsyncChild::take_stdin`/`take_stdout`/`take_stderr`, matching the
  sync `Child` contract exactly (`Some` exactly once; dropping stdin delivers
  EOF; stdout/stderr reads return 0 at EOF). Pure pass-through to the wrapped
  sync `Child`'s own `take_*` — no new async machinery, since the returned
  handle stays a plain synchronous `platform::fs::File` (fs remains sync,
  unchanged scope). `AsyncMockSpawner::script_with_output` added alongside so
  the mock backend can exercise scripted stdout content the same way the
  sync `MockSpawner` already does.
- Real end-to-end test: `/bin/cat` spawned with `Stdio::Pipe` on stdin and
  stdout, bytes written through `take_stdin`, the same bytes read back
  through `take_stdout` after EOF — not just a compile-time check that the
  methods exist.
- Closes parity-gap #5, from the `parity-loop` run against `rustils`
  (`gap-analysis.md`).

## PR #7 — parity-gap: async multi-child wait (wait_any), closes #4
**2026-08-12** · [#7](https://github.com/baileyrd/rustils_async/pull/7)

- **Added:** `AsyncChild::ready()` (borrowing, non-consuming — resolves once a
  child has terminated, without retrieving its status) and
  `AsyncSpawner::wait_any` (default implementation, built entirely on
  `ready()`) so several children can be awaited concurrently through one
  call instead of one `AsyncChild::wait` registration per child. Genuinely
  multiplexed on `platform-async-linux` — every `ready()` future registers
  with the same shared `EpollReactor`. Timeout support via a small one-shot
  `Timeout` future (a disclosed helper thread per call with a timeout, none
  without one) — no timer-wheel dependency added.
- **Fixed, caught before merge:** `PidfdReady` originally assumed "polled a
  second time" meant "the reactor woke me for my own fd" — true for the
  single-child `wait()` path, but wrong once `wait_any`'s combinator polls
  several `ready()` futures through one *shared* waker: a wake caused by a
  sibling child (or the timeout) would have made every already-registered
  child spuriously report ready. Fixed by having the reactor set a per-fd
  `Arc<AtomicBool>` flag before waking, and having `PidfdReady` check that
  flag explicitly rather than inferring readiness from being re-polled at
  all. The new `wait_any_times_out_when_nothing_exits_in_time` test would
  have failed under the old assumption.
- Closes parity-gap #4, from the `parity-loop` run against `rustils`
  (`gap-analysis.md`).

## PR #3 — parity-loop gap analysis: async process domain vs. rustils
**2026-08-12** · [#3](https://github.com/baileyrd/rustils_async/pull/3)

- **Added:** `gap-analysis.md` and a `parity_gap` issue template, from running
  the `parity-loop` skill against `rustils` (pinned at the same rev this repo
  already depends on). Scope was settled from this repo's own existing
  roadmap (README's "Reserved, not built" table + `docs/adr/0001`) rather than
  a fresh mechanical diff — fs/net/Windows/BSD stay out of scope, unchanged.
  Three in-scope gaps found in the one domain both repos are committed to
  (`process`): an async multi-child wait (`wait_any`), pipe handle retrieval
  (`take_stdin`/`take_stdout`/`take_stderr`), and Unix job-control wait
  (`wait_job`/`try_wait_job`). Filed as issues, worked one PR each.
- **Known limitation, stated plainly:** the "Existing RustyMill impl" check
  couldn't run for real — the RustyMill org's sibling repos (`rush`,
  `rusty_tokio`) are outside this session's attached owner tier and
  `add_repo` refused the cross-tier add. Marked "not checked" in
  `gap-analysis.md` rather than assumed absent; worth a real check from a
  session that has org access.

## PR #2 — Apply repo-config governance file set
**2026-08-12** · [#2](https://github.com/baileyrd/rustils_async/pull/2)

- **Added:** PR templates (feature/bug_fix/docs/chore), issue templates
  (bug_report/feature_request + contact-link config), CONTRIBUTING.md,
  CODE_OF_CONDUCT.md, SECURITY.md, CHANGELOG.md, ARCHITECTURE.md, an ADR seed
  template, and a `ci-rust.yml` workflow (fmt check, clippy `-D warnings`, `cargo
  test --workspace`) so the "PR + green CI, merge with a merge commit" workflow has
  a real gate to run against.
- **Known limitation, stated plainly:** the repo-config skill's own local template
  package was missing its entire `.github/` payload (PR templates, issue templates,
  CI workflow templates) when this ran — those pieces were hand-authored to match
  the skill's documented conventions rather than copied, so they haven't been
  cross-checked against whatever the skill's canonical templates actually say.
  Worth reconciling once the skill package is fixed upstream.
- SECURITY.md's contact is a real address (baileyrd@gmail.com), not a placeholder —
  confirmed with the repo owner during setup.

## PR #1 — Bootstrap rustils_async: native async process domain, no hidden runtime
**2026-08-12** · [#1](https://github.com/baileyrd/rustils_async/pull/1)

- **Added:** `reactor-core` (runtime-agnostic async-io primitives per Rusty-Mill
  AKB ADR-0160), `platform-async` (async `Spawner`/`Child` traits for the
  `process` domain, reusing rustils' own types via a pinned git dependency),
  `platform-async-mock`, `platform-async-linux` (a real pidfd + epoll reactor,
  explicit and `Drop`-cleaned-up, no hidden global runtime), `threading` (scoped
  thread spawn with a decoded join outcome, `Mutex`/`RwLock` with an explicit
  poisoning policy), and `coreutils-async` (`arun`, an async port of rustils'
  `rrun`, as the forcing consumer).
- **Known limitation, stated plainly:** this repo started without a named,
  working consumer forcing the work — a deliberate, documented exception to
  rustils' own consumer-gate policy, recorded in `docs/adr/0001-native-async-rustils.md`.
  Windows/BSD backends and the fs/net async domains are reserved rows, not built.
- 24 tests passing, including real spawn+wait against actual child processes
  (not just mocks) exercising the epoll reactor end to end.
