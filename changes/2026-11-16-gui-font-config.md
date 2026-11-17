---
category: Added
changelog: The window front-end reads a font fallback list, font weight, line height and letter spacing from `[window]`.
---
## 2026-11-16 - GUI font config: fallback list, weight, line height, letter spacing

- **Added:** `[window] font_fallback` takes one path or a one-line list, tried in order; `font_weight` (`regular` or `bold`) picks the base face; `line_height` (0.8-2.0, default 1.0) and `letter_spacing` (-2..8 device px, default 0) change the cell size. Out-of-range numbers clamp. Both render paths and the PTY grid read the one cell size, so they agree. Defaults leave existing configs unchanged. C61 T4a.
- **Known limitations:** no variable-font weight axis and no synthetic bold (`bold` needs a bold face file); the interface font ignores these keys; the settings page has no rows for them yet (T4b); bold-base uses the bold file for ordinary text only when one is found.
