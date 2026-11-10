
- [ ] Q44 (2026-11-10, n): Should `ssh diagnose` get an opt-in network probe (`--connect`) that logs in with BatchMode and tests a stream-local forward to the remote socket (sshd `AllowStreamLocalForwarding`, server running)? — interim choice: D191, diagnose stays read-only with no network.
- [ ] Q45 (2026-11-10, n): The `--ssh` options and `ssh -G` parsing live only in `rusty_mux_tui::ssh`. When ctl learns to open SSH (C35 E1), move them to `rusty_mux_client` so ctl and the TUI share one copy? — interim choice: D191, ctl's own copy with a test against `tui/src/ssh.rs`.
