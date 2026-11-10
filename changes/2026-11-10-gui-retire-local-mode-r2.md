---
category: Removed
changelog: rusty_term GUI local mode code removed (R2): `gui::run`, the control socket and `rusty_term ctl`, session files, the quake window and its hotkey.
---
## 2026-11-10 - GUI local mode deleted (R2)

- **Removed:** the code behind the GUI's local mode, which R1 (#1569) already made unreachable: `rusty_term --gui` runs `rusty-mux-gui`. Gone are `gui::run`, the window's local shell spawning (new tab, local split, shell and profile launchers, new window), `gui/control.rs` with its control socket, session files (`config::load_session`, `config::SessionTab`), the quake window and its global hotkey (`gui/hotkey.rs`), and `PaneIo::local` with its read-ahead thread. `rusty_term ctl` now says it was retired and points to `rusty-mux-ctl`. Pack D158 (Nano's go) and D159 (scope).
- **Kept:** every `keymap::Action` and default binding (NewTab and NewWindow now do nothing in a window), the `session`, `single_instance`, `quake_height` and `quake_hotkey` config keys and flags, which still parse and are ignored, and the TUI, which is unchanged.
- **Known limitations:** windows still carry an always-`Some` source and the local chrome bar's code; R3 (card #1422) removes them after its own go. Public API removed: `rusty_term::gui::run`, `rusty_term::gui::control`, `rusty_term::config::{load_session, SessionTab}`, `rusty_term::gui::source::PaneIo::local`.
