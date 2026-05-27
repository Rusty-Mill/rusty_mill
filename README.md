# Rusty Keys

An AI-native application skeleton in Rust. The model's agent loop is the kernel;
the application is the harness built around it.

Rusty Keys is the Rust successor to [Keystone](https://github.com/baileyrd/Keystone),
carrying forward its harness philosophy — constrain, feed, observe, compose — with
a Rust implementation that makes the architecture's natural properties (async,
type-safe, zero-overhead policy enforcement) first-class rather than aspirational.

## Why Rust

The harness is a runtime substrate — the layer that mediates every model action,
enforces policy on every tool call, and persists every observation. Those
responsibilities are a better fit for a systems language than for Python: the
constraints are not LLM-bound, they are execution-bound. The LLM calls themselves
(the slow part) are async I/O regardless of language; the harness layers that
wrap them benefit from Rust's ownership model, zero-cost abstractions, and the
native async runtime tokio provides.

## LLM Provider

Powered by [aisdk](https://aisdk.rs) — a provider-agnostic Rust LLM library
covering 70+ providers (Anthropic, OpenAI, Google, Ollama, OpenRouter, …) with
native async streaming and a `#[tool]` proc macro for type-safe tool registration.

## Quick start

```bash
cargo run -- "your prompt here"
```

Requires `RUSTYKEYS_MODEL` (any aisdk model string) and appropriate provider API
keys. All state is local: `.rustykeys/` in the workspace root.

## Architecture

Four harness verbs wrap the aisdk agent kernel:

```
┌─────────────────────────────────────────────────┐
│                    Session                       │
│  ┌──────────┐  ┌──────┐  ┌───────┐  ┌────────┐ │
│  │constrain │  │ feed │  │observe│  │compose │ │
│  └────┬─────┘  └──┬───┘  └───┬───┘  └───┬────┘ │
│       │           │          │           │      │
│  ┌────▼───────────▼──────────▼───────────▼────┐ │
│  │              aisdk Kernel                   │ │
│  └────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

This sketch shows the four verbs over the kernel. The full system is **eight
crates** — the four verbs plus `kernel`, `config`, `mcp`, and `app` (the
`Session` and its adapters). The authoritative component map and crate
dependency DAG live in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §4.

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the system view: component map, crate DAG, concurrency, topologies, faithfulness map. **Read this first.**
- [`docs/`](docs) — PRDs ([`prd/`](docs/prd)), decision records ([`adr/`](docs/adr)), the on-disk [data model](docs/architecture/data-model.md), and the [configuration reference](docs/reference/configuration.md).
- [`BACKLOG.md`](BACKLOG.md) — the development roadmap.
