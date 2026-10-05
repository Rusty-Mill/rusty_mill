# Issue #418: source-backed conformance and remaining scope

Reviewed on Beast, 2026-10-05, against fetched `origin/main`
`dab8499d034c5ccc61182f3c9c40ef23f4157bb1`. Worktree branch:
`codex/418-ssrf-conformance-beast`. The policy-preserving SSRF extraction is implemented locally and uncommitted,
ready for review. No policy reconciliation is needed or proposed.

## Follow-up ledger

All six unchecked follow-ups in [#418](https://github.com/Rusty-Mill/rusty_mill/issues/418)
were compared with current source, issue comments, merged PR metadata, and Git
ancestry. Previously merged implementation was not repeated.

| Follow-up | Current state and evidence |
| --- | --- |
| H2 orphan `connect/{config,ping,preface}.rs` | Resolved by [#464](https://github.com/Rusty-Mill/rusty_mill/pull/464), commit `512a984f` (ancestor of this base). Those files are absent; only `connect/mod.rs` remains in that directory. Existing live connection tests include `continuation_floods_are_capped`, `receive_windows_are_enforced_and_released`, and `sent_frames_use_send_transitions_and_the_send_window` in `crates/libs/net/rusty_h2/src/connect/mod.rs`. No dormant modules exported. |
| SSRF classification | Completed locally in this diff. `rusty_ip::classify` owns shared address facts; `rusty_a2a/src/server/push.rs::is_disallowed`, `nexus-memory/src/sync.rs::is_blocked_address`, and `nexus-linkpreview/src/lib.rs::is_blocked_address` keep explicit existing policy. Three real-caller conformance tests preserve the base decisions. |
| Whisper/llama admission | Resolved by merged [#497](https://github.com/Rusty-Mill/rusty_mill/pull/497), merge `99d41507737523d1e5412a2545c4825f1fb45701`. `crates/foundation/rusty_sync/src/lib.rs` owns `Admission`, `AdmissionPermit`, normal/unwind release and concurrent contention tests. `crates/libs/ai/rusty_whisper/src/server.rs` aliases `Slots`/`SlotGuard` to these types; `crates/libs/ai/rusty_llama/src/server.rs` uses `Admission::new(max_connections)`. |
| RDP rustdoc link | Resolved by [#461](https://github.com/Rusty-Mill/rusty_mill/pull/461)/[#462](https://github.com/Rusty-Mill/rusty_mill/pull/462), commit `8007b352` (ancestor). `crates/libs/net/rusty_rdp/src/crypto/bignum.rs:6` now qualifies the link as `crate::security::RsaPublicKey`. |
| Kafka deterministic timeout | Resolved by [#465](https://github.com/Rusty-Mill/rusty_mill/pull/465), commits `f6b38319`/`a0f9510e` (ancestor). `crates/libs/net/rusty_kafka/src/client.rs` has `pending_connector_is_polled_timed_out_and_cancelled`, immediate-success and immediate-I/O-failure tests. The real SYN-drop probe is explicitly ignored and requires an authorized fixture. It was not run here. |
| Jinja quoted delimiters and render budgets | Resolved by [#457](https://github.com/Rusty-Mill/rusty_mill/pull/457), commit `e8f9038a` (ancestor), and merged [#492](https://github.com/Rusty-Mill/rusty_mill/pull/492), merge `3ccdbae9e97c24d1dd901af051e3cb0ab2087dd3`. `crates/foundation/rusty_jinja/src/lib.rs` contains `RenderLimits`, `render_with_limits`, output/work-budget tests, UTF-8 intermediate-value coverage, and quoted-delimiter regressions. |

The open-PR search for `SSRF` returned no overlapping PR. This branch changes
none of the #264/#510 CI work present in its base. No commit, push, issue comment,
merge, issue closure, permission change, or external memory capture was made.

## Current classification contract

The [fixtures](../../crates/foundation/rusty_ip/tests/ssrf_conformance/addresses.tsv) are derived from the
actual source at the base above, not from a substitute definition of "global".
The [shared test helper](../../crates/foundation/rusty_ip/tests/ssrf_conformance/policy.rs) receives each real
production predicate from its crate unit test.

| Address facts | A2A | Memory | Link-preview |
| --- | --- | --- | --- |
| IPv4 loopback, RFC1918, link-local, unspecified, limited broadcast, multicast, `0/8`, CGNAT `100.64/10` | Block | Block | Block |
| IPv6 loopback, unspecified, multicast, ULA `fc00::/7`, link-local `fe80::/10` | Block | Block | Block |
| IPv4-mapped IPv6 `::ffff:0:0/96` | Apply own IPv4 rules | Apply own IPv4 rules | Apply own IPv4 rules |
| `192.0.0.0/24`, `198.18.0.0/15`, `240.0.0.0/4` | Block | Pass except limited broadcast | Pass except limited broadcast |
| Site-local IPv6 `fec0::/10` | Block | Pass | Pass |
| Well-known NAT64 `64:ff9b::/96` | Apply own embedded IPv4 rules | Pass | Pass |
| Local-use NAT64 `64:ff9b:1::/48` | No embedded IPv4 interpretation | Same | Same |
| Deprecated IPv4-compatible `::a.b.c.d` | Native IPv6 rules; only embedded zero/one become `::`/`::1` | Same | Same |
| Documentation addresses, including `203.0.113.5` and `2001:db8::` | Pass | Pass | Pass |

"Pass" describes a classifier returning false, not a claim of global routability.
A2A's broad non-global wording does not mean it implements an exhaustive IANA
denylist. Substituting `is_global` or decoding every embedded form would change
existing behavior and is outside this extraction.

## Implemented extraction boundary

`crates/foundation/rusty_ip` is a dependency-free, allocation-free `no_std` crate
using `core::net` IP types. Its `AddressClass` exposes native address classes and
separately represents mapped and well-known NAT64 embedded IPv4. `Other` makes
no reachability claim. Overlapping ranges use specific classes (unspecified
before this-network, limited broadcast before reserved). The enum is exhaustive,
so adding a class requires each caller to make an explicit decision.

The three existing predicates retain explicit local deny rules and unchanged
signatures. A2A interprets mapped and NAT64 IPv4; Nexus interprets mapped only.
A2A's dependency is optional and activated only by its existing `server` feature.
No third-party dependency was added, and no application depends on another app.
The workspace metadata/map records the foundation crate and its three consumers.

The characterization fixtures were first run through the original predicates,
then through the migrated predicates. Foundation tests separately cover facts,
embedded forms, overlapping classes, and prefix endpoints. No common allow/deny
policy, `is_global` replacement, or new address restriction was introduced.

## Preserved transport boundaries

| Caller | Current behavior retained |
| --- | --- |
| A2A | SSRF protection defaults off. When enabled, require HTTPS, validate at registration and again for delivery, reject any denied DNS answer or an empty answer set, pin validated addresses for delivery/retries, and disable redirects on the pinned client. The disabled path retains its existing client behavior. |
| Memory | HTTP/HTTPS validation remains; private-hub override comes from `allow_private_hub` or `NEXUS_MEMORY_ALLOW_PRIVATE_HUB`. Without override, reject any denied answer; even with override, resolution failures/empty answers still fail. Pin the chosen address; disable automatic redirects. |
| Link-preview | Always applies its address guard, with no private-address override. Reject any denied DNS answer or empty answer set; pin the selected address. Follow at most five redirects manually, revalidating scheme/address and pinning each hop. |

These observations are from `push.rs`, `sync.rs`, and link-preview `lib.rs`.
The new offline classifier tests do **not** claim to validate these I/O contracts.
No actual private-network endpoint was contacted by the new suite.

## Authorized instruction change and manual impact review

GitNexus was unavailable in this environment. Work initially stopped at
conformance evidence without editing Nexus production functions. The user then
explicitly requested removal of the GitNexus check on 2026-10-05. This diff
updates the matching live requirements in Nexus `AGENTS.md`, the mirrored
`CLAUDE.md` block, and the phase-5 RFC. Manual source, caller, dependency, and
test analysis remains mandatory; GitNexus is optional. No tool was uninstalled,
no unrelated check was weakened, and no earlier task exception was reused.
The instruction changes are provided as a separate review patch.

Manual impact review before editing classified this as HIGH security risk:

- A2A: `is_disallowed` is used for literal and resolved addresses by
  `validate_webhook_url`; its callers are registration checking and delivery.
  Engine registration and update notification paths retain the same checks.
- Memory and link-preview: each predicate is called by `validate_url_target`
  and `resolve_public_address`. Memory sync uses the hub path; link-preview's
  core-plugin IPC handler uses `fetch_blocking`. The exported link-preview
  predicate keeps its signature.
- Direct workspace dependents: `adk-a2a` and `agentgateway-a2a` use A2A;
  `nexus-bootstrap` and `nexus-context` use memory; `nexus-bootstrap` uses
  link-preview. New edges point only to a dependency-free foundation crate.
- Production changes are restricted to pure predicates and dependency wiring;
  URL, DNS, client construction, retry, redirect, override and fail-closed
  code is retained. The focused existing regression tests remain necessary.

## Validation on Beast

Rust `1.98.1`, `x86_64-pc-windows-msvc`; Cargo ran offline and locked after
updating the lockfile for the local crate and three local dependency edges.
Compiler scratch files use a process-local directory under
`target/issue418/temp`, because the default Windows temp path was denied.
See the [test README](../../crates/foundation/rusty_ip/tests/ssrf_conformance/README.md) for portable commands.

- PASS: three real-caller conformance tests before and after extraction:
  81 literal fixtures and 275 derived forms per caller, 1,068 decisions total.
- PASS: 6,094,848 comparisons of original and migrated predicates over a finite
  structured address grid: all IPv4 high-16-bit prefixes with seven low-word
  samples, each native/mapped/NAT64/compatible, plus every IPv6 leading word
  with three suffix samples. This is strong sampled evidence, not exhaustive
  enumeration of all IP addresses. Temporary comparison sources/logs are in
  `target/issue418/differential`; no socket operations were used.
- PASS: `cargo test --offline --locked -p rusty_ip -p nexus-linkpreview`:
  3 foundation facts tests, 32 link-preview unit tests, 13 link-preview SSRF
  integration tests, and doc-test targets.
- PASS: `cargo test --offline --locked -p nexus-memory --lib -- --skip
  sync::tests::sync_reports_unreachable_hub`: 109 tests. The one excluded test
  attempts a connection to an unowned localhost port; only tests using owned
  in-process listeners or no I/O were run.
- PASS: A2A `--features client,server --lib server::push::tests`: five tests,
  including exact checked-address reporting, DNS pinning and redirect rejection.
- PASS: A2A `--features client,server --test webhook_ssrf_protection`: five tests,
  covering default opt-out, opt-in HTTPS/private-address rules and registration.
- PASS: four affected crates, `cargo clippy --offline --locked --all-targets
  --features rusty_a2a/client,rusty_a2a/server -- -D warnings`.

- PASS: seven temporary-source mutations fail the actual policy fixtures as
  expected: shifted reserved/NAT64 prefixes, disabled mapped recognition,
  independently relaxed Nexus mapped policies, and independently broadened
  Nexus reserved-address policies. Only copies under `target/issue418` changed.
- PASS: `cargo check --offline --locked -p rusty_a2a --no-default-features`.
- PASS: `cargo check --offline --locked -p adk-a2a -p agentgateway-a2a
  -p nexus-context --all-targets` for the non-bootstrap direct consumers.
- PASS: `cargo test --offline --locked -p nexus-bootstrap --test dep_invariants`:
  all three tests, compiling the remaining direct consumer as well.
- PASS: locked all-features Cargo metadata; `check_workspace_deps.py` and
  `check_workspace_layers.py`; regenerated `docs/WORKSPACE-MAP.md` and verified
  its bytes with `generate_workspace_map.py --verify`. Python used UTF-8 mode
  because the host's default cp1252 decoder cannot read the metadata reliably.
- PASS: affected-crate `cargo fmt --check`, fixture `rustfmt --check`, and diff
  whitespace checks. Source comparison confirms code outside the predicates
  and new unit tests is unchanged in all three production files.
- PASS: `RUSTDOCFLAGS="-D warnings" cargo doc --offline --locked -p rusty_ip
  --no-deps`.
- BLOCKED: all-features checking of the three callers reaches A2A's unchanged
  gRPC build script and fails because `protoc` is unavailable on this host.
- FAIL (pre-existing documentation): strict A2A docs with `server` report nine
  broken/private intra-doc links in unchanged `src/lib.rs`, `server/store.rs`
  and `types/agent_card.rs`. Separate strict Nexus docs report nine more in
  unchanged link-preview `core_plugin.rs` and memory `db.rs`, `vector.rs`,
  `wiki.rs`. Each error-bearing file is unchanged from the base; no unrelated
  docs repair or warning suppression is bundled.

MSRV 1.85.1 is not installed; no MSRV compatibility is claimed. Hosted CI and the
full workspace/platform matrix have not run. No real SYN-drop or private-network
probe was used. No commit, push, merge, issue closure or external memory capture.

The Git warning about denied access to `C:\Users\baileyrd/.config/git/ignore`
remains. No ignore configuration, ACL, credentials, or security setting was
changed to avoid it.

## Independent-review correction: fixture ownership

The initial review patch put the shared fixture files under root `tests/`,
which has no owning Cargo package. Its current manifest changes forced broad
CI, masking that a later fixture-only edit would select no Rust packages.
The correction moves all three files under
`crates/foundation/rusty_ip/tests/ssrf_conformance/` and updates caller includes.
The fixture decisions, helper logic, production classifier and caller policies
are unchanged. No planner, workflow, or #264/#510 file is edited.

The original patches, manifest, chunks and test evidence remain unchanged under
`target/issue418`. Revised patches and correction evidence are stored separately
under `target/issue418/fixture-ownership-correction`.

Correction validation:

- PASS: feed each of `addresses.tsv`, `policy.rs`, and `README.md` independently
  to the unchanged `affected_crates.py` CLI, using fresh locked all-features
  Cargo metadata. The original root paths select zero packages; each relocated
  path is owned by `rusty_ip` and selects 16 packages across five components.
  No manifest/lock path is present in these synthetic changed-file inputs;
  `ci_plan.py` confirms neither a full-workspace nor CI-only fallback applies.
- PASS: `ci_components.py` retains the complete selected set. Relevant lanes
  are `foundation` (`rusty_ip`), `libs--protocol` (`rusty_a2a`) and
  `apps--nexus` (`nexus-memory` and `nexus-linkpreview`, plus their consumers).
  The other selected lanes are `libs--rusty_adk` and
  `apps--rusty_agent_gateway` for transitive consumers.
- PASS: inspect the unchanged test and Clippy jobs and selection exclusions.
  Both Linux/Windows jobs receive these packages and use `--all-features`;
  A2A's resolved `server` feature enables `rusty_ip`. The library test targets
  remain enabled and the nextest filter excludes only the unrelated
  `rusty_lines::windows_raw_mode` binary. This proves planner/feature selection,
  not execution of hosted jobs.
- PASS: relocated fixtures run through all three real predicates: three tests,
  1,068 decisions. Foundation tests: three passed. Four-crate all-target Clippy
  with A2A `client,server` and warnings denied: passed. Formatting and whitespace
  checks pass. Fixture data/helper bytes match the original review exactly;
  caller changes from that review are only their include paths.
- The original broader regression, sampled-comparison and mutation evidence
  remains applicable to unchanged classifier/helper logic. It was not rerun
  for this path-only correction. Known protoc, MSRV, docs and hosted limitations
  remain as recorded above.

Reproduce the saved planner proof with
`python -X utf8 target/issue418/fixture-ownership-correction/verify_fixture_impact.py`.
Its `fixture-only-plan.json` records all selected packages, components, feature
and workflow checks. The script is local review evidence, not a new CI gate.
