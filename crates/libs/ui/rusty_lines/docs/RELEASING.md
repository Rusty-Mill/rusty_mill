# Release checklist

Steps to cut a `rusty_lines` release. Releases are git-based — a tag
plus a GitHub release; the crate is **not** published to crates.io
(`publish = false` in Cargo.toml enforces this). Consumers depend on
the git repository, as rush does:

```toml
rusty_lines = { git = "https://github.com/baileyrd/rusty_lines", branch = "main" }
# or, pinned to a release:
rusty_lines = { git = "https://github.com/baileyrd/rusty_lines", tag = "vX.Y.Z" }
```

Work through the steps in order; each release should be a PR like any
other change.

## 1. Pre-flight

- [ ] `main` is green in CI — all five jobs (Linux test, Windows check,
      `Feature (libc-backend)`, clippy+rustfmt+doc, MSRV). No macOS job —
      see `.github/workflows/ci.yml`'s own comment on the `libc-backend`
      job for why that's an accepted gap, not an oversight.
- [ ] Local checkout is synced to `main` with a clean tree.
- [ ] Full local sweep passes:

  ```sh
  cargo test
  cargo test --no-default-features --features libc-backend
  cargo clippy --all-targets -- -D warnings
  cargo clippy --all-targets --no-default-features --features libc-backend -- -D warnings
  cargo fmt --check
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
  cargo check --all-targets --target x86_64-pc-windows-gnu
  ```

- [ ] MSRV is consistent in all three places: `rust-version` in
      `Cargo.toml`, the CI `msrv` job's toolchain, and the CHANGELOG
      entry that last changed it.
- [ ] The feature matrix is identical in `README.md` and
      `docs/survey.md` (they are deliberately duplicated).
- [ ] Downstream check: point rush at the release candidate revision
      (`cargo update -p rusty_lines` in rush), `cargo build`, and run
      its pty harness — `python3 tests/pty/editor_pty_test.py`, all
      scenarios must pass.

## 2. Version bump (as a PR)

- [ ] Bump `version` in `Cargo.toml`. Pre-1.0 semver: breaking changes
      bump the minor version. (`save_history` taking `&mut self` since
      0.1.0 is an example of a breaking change.)
- [ ] CHANGELOG: retitle `## Unreleased` to `## X.Y.Z` with the date;
      add a fresh empty `## Unreleased` above it.
- [ ] Open the release PR, wait for CI, merge, sync.

## 3. Tag and release

- [ ] Tag the merge commit and push the tag:

  ```sh
  git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z
  ```

- [ ] Create a GitHub release for the tag, body = that version's
      CHANGELOG section.

## 4. Post-release

- [ ] Repin rush to the released tag (or let its `branch = "main"`
      dependency pick it up with `cargo update -p rusty_lines`) and
      re-run its pty harness.
