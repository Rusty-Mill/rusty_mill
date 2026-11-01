---
category: Added
changelog: rusty_mux login mode: `rusty-mux-server --login MAP` maps each local user to an account and relays it to that account's own server (C26).
---
## 2026-11-01 - rusty_mux login mode with an account map

- **Added:** `rusty-mux-server --login MAP [--runtime DIR] <socket>`, a front door any local user may connect to (Linux, normally run as root). It maps the peer's uid (over `rusty-mux --ssh`, the SSH user) to an account through `MAP`, a line file of `uid:N`, `user:NAME` or `*` rules naming an account or `=` (the principal's own: the account of that name must have the principal's uid). The shipped map is empty, root is reached only through a rule naming the principal, and the file must be writable by root or the front door's user only. The runtime directory must be an absolute path with no symbolic link in it, and it and every directory above it must be safe from other users (sticky directories such as `/tmp` are fine above it).
- **Added:** each account gets its own server, started as the account (uid, gid, groups, `HOME`, `SHELL`) in a session of its own under `DIR` (`/run/rusty-mux` by default). Its stdio is `/dev/null`, so no descriptor the front door opened reaches the account. It keeps running when the front door process ends, so a restart keeps every session; under systemd that needs `KillMode=process`, since a unit's stop kills its whole cgroup (no unit ships yet). Relayed clients carry the principal that connected (`whoami`, takeover check), passed by a preface the account server trusts only from the front door's uid (`--forwarded-by`).
- **Added:** SIGHUP rereads the map; a bad map keeps the old one, and a good one closes only the connections whose account changed or went away.
- **Added:** `platform_linux::account` and `platform_linux::groups` (`getpwnam_r`, `getgrouplist`).
- **Known limitations:** no PAM account or session checks yet, so a locked or expired account is still served. No shared mode, no UDP for account servers, no Tailscale principals (C27). Linux only. The root path (`a_root_front_door_starts_the_server_as_the_account`) needs root and an account named `claude`, so it is ignored on CI.
