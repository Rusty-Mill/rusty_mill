---
category: Added
changelog: rusty-mux-ctl terminal.rerun starts a held block's command again, in place, under the lease.
---
## 2026-11-24 - Re-run of a held block (C45, card #1569)

- **Added:** `terminal.rerun` block call. A block whose program is gone and was started by this server run (held on exit) starts its stored command again, in the same block, under the input lease; the screen stays and a `re-run at` line says when. Restore still never runs anything by itself (D72). Needs the existing `Features::HOLD`; no new feature bit or store field.
- **Known limitations:** a block restored after a restart is refused (its launcher is not recorded), which is narrower than D338's "held or restored" and is Nano's call to widen; a block launched with its own environment, descriptors or owner is refused, agent metadata or not (it re-runs through its launcher); a non-shell program re-runs in the directory it was started in, never one its output named; a client that listed the block as exited sees it as exited until it lists again; the environment of the first run is not kept, so the re-run gets the server's. No key is built: use `rusty-mux-ctl terminal.rerun`.
