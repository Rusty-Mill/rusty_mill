---
category: Added
changelog: rusty-mux-ctl hosts pull DEST and hosts push DEST merge saved hosts between two machines over ssh.
---
## 2026-11-12 - Sync saved hosts with another machine

- **Added:** `rusty-mux-ctl hosts pull DEST [--replace] [--dry-run]` reads DEST's `hosts.json` over ssh with POSIX tools (no rusty-mux-ctl needed there) and merges it per name. New names are added, and a name both have keeps this machine's entry unless `--replace` is given. Nothing is removed. Names that this machine's config declares are skipped. Socket addresses are skipped too, because they name a server on the other machine. Only this machine's `hosts.json` is written.
- **Added:** `rusty-mux-ctl hosts push DEST [--replace] [--dry-run]` does the same in the other direction (Nano, D208). Only DEST's `hosts.json` is written, never a config.json. Names that DEST's config declares are skipped. The new file is written private (0600) through a temporary file and a rename. If DEST's file changed after push read it, push refuses and writes nothing.
- **Known limitations:** DEST is an ssh destination, not a saved host name. The remote must run a POSIX `sh`.
