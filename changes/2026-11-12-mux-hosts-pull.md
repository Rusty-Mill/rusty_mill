---
category: Added
changelog: rusty-mux-ctl hosts pull DEST merges another machine's saved hosts into this one's over ssh.
---
## 2026-11-12 - Pull saved hosts from another machine

- **Added:** `rusty-mux-ctl hosts pull DEST [--replace] [--dry-run]` reads DEST's `hosts.json` over ssh with POSIX tools (no rusty-mux-ctl needed there) and merges it per name: new names are added, a name both have keeps this machine's entry unless `--replace`, and nothing is removed. Names this machine's config declares are skipped, and so are socket addresses, which name a server on the other machine. Only this machine's `hosts.json` is written.
- **Known limitations:** `hosts push` waits for an owner decision. DEST is an ssh destination, not a saved host name.
