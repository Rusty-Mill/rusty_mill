# Release Notes

Tracks merged PRs against `main`, one entry per PR, reverse chronological.
No version tags yet (pre-1.0), so entries are keyed by PR number rather
than a version.

---

## PR #9 — Add standard repo governance files and Rust CI workflow
**2026-07-23** · [#9](https://github.com/baileyrd/rusty_h2/pull/9)

- **Added:** PR/issue templates, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`,
  `SECURITY.md`, `CHANGELOG.md`, this `RELEASE_NOTES.md`, an
  `ARCHITECTURE.md` describing the crate's real frame/hpack/stream
  boundary, an ADR seed, and a `ci-rust.yml` workflow (fmt/clippy/test) so
  merges have a required CI gate.
- **Known limitation, stated plainly:** the security contact and ADR log
  are seeded, not yet exercised — the first real ADR and any future
  security report will be the actual test of this setup.
- No functional/behavior changes; governance and CI scaffolding only.

## PR #8 — Implement HTTP/2 frame codec, HPACK, and stream state machine
**2026-07-23** · [#8](https://github.com/baileyrd/rusty_h2/pull/8)

- **Added:** the wire-format foundation for the crate — a `frame` module
  covering all ten RFC 9113 frame types plus the shared 9-octet frame
  header, a complete RFC 7541 `hpack` implementation (static table,
  Huffman codec, dynamic table, `Encoder`/`Decoder`), and the RFC 9113
  §5.1 per-stream state machine.
- **Known limitation, stated plainly:** no connection driver, async I/O
  integration, or client/server API yet — this is codec-layer work only.
- 62 tests passing, including the RFC 7541 Appendix C.1/C.3/C.4 test
  vectors decoded through a single `Decoder` instance; `cargo clippy
  --all-targets` clean.
