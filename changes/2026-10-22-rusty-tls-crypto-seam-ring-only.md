---
category: Changed
changelog: "`rusty_tls`: the hand-rolled engine's client path now reaches its cryptography through one internal module (`handrolled::crypto`, still `ring`-backed); no behaviour change. Step 1 of gap 4."
---
## 2026-10-22 - rusty_tls: one crypto seam for the hand-rolled engine's client path (gap 4, step 1)

- **Changed:** a new private module, `handrolled::crypto`, is now the only place the client path calls its cryptographic backend: SHA-256/384, HMAC, the three record AEADs, X25519/P-256/P-384 key exchange, the 11 signature verifiers and the system random fill. `kx`, `record`, `record12`, `schedule`, `schedule12`, `verify` and `client` call it instead of `ring`. The backend is still `ring`, so behaviour is unchanged. This is step 1 of gap 4 in `docs/research/TLS-NATIVE-CRYPTO-SEAM.md` (reopened for the client role on 2026-10-22), the part that needs none of the open decisions D2 to D5.
- **Not changed:** signing (`sign.rs`), the server's random fill and the session-ticket cipher (`ticket.rs`) still call `ring` directly. They are server-role or client-certificate operations, which the native backend will not cover (gap 5 is closed). No public item, gate, default, dependency or consumer changes. No native backend, feature or cfg is added yet; that is step 2 and waits on D2 (an ADR for ADR-0002 §6), D3 and D4.
- **Tests:** none added. The change moves calls without altering them, and the existing suite is the check: `cargo test -p rusty_tls --features handrolled-engine` with `--cfg rusty_tls_handrolled` gives 61 test binaries, 722 passed, 0 failed, 35 ignored, before and after. Clippy (`-D warnings`), rustdoc (`-D warnings`) and fmt are clean.
- **Version:** `rusty_tls` 0.12.0 -> 0.12.1 (no public item's shape changed; `crates/libs/net/rusty_tls/docs/versioning.md` §2).
