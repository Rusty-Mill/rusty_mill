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
  awk and sed. An interrupt restores the tty and unwinds the modes a section
  set (mode 2027, kitty keyboard flags, title, colours, mouse and paste modes).
  The OSC 52 clipboard demo needs `RT_CLIPBOARD=1`. PTY interruption tests run
  in CI (`extra/test_rt_caps.py`).
- **Known limitations:** most sections are judged by eye. On main the
  mode 2027 VS16 and keycap rows fail; the gaps the tour shows are tracked as
  issues.
