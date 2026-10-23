# Risks

| Id | Risk | Impact | Mitigation | Status |
|---|---|---|---|---|
| R1 | ConPTY OSC/sixel/resize pass-through. Both wrappers call `CreatePseudoConsole` with flags 0, so ConPTY re-renders (`MUX-ADOPT-DECISIONS.md` spike 1) | Program status, links and images may be lost on Windows | Spike 1 (flags 0 vs `PASSTHROUGH_MODE` 0x8); side channel if OSCs are stripped | open; owner running `spike1-conpty.ps1` |
| R2 | `rusty_term` fidelity: 13/18 corpus cases match the libghostty oracle today (`FIDELITY.md`); the kickoff expects 15/18 after #855 (unverified) | Wrong screens in real apps | Fidelity gate on every engine change; #855, #856 | open |
| R3 | Roaming transport unspecified | Phase 3 protocol may need rework | Spike 4 before committing the protocol to a transport | open |
| R4 | `rusty_tokio` gaps. Its README says it exists "to understand how an async runtime works, not to replace tokio"; the `rusty_term` runtime and TUI path are tokio-based | Missing features or maturity block the server or TUI client | Decided `rusty_tokio` (DECISIONS); extend it per gap with tests; build the TUI client on `rusty_term_core` (tokio-free), not on `rusty_term`'s tokio runtime | open |
| R5 | Hidden egress in anything adopted | Sovereignty breach | Outbound-connection audit before adoption; phux already fails (`MUX-ADOPT-DECISIONS.md` §1 item 4) | open |
| R6 | Engine version skew between server and clients | Clients diverge from the server's grid | Feature bits in `proto`; snapshot envelope versioned | open |
| R7 | libghostty fork / Zig build | Only matters if a second engine is ever wanted | Not in the runtime path (DECISIONS) | accepted |
| R8 | Rex memory claims (≈400 KB vs ≈5 MB) unverified | Wrong scale targets | Spike 5 with our own numbers | open |
| R9 | `rusty_term` performance: 1.6x to 4.5x slower than libghostty-vt at 10k lines (`RUSTY-TERM-PERF-BACKLOG.md` §2.1) | Server CPU under floods | Phase 0 ring buffer with numeric floors | open |
| R10 | No Windows peer identity. `rusty_tokio`'s `peer_cred` is gated to Linux and macOS, and its docs state no verified Windows mechanism exists (`crates/libs/async/rusty_tokio/src/io/unix.rs:815-819`) | A Windows server cannot tell which local user connected | Gate: no authenticated Windows use until a verified peer-identity mechanism exists (feasibility spike before Phase 3/4) | open |
