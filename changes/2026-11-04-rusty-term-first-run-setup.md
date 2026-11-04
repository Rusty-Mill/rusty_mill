---
category: Added
changelog: rusty_term opens a short first-run setup (shell, theme, clipboard, alerts) on its first windowed launch.
---
## 2026-11-04 - rusty_term first-run setup (O11)

- **Added:** a first-run setup in the windowed front-end. A plain `--gui` launch with no config file at the default location opens the settings page in setup mode: four steps (shell, theme with live preview, clipboard, alerts), `Enter` for the next step and to finish, `Esc` to skip. Finish saves only values changed from their defaults; skip restores the values setup opened with. Both write the commented config template, so setup does not return on the next launch. Owner decision D65 (Nano, 2026-11-04); details D66–D69.
- **Added:** `--setup` opens it at any time, and the `open_setup` keymap action (no default chord) does the same from `[keys]`.
- **Added:** a "Command finished alert" row on the settings page (Terminal) for the existing `command_notify_secs` key: off, 5 s, 10 s (default), 30 s, 1 min or 5 min. A custom value in the file seeds to the nearest step.
- **Known limitations:** skip writing the template is interim until the owner confirms it (the accepted text said skip writes nothing, which would show setup on every launch). The agents step (rusty_mux hook install), importers and right-click behaviour are not in this change. The `T` test notification uses the same helper as command alerts and is not confirmed to appear (for example, `notify-send` may be missing on Linux).
