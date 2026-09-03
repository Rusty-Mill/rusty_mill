# Release Notes

Tracks the **monorepo itself** — crate merges, workspace-wide CI, and
cross-crate changes (like the duplication sweeps below) — not each crate's
own internal changes, which are logged in that crate's own
`crates/<name>/RELEASE_NOTES.md` where one exists (many crates kept theirs
from before the merge; see ADR-0001 for why root and per-crate logs are
separate rather than one superseding the other).

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. Bolded inline category tags (`**Added:**` / `**Changed:**` /
`**Fixed:**`), known limitations stated plainly.

---

## Fix `rusty_tokio`'s Windows reactor orphaning a socket on a failed AFD re-arm
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `crates/rusty_tokio/src/io/reactor/windows.rs`'s `event_loop`
  discarded the result of resubmitting a socket's one-shot
  `IOCTL_AFD_POLL` after every completion (`let _ =
  self.submit_poll(&state)`, at both call sites). If that resubmission
  itself failed, no further completion would ever arrive for that
  socket, so a `readable()`/`writable()` wait registered on it
  afterward hung forever with nothing left to wake it — observed as
  four unrelated `rusty_tokio`/`rusty_tls` tests intermittently timing
  out at nextest's ~600s slow-timeout on `test (windows-latest)`, then
  passing in milliseconds on the very next retry ([#153](https://github.com/Rusty-Mill/rusty_mill/issues/153)).
- **Why not the earlier readiness-edge fix ([#140](https://github.com/Rusty-Mill/rusty_mill/pull/140)):** all four
  occurrences happened on runs *after* #140 had already merged, and its
  fix targets a different failure shape (a bit cleared out from under a
  fresh edge) than this one (no bit update ever happens again because
  nothing is watching the socket anymore).
- **How:** both re-arm call sites now check `submit_poll`'s result and,
  on failure, mark both directions ready via a new `mark_orphaned`
  helper — the same "surface both directions so the caller's own next
  syscall discovers the truth" pattern `event_loop`'s sibling
  bad-completion-status branch already used for a different failure
  mode, extended to cover this one.
- **Verified:** `cargo check`/`clippy -D warnings` clean on
  `x86_64-pc-windows-gnu` (cross-compiled from this Linux sandbox, which
  cannot execute Windows tests); the full Linux `rusty_tokio` suite
  passes unaffected (the changed code is `#[cfg(windows)]`-only). Real
  verification comes from `windows-latest` CI itself, the same oracle
  #137/#138/#140 relied on.
- **Known limitation — partial fix:** the `windows-latest` CI run on
  this very branch (33784080891) reproduced the identical hang
  signature on `rusty_tls::async_handshake::async_handshake_succeeds_and_round_trips_with_pinned_anchor`
  (TRY 1 timing out at ~600s, TRY 2 passing instantly) with this fix
  already applied. So this change is real and worth keeping — it closes
  a genuine silently-swallowed-error hole — but it does not fully
  resolve #153. #153 stays open, narrowed to the still-unexplained
  remainder.

---

## Extract `rusty_base64`; close issue #119
**2026-09-03** · branch [`docs/rusty-base64-extraction-issue-119`](https://github.com/Rusty-Mill/rusty_mill/tree/docs/rusty-base64-extraction-issue-119)

- **Added:** `rusty_base64` — `rusty_oauth::encoding::base64`'s complete
  surface (encode/decode, standard and URL-safe alphabets) extracted into
  its own crate, per [issue #119](https://github.com/Rusty-Mill/rusty_mill/issues/119).
  `rusty_request`'s own `base64.rs` was ruled out as a base: it's private,
  encode-only, and standard-alphabet-only, and extending it would have
  meant building a second base64 crate when `rusty_oauth`'s already
  covered the need.
- **Changed:** `rusty_oauth` now depends on `rusty_base64` too
  (dogfooding) instead of keeping its own copy — its public
  `encoding::base64::*` path is unchanged, so none of its own call sites
  needed edits. `rusty_acp`, `rusty-mcp`, and `rusty_a2a` swapped their
  external `base64` crate dependency for `rusty_base64` after checking
  each call site's exact API needs (standard vs. URL-safe, padded vs.
  unpadded, encode vs. decode) rather than assuming a blind swap would fit
  — the same per-crate verification issue #119 itself asked for.
- **Fixed:** the extraction rewrites `encode_with`/`decode_with`'s
  chunking from `slice::as_chunks` (`rusty_oauth`'s original) to
  `chunks_exact`/`remainder`. `as_chunks` is not yet stable at
  `rusty_acp`'s own `rust-version = "1.86"` floor — confirmed against a
  real `+1.86` toolchain before merging, since `rusty_acp`'s own CI
  convention runs `cargo +1.86 test` and this would have silently broken
  it. Behaviorally identical (verified via the original RFC 4648 test
  vectors under both a `+1.86` and the workspace's default toolchain).
- Known limitation: `rusty_croc`, `adk-a2a`, `agentgateway-auth`, and
  `agentgateway` still depend on the external `base64` crate. They weren't
  in issue #119's verified evidence (filed before three of them joined the
  workspace) and weren't checked here — left for separate follow-up rather
  than swapped without per-call-site verification.

---

## Fix `sessionmgr-pty`'s intermittent size-reporting flake
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `LinuxPty::spawn` (`crates/rustils/crates/platform-linux`) set
  a session's pty window size *after* spawning the hosted child, so the
  child could run (and, in `sessionmgr-pty`'s size-reporting test, read
  its own terminal size via `stty size`) before the parent's
  `TIOCSWINSZ` ioctl took effect — a race that had been intermittently
  failing `sessionmgr-pty::tests::the_terminal_reports_the_size_it_was_given`
  on `main`'s `test (ubuntu-latest)` CI job (silently absorbed by
  nextest's retry budget on most runs, then hard-failing it outright on
  [PR #149](https://github.com/Rusty-Mill/rusty_mill/pull/149)), tracked
  as [#150](https://github.com/Rusty-Mill/rusty_mill/issues/150). Fixed
  by setting the size on the pty master before the child is spawned,
  closing the race. `platform`/`platform-linux`/`platform-windows`/
  `platform-mock`/`platform-bsd`/`platform-parity` bumped `0.27.0` →
  `0.27.1` (patch-level: no public API shape changed).

---

## Dependency sovereignty policy (ADR-0002) and the last workspace-member git pins
**2026-09-03** · branch [`claude/review-recommended-changes-8kepe6`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/review-recommended-changes-8kepe6)

- **Added:** `docs/adr/0002-dependency-sovereignty-policy.md` — a three-tier
  classification (Sovereign / Transitional / Adapter) for how a crate's
  external dependencies relate to the workspace's dependency-minimizing
  purpose, written in response to an external Atlas-alignment review that
  found "no external dependencies" does not describe the monorepo as a
  whole (114 of 199 manifests declare a direct external normal dependency).
  A generated, per-crate ledger cross-referencing manifests to tiers is
  tracked as follow-up, not introduced here.
- **Fixed:** `crates/rusty_term/l13`, `crates/rusty_font`, and
  `crates/rusty_gpu` depended on `rusty_lsp`/`rusty_simd` via a pinned git
  URL even though both are workspace members with their own `crates/<name>`
  directory, letting the git and workspace copies silently diverge —
  contrary to `ATLAS-RWC-0050`. All three now use plain path dependencies.
- **Added:** `.github/scripts/check_workspace_deps.py` (with unit tests) and
  a new `dependency-policy` CI job that fails a PR if any workspace member's
  name resolves from a git source anywhere in the dependency graph —
  confirmed to catch the exact violation above by running it against the
  pre-fix manifests.
- Known limitation: `main` is still unprotected on GitHub (no required
  status checks, no branch protection rule), so this new CI job — like the
  rest of `ci.yml` — is not yet a merge gate. That requires a repository
  admin action outside what this PR's tooling can perform; see the Atlas
  review's `ATLAS-TOOL-0010`/`0011` findings.

## Review policy: author self-review when no independent reviewer is available
**2026-09-03** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `CONTRIBUTING.md` said "at least one approval required" while
  every PR merged on 2026-09-02 was authored, self-merged, and unreviewed
  by the same account, which the Atlas evidence review (`docs/atlas/`)
  recorded as an unenforced policy. The policy now matches practice
  honestly: an independent approval when a reviewer is reasonably
  available, otherwise a recorded author self-review against the reviewer
  checklist after CI is green. The PR description must say no independent
  reviewer was available; self-review is never represented as independent
  review (the distinction Atlas `ATLAS-GOV-REVIEW-0061`/`0064` draws).
  Security-sensitive, irreversible, or ecosystem-breaking changes still
  wait for an independent reviewer when one can be found.
- **Changed:** the four PR templates gain a checklist line — "Reviewed:
  independent approval, or self-review recorded in the description" — so
  the record is made on every PR rather than remembered.
- Known limitation: this is documented policy, not enforcement. `main`
  is still unprotected, so nothing stops a merge that skips the record.

## Retire the last `rustils` git pins to path dependencies
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `rusty_tokio`, `rusty_tls`, and `rustils_async` (root manifest
  plus `platform-async`, `platform-async-mock`, `platform-async-linux`,
  `coreutils-async`) depended on `platform`/`platform-linux`/
  `platform-bsd`/`platform-windows`/`platform-mock` through rev-pinned
  `git` dependencies on `baileyrd/rustils`, left over from before
  `rustils` joined this workspace. All twenty-one declarations are now
  `path` dependencies on `crates/rustils/crates/<name>`, the same
  retirement every other first-party pin got when its crate merged.
- **Changed:** `Cargo.lock` drops thirteen git-sourced `platform*` entries
  (three checkouts: two at 0.27.0, `rusty_tls`'s at 0.22.1). Each crate now
  resolves to one in-tree 0.27.0 instance, so consumers share one
  `platform::error::PlatformError` type instead of one per checkout.
- **Verified:** `cargo check`, `cargo clippy -D warnings`, and `cargo test`
  with `--all-features --all-targets` across the six consumers on Linux;
  the Windows and BSD backends are compiled only on their targets, so the
  Windows leg of CI is the evidence for `platform-windows` and nothing
  here exercises `platform-bsd`.
- Known limitation: `rusty_tls` moves from platform 0.22.1 to 0.27.0 in one
  step. It compiles and its tests pass, which is the check the versioning
  rule asks for, but any behavioural change in those five minor versions
  reaches `rusty_tls` with this merge.

## rusty_meshed: reverse-trace & domain-maturity crate
**2026-09-02** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Added:** `crates/rusty_meshed/crates/rusty-meshed-trace` -- the core of
  the *Reverse-Trace & Domain Maturity* spec (Phase 1): the five-level
  `Maturity` ladder, `Domain`/`Source`/`Outcome`/`Requirement` types, a pure
  `trace()` that classifies every requirement (satisfied / blocked / degraded
  / missing), caps the outcome's fidelity at its weakest required domain and
  returns a worst-first bottleneck list, TOML scenario loading via
  `rusty_codec`'s sovereign parser, JSON round-tripping via `rusty_json`, a
  Markdown "gap summary" export, and one shipped scenario (*Acquisition
  Status Dashboard*, ten domains, four outcomes). Fifteen fixture tests cover
  every verdict and edge class, ordering, what-if, and both file formats.
- **Changed:** the crate is a new workspace member; `rusty_meshed/README.md`
  gains a crate-table row and a section on the new capability.
- Known limitation: the shipped scenario's maturity levels are illustrative
  placeholders (spec open question #3), not an assessment; the renderer
  (Phase 2) lives in the source repo's `data-mesh-monitor`, not here.

## Chore: drop committed Python bytecode, ignore it going forward
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** the previous PR ran the new `.github/scripts` unit tests
  locally before staging and swept two `__pycache__/*.pyc` files into the
  commit. Removed from the tree; `__pycache__/` and `*.pyc` added to
  `.gitignore` so it cannot recur.

## CI: unit tests for the affected-crates plan step
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `.github/scripts/test_affected_crates.py` plus a `plan-tests`
  CI job. The plan step decides what every other job runs on a PR, and the
  Atlas review flagged that its nested-crate ownership and
  reverse-dependency traversal had no regression tests. Thirteen cases
  cover: a file inside a crate, outside every crate, the manifest itself,
  a nested crate winning over its parent, `crates/foo` not claiming
  `crates/foobar`, direct and transitive dependents, leaf changes not
  pulling in dependencies, non-workspace dependencies ignored, cyclic
  dev-dependency graphs terminating, sorted/deduplicated output, and a
  member missing from the resolve graph.
- **Changed:** `affected_crates.py`'s graph logic moved into
  `affected_packages(metadata, changed_files)` with type hints; the CLI
  contract (metadata path in argv, changed files on stdin, names on
  stdout) is unchanged and was checked against the real workspace metadata
  for four representative inputs.
- Known limitation: the tests use synthetic metadata; the end-to-end check
  that CI actually scopes to the right crates remains PR #68's round-trip
  test.

## Docs: repository-map corrections from the Atlas review
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** README relationship text that the Atlas review found stale:
  `rusty_simd` was described as the one crate "still outstanding" while
  merged and listed as a member; `rusty_tokio` was described as having no
  in-repo dependents while nineteen workspace packages depend on it by
  `path`; `rustils` was described as outside the monorepo's scope while
  living at `crates/rustils`. The surviving `git` pins on `rustils` in
  `rusty_tokio`, `rustils_async`, and `rusty_tls` are now stated as
  outstanding rather than implied to be by design.
- **Fixed:** `ARCHITECTURE.md` described ATLAS-300 as a seed too draft to
  cite; it is an active volume since Atlas ADR-0006. The section now points
  at `docs/atlas/` for the requirement-by-requirement crosswalk.
- **Fixed:** the Atlas review's `rusty_tokio` dependent count, which was
  taken from a manifest grep that matched a comment in `rusty_proxmox`;
  now taken from `cargo metadata --all-features` at the evidence revision.
- Known limitation: docs only. The `rustils` pin retirement itself is not
  done here.

## Atlas evidence review — revision 2 with corrections
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review.md` — the review
  of this monorepo as exercised evidence for the Atlas Engineering
  Standards Library, revised after every claim was verified against
  Rusty Mill `06ca8669`, the live PR/branch state, and Atlas `390d6b0f`.
  Concludes that ATLAS-300's deferred feature-flag trigger fired (PRs #134
  and #136), and lists the governance corrections this repo needs before
  any conformance claim: protect `main`, enforce the documented review
  policy, and fix the stale README/ARCHITECTURE map.
- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review-corrections.md`
  — the must-fix and should-fix items found in the review's first
  revision, each with the evidence that established it.
- Known limitation: the review is an alignment assessment, not a
  certification, and PR #131 (still open) is excluded from its evidence
  revision.

## Fourth-wave merge — `rusty_agent_gateway` (wave complete)
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_agent_gateway` — a Rust implementation of the
  agentgateway data plane, built as a drop-in for its `config.yaml`:
  configuration, listeners, route matching, policies, and the MCP gateway
  (several upstream MCP servers federated behind one endpoint, with
  tool-level filtering and authorization). Nine crates behind one nested
  workspace, merged via `git subtree` with full history. This completes the
  fourth wave and the monorepo consolidation.
- **Changed:** four pins retired, the most of any crate in this series, all
  to merged siblings — `rusty_a2a` (rev `b9778e1`, 11,324/360 behind),
  `rusty-mcp` (tag `v0.4.1`, 1,367/210 behind — the only tag-pinned
  dependency in the series), `rusty_tls` (rev `7ac6956e`, 109/27) and
  `rusty_tokio` (rev `6d3bb05a`, 3,158/587). The last two had to move
  together by construction, the same `AsyncRead`/`AsyncWrite` trait-identity
  constraint `rusty_request`'s retirement documented.
- **Changed:** this root's `rusty_a2a` and `rusty-mcp` entries now carry
  `default-features = false`, because a member inheriting a workspace
  dependency may not set it when the root does not — and the gateway's
  crates set it deliberately. Verified to be a no-op for their other
  consumers (`adk-a2a`, `rp-mcp`, `rp-server`): both crates' `default`
  feature is empty.
- **Fixed/Changed:** `[workspace.package]` collided on five fields, so its
  crates carry literal `[package]` fields; its `[workspace.lints]` is
  stricter than this root's (`unsafe_code = "forbid"`, `missing_docs`,
  `clippy::todo`, `clippy::unwrap_used`) and is written literally into each
  crate rather than silently downgraded — same call as `rusty_key`'s. Root
  `clap` gained `env`.
- **Known limitation (pre-existing, unchanged):** `hyper`'s `http2` feature
  is load-bearing for the shipped `agentgateway` binary (its TLS listener
  advertises `h2` over ALPN) but Cargo's feature unification means the test
  binary has it regardless — so it must be verified against a built binary
  with `curl`, not by `cargo test`, exactly as before the merge.
- 73 tests pass. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_yirp`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_yirp` — sessionmgr, a Windows-native session
  manager for AI coding-agent CLIs (Claude Code, Codex, Gemini CLI): each
  session optionally in its own git worktree, a TUI grid dashboard, and
  sessions that survive the manager closing. Eight `sessionmgr-*` crates
  plus a Tauri 2 desktop shell behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** four pins retired — `rusty_tokio` (rev `6e6f1847`, from its
  own `[workspace.dependencies]`) and `sessionmgr-pty`'s `platform`,
  `platform-linux`, `platform-windows` (`rustils` rev `ce9259d4`) — all now
  this root's path entries. `sessionmgr-pty`'s manifest warned that a
  second, differing `rustils` pin would build two non-interoperating copies
  of the platform layer; this wave merged a third consumer
  (`rusty_tailscale`, at a different rev again), and a `path` dependency
  settles that by construction.
- **Changed:** `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields.
- **Known limitation:** `sessionmgr-daemon`'s
  `a_fresh_claude_session_reaches_needs_input_on_its_own` drives a real
  `claude` session and skips when `claude` is not on `PATH` — the state of
  a CI runner. On a machine with the CLI installed but no way to complete
  its interactive trust prompt, the guard passes and the test times out.
  Same class as `mill-term`'s known environment-dependent failure. With
  `claude` off `PATH` the suite is 130/130.
- All eight non-Tauri crates cross-compile for `x86_64-pc-windows-gnu`. No
  lint or format fixes were needed.

## Fourth-wave merge — `rusty_provider`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_provider` — an AI provider router: one
  OpenAI-compatible HTTP API in front of OpenAI, Anthropic, Gemini, Groq,
  Together AI and Fireworks, with config-driven fallback chains, budgets,
  metrics, an MCP surface and a CLI. Six crates behind one nested
  workspace, merged via `git subtree` with full history.
- **Changed:** its branch-tracking `rusty-mcp` git dependency retired to a
  `path` dependency on the merged sibling. It had resolved to `ee6c7637` —
  six commits behind the commit this workspace imported, plus two since.
  Verified by running the group's full suite (905 tests) against the swap.
- **Changed:** `[workspace.package]` collided on `license`, so its crates
  carry literal `[package]` fields. Root `reqwest` gained `stream` (SSE
  deltas from upstream providers) and root `tokio` gained `full`, declared
  at the root because Cargo unifies features across the graph either way.
- 905 tests pass. No lint or format fixes were needed — `rusty_provider`
  is the first crate group in this wave to arrive already clean under this
  workspace's `-D warnings` gate and `cargo fmt`.

## Fourth-wave merge — `rusty_adk`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_adk` — a Rust port of the Agent Development Kit
  (ADK) 2.0 architecture: the same data model, graph execution engine, and
  tool/callback contracts, plus MCP and A2A bridges. Eleven library crates
  and three runnable examples behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** `adk-a2a`'s `rusty_a2a` dependency retired from a
  *branch-tracking* git dependency (no `rev`, unlike every other pin in
  this series) to a `path` dependency on the merged sibling. What it had
  actually resolved to was 42 commits behind the commit this workspace
  imported — 9,729 insertions across 66 files — plus three commits since,
  one of which changed `require_auth`'s error type. Verified by running
  `adk-a2a`'s own suite against the swap (13 unit, 8 end-to-end, 10
  remote-transport tests), not by reading the diff.
- **Changed:** `[workspace.package]` collided on `rust-version`, `license`
  and `repository`, so its crates carry literal `[package]` fields. Root
  `tokio` gained `io-std`, `uuid` gained `serde`, and `serde_json` gained
  `float_roundtrip` (which `rusty_adk`'s SQLite session store needs for
  exact f64 round-trips, and which Cargo unifies globally anyway, so it is
  declared where it is visible). `thiserror` and `schemars` stay literal on
  the `adk-*` crates — `"2"` and `"0.8"` against this root's `"1"` and
  `rusty_key`'s `"1.0.4"`.
- **Fixed:** `adk-sessions`' optional `rusqlite = "0.37"` moved to this
  root's `"0.32.1"` — the same `libsqlite3-sys` `links` conflict
  `inventory-core` hit, since Cargo's uniqueness check counts optional
  dependencies it never activates.
- 278 tests pass, 2 ignored. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_tailscale`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_tailscale` — a sovereign pure-Rust Tailscale
  client: ts2021 control plane, WireGuard data plane, DERP/STUN/disco NAT
  traversal, a userspace smoltcp stack, a daemon and a CLI. Fifteen `ts-*`
  crates plus `xtask` behind one nested workspace (whose `members` was a
  `crates/*` glob, expanded to literal entries here), merged via
  `git subtree` with full history.
- **Changed:** `[workspace.package]` collided on `version`, `edition`
  (2024 — the first edition-2024 crates here) and `repository`, so its
  crates carry literal `[package]` fields; only its dependencies were
  hoisted, with `rusty_http`/`rusty_crypto_key` re-pathed to this root's
  `crates/` layout.
- **Changed:** `ts-magicsock` and `ts-tun`'s pinned `rustils` git
  dependencies (`platform`, `platform-linux`, rev `b8bf992f`) retired to
  this root's path entries, same as `rusty_rdp`'s.
- **Fixed:** two pre-existing breaks in `rusty_tailscale`'s own `main` —
  it has no CI of its own and does not compile on Linux. `ts-magicsock`
  called three `platform::net::UdpSocket` trait methods without the trait
  in scope (true at the pinned rev too, so not drift the pin was hiding),
  and `ts-cli`'s `localapi::Error` declared a `Status(StatusCode)` variant
  nothing constructs while `request()` constructed a nonexistent
  `Api { status, body }`. Both confirmed against the standalone repo first.
- **Fixed:** four more `generic-array` 0.14.9 deprecations (`ts-control`'s
  Noise handshake and frame codec, `ts-disco`, `ts-derp`), plus a
  `cargo fmt --all` pass.
- 93 tests pass. All sixteen crates also cross-compile for
  `x86_64-pc-windows-gnu` despite the Linux-first design, so — unlike
  `rusty_stream` — no `windows-exclude` was needed.

## Fourth-wave merge — `rusty_llama`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_llama` — a from-scratch Llama/GGUF inference
  engine: CPU SIMD kernels, optional `wgpu` and CUDA backends, an
  OpenAI-compatible server, and GGUF-embedded Jinja chat templating. Merged
  via `git subtree` with full history.
- No dependency swaps: its `rusty_simd`/`rusty_std` path dependencies
  already pointed at siblings under `crates/`.
- **Fixed:** two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures. They only appear with the `cuda` feature on, which this
  workspace's `--all-features` clippy gate does and the crate's own CI
  never did.
- **Fixed:** `render_jinja_threads_context_variables` asserted a bool
  interpolates as `true`. `minijinja` 2.22 deliberately changed
  none/bool rendering to `None`/`True`/`False` for Jinja2 compatibility;
  the standalone lockfile pinned 2.21, this workspace resolves 2.24. Since
  the code path exists to render templates authored for Python Jinja2, the
  new rendering is the correct one — assertion updated, reason recorded
  inline. Nothing else in the crate interpolates a bare boolean.
- **Changed:** reformatted with `cargo fmt --all` (not fmt-clean under this
  workspace's settings).
- 248 tests pass; 49 stay ignored because they need real model weights on
  disk, by the crate's own design.

## Fourth-wave merge — `rusty_key`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_key` — Rusty Keys, an AI-native application
  skeleton where the model's agent loop is the kernel and the application
  is the harness around it (constrain / feed / observe / compose). Eight
  crates behind one nested workspace, merged via `git subtree` with full
  history.
- **Changed:** its `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields and only its
  dependencies were hoisted (`aisdk`, `schemars`, `toml`, and the `rk-*`
  path entries).
- **Fixed:** `rusty_key`'s `[workspace.lints]` (`unsafe_code = "forbid"`)
  is strictly stronger than this root's, which is `rustils`'
  (`unsafe_code = "warn"`). Leaving `[lints] workspace = true` in place
  would have silently downgraded all eight crates, so each carries a
  literal `[lints.rust] unsafe_code = "forbid"`; `rustils`' crates keep
  inheriting the root table unchanged.
- **Changed:** `crates/rusty_key` reformatted with `cargo fmt --all` — it
  was not fmt-clean under this workspace's settings, same as
  `rusty_ansder`/`rusty_boot` when they merged. No behavior change.
- **Known limitation:** its Tauri desktop shell
  (`crates/rusty_key/desktop/src-tauri`) stays a standalone workspace and
  is excluded here, exactly as its own repo had it — the opposite call from
  `inventory-tauri`, and deliberately so, since each is upstream's own.
  Verified it still builds across the boundary post-merge.
- **Known limitation:** two more duplicate-major pairs now resolve —
  `rmcp` 0.9.1 alongside 3.1.4, and `axum` 0.7.9 alongside 0.8.9. Cargo
  keeps them as unrelated crates and no type crosses between the groups.
- No dependency swaps. Full suite (194 tests) passes unmodified.

## Fourth-wave merge — `rusty_skillopt`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_skillopt` — a from-scratch Rust take on
  Microsoft's SkillOpt: optimize a skill markdown document as the trainable
  state of a frozen LLM agent, with epochs, batches and a validation gate,
  entirely in text space. Four crates behind one nested workspace, merged
  via `git subtree` with full history.
- **Changed:** its `[workspace.package]` collided on `license`, so its four
  crates carry literal `[package]` fields (the `rusty_db` treatment) and
  only its dependencies were hoisted. Root `tokio` widened to the union of
  what `rusty_search`, `rusty_db` and `rusty_skillopt` need; root `chrono`
  gained `serde`. `thiserror` stays literal on `skillopt-core`/
  `skillopt-model`, same `"2"`-vs-`"1"` reason as before.
- **Known limitation:** the workspace now resolves two `reqwest` majors —
  0.13.4 for `rusty_acp`/`rusty_mcp` and 0.12.28 for `skillopt-model`.
  Cargo treats them as unrelated crates so they coexist cleanly and no type
  crosses between the two groups; bumping `skillopt-model` would be an API
  change outside this merge's scope. A build-size cost, not a correctness
  one.
- No dependency swaps: nothing in `rusty_skillopt` depended on a sibling in
  this workspace. Full suite (68 tests, 2 environment-gated ignores) passes
  unmodified.

## Fourth-wave merge — `rusty_inventrory`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_inventrory` — a local-first encrypted index over
  the conversation history Claude Code, Codex, Cursor, Zed, Kiro and
  Antigravity write to disk, plus its `inv` CLI and a Tauri menu-bar shell.
  Three crates behind one nested workspace, merged via `git subtree` with
  full history.
- **Changed:** its `[workspace.package]` collided with this root's on
  `rust-version`, `license` and `repository`, so its three crates carry
  literal `[package]` fields (the `rusty_db` treatment); only its
  dependencies were hoisted. `thiserror` stays literal on `inventory-core`
  for the same `"2"`-vs-`"1"` reason as `rusty_test`'s `contract`.
- **Changed:** CI's Linux leg installs `libwebkit2gtk-4.1-dev`,
  `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (the Tauri
  shell) and `libdbus-1-dev` (`inventory-core`'s Secret Service keyring),
  matching what `rusty_inventrory`'s own CI installed. Windows and macOS
  use OS-native APIs for both.
- **Fixed:** `inventory-core`'s `rusqlite = "0.37"` needs
  `libsqlite3-sys ^0.35`, which cannot coexist with `sqlx-sqlite`'s
  `^0.30.1` — `libsqlite3-sys` sets `links = "sqlite3"`, so exactly one
  version may exist per graph. Moving *up* would mean `sqlx 0.9` across
  `rusty_db` and `rusty-search-sqlite-fts5`, so `inventory-core` came down
  to `rusqlite = "0.32.1"`, unifying with `rusty_sqlite`. Verified by
  running its suite, not by reading changelogs: 79 tests pass unmodified.
- **Fixed:** three deprecated `GenericArray::from_slice` calls in `db.rs`'s
  sealed-index code — same `generic-array` 0.14.9 cause as `rusty_croc`'s,
  same behavior-preserving rewrite.
- No dependency swaps: nothing in `rusty_inventrory` depended on a sibling
  in this workspace.

## Fourth-wave merge — `rusty_test`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_test` — the `portable-runtime-contract` spike:
  one execution contract (`contract`), a per-host adapter (`compat`), a
  verification layer (`conformance`), and three reference tools
  (`stat-tool`, `proc-runner`, `pty-shell`). Merged via `git subtree` with
  full history; its nested `[workspace]` table removed and its six crates
  added to this root's `members`.
- **Changed:** its `[workspace.package]` didn't collide with this root's
  (same edition, same license), so its crates keep inheriting via
  `field.workspace = true` — the `rusty_search` treatment, not
  `rusty_db`'s. Only `publish = false` was new here. `thiserror` was
  deliberately left un-hoisted: `rusty_test` wanted `"2.0"`, this root
  pins `"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
  `thiserror = "2"` instead of forcing a major bump on unrelated crates.
- **Fixed:** `conformance`'s `tests/layering.rs` reads the workspace
  manifest to enforce the layer model, resolving it two directories above
  its own crate and requiring a declared layer for every member found.
  Post-merge that is this monorepo's root — four levels up, ~100 members —
  so three of its four tests panicked. Repointed and filtered through a
  `GROUP_PREFIX` constant; the check's logic is otherwise untouched and all
  four tests pass, alongside the group's other 27.
- No dependency swaps: nothing in `rusty_test` depended on a sibling in
  this workspace.

## Fourth-wave merge — `rusty_croc`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_croc` — a Rust port of
  [croc](https://github.com/schollz/croc), wire-compatible with stock croc
  v10 (PAKE code phrases, relay, local-network hand-off, resume). Merged
  via `git subtree` with its full commit history, as with every prior
  crate import.
- **Fixed:** four `GenericArray::from_slice` calls in `crypt.rs` (AES-256-GCM
  and XChaCha20-Poly1305 nonces). The standalone repo's lockfile pinned
  `generic-array` 0.14.7; this workspace resolves 0.14.9, which deprecates
  the crate wholesale, so `-D warnings` turned them into errors. Rewritten
  to the `From<&[T]> for &GenericArray` conversion `from_slice` delegates
  to — no behavior change, 49 tests pass unmodified.
- No dependency swaps: `rusty_croc` depends only on crates.io crates, not
  on any sibling in this workspace. Its nightly-only `fuzz/` harness keeps
  its own `[workspace]` table and is excluded from this one, same as
  `rusty_tls/fuzz` and `rusty_lsp/fuzz`.

## PR #65 — Deduplicate `rusty_rdp`'s byte cursor and split `rusty_ansder`'s two crates
**2026-09-01** · [#65](https://github.com/Rusty-Mill/rusty_mill/pull/65)

- **Fixed:** `rusty_rdp`'s hand-rolled byte Reader/Writer duplicated
  `rusty_wire`'s (a dependency `rusty_rdp` already declared but never
  used) — now re-exports `rusty_wire`'s cursor types.
- **Changed:** `rusty_ansder` bundled two unrelated libraries (an ASN.1 DER
  codec and a sovereign RAG/Q&A engine). Split the RAG engine into a new
  `rusty_rag` crate; `rusty_ansder` now holds just the DER codec.
- Also investigated and deferred (different-scoped tools sharing a name,
  not true duplication): `rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER
  codec vs. `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs.
  `rusty_url::Url`.

## PR #10 — Collapse workspace duplication: to_wide, read_lines, SHA-1, IFS splitting, glob, raw-mode
**2026-08-27** · [#10](https://github.com/Rusty-Mill/rusty_mill/pull/10)

- **Fixed:** six of eight findings from a five-sweep duplication review
  (issues #1–#8) — `rusty_win32`'s 7x-duplicated `to_wide()` hoisted;
  `rsed`/`rawk`'s shared stdin-reading extracted to `read_lines()`;
  `rusty_git`/`rusty_term`'s independent SHA-1 implementations merged into
  a new `rusty_sha1` crate; `rush`'s two independent IFS-splitting
  implementations merged into `ifs_run_end()`; `rush`'s backtracking glob
  matcher now tries `rusty_regx::Glob` first; a duplicated Windows
  raw-mode flag transformation (`rusty_term`/`rusty_lines`) hoisted into
  `rusty_win32::console::raw_mode_core()`.
- **Known limitation:** two findings closed `no action` — Unix termios
  save/restore (`rusty_term`/`rusty_lines`) is the same shape by
  deliberate, different policy; a `no_std` rounding workaround in
  `rusty_font` (`round_nonneg` vs. `round_f32`) likewise.
- Filed #9 for the remaining gap (`rusty_regx::Glob` needs embedded `!(p)`
  negation support before rush's fallback matcher can be fully deleted) —
  a capability gap, not duplication; still open as of this writing.

## Earlier crate-import history

Every `Import <crate> into crates/<crate>` merge and the CI-scoping work
(`Speed up CI: affected-crate filtering, rust-cache, nextest, parallel
clippy`) predate this file. See `git log --oneline --merges` for the full
list — not backfilled entry-by-entry here since each import is already its
own reviewable commit with a descriptive message, and there are dozens of
them (see the crate table in `README.md` for the two-wave merge history).
