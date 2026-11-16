---
category: Added
changelog: UDP for account servers behind the login front door, with --udp-ports and --udp ranges.
---
## 2026-11-14 - rusty_mux UDP for account servers (#1461)

- **Login mode:** `rusty-mux-server --login MAP --udp-ports ADDR:LO-HI` gives each account's server a UDP range. The server binds the first free port in it (or serves without UDP when none is free); `--udp` itself now takes `ADDR:LO-HI`. Associations on account servers end an hour after they began (`--udp-lifetime`), tailnet principals are refused one, and the front door opens no UDP socket. No wire change. Pack rows D268-D270, mux DECISIONS row.
