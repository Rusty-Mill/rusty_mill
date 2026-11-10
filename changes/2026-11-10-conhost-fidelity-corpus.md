---
category: Added
changelog: rusty_conhost Windows fidelity corpus test: 17 program-to-terminal sequences must pass through our console host byte for byte (mux risk R1).
---
## 2026-11-10 - Windows fidelity corpus on rusty_conhost

- **Added:** `fidelity_corpus_passes_through_our_host` (rusty_conhost `own_pty` tests). Each of 17 sequences (OSC 7501, OSC 8 links, titles, cwd, progress and notifications, OSC 133, OSC 52, BEL, sixel, kitty and iTerm2 images, modes 2026/2027) is written with WriteConsoleW and with a raw WriteFile, and must reach the terminal byte for byte. ConPTY runs beside it as a baseline only.
- **Result:** 34 of 34 pass on our host on Windows 10.0.26300; ConPTY loses OSC 8 links, BEL, sixel and kitty images. Report in `docs/projects/term/conhost/fidelity-2026-11-10.md`; mux risk R1 is closed for the default host.
