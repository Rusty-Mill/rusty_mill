---
category: Added
changelog: rusty_term opens a short first-run setup (shell, theme, clipboard, alerts) on its first windowed launch.
---
## 2026-11-04 - rusty_term first-run setup (O11)

- **Added:** a first-run setup in the windowed front-end. A plain `--gui` launch with no config file at the default location opens the settings page in setup mode: four steps (shell, theme with live preview, clipboard, alerts), `Enter` for the next step and to finish, `Esc` to skip. Finish saves only the rows it really changed, onto the commented template when there is no file yet, so setup does not return. Skip (`Esc`, the tab's `×`, `Ctrl+,` or any other way out) restores the exact values setup opened with (custom colours, an undetected shell and off-step thresholds included) and writes nothing (D65): a missing config stays missing, so setup opens again on the next plain launch, and an existing one is left byte-identical. Owner decision D65 (Nano, 2026-11-04); details D66–D69.
- **Added:** `--setup` (before any `--`, `-e` or `--command`; after them it belongs to the child) opens it at any time, and the `open_setup` keymap action (no default chord) does the same from `[keys]`.
- **Added:** a "Command finished alert" row on the settings page (Terminal) for the existing `command_notify_secs` key: off, 5 s, 10 s (default), 30 s, 1 min or 5 min. A custom value in the file seeds to the nearest step and is rewritten only when this row is changed.
- **Changed:** closing the settings page writes only the rows changed on it, so values the page can only approximate (custom theme colours, an undetected shell, a custom contrast ratio or alert threshold) and a `Ctrl+=` zoom are no longer rewritten by an unrelated change.
- **Known limitations:** after a skip on a first run, setup returns on each plain launch until you finish it once or create a config file; an explicit "create config template" action may follow later. The agents step (rusty_mux hook install), importers and right-click behaviour are not in this change. The `t` test notification uses the same helper as command alerts and is not confirmed to appear (for example, `notify-send` may be missing on Linux).
