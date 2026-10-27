---
category: Added
changelog: rusty_term ships `extra/rt-caps.sh`, a guided capability tour with self-checked width, grapheme and probe rows.
---
## 2026-10-27 - rusty_term capability tour script

- **Added:** `crates/libs/ui/rusty_term/extra/rt-caps.sh`, a POSIX sh tour of
  rusty_term's capabilities (probes, colors, SGR, glyphs, widths, mode 2027,
  cursor, screen editing, sync output, OSC, images, input, throughput). Width,
  grapheme and probe sections check themselves with cursor reports and print
  PASS/FAIL; the rest print what to look for. No dependencies beyond stty, dd,
  awk and sed.
- **Known limitations:** most sections are judged by eye. On main the
  mode 2027 VS16 and keycap rows fail; the gaps the tour shows are tracked as
  issues.
