# Architecture

## Overview
`rusty_a2a` is a Rust implementation of the Agent2Agent (A2A) protocol
(https://a2a-protocol.org), covering all three spec-defined bindings
(JSON-RPC 2.0, gRPC, HTTP+JSON/REST) plus Agent Card discovery/signing. It's a
library, not a hosted service: consumers embed `AgentServer`/`AgentServices`
to expose their own agent logic over one or more bindings, or embed
`A2aClient`/`RestClient`/`GrpcClient` to talk to any spec-compliant peer. It
is not itself an agent runtime, an LLM client, or a persistence layer.

## Boundaries
Domain logic (`Engine`, in `src/server/engine.rs`) implements all eleven A2A
operations against two ports, independent of which binding a request arrived
over. Everything binding-specific and everything storage-specific is an
adapter behind one of those ports.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `AgentExecutor` (`src/server/executor.rs`) | supplied by the crate's consumer | the actual agent logic; this crate has no opinion on what an agent does, only how it's reached |
| `TaskStore` (`src/server/store.rs`) | `InMemoryTaskStore` (ships in-crate, process-local, non-persistent) | a persistent adapter (Postgres/Redis/etc.) is left to the consumer |
| `AuthVerifier` (`src/server/auth.rs`) | supplied by the crate's consumer | credential validation and per-task authorization scoping (spec Section 13.1) are deployment-specific, so this crate only defines the trait and calls it |
| Protocol binding (dispatch into `Engine`) | `router.rs` (JSON-RPC), `rest.rs` (HTTP+JSON), `grpc/mod.rs` (gRPC) | each adapter does its own framing/error-shape/auth-extraction, then calls the same `Engine` methods |
| Wire client (calls a peer's binding) | `client::rest::RestClient`, `client::grpc::GrpcClient`, plain JSON-RPC in `client::mod` | mirror image of the server-side binding adapters, from the caller's side |

## Structure
A modular monolith, organized by responsibility rather than by binding:
- `src/types/` - the wire data model (`AgentCard`, `Task`, `Message`, security
  schemes, etc.), binding-agnostic; every binding serializes the same types.
- `src/server/` - `Engine` (the port-facing core) plus one module per binding
  adapter, `store.rs` (the `TaskStore` port + its in-memory adapter),
  `auth.rs` (the `AuthVerifier` port), and `push.rs` (webhook delivery).
- `src/client/` - one module per binding's client adapter.
- `src/signing.rs` - Agent Card JWS signing/verification (spec Section 8.4).
- `src/pb.rs` / `src/grpc_convert.rs` - generated `tonic` types and their
  conversion to/from `src/types/`, isolated so the rest of the crate never
  imports `prost`/`tonic` types directly.
- `src/error.rs`, `src/codec.rs`, `src/timestamp.rs` - the shared error
  model (`A2aError`) and wire-format primitives (byte-string encoding,
  RFC 3339 timestamp (de)serialization) every binding and `src/types/`
  build on.

There's no forcing function (independent scaling, a team boundary, fault
isolation) to split any of this into separate services - it's a library
consumed in-process by whatever binds an agent to a transport.

## Data flow
A request into any binding follows the same shape:
1. The binding adapter (`router.rs`/`rest.rs`/`grpc/mod.rs`) parses the
   transport-specific envelope, extracts credentials, and calls
   `Engine::authenticate` against the configured `AuthVerifier`.
2. The adapter calls the matching `Engine` method (e.g. `send_message`,
   `get_task`), passing the resolved `AuthContext`. `Engine` is where
   per-task authorization (`authorize_task`), history/page-size validation,
   and task-store access all happen - once, regardless of which binding
   called in.
3. `Engine` drives the consumer's `AgentExecutor` and reads/writes through
   `TaskStore`; streaming responses go out over a per-task broadcast bus with
   a bounded replay log (`EVENT_LOG_CAPACITY`) so a reconnecting client can
   resume via `Last-Event-ID` instead of only seeing a point-in-time snapshot.
4. The adapter translates the `Engine` result (or `A2aError`) back into its
   own wire shape - JSON-RPC error codes, `google.rpc.Status`, or a gRPC
   `Status` with `ErrorInfo` details - so the same internal error reliably
   round-trips through any of the three client adapters.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
- Not a persistent task store: `InMemoryTaskStore` is process-local and
  non-durable by design; production persistence is a `TaskStore` the
  consumer supplies.
- Not a TLS terminator: `AgentServer::serve`/`AgentServices::serve_grpc`
  always bind plain TCP; `mtls` support only works fronted by an external
  TLS-terminating proxy.
- Not an authorization policy engine: `AuthVerifier` is a trait this crate
  calls, never a specific policy (RBAC, per-tenant ownership, etc.) it ships.
