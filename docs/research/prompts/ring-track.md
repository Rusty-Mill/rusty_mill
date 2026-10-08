# Session prompt: crypto track (native replacement for `ring`)

You are working in the `rusty_mill` monorepo (Cargo workspace, Rust). Start from branch `claude/adoring-cori-x573kc` (it holds the TLS assessment); create your own branch from it and push only there. Do not open a PR unless asked.

## Read first
1. `docs/research/TLS-ENGINE-ASSESSMENT.md`, especially sections 4 (F7), 6 and 7 (decision D2). It records that `ring` underlies every primitive in the native TLS engine and that the earlier recommendation was to keep it. The owner has now asked for a native replacement to be investigated anyway. Your first job is to make that decision well-informed, not to assume it.
2. `crates/libs/net/rusty_tls/docs/adr/0002-*.md`, section 6 ("Cryptographic primitives stay on `ring`, for now"). It says revisiting needs its own issue and ADR. You will draft that ADR; you will not accept it.
3. Workspace `docs/adr/0002-dependency-sovereignty-policy.md` (tiers; Tier S allows external dev-dependencies as test oracles only).
4. Existing crates to reuse, not duplicate: `crates/foundation/rusty_rsa` (`BigUint`, SHA-256), `rusty_rand` (OS CSPRNG), `rusty_crypto_key` (zeroize-on-drop), `rusty_sha1`, `rusty_simd`. Read them first. `rusty_rsa::BigUint` was written for public-key math and is almost certainly **not constant-time**; verify that before any secret-key use.

## What `ring` provides today (from `crates/libs/net/rusty_tls/src/handrolled/`)
SHA-256, SHA-384; HMAC; HKDF; AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305; X25519; ECDH P-256 and P-384; ECDSA P-256 and P-384 verify and sign; Ed25519 verify and sign; RSA PKCS#1 v1.5 and PSS verify (2048 to 8192 bits) and RSA-PSS sign; system randomness. `rustls` itself also uses `ring`, so removing `ring` from the build needs the TLS engine to be the default first (separate track).

## Goal
A small, first-party, dependency-free crypto crate (or a few) in `crates/foundation/`, Tier S, good enough that `rusty_tls`'s native engine could use it instead of `ring`, behind the existing `handrolled-engine` + `--cfg rusty_tls_handrolled` gate. `ring` stays a dev-dependency test oracle.

## Why this is hard (design around it)
Known-answer tests, differential tests and fuzzing cannot see a timing or cache side channel. A correct implementation can still leak a private key to a network observer. So the work is staged by risk, and the evidence bar differs by stage.

| Stage | Contents | Secret-dependent? | Main evidence |
| --- | --- | --- | --- |
| 1 | SHA-2, HMAC, HKDF | Keys only, inherently regular | NIST/RFC vectors, differential vs `ring` |
| 2 | Signature **verify**: RSA, ECDSA, Ed25519 | No (public data) | Wycheproof, differential, fuzz |
| 3 | ChaCha20-Poly1305 | Yes, but regular by design | RFC 8439 vectors, Wycheproof, differential |
| 4 | X25519 | Yes | RFC 7748, Wycheproof, constant-time evidence |
| 5 | AES-GCM | Yes; table AES leaks via cache | Bitsliced or fixsliced only, never T-tables |
| 6 | ECDSA/ECDH P-256, P-384, Ed25519 **sign** | Yes | Complete formulas, constant-time scalar multiplication |
| 7 | RSA **sign** | Yes | Blinding, constant-time bigint; likely last or never |

Stages 1 and 2 touch only public data and are the best value for the least risk. Stop at the end of any stage that does not clear its bar; each stage must be independently useful and abandonable.

## Constraints
- No new third-party dependencies, including RustCrypto crates. Dev-dependencies on `ring` and test-vector data are allowed as oracles.
- `#![forbid(unsafe_code)]` is the default. Hardware intrinsics (AES-NI, PCLMUL) need `unsafe`; that is the owner's call. Ship portable constant-time code first and propose intrinsics as a separate, reviewed step.
- No secret-dependent branches, indexes or early exits in secret-handling code. No variable-time bigint on secrets. Zeroize secrets on drop (`rusty_crypto_key` pattern). Compare MACs and tags in constant time.
- Do not change `rusty_tls`'s default engine, its gates, or any consumer. Another session owns the TLS engine and TLS 1.2 work; touch `rusty_tls` only to add an optional backend seam after the owner approves, and coordinate through the repo, not by editing its files in parallel. Do not edit the MCP crates. Nexus (`crates/apps/nexus`) is frozen.
- Never skip, disable or loosen a test to get green. A divergence from `ring` is a finding until proven otherwise.

## Decisions you do NOT make alone
Whether to proceed past stage 2. Whether to allow `unsafe` intrinsics. Whether any stage replaces `ring` in `rusty_tls`. The constant-time evidence bar. Propose, then ask.

## First task: assess and plan, write no primitive
1. Inventory the exact `ring` surface used (grep the engine and tests; the list above is a starting point, not a guarantee). Note key sizes, curves, and which APIs each consumer needs.
2. Inventory reusable code in the foundation crates above. State, with evidence, which parts are constant-time and which are not.
3. Choose the constant-time evidence method and check which tools exist in this environment: a dudect-style statistical timing harness, valgrind-based secret-taint checking (ctgrind style), and disassembly checks for secret-dependent branches. Say what each can and cannot prove. Be plain that none proves absence of a leak.
4. Check the test-vector sources available offline (NIST CAVP, RFC vectors, Wycheproof JSON). Note which need vendoring and their licences.
5. Estimate performance against `ring` for each stage (portable Rust AES and big-integer RSA are far slower than `ring`'s assembly) and say whether that is acceptable for the consumers (TLS handshakes, bulk transfer in `rusty_request`).
6. Write `docs/research/CRYPTO-REPLACEMENT-PLAN.md`: inventory, stage plan with an evidence bar per stage, effort per stage, risks, and a draft ADR text for the owner (not accepted). Then stop and report.

## Working rules
Small, modular, explicit types, `Result` + `?`, no `unwrap()` outside tests, docstrings on the public surface, tests for every non-trivial path (happy, failure, boundary). Keep diffs focused; match surrounding code.
Before each commit: `cargo fmt --all`; `cargo clippy -p <crate> --all-targets -- -D warnings`; `cargo test -p <crate>`; then workspace policy: `cargo metadata --format-version=1 --all-features --locked > $S/m.json` and `python3 .github/scripts/check_workspace_deps.py $S/m.json`, `check_workspace_layers.py $S/m.json`, `generate_workspace_map.py $S/m.json > docs/WORKSPACE-MAP.md`. Add a CHANGELOG.md entry and a dated RELEASE_NOTES.md section. Disk is limited: if a build fails with "no space", `rm -rf target/debug` and rebuild.
Report faithfully: say what you did not verify, and never describe constant-time behaviour as proven.
