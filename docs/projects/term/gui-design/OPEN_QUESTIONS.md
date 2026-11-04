# Open questions — rusty_term GUI design
Claude Code: if a question isn't blocking, add it here instead of stopping.
Format: `- [ ] Q<n> (<date>, blocking? y/n): <question> — interim choice: <what you did>`

- [ ] Q1 (2026-11-03, n): What exactly is a lease? Only "which client has input", or does it also cover resize and terminal control (C40)? — interim choice: show it as input ownership ("Agent has input").
- [ ] Q2 (2026-11-03, n): Is "run" the right name for a command plus its output and result? — interim choice: "run".
- [ ] Q3 (2026-11-03, n): Should "Take over" on an agent block act at once, or ask for confirmation first? — interim choice: design both in F3 and let Nano pick.
- [x] Q4 (2026-11-03, n): Is a light theme needed? — interim choice: dark only. — ANSWERED 2026-11-03: yes, light theme wanted [stated], see D24, D25
- [ ] Q5 (2026-11-03, n): Can OSC 7501 status carry free text ("Editing zoom.rs"), or only a state? — interim choice: free text, with a fallback to the state word.
- [ ] Q6 (2026-11-03, n, code handoff only): Runs need shell integration (OSC 133-style prompt marks); does rusty_term_core parse those? — interim choice: assume yes in the design.
- [x] Q7 (2026-11-04, n): Should window chrome use fixed pack tokens or follow the terminal theme? — interim choice: follow the theme (as today). — ANSWERED 2026-11-04: C, fixed tokens for surfaces and text, accent from the theme [stated], see D34
- [x] Q8 (2026-11-04, n): Under D34, do status colours (success, failure) in chrome stay pack verdigris/oxide, or come from the theme palette (2/1) like the accent? — interim choice: pack tokens (D7 meaning), recorded in D34. — ANSWERED 2026-11-04: fixed pack tokens, verdigris/oxide ("design pack green and red") [stated], see D36
