---
category: Added
changelog: `rusty_tokio::select!` accepts tokio's full syntax and semantics (refutable patterns, `, if` guards, `biased;`, `else`, any number of branches, random polling order), now a proc macro in `rusty_tokio-macros`.
---
## 2026-10-24 - rusty_tokio: tokio's full `select!` (#900)

- **Added:** `select!` accepts `biased;`, `, if <precondition>` guards, an `else` branch, refutable patterns and up to 64 branches (tokio's limit). The semantics follow tokio's documentation. Every guard is evaluated before any future. A disabled branch's future is still built but never polled. Branches are polled from a random start each time unless `biased;`. An output that does not match its pattern disables the branch and the race continues. With every branch disabled, `else` runs, or `select!` panics with tokio's message. The handler runs after the other futures are dropped and in the caller's context, so it can `.await`, `?`, `return`, `break` and `continue`, and can borrow what the futures borrowed. This closes gap G6 in `docs/research/ASYNC-RUNTIME-INVENTORY.md`.
- **Changed:** `select!` is now a thin `macro_rules!` that forwards to a new `select_internal` proc macro in `rusty_tokio-macros`, passing `$crate` through, as tokio does. The old declarative macro was limited to 2 to 5 branches because a proc macro meant a new dependency. `rusty_tokio-macros` already depends on `syn`/`quote`, so that reason is gone. The random start comes from a small thread-local xorshift in `rusty_tokio`, with no new dependency. Without `biased;`, ties between ready branches no longer always go to the first one. The existing tie-order test now says `biased;`. `join!`/`try_join!` are unchanged.
- **Changed (consumers):** the 7 existing `rusty_tokio::select!` call sites all relied on the old written-order polling. They now say `biased;`, so their behaviour is unchanged:
  - `rusty_hyper` and `rusty_axum` `until_stopped`, where the stop signal must beat ready work;
  - `rusty_journal`'s accept loop;
  - `sessionmgr-tui`'s event loop;
  - `rusty_tonic`'s streaming write;
  - both `run_continuous` loops.

  `rusty_term`'s `fair_select` docs no longer claim that `select!` always polls in written order.
- **Fixed (review):** without `biased;`, a fair `select!` could return the ready work instead of `None` from `until_stopped` when the stop became ready alongside it. That happened when a dropped watch sender made `changed()` ready just after the `false` precheck, or when the stop and the work became ready together. Regression tests run each case 200 times on a current-thread runtime, so the racing task sees both ready at once. They fail without `biased;` and pass with it.
- **Verified:** `tests/select.rs` has one test per syntax form and semantic rule (refutable mismatch, `else`, a false guard with no poll and tokio's evaluation order, `biased;` over 1000 rounds, fairness over 1000 rounds, eight branches, the no-`else` panic, handler control flow, drop before the handler). Each form used by the workspace's `tokio::select!` and `rusty_tokio::select!` sites has an equivalent test. Also verified: `cargo test -p rusty_tokio -p rusty_tokio-macros --all-features`, clippy `-D warnings`, fmt, `cargo check` for `x86_64-pc-windows-gnu`, and `cargo check --all-targets` of every crate that already uses `rusty_tokio::select!`.
- **Known limitations:** `select!` does not charge the cooperative budget, as before. The `tokio::select!` call sites are not migrated here.
