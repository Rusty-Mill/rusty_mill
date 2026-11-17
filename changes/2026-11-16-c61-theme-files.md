---
category: Added
changelog: rusty_term loads themes from `themes/<name>.toml` beside the config file and reloads them live.
---
## 2026-11-16 - Theme files (C61 T1)

- **Added:** `theme = "name"` (and `theme_dark`, `theme_light`, profile themes) first looks for `themes/<name>.toml` next to the config file, then the built-in presets. A user file shadows a preset. The file is a `[colors]` section; keys left out inherit the default. Editing, adding or removing a theme file reloads live; a bad file or unknown name keeps the current theme and prints one warning. Schema: `docs/projects/term/THEME-FILES.md`.
- **Known limitations:** no in-app theme picker for user themes or visible notice yet (T5); no import (T2).
