---
category: Changed
changelog: rusty_term's chrome style guide now marks where the GUI design pack's target differs from today, and the design pack records the chrome colour source.
---
## 2026-11-04 - rusty_term style guide targets and design-pack records

- **Changed:** `docs/projects/term/STYLEGUIDE.md` stays the description of the chrome as built and gains "Target:" lines that point at the GUI design pack (`docs/projects/term/gui-design/`) for colour tokens, faces, states and focus, and motion (pack D31).
- **Changed:** the GUI design pack records D31–D35. Among them is the owner's choice of chrome colour source (D34): fixed pack tokens for surfaces and text, with the accent taken from the terminal theme. D35 adds a contrast fallback for that accent.
- **Fixed:** CLAUDE.md's expert routing table and the rusty-term expert's routing line now point at the design pack's real location, `docs/projects/term/gui-design/`.
- **Known limitations:** documentation only; the GUI does not draw the pack's tokens or faces yet. That waits for the pack's implementation handoff.
