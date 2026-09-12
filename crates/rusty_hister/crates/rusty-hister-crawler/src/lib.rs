//! HTTP and JS-rendering crawler backends — a Rust port of
//! `server/crawler/`.
//!
//! **Bootstrap stage.** The non-JS-rendered `http` backend is not blocked —
//! `rusty_http`/`rusty_request` already cover it (see the sovereignty audit
//! in `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md`) — but no
//! implementation has landed yet since it should follow the same shared
//! `fetcher`/backend-abstraction shape as the JS-rendering backends rather
//! than being built first and retrofitted.
//!
//! The `chromedp`-equivalent backend is decided in `docs/decisions/
//! ADR-0003-js-rendering-crawler-approach-proposal.md` (Accepted):
//! `chromiumoxide` as a Tier A adapter dependency. The `bidi` backend is
//! **out of v1 scope** per that same ADR — explicitly descoped, not
//! deferred; do not implement it without a new ADR reopening the question.
//! See capability-inventory §8 for the backend abstraction, BFS traversal,
//! validator rules, robots.txt, proxy, and persistent-crawl-job semantics
//! this crate must reproduce — and note that only `proxy.go` has existing
//! Go test coverage; the rest of this subsystem needs first-principles Rust
//! test authorship.
