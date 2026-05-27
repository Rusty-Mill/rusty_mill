# Threat model — trust boundaries, redaction, egress

> **Authoritative source** for trust boundaries and the redaction/egress rules. Other documents (the PRDs, `data-model.md`, ADR-0026) **link here** rather than restating the deny-list, the SSRF block-set, or the trust asymmetry. If a redaction or egress rule appears anywhere else, this file wins.
>
> Rules below are **v1 intent** — the controls to build against and revisit after the Phase 1 spike, not a frozen security boundary. They describe what the constrain/observe layers enforce, not a sandbox guarantee (see [§9](#9-residual-risks--non-goals)).

Related: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (structure, concurrency) · [`data-model.md`](./data-model.md) ([§11 redaction](./data-model.md#11-secret-redaction-adr-0026--summary-full-rule-in-threat-modelmd)) · [`reference/configuration.md`](../reference/configuration.md) (the env vars that arm these controls) · [`prd/02-constrain.md`](../prd/02-constrain.md) (the enforcement point) · ADR-0007 (policy vets before dispatch), ADR-0023 (`PolicyError` enum / structured checker names), ADR-0026 (redaction-by-default).

---

## 1. Trust model

Rusty Keys is a local-first harness around an LLM agent loop. The security posture follows directly from *who is trusted with what*:

| Principal | Trust level | What it can do | Why |
|---|---|---|---|
| **The user** | **Trusted** | Sets the task, approves tool calls, edits config, runs the binary. | The user owns the workspace and the process; the harness serves them. There is no boundary to enforce *against* the user. |
| **The LLM (kernel)** | **Semi-trusted** | Emits tool calls that the harness executes (read/write files, run bash, fetch web, call MCP). | The model is the engine of work, but its tool calls are **adversary-capable**: it can be steered by prompt injection riding in tool results or fetched content, and it can make mistaken destructive choices even with benign intent. We trust the model to *reason*; we do **not** trust its tool calls to be *safe by construction*. |
| **External MCP servers** | **Untrusted input** | Return tool results that re-enter the context. | A third-party process whose output the harness cannot vouch for; its results are data, not instructions. |
| **Fetched web content** | **Untrusted input** | Returned by `web_fetch`/`web_search` into the context. | Attacker-controllable text from the open internet; a classic prompt-injection and SSRF-pivot vector. |

### The asymmetry — why the constrain layer exists

The load-bearing observation is the **trust asymmetry between reasoning and action**. The same model we rely on to plan can be *induced* (by untrusted MCP/web input) or simply *mistaken* into emitting a tool call that deletes files, leaks secrets, escalates privilege, or exfiltrates data. A wrong inference is cheap; a wrong **action** is durable.

This is why the harness cannot be a thin pass-through. Per **Ashby's Law of Requisite Variety**, a regulator must have at least as much variety as the system it governs: to constrain an agent that can emit *any* tool call, the harness must be able to recognize and reject the dangerous subset. The **constrain layer** ([PRD 02](../prd/02-constrain.md)) is where that variety-reduction happens concretely — every tool call is vetted **before** dispatch (ADR-0007), so a destructive *inference* never becomes a destructive *side effect*. Untrusted inputs (MCP results, web content) are treated as data that flows *through* the model, never as privileged instructions; the controls in this document sit on the action and emit paths precisely because the model's reasoning is steerable.

---

## 2. The five bash security checkers

The constrain layer runs a registry of synchronous `SecurityCheck`s inside `WorkspacePolicy::before_tool()` for `bash` calls ([PRD 02](../prd/02-constrain.md)). Each blocked call writes a record to `security.jsonl` whose `checker` field is the structured `PolicyError` variant name (ADR-0023), not free prose ([data-model §4.3](./data-model.md#43-securityjsonl--blocked-calls-prd-02)).

| Checker | Defends against | What it blocks |
|---|---|---|
| `CommandInjectionCheck` | Chaining a benign command into a malicious one; piping remote scripts straight into a shell. | `;`, `&&`, `\|\|` followed by network/delete; pipe-to-shell (`curl … \| sh`). |
| `PrivilegeEscalationCheck` | The agent escaping the user's privilege level or loosening permissions. | `sudo`, `su -`, `chmod 777`, `chown root`. |
| `PathTraversalCheck` | Reaching outside the workspace boundary via shell arguments the `WorkspacePolicy` path check does not see (it inspects the `path` arg, not free-form `bash` strings). | `../../`, absolute paths outside the workspace in bash args. |
| `NetworkExfilCheck` | Exfiltrating workspace data to an attacker-controlled endpoint from inside a shell command. | `curl`/`wget`/`nc` piped to external IPs inside `bash`. |
| `DestructiveCommandCheck` | Irreversible data loss from a mistaken or injected command. | `rm -rf /`, `git reset --hard`, SQL `DROP TABLE`. |

**These checkers are defense-in-depth, not a complete sandbox.** They are heuristic string/AST inspections of `bash` arguments; a determined or cleverly-injected command can be obfuscated past them. The *primary* boundary is the canonicalized `WorkspacePolicy` ([§5](#5-config-mutation-boundary)) plus the `PermissionMode` gate; the checkers are a second layer that catches the common dangerous shapes. They reduce the blast radius of a steered model — they do not make `bash` safe to hand to an adversary. See [§9](#9-residual-risks--non-goals) for what is explicitly out of scope (seccomp, namespaces, a real sandbox).

> **Coverage gap (v1):** `NetworkExfilCheck` inspects **`bash` only**. The web tools (`web_fetch`/`web_search`) take an arbitrary URL and are *not* routed through it — their egress is governed separately by [§4](#4-web-tool-egress--ssrf-guard).

---

## 3. Secret redaction (REQUIRED DEFAULT — ADR-0026)

Tool `args` and results routinely carry secrets: MCP/SSE auth tokens, `web_*` API keys, and anything the model passes as an argument. Without redaction these flow verbatim into append-only on-disk journals and across the IPC/HTTP boundary. Redaction is therefore a **required default**, not an optional policy seam (ADR-0026, promoting the former `RedactPolicy` seam to the emit path). It is enabled by `RUSTYKEYS_REDACT=1` (default; disabling is strongly discouraged — see [configuration.md](../reference/configuration.md#permissions--safety)).

### Where it applies

The scrub runs **before** any tool args/results reach a sink — at the observe/emit boundary, so *every* downstream consumer sees only redacted data:

- `.rustykeys/evidence.jsonl` (verification packages, tool events)
- `.rustykeys/security.jsonl` (blocked-call records)
- episode packages (`.rustykeys/episodes/<turn_id>.json`)
- `GET /evidence` (gateway HTTP)
- `rk://tool_event` (Tauri/IPC event bridge) and `session_evidence_recent`

### The rule

1. **Deny-list of argument keys** — any key matching one of these case-insensitive globs has its value replaced with the literal `"<redacted>"`:

   | Pattern | Matches (examples) |
   |---|---|
   | `*token*` | `token`, `auth_token`, `access_token`, `refresh_token` |
   | `*key*` | `api_key`, `apiKey`, `secret_key`, `private_key` |
   | `*secret*` | `secret`, `client_secret`, `gateway_secret` |
   | `auth*` | `auth`, `authorization`, `auth_header` |
   | `password*` | `password`, `passwd`, `password_hash` |

2. **High-entropy value scrub** — independent of the key name, string values that look like credentials (long base64 / hex runs above an entropy threshold) are replaced with `"<redacted>"`. This catches a token passed under an innocuous key (e.g. a bare positional `command` arg containing an inline API key).

### Scrubs values, not structure

Redaction replaces **values**, never keys, array shape, or record fields. The attribution and verification traces (`action_trace`, `tool_trace`, `verification_trace`, `attribution_log` — [data-model §5](./data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)) and the `/evidence` schema must remain intact and parseable after redaction. A redacted `tool_event` still records *that* `read_file` ran with status `ok`; it only blanks a secret-bearing argument value. Redaction must not break attribution completeness or the verification trace — the deny-list targets credentials, not provenance.

> **Tuning note.** Over-redaction is possible for fields that merely *look* like secrets (ADR-0026 consequences). The deny-list above is the v1 default; it is the single tuning surface.

---

## 4. Web-tool egress / SSRF guard

`web_fetch`/`web_search` ([PRD 03](../prd/03-feed.md)) take a model-supplied URL. Today they are gated only by the boolean `RUSTYKEYS_ALLOW_WEB`, and `NetworkExfilCheck` does **not** see them ([§2](#2-the-five-bash-security-checkers)). With web enabled, a prompt-injected or mistaken model could fetch internal services or cloud-metadata endpoints — a server-side request forgery (SSRF) pivot. The egress rule below binds web tools to a network allowlist analogous to the bash checkers.

### Block-set (always denied, before any allowlist)

Resolve the target host and **reject** any request to:

- **Loopback** — `127.0.0.0/8`, `::1`, `localhost`.
- **RFC1918 private ranges** — `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`.
- **Link-local** — `169.254.0.0/16`, `fe80::/10`.
- **Cloud instance metadata** — `169.254.169.254` (and IMDS variants), called out explicitly because it is the canonical SSRF credential-theft target.

The block-set applies **after DNS resolution and on every redirect hop**, so a public hostname that resolves to (or redirects to) a private address is still rejected. It cannot be overridden by the allowlist; it is the floor.

### Allow / deny configuration

- `RUSTYKEYS_WEB_ALLOWLIST` — comma-separated host allowlist. Empty ⇒ allow all *non-blocked* hosts. When set, only listed hosts are reachable.
- `RUSTYKEYS_WEB_DENYLIST` — extends the built-in SSRF block-set with additional hosts; it can only *add* denials, never remove the floor above.

### Redirect and response-size caps

- **Redirects** are capped (small bounded count); each hop is re-validated against the block-set.
- **Response size** is capped (the existing ~50k-char `web_fetch` cap, enforced as a hard byte/char ceiling so a hostile server cannot exhaust memory).

See [configuration.md → Tools](../reference/configuration.md#tools) for the env vars.

---

## 5. Config-mutation boundary

`/config set KEY VALUE` (and the IPC `config_set`) apply for the session. Most keys are hot-reloadable, but a subset is **restart-only because hot-mutation is a security risk**, not merely an implementation inconvenience ([configuration.md → hot-reload vs restart-only](../reference/configuration.md#hot-reload-vs-restart-only)):

| Restart-only key | Why mutating it mid-session is a security risk |
|---|---|
| `RUSTYKEYS_WORKSPACE` | The `WorkspacePolicy` canonicalizes this path at `Session::new()` and every filesystem check is relative to it ([PRD 02](../prd/02-constrain.md)). Changing it mid-session would **void the canonicalized boundary** — an attacker-steered `config_set` could relocate the workspace to `/` and neuter every path check. The boundary must be fixed for the life of the session. |
| `RUSTYKEYS_MODEL` | Rebinds the kernel to a different model. A mid-session swap would change the reasoning engine under an in-flight task and invalidate session lineage; the kernel model is fixed per session. |
| `RUSTYKEYS_MODE`, backend selectors, gateway/MCP transport+port | Restart-only for consistency/lifecycle reasons (process topology is fixed at startup). |

Attempting to set a restart-only key mid-session is rejected (or requires a new session). The redaction toggle, recall/compaction tunables, and per-role models remain hot-reloadable.

---

## 6. Gateway / MCP authentication

In `gateway` and `mcp` modes the harness is exposed over the network, so the trust boundary moves to the wire.

- **Bearer token.** `RUSTYKEYS_GATEWAY_SECRET`, if set, is required on every request (`Authorization: Bearer <token>`). For MCP SSE clients the token is supplied via the server's `auth_token_env` entry in `mcp.toml` ([PRD 07](../prd/07-mcp.md)) and sent as the same bearer header; server-side SSE should reuse a `RUSTYKEYS_GATEWAY_SECRET`-style token.
- **Multi-session scoping.** In `RUSTYKEYS_GATEWAY_MODE=multi`, the bearer token **scopes which `session_id`s are reachable**. Without this binding, `session_id` is a bare map key and a caller could **guess or enumerate another tenant's `session_id`** to read its history/evidence — a cross-tenant exposure. The token-to-session binding is the control that closes `session_id`-guessing.
- **TLS for non-loopback.** Any SSE/HTTP listener bound to a non-loopback address **must** use TLS; a bearer token over plaintext is trivially sniffable. Loopback-only deployments (the default local-first case) may run plaintext.

---

## 7. Bypass mode

`PermissionMode::Bypass` (`RUSTYKEYS_PERMISSION_MODE=bypass`) **disables all policy checks** — the workspace boundary, every bash checker, the web egress guard, and the approval gate. To prevent accidental activation it requires a second, explicit opt-in: `RUSTYKEYS_ALLOW_BYPASS=1` ([PRD 02](../prd/02-constrain.md), [configuration.md](../reference/configuration.md#permissions--safety)). With both set, the semi-trusted model's tool calls execute **unconstrained**.

> **Risk.** In bypass mode the entire constrain layer is off: a prompt-injected or mistaken model can delete files anywhere, escalate privilege, and exfiltrate data with no harness check. It exists only for fully-trusted, sandboxed-by-other-means contexts (e.g. an ephemeral CI container). It must never be the default and should never be combined with an internet-exposed gateway. Redaction ([§3](#3-secret-redaction-required-default--adr-0026)) is independent of permission mode and still applies.

---

## 8. Boundary summary

| Boundary | Untrusted side | Control |
|---|---|---|
| Model → filesystem | model tool calls | `WorkspacePolicy` canonical-path check ([§5](#5-config-mutation-boundary)) + `PathTraversalCheck`. |
| Model → shell | model `bash` args | `PermissionMode` gate + five checkers ([§2](#2-the-five-bash-security-checkers)). |
| Model → web | model URL + remote response | SSRF block-set + allow/deny-list + caps ([§4](#4-web-tool-egress--ssrf-guard)). |
| MCP server / web → context | returned results | treated as data, not instructions; results are observed, never privileged ([§1](#1-trust-model)). |
| Tool args/results → logs / IPC | secrets in args | redaction-by-default ([§3](#3-secret-redaction-required-default--adr-0026)). |
| Network client → gateway/MCP | remote caller | bearer token + multi-session scoping + TLS ([§6](#6-gateway--mcp-authentication)). |
| Session config | mid-session mutation | restart-only keys ([§5](#5-config-mutation-boundary)). |

---

## 9. Residual risks & non-goals

Rusty Keys v1 is **not a sandbox**. It does not use OS-level isolation — no seccomp filters, no namespaces/cgroups, no syscall interception, no container boundary. The controls in this document are **application-level**: the canonicalized **workspace boundary**, the **security checkers**, and **secret redaction**, plus the gateway/MCP auth and the config-mutation boundary. Together they reduce the blast radius of a semi-trusted, steerable model; they do not contain a determined attacker who can run arbitrary code (e.g. via an obfuscated `bash` command that slips past the heuristic checkers, or in `bypass` mode).

Explicit non-goals for v1, and the residual risk each leaves:

- **No process sandbox (seccomp/namespaces/containers).** Once a `bash` command passes the checkers it runs with the harness's full OS privileges. Residual risk: a checker bypass is a full-privilege bypass. *Mitigation:* run the harness inside an externally-provided sandbox for untrusted workloads.
- **Heuristic, not exhaustive, bash inspection.** The five checkers match known-dangerous shapes; obfuscation can evade them. *Mitigation:* defense-in-depth — the workspace boundary and permission mode still apply.
- **`RUSTYKEYS_FSYNC=0` by default.** Append-only security/evidence logs rely on the OS page cache; a crash can lose the last partial record ([data-model §10](./data-model.md#10-append-only-durability)). Audit completeness is best-effort, not guaranteed, unless `fsync` is enabled.
- **No log rotation/retention or tamper-evidence.** `security.jsonl`/`evidence.jsonl` grow unbounded and are plain local files a privileged process can edit; they are an audit aid, not a forensic guarantee. Rotation is a future seam (ADR-0015).
- **Over-redaction.** The deny-list may blank non-secret fields that match its patterns; this is a usability cost, accepted to keep secrets out of durable logs ([§3](#3-secret-redaction-required-default--adr-0026)).

The intended deployment for untrusted or production-exposed use is: this harness's controls **plus** an external isolation layer. The v1 controls are the *requisite-variety* regulator over a steerable model — not a substitute for OS-level containment.
