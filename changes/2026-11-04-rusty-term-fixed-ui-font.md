---
category: Changed
changelog: rusty_term GUI chrome and status bars keep a fixed UI font size; the configured font size now changes only the terminal grid.
---
## 2026-11-04 - Fixed chrome font size

- **Changed:** the tab strip and status ribbon draw at a fixed 18 logical px times the display scale. Changing `font_size` in settings or config resizes only the terminal text, like zoom already did (#1388). The default config looks the same as before.
- **Fixed:** on a scaled display (for example 150% or 200%) the settings page showed and saved the font size in physical px, so editing it shrank the text and wrote the wrong value. It now shows and saves logical px and applies them at the display scale.
- **Known limitations:** the settings page still draws in the terminal grid font, so they follow `font_size` and zoom. A `[window] ui_scale` is not added yet.
