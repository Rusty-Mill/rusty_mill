# Spikes

Report-only. Results go into [RISKS.md](RISKS.md) and [DECISIONS.md](DECISIONS.md).

| # | Question | Status | Owner | Notes |
|---|---|---|---|---|
| 1 | ConPTY tee: do OSC, sixel and resize pass through (flags 0 vs `PASSTHROUGH_MODE` 0x8)? | in-progress | Owner (Windows box) | Owner runs `spike1-conpty.ps1`. CC feeds the `.bin` captures to `rusty_term_core`, diffs final grids, updates R1 |
| 2 | phux-server on Windows | done | CC | Dependencies fail first (54 errors); server structurally Unix (`MUX-ADOPT-DECISIONS.md` §3) |
| 3 | libghostty-vt on Windows and musl | partly done; closed | CC | Fork builds on linux-gnu; from-source blocked here. Moot while libghostty is oracle-only |
| 4 | Minimal roaming transport (UDP or QUIC, resume after IP change, predictive-echo hook) | done | CC | [SPIKE-4-ROAMING.md](SPIKE-4-ROAMING.md): TCP + `rusty_tls` with resume first, own UDP datagrams for C13, not QUIC. Found a 50 ms wake stall in `rusty_tokio`'s `LocalRuntime` |
| 5 | Idle-terminal memory with and without parking | not-started | CC | Our own numbers only |
