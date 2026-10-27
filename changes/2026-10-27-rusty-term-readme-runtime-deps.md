---
category: Fixed
changelog: rusty_term README names the libxkbcommon runtime libraries the window needs and no longer says the runtime is tokio.
---
## 2026-10-27 - rusty_term README: window runtime libraries, rusty_tokio runtime

- **Fixed:** the README now says that on Linux the window loads `libxkbcommon` (and `libxkbcommon-x11` on X11) at runtime, with package names, and that a missing one gives `cannot open a window: …`.
- **Fixed:** the README described the runtime as tokio and pointed at `src/runtime/tokio_rt.rs`. The runtime has been rusty_tokio (`src/runtime/reactor.rs`, `fair.rs`) since the migration, and no feature brings tokio into the tree.
- **Known limitations:** docs only. The rusty_term docs under `docs/projects/mux/` are not moved here.
