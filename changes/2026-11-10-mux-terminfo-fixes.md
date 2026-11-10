---
category: Fixed
changelog: rusty-mux-ctl terminfo exits 4 for an unreachable host on setup, refuses an empty remote HOME, and ignores rc-file noise in check.
---
## 2026-11-10 - rusty-mux-ctl terminfo: review fixes (C47)

- **Fixed:** `terminfo setup` judges a failed write to ssh only after ssh exits, so an unreachable host exits 4 as documented, not 1.
- **Fixed:** `setup` and `remove` stop when the remote `$HOME` is empty instead of writing under `/.terminfo`.
- **Fixed:** `check` marks its own lines, so output from the remote's rc files no longer breaks it.
- **Docs:** pack D192 describes the `mktemp` file `setup` compiles; D193 records Nano's answer on where these commands may write.
