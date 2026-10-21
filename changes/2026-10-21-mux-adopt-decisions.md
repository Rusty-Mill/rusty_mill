---
category: Added
changelog: Research note on reusing rusty_mill crates and phux for an own terminal multiplexer, with Windows and libghostty spike results.
---
## 2026-10-21 - Terminal multiplexer adopt decisions

- **Added:** `docs/research/MUX-ADOPT-DECISIONS.md`: one row per target component (core, pty, term, proto, transport, auth, server, clients, ctl, mcp, status, script) with candidate, license, Windows status, size and decision. Recommends building on `platform::pty`, `rusty_term` core, `rusty_mcp_server`, `rusty_tls` and the sessionmgr survival pattern instead of forking or carving phux.
- **Added:** spike results: `phux-server` does not check for Windows (54 errors in four dependencies before the server; 326 Unix API sites in the server); phux's snapshot bootstrap needs fork-only libghostty API (207 vs 173 FFI functions).
- **Known limitations:** the ConPTY tee spike needs a Windows machine and was not run; libghostty from-source builds for musl and Windows were blocked by sandbox egress. No implementation code.
