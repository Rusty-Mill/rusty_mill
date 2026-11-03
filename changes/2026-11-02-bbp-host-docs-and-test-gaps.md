---
category: Fixed
changelog: rusty_bbp_host and orch-bbp docs describe session `bbp mcp` servers and the full `bbp mod` usage; tests now guard the session-binding refusal, the 600 s `wait` cap, the inotify wake and `relay`'s lease check (#1334).
---
## 2026-11-02 - BBP host stale docs and test gaps

- **Fixed:** the `rusty_bbp_host` README, `lib.rs` and `broker.rs` no longer say
  `bbp mcp` is always one process per turn; the README's `bbp mod` usage lists
  `--max-usd`/`--criteria` and the slot flags.
- **Fixed:** new tests for a provider launcher refusing a session binding, the
  600 s cap on `wait`'s `timeout_s`, a session `wait` woken by the log watch
  (its recheck pushed past the timeout), and `orch-bbp`'s `relay` refusing a
  changed or ended lease before BBP is asked. Removing that lease check now
  fails `an_unleased_task_refuses_a_leased_holder`.
- **Fixed:** a session `wait` installs its log watch before its first check,
  so an append between the check and the watch wakes it at once instead of
  after the 1 s recheck.
- **Known limitations:** the inotify wake test runs on Linux only.
