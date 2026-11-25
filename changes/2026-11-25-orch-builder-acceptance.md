---
category: Added
changelog: rusty_orch: the #1318 builder acceptance tests run in CI against a real mux, jail and bbp with a fake ACP agent (#1318 S6).
---
## 2026-11-25 - rusty_orch: builder acceptance in CI (#1318 S6)

- **Added:** `builder::e2e`: a stored candidate under the jail (mux variable, `AF_UNIX` and outside writes all refused; slot and cgroup freed), a mux restart ending `aborted` with nothing debited, and the holder's deadline ending a session whose orch was killed.
- **CI:** the "Orch BBP confined acceptance (Linux)" job builds `rusty-mux-server`, `rusty-mux-ctl` and `lel-jail`, delegates `/sys/fs/cgroup/lel-ci` to the runner with `sudo`, moves its shell inside, and runs the three cases.
