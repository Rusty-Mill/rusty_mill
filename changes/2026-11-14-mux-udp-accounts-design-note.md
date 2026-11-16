---
category: Added
changelog: Design note for UDP for account servers behind the login front door.
---
## 2026-11-14 - rusty_mux UDP for account servers design note (#1461)

- **Docs:** `docs/projects/mux/UDP-ACCOUNTS.md` proposes `--login MAP --udp-ports ADDR:LO-HI`: each account server binds the first free port in the range, no wire change, tailnet principals refused. Design only; nothing is built. Pack rows D268, D269, Q66.
