# Rusty Keys — documentation index

Start here. This tree is the design and engineering specification for Rusty Keys —
an AI-native application harness in Rust (the model's agent loop is the kernel; the
application is the harness built around it).

## Read first

- **[ARCHITECTURE.md](./ARCHITECTURE.md)** — the system view: component map, crate DAG,
  concurrency and runtime model, deployment topologies, quality attributes, failure
  modes, and the faithfulness map to the research paper. *Authoritative for structure.*
- **[../BACKLOG.md](../BACKLOG.md)** — the 15-phase implementation roadmap (DoD,
  acceptance criteria, sizing, dependencies, test gates, risks per phase).

## Product requirements (per component)

| PRD | Component |
|---|---|
| [prd/00-overview.md](./prd/00-overview.md) | Product brief + index (concept, why-Rust, goals) |
| [prd/01-kernel.md](./prd/01-kernel.md) | The aisdk agent loop |
| [prd/02-constrain.md](./prd/02-constrain.md) | Policy, permission modes, security checkers |
| [prd/03-feed.md](./prd/03-feed.md) | Tools, context, memory, Task State |
| [prd/04-observe.md](./prd/04-observe.md) | Tracer, interventions/M-HIR, entropy auditor |
| [prd/05-compose.md](./prd/05-compose.md) | Verification, attribution, evidence, episode packages |
| [prd/06-app.md](./prd/06-app.md) | Session, CLI, web gateway, Tauri bridge |
| [prd/07-mcp.md](./prd/07-mcp.md) | MCP client + server |
| [prd/08-frontend.md](./prd/08-frontend.md) | Tauri 2 + SolidJS desktop UI |

## Authoritative references (single source of truth)

| Doc | Owns |
|---|---|
| [architecture/data-model.md](./architecture/data-model.md) | On-disk state: `.rustykeys/` tree, SQLite DDL, every JSONL/episode/task schema, serde + versioning |
| [reference/configuration.md](./reference/configuration.md) | Every `RUSTYKEYS_*` environment variable |
| [architecture/threat-model.md](./architecture/threat-model.md) | Trust boundaries, secret redaction, web egress, auth |
| [reference/glossary.md](./reference/glossary.md) | The harness vocabulary |
| [adr/](./adr/) | Architecture Decision Records (0001–0028) |

## Engineering substrate (dev)

| Doc | Topic |
|---|---|
| [dev/error-handling.md](./dev/error-handling.md) | Error taxonomy + the `ToolOutcome` contract |
| [dev/testing-strategy.md](./dev/testing-strategy.md) | Test tiers + `FakeLanguageModel` + golden-episode replay |
| [dev/eval-plan.md](./dev/eval-plan.md) | Maturity measurement: M-HIR, outcome taxonomy, H0→H3 gates |
| [dev/coding-standards.md](./dev/coding-standards.md) | MSRV, lints, async-trait, features, CI |

## Research & review

- [research/2605.13357v1.pdf](./research/2605.13357v1.pdf) — *AI Harness Engineering* (the grounding paper).
- [research/references.md](./research/references.md) — sourced bibliography of every research item, comparative implementation, protocol, concept, and technology used.
- [review/](./review/) — point-in-time findings from the five-persona refinement
  (systems/software/AI/AI-harness/integration architects) and the consolidated plan.
  These are a **historical audit trail**, superseded by the canonical docs above —
  cite the canonical docs, not the review files.
