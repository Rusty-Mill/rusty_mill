//! Hister's HTTP/JSON API and WebSocket search protocol — a Rust port of
//! `server/api.go`, `server/endpoints.go`, `server/session.go`, and
//! `server/oauth_handler.go`. This is the primary v1 deliverable (see
//! `docs/PROJECT-STATUS.md`): a backend that stays wire-compatible with
//! Hister's existing SvelteKit `webui` and the browser extension/qutebrowser
//! companion, none of which are being ported themselves.
//!
//! **Bootstrap stage — no implementation yet.** See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §1 for the
//! full 39-route table (verbatim descriptions preserved for API-consumer
//! parity), the session/OAuth model (GitHub, Google, OIDC, PKCE), the CSRF
//! model, and the WebSocket search protocol shared with the (out-of-v1) TUI.
