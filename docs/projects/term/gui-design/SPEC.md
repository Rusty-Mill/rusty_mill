# rusty_term GUI design (rusty_term + rusty_mux): spec
Design chat: https://claude.ai/code/session_017VLiNhbGw8DQHY3WeZ52jS (2026-11-03). remind-me tags: `rusty_term`, `ui-design`.
Tags: [stated] [agreed] [default] [open].

## Intent
Design the GUI that rusty_term and rusty_mux share: one window for working across sessions, windows and
split blocks, including blocks driven by agents. The output of this handoff is a set of frames in **Paper**,
not code. [stated] Implementing the frames in `crates/libs/ui/rusty_term` is a later handoff.

## Scope
- In: frames F1–F8 below, built in Paper from the v2 concept in `assets/terminal-mux.svg`, plus a token
  and component sheet.
- Added 2026-11-03 [stated] (D24): light theme, alternative layouts, settings pages, full state matrix; smaller screens out.
- Out: Rust or GUI code changes; wire protocol changes; marketing
  pages.

- Added 2026-11-04 [stated] (D37-D41): Let Em Loose full-app view (stage strip, read-only intent brief, claims map, decision inbox) as a dedicated LEL tab plus an inbox chip in every window's top strip; frames L1-L3 planned on `assets/html/lel.html`; no new colour; no BBP/orch change.

## Constraints
- One interface serves rusty_term and rusty_mux together. [stated]
- The server owns the layout: a session holds windows, and each window holds a split tree of blocks.
  Clients only draw it. [stated] (decision of 2026-10-27, PR #1167)
- "Block" means a pane in the layout (`swap-blocks`, `detach-block`, `REX_BLOCK`). Don't use the word for
  anything else. [stated]
- Layout keys: the Ctrl-] prefix, then `d` detach, arrows or hjkl to move focus, `z` zoom, `|` and `-` to
  split, `x` close. Ctrl-] on its own doesn't detach. [stated]
- The interface (bars) uses its own UI font, separate from the terminal font; zoom scales terminal text
  only. [stated] (PR #1388)
- Nano prefers: small, terse, coherent; a visual learner; Windows and PowerShell locally. [stated]
- Quality floor: text contrast ≥ 4.5:1 (greyed-out suggestion text excepted); keyboard focus always
  visible; reduced-motion versions of every animation. [default]

## Shape
```
┌ top strip ── session switcher │ windows (split-tree thumbnails) + │ clients attached ┐
│ ┌ block (focused) ───────────────┐ ┌ block: agent ──────────────────────┐ │
│ │ header: cwd  branch  changes  ⤢│ │ header: status (OSC 7501)  lease   │ │
│ │ runs: cmd + output + result    │ │ output                             │ │
│ │ input + greyed-out suggestion  │ ├ block: idle ───────────────────────┤ │
│ └────────────────────────────────┘ └────────────────────────────────────┘ │
└ ribbon ── connection  blocks  shell  enc │ Ctrl-] then | - z d hints ─────────┘
```
Frames to build:
- F1 Rest state: match `assets/terminal-mux.svg`.
- F2 Prefix mode: after Ctrl-], each block shows a number for hjkl and number jumps; the ribbon hints
  become active choices.
- F3 Take over: confirm taking an agent block's input (see Q3), then the state after.
- F4 Session switcher open: sessions grouped by host, with attached clients.
- F5 Reconnecting: within rusty_mux's 30 s grace period (ORPHAN_GRACE), with a countdown and what to
  do, then the takeover result.
- F6 Detached block: what a detached block looks like and how to reattach it.
- F7 Zoomed block: a zoomed block, with a clear way out.
- F8 Token and component sheet: colours, type scale, spacing, block header, run, input, keycap, tab and
  thumbnail.

## Acceptance — first slice
- [ ] F8 tokens and components exist in Paper as reusable components, with values from DECISIONS D5–D7.
- [ ] F1 in Paper matches `assets/terminal-mux.png` at 1440×900, built from the F8 components (not a
      pasted image).
- [ ] Every text style passes the contrast floor above, checked with a contrast calculator.
- [ ] Any new choices are recorded through `rusty-term-design-expert`.
- Then F2–F7, each a separate frame on the same page.
