---
category: Added
changelog: "`rusty_tls`: the `native-crypto` feature: with `--cfg rusty_tls_native_crypto`, the hand-rolled engine's client path runs on the first-party `rusty_sha2`, `rusty_pk`, `rusty_aead` and `rusty_rand` instead of `ring` (ADR-0005), with signing and the server role compiled out."
---
## 2026-10-22 - rusty_tls: the hand-rolled engine on first-party primitives (gap 4, step 2)

- **Added:** a native backend for `handrolled::crypto` (`crypto/native.rs`), beside the `ring` one (`crypto/ring.rs`), covering the same items: digests, HMAC, the three AEADs, X25519/P-256/P-384, the 11 signature verifiers and the random fill. It is selected by the new `native-crypto` feature together with `--cfg rusty_tls_native_crypto`, the ADR-0002 §1 two-part gate. A cfg without the feature is a compile error that names the feature. `native-crypto` does not imply `handrolled-engine` (the narrow ADR-0002 §1 exception in ADR-0005, owner decision 2026-10-22), so that build has no direct `ring` dependency and the compiler checks that the engine makes no `ring` call. `ring` stays in the build through `rustls`.
- **Changed (native backend only):**
  - `sign.rs`'s `SigningKey` is an enum with no variants, so no key or client certificate can exist.
  - `server`, `server12`, `ticket` and the server half of `negotiate` are compiled out.
  - ChaCha20-Poly1305 is offered first for TLS 1.3 and 1.2 (ADR-0005 D4).

  The `ring` build is unchanged.
- **Tests:**
  - **Seam differential:** 10 tests in `crypto::native::tests` put the same inputs through both backends. Digests, HMACs, AEAD output and key-exchange secrets must be byte-identical, and both backends must refuse the same bad inputs (tampered ciphertext, tag, AAD and nonce; short buffers; wrong key lengths; off-curve, truncated and low-order peer keys; bad signatures). The tenth checks that the X25519 shared secret is wiped even when the caller's closure panics, through a wipe-on-drop guard; it fails against an explicit wipe after the call.
  - **Whole engine suite on the native backend,** before the server role was compiled out: 61 binaries, 722 passed, 0 failed, 35 ignored, identical to `ring`.
  - **After compiling the server role out:** 11 server or signing suites are marked `// ring-only suite (ADR-0005)`. Client-certificate tests inside the client suites are gated individually. The TLS 1.2 scripted hostile server signs with a test-only `ring` signer on the native backend, so its refusal tests still run there.
- **CI:** a new `rusty-tls-engine-native` job in `required-gate`. It:
  - asserts there is no direct `ring` dependency;
  - runs clippy, rustdoc with `-D warnings` and the tests;
  - runs OpenSSL interop in the client direction;
  - runs a zero-tests guard that requires every non-ring-only suite to run and every ring-only suite to run nothing;
  - runs the x509-limbo differential.
- **Known limitations:**
  - BoGo does not run on the native backend yet, because its shim needs the server role. ADR-0005 asks for a native BoGo floor; the owner agreed on 2026-10-22 to defer it until a client-only shim exists. It is a deferral, not equivalent coverage.
  - The native build cannot present a client certificate or act as a server.
  - AES-GCM is portable and slow.
  - D5 (AES-GCM's review status) must be settled before this merges (ADR-0005). The automated, bounded AES-GCM source review on #651 is now recorded, with no human review. Plan section 11b re-checks the plan's section 11 table for this backend, primitive by primitive, and names what has no human review on record.
- **Version:** `rusty_tls` 0.12.1 -> 0.13.0 (a new feature; `docs/versioning.md` §2).
