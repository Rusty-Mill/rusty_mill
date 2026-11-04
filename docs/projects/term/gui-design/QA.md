# Implementer Q&A — rusty_term GUI design

## Intent and scope
- **Q:** Is the deliverable code? **A:** No. It's frames in Paper. Code is a later handoff. [stated]
- **Q:** Who is it for? **A:** Nano, working across repos, sessions on baileyai, and agent-driven blocks. [stated]
- **Q:** Light theme? **A:** Not now; see Q4. [default]
- **Q:** Spike or lasting? **A:** Lasting design source for the GUI; build reusable components. [default]

## Source material
- **Q:** What's the reference? **A:** `assets/terminal-mux.svg` and `.png` (v2, combined), `assets/terminal-modern.svg` (v1, single terminal). `assets/build_mux.py` holds the exact coordinates, sizes and sample content. [default]
- **Q:** Exact values? **A:** Canvas 1440×900; window 1392×852, radius 12; top strip 52; ribbon 34; block gap 8; block radius 9; block header 38; run rail 3; mono 13 px with 21 px line height; interface text 12–13 px. [default]
- **Q:** Split ratios? **A:** 60/40 horizontal; the right side splits 60/40 vertically. [default]
- **Q:** Fonts in Paper? **A:** Instrument Sans and JetBrains Mono are both on Google Fonts; use those. [default]

## Content
- **Q:** What sample content? **A:** Real rusty_mill work: clippy, git status, cargo test, the rejected git push on `feat/zoom-text-only`; a claude agent editing zoom.rs; cargo watch idle; session `rusty_mill on baileyai`; windows `build`, `agents`, `baileyai`. [default]
- **Q:** Copy style? **A:** Sentence case, plain verbs, no "→" on buttons, no all-caps labels, no middle-dot separators. Button names match their results ("Take over" leads to "You have input"). [default]

## Behaviour per frame
- **Q:** F2 prefix mode: what changes? **A:** The ribbon hints switch from at-rest to active; each block shows a numbered badge; the focused block keeps its border. A timeout or Esc leaves the mode. [default]
- **Q:** F5 reconnecting: what's real? **A:** rusty_mux keeps a 30 s grace period (ORPHAN_GRACE) with a lease, then takes over on a confirmed token; hidden blocks detach in the TUI. Show the countdown and the result. [stated] (#1251)
- **Q:** F6 detached block? **A:** `detach-block` exists (C41, #1393). Show a detached block as a removable chip in the strip, or in a list, with Reattach. [default]
- **Q:** Errors? **A:** Say what happened and what to do, in the interface's voice. Never vague, never apologetic. [default]

## Accessibility
- **Q:** Contrast floor? **A:** 4.5:1 for all text except the greyed-out suggestion (about 3.6:1, deliberately faint). [default]
- **Q:** Status by colour alone? **A:** No. Every status also has an icon and a word (Passed, Exit 1, Idle). [default]

## Paper
- **Q:** How to reach Paper? **A:** Through the Paper MCP server in the Claude desktop app or local Claude Code. If it fails with `spawn EPERM`, copy `paper.exe` to `C:\tools\paper\` and point the MCP `command` at it. [stated]
- **Q:** Which Paper tools? **A:** Discover them from the MCP tool list. Don't assume names. [default]
- **Q:** One page or several? **A:** One page, "rusty_term GUI", with frames F1–F8 left to right. [default]
