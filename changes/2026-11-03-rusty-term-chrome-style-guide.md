---
category: Changed
changelog: rusty_term search highlights follow the theme where that keeps them visible; chrome style guide added.
---
## 2026-11-03 - rusty_term chrome style guide and theme-derived search highlights

- **Changed:** search matches in the windowed front-end use whichever of the theme's yellow and the old fixed amber stands out more from the background, and the active match the same between a red-yellow orange from the theme and the fixed orange. Glyphs on a highlight keep 4.5:1 contrast. On light presets such as solarized-light and catppuccin-latte the fixed amber stood out by only about 1.3:1; they now get the theme's darker yellow. The default and most dark presets keep the amber match; several dark presets get the theme's own colour for the active match. Pages without a theme (settings, dock) keep the fixed colours.
- **Added:** `docs/projects/term/STYLEGUIDE.md`, the chrome style guide, recording the owner's 2026-11-03 decision that the chrome uses a fixed scale independent of terminal font size.
- **Changed:** `rusty_term_core::contrast_ratio` is now public (behind the `gui` feature, beside `ensure_contrast`).
- **Known limitations:** the configured `font-size` still sizes the chrome's UI font; decoupling it is a follow-up. Search colours follow the window's theme, not a per-profile tab theme.
