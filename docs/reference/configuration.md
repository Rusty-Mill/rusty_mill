# Configuration reference — `RUSTYKEYS_*`

> **Authoritative source** for runtime configuration. All settings resolve from environment variables at startup (PRD 07 `config` crate); `/config` shows current values and `/config set KEY VALUE` overrides for the session (hot-reload rules below). PRD 06 links here instead of carrying its own copy.

**Legend:** † = added by the five-persona refinement (not in the original PRD 06 table). Defaults in `code`.

## Core
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_MODEL` | `anthropic/claude-opus-4-7` | Any aisdk model string; drives the kernel (and, unless overridden, every other LLM call). |
| `RUSTYKEYS_WORKSPACE` | `.` | Workspace root — the policy boundary. **Restart-only** (changing it mid-session would void the canonicalized `WorkspacePolicy`). |
| `RUSTYKEYS_MODE` | `cli` | Binary mode: `cli` \| `gateway` \| `mcp`. Restart-only. |
| `RUSTYKEYS_TRACE` | `1` | Structured trace logging to stderr. |

## Model selection (per-role) †
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_JUDGE_MODEL` † | _(= `RUSTYKEYS_MODEL`)_ | Model for the `CriteriaJudge`; wire it to `CriteriaJudge.model`. A cheaper/faster model is usually fine. |
| `RUSTYKEYS_CONSOLIDATE_MODEL` † | _(= `RUSTYKEYS_MODEL`)_ | Model for memory consolidation. |
| `RUSTYKEYS_COMPACT_MODEL` † | _(= `RUSTYKEYS_MODEL`)_ | Model for session/full compaction summaries. |
| `RUSTYKEYS_EMBED_MODEL` | _(none)_ | aisdk embed string; **absent ⇒ lexical (FTS5) recall**. Enables semantic recall when set. |

> Rationale: the kernel needs the strongest model; judge/consolidation/compaction/embeddings can trade quality for cost/latency. See PRD 03 (AI engineering) for guidance.

## Provider / aisdk integration policy †
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_REQUEST_TIMEOUT_MS` † | `120000` | Per-LLM-call timeout. |
| `RUSTYKEYS_RETRY_MAX` † | `4` | Max retries on retryable provider errors (`429`, `5xx`, transport). |
| `RUSTYKEYS_RETRY_BASE_MS` † | `500` | Base for exponential backoff + jitter; honors `Retry-After` when present. |

> These govern a shared aisdk-client wrapper used by *every* call site (kernel, judge, consolidation, embed, summarisation), not just the kernel. See PRD 01.

## Permissions & safety
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_PERMISSION_MODE` | `default` | `default` \| `plan` \| `accept_edits` \| `read_only` \| `restricted` \| `bypass` (PRD 02). |
| `RUSTYKEYS_ALLOWED_TOOLS` | *(unset)* | CSV allowlist consulted only in `restricted` mode (PRD 02). |
| `RUSTYKEYS_ALLOW_BYPASS` † | `0` | Required to be `1` before `bypass` mode disables policy checks (referenced by PRD 02 but previously absent from the table). |
| `RUSTYKEYS_REDACT` † | `1` | Secret redaction of tool args/results before logging/journaling/IPC (ADR-0026). Disabling is strongly discouraged. |
| `RUSTYKEYS_ISOLATION` † | `none` | `none` \| `sandboxed` (ADR-0030; roadmap-phase capability isolation). `none` keeps today's in-process behaviour. `sandboxed` runs tool side-effects (esp. `bash`) inside an OS sandbox with **network-deny-by-default**, egress enforced at the sandbox boundary; selects the `ToolExecutor` isolation seam below `feed` (does not change the `constrain` vetting contract). **Restart-only.** |

## Tools
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_ALLOW_WEB` | `0` | Enable `web_fetch` / `web_search`. |
| `RUSTYKEYS_WEB_ALLOWLIST` † | _(none)_ | Comma-separated host allowlist for web tools. Empty ⇒ allow all non-blocked. |
| `RUSTYKEYS_WEB_DENYLIST` † | _(built-in SSRF set)_ | Always blocks loopback, RFC1918, link-local, `169.254.169.254`; this var extends it. |
| `RUSTYKEYS_SEARCH_PROVIDER` | `brave` | `brave` \| `serper` \| `duckduckgo`. |
| `RUSTYKEYS_SEARCH_API_KEY` | _(none)_ | API key for the search provider. |
| `RUSTYKEYS_MAX_AGENT_DEPTH` | `3` | Max subagent recursion depth. |

## Harness & verification
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_HARNESS_LEVEL` | `h1` | `h0` † \| `h1` \| `h2` \| `h3`. `h0` (no tool registry) is the paper's ablation floor — selectable or eval-only per ADR-0028. |
| `RUSTYKEYS_VERIFY` | `1` | Enable verification + evidence journal. |
| `RUSTYKEYS_MAX_STEPS` | `10` | Kernel loop step limit. |

## Observability
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_OTLP_ENDPOINT` † | _(none)_ | OTLP collector endpoint for the pull-based OTLP exporter bound to the `KernelEvent` stream (token/cost/latency attributes; ADR-0034). Absent ⇒ exporter inert; `RUSTYKEYS_TRACE` stderr logging is unaffected. **v1 intent** (the exporter is a seam; see [coding-standards §9](../dev/coding-standards.md#9-kernelevent--the-unified-lifecycle-event-adr-0034)). |

## Memory
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_SHORT_TERM_BACKEND` | `sqlite` | `sqlite` (only option in v1). |
| `RUSTYKEYS_LONG_TERM_BACKEND` | `sqlite` | `sqlite` \| `duckdb` (Phase 5). |
| `RUSTYKEYS_RECALL_K` | `6` | Top-k memories retrieved per turn. |
| `RUSTYKEYS_RECALL_WINDOW` | `6` | Recent turns used to build the recall query. |
| `RUSTYKEYS_IDLE_THRESHOLD` | `8` | Observations before idle consolidation fires. |
| `RUSTYKEYS_SKILL_GROOM_THRESHOLD` | `12` | Skill count before grooming fires on sleep. |

## Tokens & compaction
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_CONTEXT_LIMIT` | `200000` | Context-window token budget. |
| `RUSTYKEYS_COMPACT_MICRO` | `0.80` | Micro-compact threshold. |
| `RUSTYKEYS_COMPACT_SESSION` | `0.90` | Session-summary threshold. |
| `RUSTYKEYS_COMPACT_FULL` | `0.95` | Full-compaction threshold. |

## Storage paths
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_EVIDENCE_LOG` | `.rustykeys/evidence.jsonl` | Evidence journal. |
| `RUSTYKEYS_INTERVENTIONS_LOG` | `.rustykeys/interventions.jsonl` | Intervention log. |
| `RUSTYKEYS_ENTROPY_LOG` | `.rustykeys/entropy.jsonl` | Entropy audit log. |
| `RUSTYKEYS_SECURITY_LOG` | `.rustykeys/security.jsonl` | Security event log. |
| `RUSTYKEYS_TASK_FILE` | `.rustykeys/task.json` | TaskState persistence. |
| `RUSTYKEYS_MCP_CONFIG` | `.rustykeys/mcp.toml` | MCP server config. |
| `RUSTYKEYS_FSYNC` † | `0` | `fsync` each append-only record (durability vs latency; see data-model §10). |

## Gateway (`RUSTYKEYS_MODE=gateway`)
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_GATEWAY_MODE` | `single` | `single` (`Arc<Mutex<Session>>`) \| `multi` (`session_id`-routed). |
| `RUSTYKEYS_GATEWAY_PORT` | `3000` | HTTP port. |
| `RUSTYKEYS_GATEWAY_SECRET` | _(none)_ | Bearer token; if set, all requests require it. In `multi`, scopes which `session_id`s are reachable. |
| `RUSTYKEYS_GATEWAY_CORS_ORIGIN` | `*` | CORS origin. |
| `RUSTYKEYS_SESSION_TTL_SECS` † | `3600` | Idle TTL before a `multi`-mode session is evicted. |
| `RUSTYKEYS_MAX_SESSIONS` † | `64` | Max concurrent sessions in `multi` mode. |

## MCP (`RUSTYKEYS_MODE=mcp` and the client)
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_MCP_SERVER_TRANSPORT` | `stdio` | `stdio` \| `sse`. |
| `RUSTYKEYS_MCP_SERVER_PORT` | `3001` | SSE transport port. |

## Frontend
| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_VOICE` † | `0` | Enable Whisper voice input in the desktop Composer (PRD 08 seam). |

## Hot-reload vs restart-only
`/config set` applies for the session. **Restart-only** keys (changing them mid-session is rejected or requires a new session): `RUSTYKEYS_WORKSPACE`, `RUSTYKEYS_MODE`, `RUSTYKEYS_MODEL` (rebinds the kernel), `RUSTYKEYS_ISOLATION` † (the `ToolExecutor` isolation seam is bound at startup), backend selectors, gateway/MCP transport+port. Everything else (recall/compaction tunables, harness level within a level that doesn't change tool registry, per-role models, redaction toggle, `RUSTYKEYS_OTLP_ENDPOINT`) is hot-reloadable.
