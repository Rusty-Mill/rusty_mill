---
category: Changed
changelog: rusty_term project docs move to docs/projects/term/; the crate README names the window's libxkbcommon runtime libraries and the rusty_tokio runtime.
---
## 2026-10-27 - rusty_term docs separated from mux

- **Changed:** `docs/projects/mux/RUSTY-TERM-HANDOFF.md` and `RUSTY-TERM-NOTES.md` move to `docs/projects/term/HANDOFF.md` and `NOTES.md`, with an index (`docs/projects/term/README.md`). The mux charter links it. `DELTA.md` and `REPLAY.md` stay in mux (attach protocol); the perf backlog and harness stay in `docs/research/`.
- **Fixed:** the crate README says that on Linux the window loads `libxkbcommon` (and `libxkbcommon-x11` on X11) at runtime, with package names, and that a missing one gives `cannot open a window: …`.
- **Fixed:** the crate README described the runtime as tokio and pointed at `src/runtime/tokio_rt.rs`. It is rusty_tokio (`src/runtime/reactor.rs`, `fair.rs`), and no feature brings tokio into the tree.
- **Known limitations:** docs only. The moved docs' content is unchanged apart from their own links, so they still read as of 2026-10-23. Change fragments that cite the old paths are left as history.
