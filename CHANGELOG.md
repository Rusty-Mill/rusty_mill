# Changelog

All notable changes to **this monorepo itself** are documented here —
crate merges, workspace-wide CI, cross-crate changes. Each crate's own
internal changes are logged in that crate's own
`crates/<name>/CHANGELOG.md` where one exists (see ADR-0001 for why root
and per-crate logs are separate). Format: Added / Changed / Deprecated /
Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `crates/rusty_rand` — new workspace member: OS-backed CSPRNG bytes
  (`fill`/`bytes`, `Result`-returning; cached `/dev/urandom` handle on
  Unix, hand-declared `BCryptGenRandom` FFI on Windows), no external
  dependencies. Extracted from three identical copies in `rusty_oauth`,
  `rusty_uuid`, and `sessionmgr-proc`, all of which now wrap it
  (repo-inspector Section 1 row 6).
- `rusty_simd::f32_to_f16` — the reverse of the existing `f16_to_f32`
  (round-to-nearest-even, NaN preserved, overflow → ∞, exhaustive
  round-trip test); `rusty_llama` and `rusty_whisper` re-export both
  directions instead of carrying their own (rows 3–4).
- `rusty_wiremock::canned` (behind a new `std` feature) — a working
  sequential canned-response HTTP mock server, moved out of the four
  identical `tests/support/mod.rs` copies in `rusty_proxmox`,
  `rusty_opnsense`, `rusty_fedora`, and `rusty_homelab_mcp` (row 8).
- `repo-inspector-report.md` gained a **Disposition** section recording
  what was done, or deliberately not, for every row of both sections.
### Changed
- `rusty_base64`'s decoder now rejects misplaced or excess `=` padding
  (`Z=9v`, `Zm9v====`, `Zg==Zg==`) and a padded input that is not
  4-aligned, instead of stripping every `=` and guessing; `DecodeError`
  variants carry the offending index/byte/length. Missing padding is
  still accepted (base64url needs it).
- The last three hand-rolled base64 copies (`rusty_request`, `ts-control`,
  `sessionmgr-protocol`) and the last four external `base64` crate users
  (`adk-a2a`, `rusty-croc`, and `agentgateway`/`agentgateway-auth`'s test
  suites) all use `rusty_base64`; external `base64` is gone from every
  workspace manifest (Section 1 row 5 + Section 2 `base64`).
- `adk-core` mints ids with `rusty_uuid` instead of external `uuid`;
  `rk-feed` validates egress URLs with `rusty_url` instead of external
  `url` (Section 2 `uuid`/`url`).
- `sessionmgr-proc` no longer needs `windows-sys`'s
  `Win32_Security_Cryptography` feature (its `os_random` is `rusty_rand`).
- `crates/rusty_fedora_agent` — new workspace member: an unprivileged local
  agent exposing scoped systemd/dnf/config-file control over a small
  synchronous (`tiny_http`) HTTP API, for managing a Fedora Server host
  (e.g. baileyai) that has no REST management API of its own. Built on
  `rustils`' `platform`/`platform-linux` process-spawning layer
  (`SystemController`/`PackageController` ports, `SystemdAdapter`/
  `DnfController` adapters); privilege scoping (polkit unit allowlist,
  sudoers-scoped `dnf install`/`remove`, config-path allowlist with
  automatic `.bak` on write) ships as reviewable templates under
  `deploy/`, not applied automatically. `tiny_http` is a new external
  dependency (workspace-hoisted) — deliberately synchronous, no
  tokio/axum, matching `rustils`' own reasoning for keeping tokio out of
  its platform layer.
- `crates/rusty_fedora` — new workspace member: async typed client for
  `rusty_fedora_agent`'s HTTP API, same shape as `rusty_opnsense`/
  `rusty_proxmox` (built on `rusty_request`, passthrough JSON).
- `rusty_homelab_mcp` gained a `fedora` module: 10 new tools
  (`fedora_system_status`, `fedora_list_services`,
  `fedora_service_control`, `fedora_read_journal`,
  `fedora_dnf_list_updates`, `fedora_dnf_install`/`fedora_dnf_remove`,
  `fedora_task_status`, `fedora_read_config`/`fedora_write_config`),
  following the existing OPNsense/Proxmox discovery-then-mutate and
  `$defs` enum conventions exactly.
- `rusty_base64` — hand-rolled, dependency-free Base64 (RFC 4648, standard
  and URL-safe alphabets, encode/decode) extracted from
  `rusty_oauth::encoding::base64`, closing issue #119. `rusty_oauth` now
  depends on it too (dogfooding); `rusty_acp`, `rusty-mcp`, and `rusty_a2a`
  swapped their external `base64` dependency for it after their exact
  call-site needs were verified. Chunking uses `chunks_exact`/`remainder`
  rather than `rusty_oauth`'s original `slice::as_chunks`, which is not
  stable at `rusty_acp`'s `rust-version = "1.86"` floor (confirmed against
  a real `+1.86` toolchain before merging). `rusty_croc`, `adk-a2a`,
  `agentgateway-auth`, and `agentgateway` still depend on external
  `base64` — out of this issue's verified scope, left for separate
  follow-up.
- `crates/rusty_meshed/crates/rusty-meshed-trace` — reverse-trace and
  domain-maturity model for `rusty_meshed` (maturity ladder, scenario types,
  pure `trace()` with fidelity verdict and worst-first bottlenecks, TOML
  scenarios via `rusty_codec`, JSON via `rusty_json`, Markdown gap summary,
  one shipped scenario); new workspace member
- `docs/adr/0002-dependency-sovereignty-policy.md` — a Sovereign /
  Transitional / Adapter tier classification for crate external-dependency
  posture, plus `.github/scripts/check_workspace_deps.py` (with unit tests)
  and a `dependency-policy` CI job enforcing `ATLAS-RWC-0050`: no workspace
  member may resolve from a git source anywhere in the dependency graph
### Fixed
- `crates/rusty_tokio`'s Windows reactor discarded the result of
  re-arming a socket's one-shot `IOCTL_AFD_POLL` after every completion;
  a failed re-arm silently stopped monitoring that socket forever,
  hanging any later `readable()`/`writable()` wait on it — surfaced as
  intermittent ~600s timeouts on unrelated `rusty_tokio`/`rusty_tls`
  tests on `test (windows-latest)` (#153). Both re-arm sites now mark
  both directions ready on a failed resubmission instead of hanging
- `crates/rustils/crates/platform-linux`'s `LinuxPty::spawn` set a
  session's window size *after* spawning the hosted child, racing the
  child's own reads of its terminal size against the resize ioctl;
  surfaced as an intermittent `sessionmgr-pty` test flake on `main`
  (#150). Reordered to size the pty before the child starts
- `crates/rusty_term/l13`, `crates/rusty_font`, and `crates/rusty_gpu`
  resolved `rusty_lsp`/`rusty_simd` via a pinned git dependency instead of
  the workspace's own copy of those crates; switched to path dependencies
### Added
- `.github/scripts/test_affected_crates.py` — unit tests for the CI plan
  step's ownership and reverse-dependency logic (nested crates, directory
  name prefixes, transitive dependents, external deps, cycles), run by a
  new `plan-tests` job; `affected_crates.py`'s graph logic moved into an
  `affected_packages()` function so the tests can drive it without cargo
- `docs/atlas/` — the Rusty Mill → Atlas evidence review (revision 2,
  verified against Rusty Mill `06ca8669` and Atlas `390d6b0f`) and the
  list of corrections applied to its first revision
- `rusty_croc` merged into `crates/rusty_croc` via `git subtree` (fourth
  wave), full history preserved
- `rusty_test` merged into `crates/rusty_test` via `git subtree` (fourth
  wave) — six crates (`contract`, `compat`, `conformance`, `stat-tool`,
  `proc-runner`, `pty-shell`) behind one nested workspace
- `rusty_inventrory` merged into `crates/rusty_inventrory` via `git subtree`
  (fourth wave) — three crates (`inventory-core`, `inventory-cli`,
  `inventory-tauri`) behind one nested workspace
- `rusty_skillopt` merged into `crates/rusty_skillopt` via `git subtree`
  (fourth wave) — four crates (`skillopt-core`, `skillopt-model`,
  `skillopt-envs`, `skillopt-cli`) behind one nested workspace
- `rusty_key` merged into `crates/rusty_key` via `git subtree` (fourth
  wave) — eight crates (`rk-config`, `rk-observe`, `rk-constrain`,
  `rk-feed`, `rk-kernel`, `rk-mcp`, `rk-compose`, `rk-app`) behind one
  nested workspace; its Tauri desktop shell stays excluded, as upstream
  had it
- `rusty_llama` merged into `crates/rusty_llama` via `git subtree` (fourth
  wave) — a single crate; its `rusty_simd`/`rusty_std` path dependencies
  already resolved to merged siblings
- `rusty_tailscale` merged into `crates/rusty_tailscale` via `git subtree`
  (fourth wave) — fifteen `ts-*` crates plus `xtask` behind one nested
  workspace; its `platform`/`platform-linux` git pins retired to this
  root's `rustils` path dependencies
- `rusty_adk` merged into `crates/rusty_adk` via `git subtree` (fourth
  wave) — eleven `adk-*`/`rusty-adk` crates plus three examples behind one
  nested workspace; `adk-a2a`'s branch-tracking `rusty_a2a` git dependency
  retired to a path dependency on the merged sibling
- `rusty_provider` merged into `crates/rusty_provider` via `git subtree`
  (fourth wave) — six `rp-*` crates behind one nested workspace; its
  branch-tracking `rusty-mcp` git dependency retired to a path dependency
  on the merged sibling
- `rusty_yirp` merged into `crates/rusty_yirp` via `git subtree` (fourth
  wave) — eight `sessionmgr-*` crates plus a Tauri desktop shell behind one
  nested workspace; its `rusty_tokio` pin and `sessionmgr-pty`'s three
  `rustils` pins retired to this root's path dependencies
- `rusty_agent_gateway` merged into `crates/rusty_agent_gateway` via
  `git subtree` (fourth wave, and the last of it) — nine
  `agentgateway-*` crates behind one nested workspace; four pins retired
  (`rusty_a2a`, `rusty-mcp` at tag `v0.4.1`, `rusty_tls`, `rusty_tokio`)
- CI's Linux leg now installs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libdbus-1-dev` for
  `inventory-tauri` and `inventory-core`'s Secret Service keyring backend

### Changed
- `CONTRIBUTING.md` review policy: one independent approval when a reviewer
  is reasonably available; otherwise the author self-reviews against the
  reviewer checklist after CI is green and records that in the PR
  description. Self-review is never represented as independent review, and
  security-sensitive, irreversible, or ecosystem-breaking changes still wait
  for an independent reviewer. All four PR templates gain a matching
  checklist line
- The last `git` pins on `baileyrd/rustils` retired to workspace `path`
  dependencies: `rusty_tokio` (rev `ce9259d4`), `rusty_tls` (rev `93b00ce9`,
  platform 0.22.1), and `rustils_async`'s root plus four member crates (rev
  `83ab7a9e`). Thirteen git-sourced `platform*` lockfile entries collapse
  into the single in-tree 0.27.0 copy of each crate
- Root `[workspace.dependencies]`: `tokio`'s feature list widened to the
  union `rusty_search`/`rusty_db`/`rusty_skillopt`/`rusty_adk` need (`fs`,
  `process`, `io-util`, `io-std`); `chrono` gained `serde` for
  `skillopt-core`; `uuid` gained `serde` and `serde_json` gained
  `float_roundtrip` for `rusty_adk`; `reqwest` gained `stream` and `tokio`
  gained `full` for `rusty_provider`; `clap` gained `env` for
  `rusty_agent_gateway`
- Root `rusty_a2a` and `rusty-mcp` entries carry `default-features = false`
  so `rusty_agent_gateway`'s crates can inherit them — a no-op for their
  other consumers, since both crates' `default` feature is empty

### Fixed
- Two `__pycache__/*.pyc` files committed alongside the affected-crates
  tests removed; `__pycache__/` and `*.pyc` are now ignored
- `rusty_croc`'s four deprecated `GenericArray::from_slice` nonce
  constructions rewritten to the equivalent `From<&[T]>` conversion — the
  workspace resolves `generic-array` 0.14.9, where they fail this repo's
  `-D warnings` clippy gate
- `conformance`'s `layering.rs` layer-boundary check repointed at the
  monorepo root and scoped to `crates/rusty_test/` — it had located the
  workspace manifest two directories up and demanded a layer assignment for
  every member it found
- `inventory-core` pinned to `rusqlite = "0.32.1"` (from `"0.37"`): its
  `libsqlite3-sys ^0.35` requirement conflicts with `sqlx-sqlite`'s
  `^0.30.1` on the `sqlite3` `links` key, the same constraint `rusty_sqlite`
  hit; 79 tests pass unmodified against the pin
- `inventory-core`'s three deprecated `GenericArray::from_slice` calls in
  `db.rs` rewritten, same `generic-array` 0.14.9 cause as `rusty_croc`'s
- `rusty_key`'s eight crates keep a literal `[lints.rust] unsafe_code =
  "forbid"` instead of inheriting this root's `[workspace.lints]`, which is
  `rustils`' weaker `"warn"` — inheriting would have silently downgraded
  the policy
- `crates/rusty_key` reformatted with `cargo fmt --all` (it was not
  fmt-clean under this workspace's settings)
- `rusty_llama`'s two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures, only visible with `--all-features`
- `rusty_llama`'s `render_jinja_threads_context_variables` expectation
  updated from `true` to `True`: `minijinja` 2.22 changed bool rendering
  for Jinja2 compatibility, and this workspace resolves 2.24 where the
  standalone lockfile pinned 2.21
- `crates/rusty_llama` reformatted with `cargo fmt --all`
- `ts-magicsock` now imports `platform::net::UdpSocket`, without which
  `send_to`/`recv_from`/`local_addr` do not resolve — a pre-existing break
  in `rusty_tailscale`'s own `main` (it has no CI), confirmed against the
  standalone repo
- `ts-cli`'s `localapi::Error::Status(StatusCode)` replaced with the
  `Api { status, body }` variant `request()` actually constructs — the same
  pre-existing break
- Four more `generic-array` 0.14.9 deprecations across `ts-control`,
  `ts-disco` and `ts-derp`; `crates/rusty_tailscale` reformatted
- `adk-sessions` moved from `rusqlite = "0.37"` to this root's `"0.32.1"`
  entry — the same `libsqlite3-sys` `links` conflict `inventory-core` hit,
  and the same resolution
- README: `rusty_simd` is no longer "still outstanding", `rusty_tokio` has
  nineteen in-repo dependents rather than none, and `rustils` is in-tree
  (its surviving `git` pins in `rusty_tokio`/`rustils_async`/`rusty_tls`
  are noted as not yet retired). ARCHITECTURE: the ATLAS-300 reference no
  longer describes it as a seed and points at `docs/atlas/` for the
  crosswalk. `docs/atlas/`: the `rusty_tokio` dependent count corrected to
  the `cargo metadata` figure

## [workspace] - 2026-09-01
### Fixed
- `rusty_rdp`'s hand-rolled byte cursor deduplicated against `rusty_wire`
  ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))
- Six workspace-wide duplication findings resolved (glob matching, SHA-1,
  `to_wide()`, `read_lines()`, Windows raw-mode flags, IFS splitting)
  ([#10](https://github.com/Rusty-Mill/rusty_mill/pull/10))

### Changed
- `rusty_ansder` split into itself (DER codec only) and a new `rusty_rag`
  crate (RAG/Q&A engine) ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))

<!-- No version tags on this repo as such — "[workspace] - DATE" entries
     group changes to the monorepo's own build/governance surface, distinct
     from any per-crate version a crate under crates/<name> might carry on
     its own. Earlier crate-import history isn't backfilled here entry-by-
     entry; see RELEASE_NOTES.md's note on that and `git log --oneline
     --merges` for the full list. -->
