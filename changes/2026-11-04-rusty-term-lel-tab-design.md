---
category: Added
changelog: Design the Let Em Loose tab for the rusty_term GUI (lel.html, decisions D37 to D41), and record the matching Let Em Loose decision D83.
---
## 2026-11-04 - rusty_term GUI: Let Em Loose tab design

- **Added:** `lel.html` in the GUI design pack: the Let Em Loose tab (full-app view in dark and light, inbox chip and list, irreversible-approve confirm, claim, stage and empty states). New rows D37 to D41 in the pack's DECISIONS, questions Q9 and Q10, and a SPEC scope line.
- **Added:** Let Em Loose D83 (stage strip is a derived read-only display; product intent is a read-only brief) and open questions Q15 to Q20 for the GUI surfaces.
- **Changed:** `gui_lib.py` and `build_gui.py` gain the Let Em Loose styles, four icons and the top-strip inbox chip. The other generated pages pick up only the shared CSS and icons.
- **Known limitations:** Design only; nothing in `crates/` changes. All names and numbers are sample data, and the real fields to bind are still open (Q9). The pages were checked in headless Chrome but not by a person. Paper references elsewhere in the pack are left as they are.
