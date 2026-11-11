---
category: Fixed
changelog: rusty-mux-ctl ssh setup matches host names in any case and undoes a failed setup completely; --ssh works without /bin/sh.
---
## 2026-11-11 - Mux ssh setup review notes

- **Fixed:** `rusty-mux-ctl ssh setup --write` finds an existing host whatever its case, as ssh does, and reads dot files an `Include` pattern starting with `.` names. An `Include ~other/...` is refused rather than guessed at.
- **Fixed:** a failed setup leaves nothing behind: no backup made in that run, no folder it created, and an empty file of the user's stays.
- **Fixed:** `rusty-mux --ssh` and `--host` ssh addresses work where there is no `/bin/sh`, without the cleanup that ends ssh when the client is killed, and say so.
