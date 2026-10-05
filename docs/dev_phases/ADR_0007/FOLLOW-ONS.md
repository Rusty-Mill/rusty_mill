# rusty_agui: status and follow-ons

As of 2026-10-05 · follows [ADR-0007](../../adr/0007-agui-and-json-patch.md)

## Status

ADR-0007 is merged, and steps 1 and 2 of the build order below are done:
the workspace speaks AG-UI on both sides, the reference TypeScript client
accepts what the server sends, and a headless TypeScript core mirrors the
crate against shared fixtures.

| PR | Merged | What it shipped |
| --- | --- | --- |
| [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) | step 2 | `@rusty-mill/agui-core`, the headless TypeScript core; shared `fixtures/`; the core run in conformance beside the reference client |
| [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) | step 1 | `rusty_agui`'s `client` feature (`HttpAgent`, blocking, on `rusty_http`); the `echo_agent` example; `conformance/`, where `@ag-ui/client` drives the example in CI |
| [#512](https://github.com/Rusty-Mill/rusty_mill/pull/512) | 2026-10-05 | `rusty_json_patch` (RFC 6901/6902/7386), `rusty_agui` (types, events, codec, SSE, verifier, reducer, `serve` feature with `Agent` and `AgentHandler`), streaming bodies in `rusty_serve` |

## The goal this document plans for

Two long-horizon targets, set by the owner on 2026-10-05, both built on
`rusty_agui`:

1. **Frontend SDKs** for React, Angular, Vue, iOS and Android: chat
   components, readable-context and action hooks, generative UI, shared
   state, and a headless mode. The shape is CopilotKit's open SDK.
2. **An agent platform** in the shape of OpenBot: per-bot sandboxes, a
   CEL deny-by-default gateway with a decision recorded before every
   action, Slack, Teams and SMS channels, routines, and the rule that any
   AG-UI endpoint becomes a bot.

Neither is current work. This document records what exists, what is
missing, the order to build in, and where the plan departs from the
targets as named.

## What the workspace already has

| Need | Where it is | Layer | Reusable as is? |
| --- | --- | --- | --- |
| AG-UI server: run framing, verification, SSE streaming | `rusty_agui::serve` | libs | Yes |
| Event verifier and state reducer, with wire-sample tests | `rusty_agui::verify`, `rusty_agui::reduce` | libs | Yes; the tests become the TypeScript core's fixtures |
| React application stack (React 18, Vite, zustand, Tailwind, Vitest, Playwright) | `rusty_tick/web`, `rusty_fair_play/web` | apps | As the first two consumers |
| CEL authorization over calls and callers, deny wins | `rusty_agent_gateway` (`cel` 0.14, Tier A) | apps (gateway) | Yes, inside the gateway |
| Audit log, capability risk metadata | `nexus-security` | apps | No: hoist or re-implement in libs |
| Sandboxed execution (Landlock, seccomp, rlimits), fails closed | `rusty_rsi`'s `ProcessExecutor` | apps | No: hoist to libs; Linux only |
| Outbound notifications: Discord, Telegram, email, webhook (Slack incoming webhooks fit) | `nexus-notifications` | apps | Outbound only; no inbound |
| Task scheduler and typed event loop | `nexus-ai-runtime` | apps | Possibly, for routines |
| Agent runtimes with human-in-the-loop | `rusty_adk`, `nexus-agent`, `rusty_key` | libs, apps | Each needs an adapter to `rusty_agui` |

## What is missing

| Gap | Needed by |
| --- | --- |
| ~~A Rust AG-UI client~~ Done in step 1, on `rusty_http` directly (see below) | The gateway proxy, routines, channels: anything that *consumes* an AG-UI endpoint |
| ~~A headless TypeScript core~~ Done in step 2 | Every frontend SDK |
| Framework bindings (hooks, components) | Each SDK |
| An AG-UI route in the gateway: CEL decision, audit record before and after, forward | The platform |
| A bot registry (name, endpoint, policy, channel bindings) | The platform |
| Inbound channel adapters: Slack Events API, Teams Bot Framework, Twilio SMS | Channels |
| A routines crate (cron schedule, run as requester, disable after N failures) | Routines |
| Per-bot isolation | Sandboxes |
| CORS and authentication on the agent endpoint | Any cross-origin or multi-user deployment |
| Adapters from `rusty_adk`, `nexus-ai-runtime` and `rusty_key` event types to AG-UI | Putting existing agents behind the SDKs |

## Build order

Both targets start with the same two pieces, so they come first.

| Step | What | Size | Needs sign-off |
| --- | --- | --- | --- |
| 1 | **Done.** `rusty_agui` client and a conformance test against the reference client (`@ag-ui/client`, the package CopilotKit's React SDK and OpenBot drive agents with) | 1 PR | No |
| 2 | **Done.** `@rusty-mill/agui-core` (TypeScript, headless): parser, SSE decoder, JSON patch, verifier, reducer, `runAgent`; `fixtures/` shared with the Rust tests | 1 PR | No |
| 3 | React binding: `useAgent`, `useReadable`, `useAction` (frontend tools and generative UI through its render function), `useSharedState`; `rusty_tick` as the first consumer | 1–2 PRs | No |
| 4 | Gateway AG-UI route: CEL over run input and caller, deny by default, one audit record before the upstream call and one after | 1–2 PRs, touches `rusty_agent_gateway` | Yes: new gateway surface |
| 5 | Channels: one `Channel` trait in libs mapping an inbound message to `RunAgentInput` and reply events back; Slack first, then Teams, then SMS | 1 PR per channel | Yes: external service credentials and dependencies |
| 6 | Routines: cron schedule posting a `RunAgentInput` through the gateway as the requester; disable after N consecutive failures | 1 PR | No |
| 7 | Per-bot sandboxes on `rusty_rsi`'s executor, hoisted to libs | 2 PRs | Yes: ADR-0005 scoped the executor to `rsi` |
| 8 | Angular and Vue bindings over `agui-core` | 1 PR each, when a consumer exists | No |
| 9 | iOS (Swift) and Android (Kotlin) ports of the core | Large, when a consumer exists | Yes: new languages in the workspace |
| 10 | Adapters from `rusty_adk`, `nexus-ai-runtime`, `rusty_key` | 1 PR each, in each family | No |

Chat components come after step 3 as a thin layer over the hooks, not
before: the hooks are the API, the components are one rendering of it.

## Departures made while building

- **Step 1's client is on `rusty_http`, not `rusty_request`.** `rusty_request`
  is async on `rusty_tokio` with `rusty_tls`. The server side is blocking
  on `rusty_serve`, and the one consumer that will be async (the gateway)
  runs on tokio and axum with its own HTTP stack. A blocking client over
  `std::net` mirrors `serve`, keeps the crate's manifest first-party only,
  and is what a routine or a channel adapter on the same host needs. An
  async adapter can be added behind a feature when a consumer appears.
- **Step 1's smoke test is `@ag-ui/client`, not a browser-driven React
  app.** That package is the client CopilotKit's React SDK and OpenBot
  embed, so a run it verifies and reduces is a run those products accept,
  and it runs in Node in CI without a browser.

## Where this departs from the targets as named

- **One core, then bindings.** Five SDKs written in parallel is speculative
  generality. A headless TypeScript core plus a React binding gives every
  capability the target names on the web. Angular and Vue are thin
  bindings; iOS and Android are ports in other languages with no consumer
  in the workspace. They wait for one.
- **Sandboxes on the executor, not containers.** OpenBot gives each bot a
  container with Chromium, files and Postgres. This workspace has a
  fail-closed Landlock and seccomp executor already. It covers the
  isolation requirement for local bots without Docker, and keeps the
  Linux-only limitation `rusty_rsi` already carries. Containers are a
  deployment decision for when a multi-tenant host forces it.
- **CEL stays external.** The gateway's `cel` dependency is Tier A behind a
  first-party boundary (ADR-0002). A hand-rolled CEL evaluator is a large
  project with no sovereignty payoff outside the gateway.
- **Channels are adapters, not products.** Each channel is one inbound
  mapping to `RunAgentInput` and one outbound mapping from events. The
  bot, policy and audit live in the gateway, so a channel never decides
  anything.

## Open questions

1. ~~Does the TypeScript core live in this workspace or in a separate
   repository?~~ Answered in step 2: in-workspace, at
   `crates/libs/protocol/rusty_agui/packages/agui-core`, so the `agui`
   planner flag covers it and the shared fixtures sit beside the Rust
   tests. Publishing to a registry is a later decision.
2. Where does the bot registry persist? `rusty_multimodal_db_engine` is the
   workspace's own store and already backs `rusty_tick` and `remind_me`.
3. Which agent is the first bot? `rusty_adk`'s weather example is the
   smallest with human-in-the-loop; `rusty_key` has the richest existing
   contract.
