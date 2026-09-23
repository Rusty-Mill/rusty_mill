# ADR-0094: The Binaries Refuse an Exposed, Unprotected Listener

- Status: **Proposed and implemented on one branch; the owner asked for
  the hardening** (2026-09-21). The third "do now" item of the
  release-readiness review, after `ADR-0092`/`ADR-0093`.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-EXPOSURE-CHECK-DESIGN.md` (the full
  design), `ADR-0012` (auth; "a server configured with no tokens is
  exactly as open as before"), `ADR-0014`/`ADR-0023` (TLS, mTLS),
  `ADR-0028` (certificates-only is "configured"), `ADR-0032`
  (`ServeOptions::default()` reproduces the original server),
  `README.md` ("do not expose beyond localhost unless both
  authentication and TLS are configured").
- Supersedes/Superseded by: none. Additive: `server::exposure`
  (`Exposure`, `bind_is_loopback`, `check_exposure`,
  `allow_insecure_from_env`, `ALLOW_INSECURE_VAR`), one check in each
  of the four binaries' `main`. `serve`/`serve_tables`/`ServeOptions`
  unchanged; no wire change.

## Context

With no token configured every connection starts `ReadWrite`
(`handle_connection`'s "nothing configured" arm), and the README's
rule against exposing such a server lived only in prose and a banner.
A `memory_server 0.0.0.0:7881` with nothing else set served every
peer on the network read-write in the clear. The release-readiness
review rated this High. The library default is right for a library —
every test binds `127.0.0.1:0` — so the rule belongs in the binaries.

## Decision

Implement: a pure `check_exposure(addr, &options)` — a loopback bind
needs nothing; any other bind (unspecified, LAN, or a hostname the
check cannot classify) needs authentication configured *and* TLS,
else `Exposure::NoAuth`/`NoTls` naming the variables to set. Each
binary calls it after folding TLS into its options: a refusal is a
startup panic naming the address, the reason, and the override;
`SERVER_ALLOW_INSECURE=1` turns the refusal into a warning with the
same text. Proven against the compiled `memory_server` on
`0.0.0.0` with nothing, with a token only, and with the override.

## Consequences

- Positive: the README's rule is enforced where it was only stated;
  an accidental exposed deployment fails at startup, not in an audit.
- Negative / tradeoffs: an operator who fronts the server with a TLS
  proxy on the same host but binds a LAN address must set the override
  (or bind loopback, the recommended shape). A hostname bind is treated
  as exposed because the check does not resolve it.
- Named, not hidden: the library's `ServeOptions::default()` is still
  open; a caller of `serve` with its own binary makes its own call.
  Option (b) — refusing in `serve` itself — is the fork for the owner.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.79.0 / `FR-091`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 654 (up from 651), `memory_server_exposure` 3 (new), 954 tests across 42 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
