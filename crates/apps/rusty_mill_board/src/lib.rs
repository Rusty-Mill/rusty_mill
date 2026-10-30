//! Mill Board: a capability-gated kanban board for humans and agents.
//!
//! - [`api`]: the operation table shared by MCP and web.
//! - [`identities`]: per-model bearer tokens bound to an actor and role.
//! - [`board`]: the service: roles, clock, commit with retry.
//! - [`projects`]: the applications one board serves, and the paths each may change.
//! - [`domain`]: pure cards, columns, capabilities and the transition table.
//! - [`roadmap`]: a project's roadmap lanes, derived from its cards.
//! - [`metrics`]: work in progress, flow and lead time, derived from cards.
//! - [`store`]: the storage port and an in-memory adapter.
//! - [`mcp`], [`web`]: the two surfaces over [`api`].
//! - [`github`]: the GitHub issues adapter.
//! - [`sync`]: forge reconcile (issues <-> cards) over a `Forge` port; [`syncer`] runs it in the background.
//! - [`settings`]: admin changes (agents, repositories, projects, roles, GitHub) persisted to files and applied live.
//! - [`tick`]: the rusty_tick adapter, where the cards live (ADR-0012).
//! - [`drain`]: work in flight, so a stop can wait for it.

pub mod api;
pub mod board;
pub mod domain;
pub mod drain;
pub mod github;
pub mod http;
pub mod identities;
pub mod mcp;
pub mod metrics;
pub mod projects;
pub mod roadmap;
pub mod settings;
pub mod store;
pub mod sync;
pub mod syncer;
pub mod tick;
pub mod web;
