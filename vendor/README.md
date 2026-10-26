# Temporary RustCrypto migrations for RUSTSEC-2026-0387

These are source patches to **aws-sigv4 1.6.0** and **jsonwebtoken 11.0.0**,
not new releases. Their upstream package versions remain unchanged. Cargo's
root `[patch.crates-io]` selects these checked-in paths, whose exact content
is pinned by the reviewed repository commit. They are excluded from workspace
membership: they remain third-party code with their upstream licenses intact.

The owner approved temporary maintained patches on 2026-10-24. Scope: remove
the affected bigint graph while retaining SigV4a and JWT's pure-Rust backend,
algorithms and key formats. No advisory exception or security-policy change.

## Provenance and reproduction

| Crate | Upstream source revision | Published archive SHA-256 |
| --- | --- | --- |
| aws-sigv4 1.6.0 | [aws-sdk-rust dd4d62780ee72db9adae16134e5dac47a9b6393e](https://github.com/awslabs/aws-sdk-rust/tree/dd4d62780ee72db9adae16134e5dac47a9b6393e/sdk/aws-sigv4) | `2312577f088c9fbf4206dfdb884cf1de9407b43e1a923cbed5237775116fc24b` |
| jsonwebtoken 11.0.0 | [jsonwebtoken 900010fb1be550ec2cfa8c32038eb154f9ef56d7](https://github.com/Keats/jsonwebtoken/tree/900010fb1be550ec2cfa8c32038eb154f9ef56d7) | `881733cbc631fc9e472e24447ce32a64bedf2da498d6d8570b08edc87de71f65` |

`sources.json` records exact immutable archive URLs, the checksums from the
original workspace lockfile, and each archive's `.cargo_vcs_info.json` revision.
`patches/*.patch` is the complete delta from those verified published archives,
including standalone test lockfiles. All other upstream files are unchanged.
The local `.gitattributes` preserves their exact bytes, including CRLF PEM
fixtures; it does not hide diffs or suppress whitespace diagnostics. A full
import whitespace check reports upstream wire-fixture blank lines, that CRLF
fixture and the required context-space lines in unified patches.

Run `python3 vendor/verify.py` to download the archives, verify checksums, apply
the patches in a temporary directory, and compare every vendored file. It never
writes to the working copies. It also checks that maintained and normalized
manifests agree on the patched global dependency scopes and RustCrypto feature
wiring; a WASM-only signature-v3 alias is rejected. Review the small source/manifest hunks separately
from lockfile churn and the unchanged imported source.

## Compatibility boundary

- AWS: only the two manifests and standalone lock change. `sigv4a` still enables
  P-256 ECDSA and key derivation; its existing `U256` operations compile unchanged
  against **crypto-bigint =0.7.6**. P-256 moves to published 0.14.0. No request,
  canonicalization, signing, key-generation or public-API source is rewritten.
- JWT: P-256/P-384 move to published 0.14.0. `signature` 2 stays the public
  provider interface and the RSA/Ed25519/HMAC interface. A feature-gated
  `signature-v3` alias bridges only ECDSA verification errors. Recoverable
  deterministic signing now returns its result directly; raw fixed-width
  `r || s` signatures stay unchanged. SEC1 methods and Ed25519's separate
  PKCS#8 decoding trait are imported from their corresponding versions.
  The RustCrypto backend and every algorithm are retained; no AWS-LC switch.
  Regression coverage compares ES256/ES384 signatures byte-for-byte with the
  unmodified 11.0.0 release and rejects altered, zero and truncated signatures.
- Croc (outside this directory): published P-256/P-384/P-521 0.14.0 replace
  0.13, with SEC1 conversion names updated. The scalar reduction, affine wire
  coordinates, PAKE transcript and SIEC backend are unchanged.
- Root `Cargo.lock` pins the complete shipping graph. The vendored standalone
  locks pin each upstream test harness; always run them with `--locked`.
  AWS's existing MSRV is 1.94.1; JWT and Croc retain 1.88. The selected bigint
  requires 1.87, below the workspace's pinned Rust 1.98.1.

## Validation and maintenance

Run from the repository root with its pinned Rust toolchain:

```sh
python3 vendor/verify.py
cargo test --locked --manifest-path vendor/aws-sigv4/Cargo.toml --all-features
cargo test --locked --manifest-path vendor/jsonwebtoken/Cargo.toml --no-default-features --features rust_crypto,use_pem
cargo test --locked --manifest-path vendor/jsonwebtoken/Cargo.toml --no-default-features --features rust_crypto
cargo test --locked -p agentgateway-auth
cargo test --locked -p rusty-mcp --features jwt
cargo test --locked -p remind_me_core --features cloud-backup --lib cloud_backup::
cargo test --locked -p rusty-croc
bash crates/apps/rusty_croc/scripts/interop_test.sh
cargo deny --workspace check
```

The Croc script uses stock Go croc v10.7.0 revision
`2d9db41befec73287d727506409d87493f513a00` by default. It exercises both wire
directions for p256/p384/p521/siec, relay handshakes, and the existing transfer
matrix. An explicit `CROC_SRC` can supply a different oracle and prints its head.
Tests use loopback services and bundled fixtures; no live AWS service is needed.

The monorepo maintainer owns these patches until removal. On every upstream
security/release update, compare its manifests and RustCrypto changes against
these diffs. Do not let patch versions silently drift: refresh archive hashes,
source revisions, diff files, standalone locks and validation together.

Remove each root patch and its vendored directory once a compatible upstream
release uses only fixed bigint arithmetic with the existing features intact.
Update the root lockfile, rerun the affected tests and all-features cargo-deny,
and require review of that exact head before removal. Croc can remain on the
published curve releases. Do not remove or relabel RUSTSEC-2026-0387; the old
0.5 dependency must remain absent. Existing unrelated advisory exceptions in
`deny.toml` retain their separate maintenance obligations.

## Review correction: source scope and lockfile boundaries

`jsonwebtoken/Cargo.toml.orig` and normalized `Cargo.toml` both declare
`signature-v3` in **global** dependencies, enabled by `rust_crypto`. To exercise
future maintenance from the source manifest itself, copy the vendored directory
to a temporary directory, replace its `Cargo.toml` with `Cargo.toml.orig`, and
run its locked RustCrypto+PEM test suite. This runs on native Linux, so an alias
accidentally placed under the WASM target header cannot pass unnoticed.

Three semver-legal but unrelated resolver reselections were reverted to main's
original edges. These ranges were read from the published crate archives after
verifying each archive against its unchanged root-lock checksum, rather than
inferred from regenerated output:

| Unchanged package | Published dependency requirement | Preserved root-lock edge |
| --- | --- | --- |
| hyper-util 0.1.20 | socket2 `>=0.5.9, <0.7` | socket2 0.5.10 |
| quinn 0.11.11 | socket2 `>=0.5, <0.7` on non-browser-WASM targets | socket2 0.5.10 |
| malloc_buf 0.0.6 | libc `>=0.1, <0.3` | libc 0.1.12 |

All three package versions and checksums remain unchanged. Locked all-features
metadata confirms those exact edges and only crypto-bigint 0.7.6. The native
consumer build covers hyper-util; malloc_buf is a macOS dependency and is
validated here by its authenticated requirement and all-target metadata, not a
claim that it ran on macOS.

The restored Quinn edge is also exercised through a real workspace consumer:

```sh
RUSTFLAGS='--cfg reqwest_unstable' cargo check --locked -p agentgateway-auth --features reqwest/http3
```

The cfg is reqwest's documented opt-in for its HTTP/3 feature, used only by this
additional compile check; no committed feature or provider configuration changes.
A direct `cargo check -p hyper-util -p quinn` invocation hit Cargo 1.98.1's
feature-resolver panic when selecting the non-workspace optional dependency.
The normal consumer check above succeeds and compiles the actual HTTP/3 closure.
