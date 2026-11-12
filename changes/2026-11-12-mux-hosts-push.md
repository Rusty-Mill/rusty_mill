---
category: Added
changelog: rusty-mux-ctl hosts push DEST merges this machine's saved hosts into another machine's over ssh.
---
## 2026-11-12 - Push saved hosts to another machine

- **Added:** `rusty-mux-ctl hosts push DEST [--replace] [--dry-run]` is `hosts pull` in the other direction (Nano, D208).
  - **What it writes:** only DEST's `hosts.json`, never a config.json. Names that DEST's config declares are skipped.
  - **How it writes:** the new file is private (0600) and goes in through a temporary file and a rename.
  - **Changed in between:** if DEST's file changed after push read it, push refuses and writes nothing.
- **Changed:** pull now reads the remote's answer by byte count, so text inside the file can't pass for the script's markers.
- **Known limitations:** DEST is an ssh destination, not a saved host name. The remote must run a POSIX `sh`.
