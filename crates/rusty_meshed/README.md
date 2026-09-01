# rusty_meshed

The Rust port of [baileyrd/meshed](https://github.com/baileyrd/meshed) — a
data mesh platform (self-serve registry, computational governance, an
event SDK, and lineage/observability, anchored in a manpower domain) —
landing as a multi-crate namespace inside the `rusty_mill` workspace,
following the same pattern as `rusty_search`/`rusty_mcp`/`rusty_db`.

This is an active migration, run under the `rust-migration` skill's
boundary contract: every capability the source repo exhibits defaults to
**REQUIRED** until a specific, user-attributed line in
[`capability-manifest.md`](./capability-manifest.md) moves it to
`OUT-OF-SCOPE`. See that file for the full capability inventory and
migration status, and the tracking issues on this repo (label
`migration-item`) for the per-cluster work.

## Crates

| Crate | Ports from (meshed) | Depends on |
| --- | --- | --- |
| `rusty-meshed-core` | `meshed.core.config` | — |
| `rusty-meshed-schema-registry` | `meshed.schema_registry` | core |
| `rusty-meshed-governance` | `meshed.governance` | — |
| `rusty-meshed-observability` | `meshed.observability` | schema-registry, `rusty_kafka`, `rusty_sqlite` |
| `rusty-meshed-sdk` | `meshed.sdk`, `meshed.infrastructure` | core, schema-registry, observability, `rusty_kafka` |
| `rusty-meshed-registry` | `meshed.registry` | core, governance, observability, schema-registry, `rusty_http` |
| `rusty-meshed-cli` | `meshed.cli` | core, observability, registry |
| `rusty-meshed-domains` | `meshed.domains` | core, sdk |

`meshed`'s Kafka dependency (`confluent-kafka`) has no existing RustyMill
equivalent, so it's being hand-rolled as a new sibling crate,
[`rusty_kafka`](../rusty_kafka), rather than pulled in as a crates.io
dependency — a prerequisite for most of the SDK/registry/observability
capabilities above.

The `data-mesh-monitor` Vite/React dashboard from the source repo is
explicitly out of scope for this migration (2026-09-01, user decision) —
it continues to run unmodified against whichever backend is live.

## Local dev

[`compose.yaml`](./compose.yaml) brings up the same Kafka broker, Schema
Registry, and Kafka UI the source repo's own `compose.yaml` does (`docker
compose up -d` / `podman-compose up -d`, from this directory). Once it's up,
`cargo run -p rusty-meshed-cli --bin init_registry` sets the registry's
global compatibility to `FULL_TRANSITIVE`, and
`MESHED_COMPOSE_UP=1 cargo test -p rusty-meshed-cli --test compose_smoke`
verifies all three services are reachable.

## Status

Most rows across `REG`/`XFM`/`GOV`/`SDK`/`DOM`/`CLI` are `DONE` -- see
`capability-manifest.md` for row-by-row status. The main thing blocking full
parity is `rusty_kafka` having no `Produce` request implementation yet (its
own module doc explains why); everything downstream of publishing to
Kafka -- `SLOViolationPublisher`, the domain producers/consumers,
`OutboxRelay`, the CLI `slo` subcommand -- is tracked but not yet built for
that reason.
