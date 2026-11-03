---
category: Fixed
changelog: rusty_term GUI font zoom (Ctrl+= / Ctrl+-) scales only the terminal text; the tab bar, caption buttons and status bar keep their size.
---
## 2026-11-02 - rusty_term zoom scales only the terminal text

- **Fixed:** Ctrl+= / Ctrl+- (and Ctrl+0) in `rusty_term --gui` resized the whole window chrome along with the text: the tabs, the `+` button, the minimize/maximize/close buttons and the status bar are cell rows, and they were drawn in the terminal font. The bars now draw in their own UI font at the configured font size, on both the CPU and GPU renderers, so a zoom changes only the panes' font and cell size.
- **Fixed:** a zoom no longer overwrites the configured font size: Ctrl+0 now returns to the configured size instead of the last zoomed one, and opening the settings page after a zoom no longer saves the zoomed size.
- **Known limitations:** the bars follow the configured `font_size` (changing it in settings still resizes them); a fixed UI size independent of the font setting would be a separate change.
