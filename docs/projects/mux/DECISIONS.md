# Decisions

| Date | Decision | Why | Owner/CC |
|---|---|---|---|
| 2026-10-21 | Build fresh on rusty_mill crates (brief option 3), borrowing phux's protocol ideas | phux's Windows gap is structural; its snapshot bootstrap needs a forked libghostty; most components already exist here (`MUX-ADOPT-DECISIONS.md` §1) | Owner |
| 2026-10-21 | tokio is the async runtime | `rusty_term` is tokio-based; one runtime per binary (`MUX-ADOPT-DECISIONS.md` §2, UDS row) | Owner |
| 2026-10-21 | `rusty_term` is the single VT engine; libghostty-vt is a comparison oracle only | Reuse rule; no Zig/C toolchain; sovereignty | Owner |
| 2026-10-21 | DECCOLM never resizes; honor mode ?40, default off. Verify against Ghostty first | Matches Alacritty and kitty; resizing is xterm opt-in (#856) | Owner |
| 2026-10-21 | #850 (tokio-free `l13`) is low priority | The mux does not need `l13` in Phase 1 | Owner |
| 2026-10-22 | Project and crate prefix: `rusty_mux` | Owner confirmed in the project thread | Owner |
