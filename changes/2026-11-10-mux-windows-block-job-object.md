---
category: Changed
changelog: On Windows, closing a rusty_mux block now also ends the programs it started outside its console (a kill-on-close Job Object); agent-owned blocks are untouched.
---
## 2026-11-10 - Windows mux blocks end what they started

- **Changed:** on Windows a rusty_mux block without an owner runs in a kill-on-close Job Object on both console hosts, so closing it ends everything it started, including `start ping -t` and GUI apps such as `start notepad` (RISKS R11, card #282 item 4, handoff pack D181-D185). The job closes once the console has shut down, so programs still on the console keep their 5 s close handling. Agent-owned blocks (`Launch.own`) get no job and are never ended by it (D87).
- **Changed:** `platform-windows` honours `GroupSpec::NewGroup` on `WindowsPty` and `condrv::spawn_attached` (spawn suspended, assign to a fresh kill-on-close job, resume), so `kill_tree` works on those children; `Inherit` is unchanged. The `platform::pty::Pty::spawn` contract doc says so.
- **Known limitations:** no breakaway is allowed, so a GUI app started from a block closes with it.
