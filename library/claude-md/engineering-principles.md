---
name: engineering-principles
description: CLAUDE.md section - small, modular, typed, tested code; ports-and-adapters; ask before irreversible changes.
tags: [style, architecture, python, rust]
---
## Engineering principles

- Small, terse, modular, coherent. Simple > complex; flat > nested (guard clauses, early returns, <= 3 levels).
- Composition over inheritance. Modular monolith by default; extract a service only for a concrete forcing function (independent scaling, team/language boundary, fault isolation).
- Ports and adapters: domain logic has no I/O or framework imports.
- Explicit over implicit. Type annotations always. Immutable data; make illegal states unrepresentable.
- No abstraction before two real call sites. No speculative generality.
- Never swallow errors: handle, propagate with context, or log. Rust: `Result` and `?` in library code, no `unwrap()`/`expect()` outside tests.
- Tests for non-trivial logic: happy path plus failure and boundary cases.
- Minimal dependencies; justify each new one. Never commit or log secrets. Validate input at the boundary.
- Match surrounding conventions; keep diffs focused. Ask before schema/data migrations, public API changes, deletions, or dependency/toolchain bumps.
