---
category: Fixed
changelog: rusty-mux-ctl ssh setup --write checks your whole ssh config before adding a host, leaves both files alone when it can't finish, and --ssh forwards no longer outlive a killed client.
---
## 2026-11-11 - ssh setup and --ssh forward follow-ups

- **Fixed:** `rusty-mux-ctl ssh setup DEST --write` also finds a host you already have through wildcard `Include`s (`*` and `?` in the file name, as ssh reads them) and `Match host`/`originalhost` lists. It refuses, writing nothing, when an included file is there but can't be read, or when an `Include` pattern can't be checked (card #1441).
- **Fixed:** if writing `~/.ssh/config` fails, `setup --write` puts `~/.ssh/rusty_mux/config` back, so both files are as they were.
- **Fixed:** `ssh remove DEST --write` deletes a `~/.ssh/config` that setup created and that is now empty. A config you already had, even an empty one, is kept.
- **Fixed:** on Windows the `ssh` commands use `%USERPROFILE%\.ssh`, the folder Win32-OpenSSH's `~` means, even when `HOME` is set (Git Bash, MSYS).
- **Fixed:** the ssh that a `--ssh` forward or a `--host` ssh address starts now ends when its client dies without cleaning up, for example when killed. A small `/bin/sh` watchdog holds a pipe from the client, and ends ssh when the pipe closes without the client saying it's done.
- **Changed:** CAPABILITIES C03, C35, C47 and C48 are marked done.
