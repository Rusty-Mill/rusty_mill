# Monorepo improvement review — rusty_mill (round 7)

Reviewed on 2026-09-14 against `main`, via `/codex-build`. This is a seventh,
independent pass — it does not reopen any crate/area a `## \`crate-name\``
header in rounds 1-6 already covered at depth (`CODEX-MONOREPO-REVIEW.md`,
33 findings; `CODEX-MONOREPO-REVIEW-2026-09-12.md`, 63; `-round3.md`, 40;
`-round4.md`, 36; `-round5.md`, 33; `-round6.md`, 34 — all six merged), nor
`repo-inspector-report.md`'s duplication-cluster/sovereignty concern.

All 30 findings below were fixed in the same working session that
produced this report, each with a regression test that fails on the
pre-fix code and passes post-fix.

## Method and scope

Twelve parallel read-only scout passes, run in three waves, covered
surface no prior round had touched (or had only given one shallow
top-level pass despite containing 6-9 distinct sub-crates):

- **Wave 1** (6 scouts): the six never-reviewed `nexus-*` subsystems in
  three groups (kernel/kv/plugins/plugin-api/hashline/types;
  lsp/dap/acp/cli/tui/theme; linkpreview/notifications/comments/panic-log/
  audio/collab/memory/memory-hub/context/protocol); every never-reviewed
  standalone foundational crate (`rusty_gpu`, `rusty_vulkan`, `rusty_simd`,
  `rpath`, `rusty_diff`, `rusty_compress`, `rusty_git`, `rusty_sha1`,
  `rusty_text`, `rusty_sync`, `rusty_crypto_key`, `rusty_ansi`,
  `rusty_regx`, `rusty_wiremock`, `rusty_libc`, `mill-term`, `rush`,
  `rusty_term`); `rusty_mcp` + `rusty_meshed`'s six un-dived sub-crates +
  `rusty_search`'s nine backend adapters; and a monorepo-structural pass
  (workspace manifest, CI, governance files, generated docs).
- **Wave 2** (4 scouts): per-sub-crate deep dives of `rusty_key`,
  `rusty_yirp`, `rusty_agent_gateway`, and `rusty_provider` — each had
  received only one shallow top-level pass in an earlier round despite
  containing 6-9 distinct sub-crates.
- **Wave 3** (2 scouts): `rustils_async`'s six sub-crates +
  `rusty_test`'s three helper-tool binaries (never individually named in
  any prior round); a fresh README/ARCHITECTURE/RELEASE_NOTES/ADR accuracy
  pass distinct from wave 1's monorepo-structural findings.

Each scout was instructed to report only concrete, triggerable defects (or,
for the two documentation-focused scouts, concrete evidenced drift) with
file:line citations and a specific triggering input, at high confidence,
and to return nothing rather than pad with style nits. Several scouts
explicitly reported large swaths of their assigned surface as already
well-hardened and gave the specific mechanisms/tests that made them
confident of that (preserved in each scout's own transcript, linked below
per finding). One scout (`GatewayScout`, all 9 `rusty_agent_gateway`
sub-crates) found the codebase "unusually well-engineered and defensively
documented" with explicit tested mitigations for nearly every attack
surface it was asked to probe, returning exactly one finding; similarly
`YirpScout` (all 9 `rusty_yirp`/`sessionmgr-*` sub-crates) found "nearly
every classic bug class ... has an explicit, tested mitigation with an
inline comment referencing a numbered 'adversarial finding'", also
returning exactly one finding.

All locations are relative to this checkout at the time of review.
Severity: **high** = memory unsafety, durable data loss, credential
exposure, hostile-input resource exhaustion, or a security-policy bypass
reachable from untrusted/public input; **medium** = bounded correctness/
reliability defect or a security gap with mitigating preconditions;
**low** = documentation/config drift or a small avoidable issue.

## Disposition

All 30 findings were fixed by 23 parallel fix tasks (one per disjoint
crate/finding group), each with a regression test that fails pre-fix and
passes post-fix. Several tasks proved the pre-fix defect directly rather
than relying only on a diff of test names: `FixRustyGitIndexAlloc` and
`FixRushArithRecursion` reproduced/measured real native-stack overflows
against the reverted code; `FixEpollReactorLeak` measured `Arc::strong_count`
staying above zero forever pre-fix (confirming the documented shutdown
contract was false); `FixRushGlobExtglob` instrumented the reverted
matcher directly (272,680 calls / 121.8ms at 10 chained groups, a 60s+
hang at 15) before memoizing it; `FixNexusMemorySsrf`/`FixRustyKeySymlinkAndGuide`
spun up real loopback hubs/symlinks proving the pre-fix leak. Two fix
tasks (`FixRustyA2aVersionDrift`, `FixRustyKeySymlinkAndGuide`) required
real structural changes beyond the literal finding — an axum 0.7→0.8 API
migration, and relocating a trait (`ReturnInspector`) across crates to
resolve a dependency cycle — both completed and verified rather than
deferred. After all 23 landed, a workspace-lint sweep (`cargo fmt` +
`cargo clippy --all-targets --all-features -D warnings`, run per touched
crate; `platform-async-linux` via this workstation's WSL Fedora checkout,
being Linux-only) caught 4 additional real lint violations across 4 crates
(`rush`: a `too_many_arguments` violation from the new memoization
parameter, fixed by bundling the recursion's invariant context into a
small struct; `agentgateway-llm`: 5 `unwrap_used` violations in
pre-existing tests whose call now returns `Result`, switched to
`.expect(...)`; `nexus-collab`: an `empty_line_after_doc_comments`;
`nexus-lsp`: a `map_unwrap_or`) — all fixed directly and re-verified
clean, then re-tested with no regressions. A full combined `cargo test`
run across all touched crates afterward found one unrelated failure
(`sessionmgr-daemon`'s `switch_agent` integration test, which needs the
`codex` CLI installed and isn't — confirmed via `git diff --stat` that
the failing test file was never touched by this round's fix, and via
`where codex` finding nothing on this workstation) — a pre-existing
environmental gap, not a regression from any of the 30 fixes.

| # | Disposition | # | Disposition | # | Disposition |
| - | - | - | - | - |
| 1 | fixed | 11 | fixed | 21 | fixed |
| 2 | fixed | 12 | fixed | 22 | fixed |
| 3 | fixed | 13 | fixed | 23 | fixed |
| 4 | fixed | 14 | fixed | 24 | fixed |
| 5 | fixed | 15 | fixed | 25 | fixed |
| 6 | fixed | 16 | fixed | 26 | fixed |
| 7 | fixed | 17 | fixed | 27 | fixed |
| 8 | fixed | 18 | fixed | 28 | fixed |
| 9 | fixed | 19 | fixed | 29 | fixed |
| 10 | fixed | 20 | fixed | 30 | fixed |

| # | Severity | # | Severity | # | Severity |
| - | - | - | - | - |
| 1 | medium | 11 | medium | 21 | low-medium |
| 2 | low | 12 | high | 22 | medium |
| 3 | medium | 13 | medium | 23 | high |
| 4 | medium | 14 | medium | 24 | high |
| 5 | high | 15 | medium | 25 | medium |
| 6 | high | 16 | medium | 26 | medium-high |
| 7 | high | 17 | medium | 27 | high |
| 8 | medium | 18 | medium | 28 | low-medium |
| 9 | high | 19 | medium | 29 | low-medium |
| 10 | medium | 20 | low | 30 | low |

---

## `nexus-plugins` / `nexus-plugin-api` / `nexus-kernel`

**1. `CompositeIpcDispatcher` silently drops per-handler capability gating and Core-trust-only gating for every call it routes.**
Location: `crates/nexus/crates/nexus-plugins/src/composite.rs:79-119` — implements `IpcDispatcher::dispatch`/`dispatch_async` but never overrides `required_caller_caps`/`required_caller_caps_for_args`/`is_handler_internal_only` (`crates/nexus/crates/nexus-plugin-api/src/ipc.rs:87-135`), so it inherits the trait's safe-for-a-leaf-dispatcher defaults (empty caps, `internal_only = false`) — the wrong defaults for a *wrapper*.
Trigger: `KernelPluginContext::ipc_call_inner` (`crates/nexus/crates/nexus-kernel/src/context_impl.rs:186-214`) and the WASM-guest-facing `host::invoke_command` (`crates/nexus/crates/nexus-plugins/src/host_fns.rs`) both consult exactly these methods on whatever `IpcDispatcher` is installed *before* delegating to `dispatch`. Any caller that installs `CompositeIpcDispatcher` (documented in `docs/0.1.2/crates/nexus-plugins.md:75` as the intended community→core IPC boundary-crossing mechanism, and exported as first-class public API via `pub use composite::{CompositeIpcDispatcher, FallbackCell}`) lets a community/WASM caller holding only the blanket `Capability::IpcCall` reach any handler the underlying dispatcher would otherwise gate — including ones marked `internal = true` (Core-trust-only, e.g. `com.nexus.ai::resolve_credentials` per `host_fns.rs`'s own comment).
Caveat: the only current constructor call site is `crates/nexus/crates/nexus-bootstrap/tests/community_to_core_ipc.rs` (a test); the `nexus-app`/`TauriIpcDispatcher` production consumer described in `docs/archive/planning/UI-AUDIT.md:72-76` no longer exists in the tree. No confirmed live production call path today — severity kept at medium, escalates to high the moment any caller wires this documented public API the way its own docstring describes.
Fix direction: add the three missing trait-method overrides, delegating to whichever of primary/fallback owns `target_plugin_id` (mirroring `dispatch`'s own `PluginNotFound`-triggers-fallback logic); `required_caller_caps_for_args` should union/prefer the non-empty answer, `is_handler_internal_only` should return `true` if either delegate says so.
Severity: medium. Source: `NexusScoutA` (`history://NexusScoutA`).

**2. `CapabilitySet`'s doc comment claims a bitmask representation it doesn't have.**
Location: `crates/nexus/crates/nexus-plugin-api/src/capability.rs:326-328` — comment says "Internally a bitmask over the `Capability` discriminant for O(1) contains"; the field is `std::collections::HashSet<Capability>`. No behavioral bug (`HashSet::contains` is also O(1) amortized), but could mislead a future contributor into assuming bitwise union/intersection ops are available.
Fix direction: correct the comment to describe the actual `HashSet`-backed representation.
Severity: low. Source: `NexusScoutA`.

## `nexus-lsp`

**3. Two independent, unencoded `file://` URI builders break on paths containing spaces, `#`, `?`, or non-ASCII bytes.**
Location: `crates/nexus/crates/nexus-lsp/src/client.rs:724-730` (`file_uri`, used for `rootUri`/`workspaceFolders[].uri` in the `initialize` handshake) and `crates/nexus/crates/nexus-lsp/src/core_plugin.rs:1013-1019` (`file_uri_from_path`, used by every `textDocument/*` handler: `open_file`, `hover`, `definition`, `rename`, `code_actions`, etc.).
Trigger: any forge file whose path contains a space, a literal `#` (a legal Unix filename byte, e.g. `src/utils#v2.rs`), a `?`, or non-ASCII characters produces an unencoded URI; per RFC 3986/LSP spec, a `#` starts a fragment and a compliant server-side URI parser truncates everything after it, silently mis-resolving or failing `didOpen`/`hover`/`definition`/`rename`. Confirmed as an avoidable gap, not a design tradeoff: the sibling `rusty_lsp` crate in this same monorepo does this correctly with a passing regression test (`crates/rusty_lsp/src/lsp/base.rs:554-558`), and `crates/rusty_url/src/file_path.rs` already vendors a correct converter.
Fix direction: replace both hand-rolled helpers with one shared conversion built on `rusty_url::Url::from_file_path` (or `rusty_lsp`'s own `Uri::join`/percent-encode helper), and add a decode step wherever a server-returned URI is converted back to a filesystem path.
Severity: medium. Source: `NexusScoutB` (`history://NexusScoutB`).

## `nexus-acp`

**4. `AcpServer::serve`'s main loop processes exactly one JSON-RPC request at a time, blocking unrelated requests behind a long-running `agent/run`.**
Location: `crates/nexus/crates/nexus-acp/src/server.rs:103-133` — reads one request, `await`s `dispatch_request` to completion (up to the documented 600s `DEFAULT_DISPATCH_TIMEOUT` at line 44, intentionally long because `agent/run` "can drive an LLM tool loop with many round trips"), writes the response, only then reads the next message.
Trigger: a parent process sends `agent/run`, then sends `agent/list`/`agent/get` on the same stdio connection expecting a fast status reply — the second request isn't even read off the stream until the first fully resolves, so a status/list check can be delayed up to 600s behind an in-flight run, with no multiplexing workaround (single connection).
Fix direction: spawn each dispatched request on its own task (bounding concurrency, preserving the existing `Mutex`-guarded writer for interleaved-write safety), so `agent/list`/`agent/get` can be serviced concurrently with an in-flight `agent/run`.
Severity: medium. Source: `NexusScoutB`.

## `nexus-collab`

**5. Relay-peer topic spoofing bypasses the kernel's namespace anti-spoof guard.**
Location: `crates/nexus/crates/nexus-collab/src/server.rs:373-380` (`pump_reads`, forwards any peer's `topic` unvalidated) → `client.rs:482-503`/`reconnect_client.rs:482-521` (`run_inbound`, passes the wire-supplied topic straight to `bridge_republish`) → `client.rs:553-562` (`bridge_republish`, calls `bus.publish_core(COLLAB_BRIDGE_PLUGIN_ID, event)` — explicitly documented, per the module's own comment at `client.rs:58-62`, as bypassing the kernel's namespace anti-spoof check).
Trigger: any peer holding a valid relay token (the `TokenSet` design supports multiple named per-user tokens, so one compromised collaborator's credential suffices) sends `{"kind":"envelope","topic":"com.nexus.<any-other-plugin>.<anything>","payload":{...}}`; every other connected peer's bridge republishes it on its **local kernel bus** under an arbitrary plugin's namespace — exactly what `type_id_in_namespace`/`publish_plugin` (`nexus-kernel/src/event_bus.rs:96-107`, added for issue #79 with its own regression tests) was built to prevent. Any local subscriber that trusts `EventFilter::CustomPrefix("com.nexus.<victim-plugin>.")` events as authentic can be fed forged events by a remote collab peer.
Fix direction: validate the inbound `topic` against an explicit allowlist of prefixes the bridge is meant to relay before calling `bridge_republish`; alternatively route through `publish_plugin` with a plugin id that only owns a legitimately-collab-owned namespace instead of the always-open `publish_core` hatch.
Severity: high. Source: `NexusScoutC` (`history://NexusScoutC`).

## `nexus-memory`

**6. Hub-sync client has no SSRF guard and reads an unbounded response body.**
Location: `crates/nexus/crates/nexus-memory/src/sync.rs:53-66` (`parse_config`, resolves `hub_url` from caller-supplied IPC args or env with no scheme/host validation) and `pull` (~L145-175, `resp.json::<Value>().await` with no byte cap) — contrast with sibling `nexus-linkpreview`'s full `is_blocked_address`/DNS-pinning guard + `MAX_BODY_BYTES` cap.
Trigger: an MCP tool call / agent-driven `com.nexus.memory::sync` invocation with `{"hub_url": "http://169.254.169.254/latest/meta-data/...", ...}` (or any internal `10.x`/`192.168.x`/`127.0.0.1` target) makes the process issue an authenticated-looking GET to that internal endpoint; a JSON response with a `records` array gets `serde_json::from_value`'d and `db.upsert_lww`'d straight into the **persistent** memory store — an internal-network probing primitive plus a way to smuggle attacker-chosen content into memories the agent later recalls. A malicious/compromised target returning an arbitrarily large body also exhausts memory via the uncapped read.
Fix direction: reuse (or extract to a shared helper) `nexus-linkpreview`'s `is_blocked_address`/DNS-pinning logic before connecting to `hub_url` in both `push` and `pull`; cap the response body size before JSON-decoding.
Severity: high. Source: `NexusScoutC`.

## `rusty_git`

**7. Unbounded allocation from an untrusted `.git/index` `count` field.**
Location: `crates/rusty_git/src/index.rs:171,181` (`Index::from_bytes`) — `count` (u32, fully attacker-controlled) drives `Vec::with_capacity(count)` before any per-entry bounds check runs; the file's own checksum is a plain SHA-1 integrity check over its own bytes (not a keyed MAC), so an attacker who controls the file trivially produces a matching checksum for hostile content.
Trigger: a ~32-byte crafted index (`DIRC`, version 2, `count = 0xFFFFFFFF`, correct trailing SHA-1 of those exact bytes) passes the checksum check and then attempts a ~240GB allocation, aborting the process before the per-entry bounds check three lines later ever runs.
Fix direction: derive a cap on `count` from `content_len` (e.g. `count.min((content_len - 12) / ENTRY_FIXED_LEN)`) before `with_capacity`, or build the vec incrementally.
Severity: high. Source: `FoundationalScout` (`history://FoundationalScout`).

## `rusty_diff`

**8. O((n+m)²) memory blowup on maximally-different diff inputs.**
Location: `crates/rusty_diff/src/lib.rs:19-27` (`diff_myers`) — `trace.push(v.clone())` clones a `2*max_d+1`-length `Vec<isize>` once per edit-distance step up to `d = n+m`, with no size cap anywhere in the crate.
Trigger: `format_unified_diff`/`diff_myers` on two inputs sharing no common subsequence (e.g. two independently-generated ~50k-line files) reaches `max_d = n+m`, producing multi-GB of `isize` allocation.
Fix direction: cap `n+m` before running the O(D²) variant (reject, or fall back to a linear-space Myers/Hunt-McIlroy variant for large inputs).
Severity: medium. Source: `FoundationalScout`.

## `rush`

**9. `$((...))` arithmetic parser/evaluator recurse with no depth limit.**
Location: `crates/rush/src/arith.rs:566-573` (`Parser::parse_primary`'s `(` branch, recursing through ~15 precedence-level functions per nesting level) and the mirrored recursive `eval_expr` (~line 622) — no depth counter anywhere, unlike `rusty_regx`'s own parser in this same monorepo (`MAX_NESTING_DEPTH = 250`, `crates/rusty_regx/src/parser.rs`) which explicitly guards the identical bug class.
Trigger: `$((` + `(` × 50,000 + `1` + `)` × 50,000 + `))` (ordinary shell script content, no special privilege) overflows the native call stack and crashes the process — not a catchable panic.
Fix direction: track a nesting-depth counter through `Parser` (mirroring `rusty_regx`'s `check_depth`) and reject past a fixed cap instead of recursing unbounded.
Severity: high. Source: `FoundationalScout`.

**10. Extglob `@()`/`?()`/`!()` fallback matcher is exponential-time, unlike the sibling `*()`/`+()` arms which were already memoized after a prior fuzzer-found 30-minute hang.**
Location: `crates/rush/src/glob.rs:382-387` (`match_extglob`'s `@`/`?`/`!` arms, each calling `matches(p, rest, s, k)` unmemoized for every split point) vs. lines 399-419 (the `*`/`+` arms, memoized specifically to fix that prior hang).
Trigger: with `extglob` enabled (or any embedded `!(...)` negation, which always falls back to this matcher regardless of the shopt) a pattern chaining several `@(...)`/`?(...)`/`!(...)` groups against a moderately long filename reproduces the same O(n^m) DoS the `*`/`+` fix was written to close, just through the three untouched sibling operators.
Fix direction: apply the same `memo: &mut [Option<bool>]` technique already used for `*`/`+` to the `@`/`?`/`!` arms.
Severity: medium. Source: `FoundationalScout`.

## `rusty_text`

**11. `awk` interpreter allows unbounded allocation via an attacker-controlled field index or `NF`.**
Location: `crates/rusty_text/src/awk/interp.rs:159` (`set_field`, `self.fields.resize(idx + 1, ...)`) and `:183` (`set_var("NF", ...)`, `self.fields.resize(n, ...)`) — both resize driven directly by a computed/data-derived value with no upper cap; `to_num() as usize` saturates rather than panicking, but a resulting index in the billions still drives a multi-GB `Vec::resize`.
Trigger: an awk script indexing fields by a computed/data-derived value processing untrusted input, e.g. `{ $($1) = "x" }` over a data file where `$1` is `999999999999`, or a script setting `NF = <huge>`.
Fix direction: cap the resize target (e.g. a `MAX_FIELDS` constant) and error past it instead of allocating unconditionally.
Severity: medium. Source: `FoundationalScout`.

## `rusty_mcp`

**12. Percent-encoded slash bypasses the URI-template path-traversal guard.**
Location: `crates/rusty_mcp/crates/rusty-mcp/src/resources.rs` (`UriTemplate::match_uri`, ~lines 460-490) — the `value.contains('/')` rejection check runs on the raw, still-percent-encoded captured substring; `percent_decode` runs afterward, on the way into `params`. The module's own doc comment claims a variable "never matches across `/`, which keeps `file:///logs/{name}` from capturing `../../etc/passwd`" — this does not hold for percent-encoded slashes.
Trigger: register the crate's own documented example template `file:///logs/{name}.log` with a reader that joins `params.get("name")` onto a base directory; a client calling `resources/read` with URI `file:///logs/..%2f..%2fetc%2fpasswd.log` passes the raw `contains('/')` check (no literal `/`), then decodes to `../../etc/passwd` before being handed to the reader — defeating the one documented protection this API provides against directory traversal via a resource template.
Fix direction: decode the captured segment before running the `/`-rejection check (or additionally reject `%2f`/`%2F` in the raw segment).
Severity: high. Source: `McpMeshSearchScout` (`history://McpMeshSearchScout`).

## `rusty_meshed`

**13. `rusty-meshed-registry`'s `data_products::list()` validates only the upper limit bound, letting a negative `limit` bypass the page-size cap entirely.**
Location: `crates/rusty_meshed/crates/rusty-meshed-registry/src/routers/data_products.rs:186-196` — `offset` gets a symmetric `offset < 0` rejection; `limit` only gets `limit > 100`. SQLite treats a negative `LIMIT` as "no limit"; the same bug class was already identified and fixed in the sibling `rusty-search-sqlite-fts5::query_map::compile` (see its `compile_clamps_usize_max_limit_and_offset_to_non_negative_i64` test) but never propagated here.
Trigger: `GET /data-products?limit=-1` returns the entire table (optionally joined against `output_ports`) regardless of the intended ≤100-row cap.
Fix direction: add `if limit < 0 { return detail_error(...); }` (or `.max(0)`) mirroring `offset`'s existing check.
Severity: medium. Source: `McpMeshSearchScout`.

**14. `rusty-meshed-schema-registry`'s client splices caller-supplied `subject` unescaped into the request URL path.**
Location: `crates/rusty_meshed/crates/rusty-meshed-schema-registry/src/client.rs` — `set_compatibility`/`get_subject_compatibility` (~line 122, 143: `format!("{}/config/{subject}", ...)`) and `register_schema` (~line 168: `format!("{}/subjects/{subject}/versions", ...)`) — no percent-encoding, unlike the identical pattern already fixed in `rusty-search-elasticsearch`/`solr`/`algolia`/`azure-search` (each cites "finding 31 of CODEX-MONOREPO-REVIEW-2026-09-12.md" in its own comment).
Trigger: `set_subject_compatibility("../config", "NONE")` produces `{base_url}/config/../config`, which a compliant proxy normalizes to `{base_url}/config` — silently setting the *global* compatibility mode instead of the intended per-subject override, defeating the enforcer's whole purpose. Today's in-repo callers pass developer-configured names (bounding current exploitability), but the pattern is exactly what was rated a real defect and fixed in the sibling search-backend crates.
Fix direction: add the same `encode_path_segment` helper used in the search-backend crates, applied to `subject` before formatting into a URL.
Severity: medium. Source: `McpMeshSearchScout`.

**15. `rusty-meshed-observability`'s `contract_gate.rs` has the identical unescaped-subject-splice defect.**
Location: `crates/rusty_meshed/crates/rusty-meshed-observability/src/contract_gate.rs` — `register_consumer_contract` (~line 58) and `assert_schema_compatible` (~line 84) splice `contract_subject` (`format!("{consumer_group}.contracts.{producer_subject}")`, both halves caller-supplied) into the Schema Registry URL path unescaped — same defect class and same fix as finding 14, but a distinct crate/call site.
Fix direction: same `encode_path_segment` helper, applied to `consumer_group`/`producer_subject` before composing `contract_subject`, and to the composed value before URL formatting.
Severity: medium. Source: `McpMeshSearchScout`.

## Monorepo workspace / CI

**16. Stale leftover nested-workspace `Cargo.toml`/`Cargo.lock` for `rusty_mcp` silently shadows the root workspace for anyone building from inside that directory.**
Location: `crates/rusty_mcp/Cargo.toml:1-3` (`[workspace] resolver = "3" members = [...]`, overlapping the root's own `crates/rusty_mcp/crates/rusty-mcp{,-demo}` member entries) plus a committed `crates/rusty_mcp/Cargo.lock` proving it has actually been built standalone at some point.
Trigger: `cd crates/rusty_mcp && cargo build` silently resolves against this stale, narrower dependency graph/resolver/lints instead of the root's, with no error — CI never touches it (always invoked `--workspace`/`-p` from repo root), so this is purely a local-dev trap that can also produce a further-diverging committed `Cargo.lock`.
Fix direction: delete `crates/rusty_mcp/Cargo.toml` and `crates/rusty_mcp/Cargo.lock`, mirroring how nexus's and rusty_search's own former nested workspace tables were fully removed during their merges.
Severity: medium. Source: `MonorepoMetaScout` (`history://MonorepoMetaScout`).

**17. Same stale-nested-workspace defect for `rusty_db`, with an actually-diverged dependency pin.**
Location: `crates/rusty_db/Cargo.toml:1-8` (own `[workspace.dependencies]` pinning `sqlx = "0.8"` vs. root `Cargo.toml`'s `sqlx = "0.9"` — the root's own comment explains 0.8 hard-pins an `libsqlite3-sys` version incompatible with nexus's `rusqlite`, a collision this isolated graph never has to face) plus a committed `crates/rusty_db/Cargo.lock`.
Fix direction: delete `crates/rusty_db/Cargo.toml` and `crates/rusty_db/Cargo.lock`.
Severity: medium. Source: `MonorepoMetaScout`.

**18. `rusty_a2a` pins `reqwest 0.12`/`axum 0.7`, a full major behind the workspace-hoisted `0.13`/`0.8` every other consumer uses, undocumented in the root Cargo.toml's otherwise-thorough drift commentary.**
Location: `crates/rusty_a2a/Cargo.toml:20` (`reqwest = { version = "0.12", ... }`), `:26` (`axum = { version = "0.7", ... }`) vs. root `Cargo.toml`'s hoisted `0.13`/`0.8`, consumed by `agentgateway-a2a`, `adk-a2a`, `rusty-mcp`. Because `rusty_a2a` is a plain path dependency of `agentgateway-a2a`/`adk-a2a`, a `--all-features` sweep (which CI runs on every push to main) legally links both major versions of each crate simultaneously.
Fix direction: bump to the workspace-hoisted versions (a real migration, like adk-a2a's own documented axum 0.7→0.8 move) and inherit via `{ workspace = true }`, or explicitly document the split alongside every other known one.
Severity: medium. Source: `MonorepoMetaScout`.

**19. `rusty_agent_gateway`'s `rust-toolchain.toml` pin is structurally invisible to CI, contradicting its own stated purpose.**
Location: `crates/rusty_agent_gateway/rust-toolchain.toml` (`channel = "1.97.0"`, header comment: "Pinned so a local check and CI check the same thing... A floating toolchain makes that a recurring surprise"). `.github/workflows/ci.yml`'s `clippy`/`test` jobs use `dtolnay/rust-toolchain@stable` from the repo root and never `cd` into this subdirectory; rustup's toolchain-file discovery only walks upward from cwd, never descends into subdirectories, so this file is never read by CI.
Trigger: a contributor who `cd`s into the crate and runs `cargo clippy -- -D warnings` gets 1.97.0's lint set; CI's `-D warnings` clippy job for the same crate gets whatever `stable` currently resolves to — a lint introduced after 1.97.0 fails CI while passing locally, exactly the "recurring surprise" the file claims to prevent.
Fix direction: either add a root-level `rust-toolchain.toml` pinning the same version, or change this file to `channel = "stable"` like `nexus`/`rusty_key`'s equivalent files (which pin the same floating target CI already uses, so they have no actual drift risk despite the identical invisibility).
Severity: medium. Source: `MonorepoMetaScout`.

**20. `rusty_agent_gateway/SECURITY.md` ships a literal unfilled contact placeholder.**
Location: `crates/rusty_agent_gateway/SECURITY.md:6` — `"...or reach <fill in — team alias or individual> directly..."`, unlike every other sampled crate's SECURITY.md (`rusty_provider`, `rusty_win32`, `rusty_a2a`, root `SECURITY.md`), which all list a real contact (`baileyrd@gmail.com` / `@baileyrd`).
Fix direction: fill in the same contact used by every other crate's SECURITY.md.
Severity: low. Source: `MonorepoMetaScout`.

**21. Root `docs/generated/capabilities.md` is a stray, unmaintained duplicate that will silently go stale.**
Location: `docs/generated/capabilities.md` (repo root) is byte-identical today to `crates/nexus/docs/generated/capabilities.md`, but `crates/nexus/scripts/check_ipc_drift.sh:116-133` — the only tooling that regenerates/diff-checks it — resolves its `docs/generated` path relative to `crates/nexus/`, never touching the root copy.
Trigger: the next time a capability is added/removed/renamed in `nexus-plugin-api`'s `Capability` enum and a contributor runs the regen script from its natural invocation directory (`crates/nexus/`), only the `crates/nexus` copy updates; the root copy silently drifts with no CI or script ever comparing it against the real enum again.
Fix direction: delete the root-level copy (or replace with a pointer to the `crates/nexus` copy) so there is exactly one copy the regen script actually maintains.
Severity: low-medium. Source: `MonorepoMetaScout`.

## `rusty_provider`

**22. Gemini provider adapter splices an unescaped, client-controlled `model` string into its request URL path.**
Location: `crates/rusty_provider/crates/providers/src/gemini.rs:47-49` (`fn endpoint(&self, model: &str, method: &str) -> String { format!("{}/v1beta/models/{model}:{method}", self.base_url) }`) — `model` is the bare model half of a client-supplied `"provider/model"` string, split via `str::split_once('/')` in `rp-router`'s `resolve_chain` (`router/src/lib.rs:1757-1760`) with no charset/format validation before reaching this `format!`, used by chat (line ~526), streaming (~597-601), and embeddings (~694).
Trigger: a caller embeds `?`, `#`, `/`, or `..` in the model half (via the HTTP API or the MCP `chat_completion`/`embeddings` tools, which pass `args.model` through the same path unchanged) to inject query parameters, truncate/redirect the intended `:generateContent` suffix, or path-traverse to a different `v1beta` endpoint on the same host — all still authenticated with the operator's own configured API key (confused-deputy: the client never sees the key but can redirect its use).
Fix direction: percent-encode `model` as a URL path segment (or reject any value containing `/`, `?`, `#`) once, in `resolve_chain` or in `GeminiProvider::endpoint` itself, covering both the HTTP and MCP call paths.
Severity: medium. Source: `ProviderScout` (`history://ProviderScout`).

## `rusty_agent_gateway`

**23. LLM response-streaming SSE re-framer has no cap on its internal buffer, unlike every other buffered path in the same crate.**
Location: `crates/rusty_agent_gateway/crates/agentgateway-llm/src/stream.rs:397-419` (`EventParser::push`, `self.buffer.push_str(...)` with no size check) consumed by `crates/rusty_agent_gateway/crates/agentgateway-llm/src/lib.rs:738` (`LlmBackend::stream()`) — the crate's `buffered()`/`guarded_stream()`/error paths all call `read_capped(response, MAX_RESPONSE_BYTES)` (32 MiB cap, `lib.rs:66`), but the true-streaming path bypasses that cap for the forwarding logic *and* drops the parsing buffer's bound in the process.
Trigger: any upstream (a misbehaving/compromised LLM provider, or — chained with the intentionally-permissive `dynamic` backend in `agentgateway-proxy/src/dynamic.rs` — an attacker-steered route) that streams SSE content without ever emitting the blank-line terminator keeps growing `buffer` unboundedly for the connection's lifetime, growing gateway memory without bound until client/upstream disconnect.
Fix direction: cap `EventParser::buffer` (mirroring `agentgateway-proxy::MAX_REPLAY_BYTES`'s pattern) and terminate the stream with a gateway error when exceeded.
Severity: high. Source: `GatewayScout` (`history://GatewayScout`).

## `rusty_key`

**24. Workspace-boundary confinement is purely lexical and never resolves symlinks, while every filesystem tool that consumes it follows them.**
Location: `crates/rusty_key/crates/constrain/src/policy.rs` (`within_workspace`/`WorkspacePolicy` — lexical-only path containment) vs. `crates/rusty_key/crates/feed/src/builtins.rs` (`resolve()`/`read_file_impl`/`write_file_impl`/`edit_file_impl`/`list_directory_impl`/`glob_impl`/`grep_impl` — open the policy-vetted path via std/tokio fs calls, which follow symlinks).
Trigger: a symlink committed inside the workspace (trivial in git) that points outside the workspace root passes the lexical `within_workspace` check (the symlink's own path is inside the workspace) but resolves, once opened, to an arbitrary location outside it — defeating the entire workspace-confinement guarantee for both reads and writes, the core safety property `PolicyChain` exists to enforce.
Fix direction: canonicalize the resolved path and re-verify it against the workspace root (or reject symlinks outright) before any fs operation in `feed::builtins`.
Severity: high. Source: `KeyScout` (`history://KeyScout`).

**25. `AGENT_GUIDE.md` is loaded from the (possibly untrusted) repository with no size cap and without the crate's own prompt-injection inspection.**
Location: `crates/rusty_key/crates/feed/src/guide.rs` (`GuideLoader::load_layers`/`read_trimmed` — no size cap, no `ReturnInspector`-style vetting), loaded unconditionally at every session start via `crates/rusty_key/crates/app/src/session.rs:~302` (`GuideLoader::load(&config.workspace)`), then folded into the cached system prompt — contrast with `crates/rusty_key/crates/mcp/src/inspect.rs`'s `DefaultInspector`, applied to every MCP/web tool return but not to guide content.
Trigger: a hostile/compromised repository's `AGENT_GUIDE.md` is injected straight into the agent's system prompt at every session start with no size bound and no prompt-injection classification, unlike any other untrusted-content channel in the crate.
Fix direction: apply a size cap and run guide content through the same `ReturnInspector` classification path used for MCP/web tool output before caching it into the system prompt.
Severity: medium. Source: `KeyScout`.

## `rusty_yirp` / `sessionmgr`

**26. Session state and full PTY transcripts are persisted with default, unhardened Unix file permissions — world-readable on any multi-user host.**
Location: `crates/rusty_yirp/crates/sessionmgr-daemon/src/paths.rs` (`state_root()`/`ensure_dir()`, plain `std::fs::create_dir_all`) and `src/catalog.rs` (`write_session()`/`append_transcript()`, plain `std::fs::write`/`OpenOptions::append`) — zero occurrences of `set_permissions`/`chmod`/`mode(`/`0o600`/`0o700` anywhere in the crate, so the ambient umask (commonly `022`) yields `0755` directories and `0644` files.
Trigger: run any session as user A on a shared Linux/macOS host with the common `umask 022`; user B on the same box can `cat ~A/.local/state/sessionmgr/sessions/*/state.json` (containing the full launch `command: Vec<String>`) and `transcript.jsonl` (every byte the session ever printed/received) — reading another user's agent-CLI prompts, source code, and full terminal content. The project's own `sessionmgr-daemon/src/hooks/dispatch.rs` (lines ~9-14, ~48-53) already recognizes this exact data as sensitive, deliberately excluding command line and transcript from outbound webhook payloads "because an initial prompt can carry a pasted secret" — yet writes both unprotected to disk.
Fix direction: `#[cfg(unix)]`-gated `set_permissions` to `0o700` on the state root and each `sessions/<id>` directory, and `0o600` on `state.json`/`transcript.jsonl`/`daemon.json`/the daemon socket, mirroring the crate's existing platform-conditional-hardening style. (Windows is unaffected — per-user NTFS ACLs already restrict cross-account access by default.)
Severity: medium-high (local information disclosure of secrets on a shared host; not remotely triggerable). Source: `YirpScout` (`history://YirpScout`).

## `rustils_async`

**27. `EpollReactor`'s background thread holds its own strong `Arc<Self>`, so the reactor's `Drop` — which is the only place `shutdown.trigger()` is called — can never run via ordinary Arc-drop; the thread and its epoll fd leak permanently.**
Location: `crates/rustils_async/crates/platform-async-linux/src/sys/reactor.rs:54-80` (`EpollReactor::new()` clones the freshly-created `Arc<Self>` into the spawned thread's closure, `.spawn(move || worker.run())`) and `:179-225` (`fn run(self: Arc<Self>)` takes ownership by value and only returns once `self.shutdown.is_triggered()`, but `Drop::drop` — the only caller of `shutdown.trigger()` — only runs once the strong count reaches zero, which it structurally cannot while the thread is one of the strong holders still executing `run()`).
Trigger: any long-lived process that constructs and drops more than one `AsyncLinuxSpawner`/`EpollReactor` over its lifetime (e.g. a service spinning up a fresh spawner per request, or a test harness per test case) leaks one thread + one epoll fd per drop, unboundedly. Currently latent for the sole real consumer (`arun`/`coreutils-async::block_on`) only because that binary exits almost immediately after one spawner is used and process exit tears down all threads regardless. Directly contradicts the crate's own documented contract (`reactor.rs:51-53`: "the reactor stops and its thread is joined when the last `Arc` is dropped").
Fix direction: have the background thread hold a `Weak<Self>` instead of an owned `Arc<Self>`, upgrading once per `epoll_wait` iteration and exiting the loop as soon as `upgrade()` returns `None`.
Severity: high (correctness bug / resource leak — no memory unsafety, but a real, currently-untested violation of the crate's own documented shutdown contract). Source: `TestToolsAsyncScout` (`history://TestToolsAsyncScout`).

## `rusty_test`

**28. `proc-runner` panics on non-UTF-8 command-line arguments.**
Location: `crates/rusty_test/tools/proc-runner/src/main.rs:10` (`std::env::args().skip(1)`) — `proc-runner`'s entire purpose is forwarding an arbitrary `<program> [args...]` to a subprocess, exactly the kind of tool legitimately handed non-UTF-8 argv. This is the identical bug class `crates/rustils/crates/coreutils/src/args.rs` was written to eliminate elsewhere in this same repo (`args_os()`/`collect_lossy`, applied to all 15 `rustils` coreutils binaries) — `proc-runner` (a separate workspace) reintroduces the pre-fix pattern.
Trigger: `proc-runner /bin/echo $'\xff'` on Linux panics instead of erroring cleanly.
Fix direction: switch to `std::env::args_os()` + lossy decode (mirroring `coreutils::args::collect_lossy`) — `ProcessSpec.args: Vec<String>`'s typing requires either lossy conversion or widening the contract to `OsString`.
Severity: low-medium. Source: `TestToolsAsyncScout`.

**29. `stat-tool` has the same non-UTF-8-argv panic.**
Location: `crates/rusty_test/tools/stat-tool/src/main.rs:11` (`std::env::args().nth(1).unwrap_or_else(|| ".".to_string())`) — a non-UTF-8 path argument (plausible for a filesystem-stat tool pointed at an arbitrary directory entry) panics instead of being handled; the value is immediately turned into a `&Path` anyway, so no `String` round-trip is even needed.
Trigger: `stat-tool <path-with-invalid-utf8-byte>` panics instead of stat'ing the entry or reporting a clean error.
Fix direction: `std::env::args_os().nth(1)` feeding directly into `Path::new(&OsString)`.
Severity: low-medium. Source: `TestToolsAsyncScout`.

## Documentation

**30. Root `README.md`'s crate-provenance "History" narrative omits 5 real, currently-present workspace crates.**
Location: `README.md`'s History section (lines 7-33 intro, wave-by-wave lists at lines 1225-1308) describes itself as the *complete* provenance of every crate under `crates/`: four numbered `baileyrd/rusty_*` merge waves ("All four waves are complete... every repo that was in scope now lives under `crates/`", line 15-16) plus three explicitly-called-out non-wave merges (nexus, `rusty_multimodal_db`, `rusty_hister`). Every crate name in all four wave lists was individually re-tallied and matches exactly (14 + 15 + 26 + 11) — but `rusty_proxmox`, `rusty_opnsense`, `rusty_homelab_mcp`, `rusty_fedora_agent`, and `rusty_fedora` appear in none of the four lists nor the three non-wave paragraphs, despite all five being real, current workspace members (confirmed in root `Cargo.toml`) with their own rows in the README's own crate table (lines 88-92). Notably `ARCHITECTURE.md`'s dependency-layering table *does* name `rusty_proxmox`/`rusty_opnsense`/`rusty_homelab_mcp` explicitly — this is specifically a hole in README's self-described-as-complete merge-history narrative, not an obscure/unused crate. `RELEASE_NOTES.md`'s earliest entries only go back to 2026-08-27, with no "Added: rusty_proxmox/opnsense/homelab_mcp" entry either — the origin of this crate cluster isn't recorded in any root doc.
Fix direction: add a wave/merge entry to README.md's History section (folding these 5 crates into an existing wave paragraph with a landing note, or describing them as their own out-of-band addition, following the nexus/multimodal_db/hister pattern) so the "all four waves are complete" claim actually accounts for every crate the table lists.
Severity: low. Source: `GovDocsScout` (`history://GovDocsScout`).
