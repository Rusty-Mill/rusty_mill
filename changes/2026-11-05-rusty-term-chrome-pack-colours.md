---
category: Changed
changelog: rusty_term --gui chrome (tabs, ribbon, dock, search, settings and setup pages) uses the design pack's fixed colours, with the accent from the terminal theme.
---
## 2026-11-05 - rusty_term chrome uses the design pack colours

- **Changed:** the `--gui` chrome draws its surfaces and text from the design pack's fixed
  tokens (dark D5 or light D25, picked by the terminal theme's background) instead of blends of
  the theme's foreground and background. The accent still comes from the theme's cursor colour,
  falling back to the pack verdigris when it would be hard to see. Success and failure marks are
  always the pack verdigris and oxide (pack D34 to D39).
- **Known limitations:** first slice of the GUI redesign. Focus ring and state matrix, reduced
  motion and the Instrument Sans interface font are later slices. The terminal grid keeps the
  theme's own colours.
