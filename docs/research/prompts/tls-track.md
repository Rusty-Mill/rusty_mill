# Session prompt: TLS track (make `rusty_tls`'s native engine a valid replacement for rustls)

You are working in the `rusty_mill` monorepo (Cargo workspace, Rust). Start from branch `claude/peaceful-dirac-200syz`; create your own branch from it and push only there. Do not open a PR unless asked.

## Read first
1. `crates/libs/net/rusty_tls/README.md` and `ARCHITECTURE.md`.
2. `crates/libs/net/rusty_tls/docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md` (667 lines; read all of it). It records the native engine as **permanently non-default** because a wrong TLS stack fails silently in an attacker's favour.
3. `docs/research/MCP-NATIVE-PLAN.md`, "Track B". The owner now wants the native engine to become the default, which means superseding that ADR with an explicit evidence bar.
4. Workspace `docs/adr/0002-*.md` (dependency tiers).

## Goal
Make the hand-rolled engine (`crates/libs/net/rusty_tls/src/handrolled/`, about 10.5k lines: record, handshake, key schedule, kx, x509, path, verify, sign, tickets) a valid replacement for `rustls` behind the existing `rusty_tls` seam, so consumers (`rusty_request`, `rusty_rdp`, and the MCP crates later) change nothing. Today it builds only with feature `handrolled-engine` **and** `RUSTFLAGS='--cfg rusty_tls_handrolled'`, still uses `ring` for AEAD, and `rustls` is the engine behind every exported type (`TlsStream`, `AsyncTlsStream`, `TrustPolicy`, `TlsAcceptor`).

## Constraints
- Do not change `rusty_tls`'s default engine or remove either gate until the owner has approved an evidence bar and you have met it. Security decisions are the owner's.
- Do not edit the MCP crates; another session owns them and depends only on the `rusty_tls` seam.
- Nexus (`crates/apps/nexus`) is frozen: do not touch it.
- Never skip, disable or loosen a test to get green. Failures that look like engine bugs are findings, not noise.

## Decisions you do NOT make alone
The evidence bar for default status; whether to replace `ring`; whether to supersede the crate ADR. Propose, then ask.

## First task: assess, change nothing
1. Build and run the engine's tests: `RUSTFLAGS='--cfg rusty_tls_handrolled' cargo test -p rusty_tls --features handrolled-engine`, plus the normal `cargo test -p rusty_tls`. Record pass/fail counts.
2. Inventory the evidence that exists: differential tests against rustls, known-answer tests (RFC 8448, NIST/Wycheproof vectors), fuzz targets, interop tests against a real server, negative certificate-path tests (name constraints, expiry, wrong EKU, bad signatures, unknown critical extensions). List what is missing for each of: TLS 1.3 client, TLS 1.3 server, TLS 1.2, session resumption, client auth.
3. Review the highest-risk code for correctness gaps: `x509.rs`, `path.rs`, `verify.rs`, `sign.rs`, `der.rs` (parsing strictness, constant-time comparisons, error paths that accept by default).
4. Check rusty_tls#25 (and any linked issues) for stated acceptance criteria.
5. Write `docs/research/TLS-ENGINE-ASSESSMENT.md`: what exists, what evidence is missing, a ranked list of gaps, a proposed evidence bar, and an effort estimate per gap. Then stop and report.

## Working rules
Small, modular, minimal dependencies (the aim is to remove `ring` and `rustls` eventually, not add). Explicit types, `Result` + `?`, no `unwrap()` outside tests. Keep diffs focused; match surrounding code.
Before each commit: `cargo fmt --all`; `cargo clippy -p rusty_tls --all-targets -- -D warnings` (also with the handrolled cfg); `cargo test -p rusty_tls`; then workspace policy: `cargo metadata --format-version=1 --all-features --locked > $S/m.json` and `.github/scripts/check_workspace_deps.py`, `check_workspace_layers.py`, `generate_workspace_map.py > docs/WORKSPACE-MAP.md`. Add a CHANGELOG.md entry and a dated RELEASE_NOTES.md section. Disk is limited: if a build fails with "no space", `rm -rf target/debug` and rebuild.
Report faithfully: say what you did not verify.
