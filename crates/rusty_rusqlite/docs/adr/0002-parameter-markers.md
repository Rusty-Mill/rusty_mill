# ADR-0002: Bound SQL parameter markers (`?`/`?N`/`:name`/`@name`/`$name`)

Status: Accepted
Date: 2026-08-14

## Context

`rusqlite` parity requires binding values into `?`/`:name`-style SQL
parameters (`Statement::execute`/`query`, the `Params`/`BindIndex`/`Name`
traits, `params!`/`named_params!`). Every attempt to build any part of
that surface (issues #25, #26–#30, #42, #44) was flagged and deferred
across this project's history as blocked on one question: how does a
parameter marker get represented once it's inside a parsed statement?

The engine's `Expr` tree (`Column`/`Literal`/`BinaryOp`/`FunctionCall`)
already models "a value not known until evaluated" for `WHERE` clauses.
`INSERT`'s value slots did not go through `Expr` at all —
`Insert::rows: Vec<Vec<Value>>` parses each `VALUES (...)` slot straight
into a storage-class [`crate::Value`], with no room for "this slot is a
placeholder, fill it in later." Adding parameter support to `INSERT`
therefore means changing `Insert::rows`'s element type — a breaking
change to an already-shipped public field, which this project's own
policy (see `RELEASE_NOTES.md`'s many "already-shipped signature, not
touching it" notes) treats as needing explicit human sign-off rather
than something to push through unilaterally. That sign-off is this ADR.

## Decision

1. **Tokenizer**: recognize `?` (optionally followed by digits, e.g.
   `?7`) and `:name`/`@name`/`$name` (sigil + identifier characters) as a
   single `Token::Param(String)`. The stored string is the digits after
   `?` (empty for a bare `?`), or the sigil-plus-name text for the named
   forms — SQLite treats `:foo`/`@foo`/`$foo` as distinct parameters
   even when the name text matches, so the sigil is part of the identity,
   not decoration.

2. **AST**: a new `dml_select::ParamMarker` enum (`Anonymous` / `Numbered(usize)`
   / `Named(String)`) and a new `Expr::Parameter(ParamMarker)` variant.
   `WHERE`-clause and aggregate-argument parsing already flow through the
   shared operand parser, so parameter markers are usable there for free
   once the tokenizer/AST support them.

3. **`Insert::rows` changes from `Vec<Vec<Value>>` to `Vec<Vec<Expr>>`.**
   This is the breaking change flagged above, now made deliberately: each
   `VALUES` slot parses to `Expr::Literal(v)` (unchanged shape) or
   `Expr::Parameter(marker)` — nothing else (the `INSERT` parser still
   only accepts literals and parameter markers per slot, not full
   expressions; that restriction is unrelated to this decision and stays
   as-is).

4. **Index resolution happens once, at `Connection::prepare` time**, not
   per-execution: a `Statement` walks its parsed `INSERT`/`SELECT` tree
   left-to-right, assigning each parameter occurrence a 1-based index
   using SQLite's own rule (bare `?` auto-increments; `?N` claims index
   `N` and bumps the auto-counter past it; a repeated `:name`/`@name`/
   `$name` reuses the index it was first assigned). Every `Parameter`
   node in the statement's own copy of the tree is rewritten to
   `ParamMarker::Numbered(resolved_index)` during this pass, and a
   parallel `index -> name` table is kept for `Statement::parameter_name`/
   `parameter_index`/`parameter_count`.

5. **`Statement::raw_bind_parameter(index, value)` stores into a
   `HashMap<usize, Value>`.** Before `execute`/`query*` hands a statement
   to the existing (unparameterized) engine functions, `Statement`
   substitutes every resolved `Parameter` node with
   `Expr::Literal(bound value, or NULL if unbound)` — matching real
   SQLite's own behavior for an unbound parameter — producing a
   fully-concrete tree. **This means the existing `execute_insert*`/
   `evaluate*`/`execute_select*` engine functions never need to know
   parameters exist at all**: they only ever see `Expr::Literal`, so
   their already-shipped signatures don't change. The one new thing they
   *do* need is a match arm for the new `Expr::Parameter` variant
   (unavoidable — it's a new enum variant, not a signature change), which
   defaults to `Value::Null` for any caller that isn't `Statement` (e.g.
   `Connection::query_map` given SQL text containing a literal `?`) —
   again matching real SQLite's unbound-parameter-is-NULL rule, so this
   isn't a special case invented for this crate.

## Alternatives considered

- **Add a `Value::Parameter` variant instead of extending `Expr`.**
  Rejected: `Value` represents SQLite storage classes (what a column
  actually holds); a parameter placeholder isn't one, and adding it would
  force every `Value` consumer (`ToSql`/`FromSql`/`compare_values`/
  `serialize.rs`/...) to handle a case that can never reach them in
  practice, for no benefit over reusing `Expr`, which already exists
  precisely to model "not evaluated yet."
- **Thread `bindings: &HashMap<usize, Value>` through `evaluate_with_functions`/
  `execute_insert_returning_rowids`/etc. as new parameters.** Rejected in
  favor of pre-resolving to a concrete `Expr::Literal` tree inside
  `Statement` before calling the existing engine functions: threading
  bindings through every layer means either breaking those functions'
  already-shipped signatures or growing a second `_with_bindings` sibling
  next to every one of them (six-plus new functions for one feature).
  Pre-resolution keeps the blast radius to "one new `Expr` variant, one
  new match arm per existing exhaustive match over `Expr`."
- **Change `Connection::execute`/`query_map` to accept a `params`
  argument**, matching real `rusqlite`'s signatures exactly. Rejected for
  this ADR's scope: those are already-shipped signatures from earlier
  PRs, and binding params through the low-level `Statement::raw_bind_parameter`
  primitive (what issue #25 actually asks for) doesn't require it. Adding
  an ergonomic `params`-argument surface on `Connection` (mirroring real
  `rusqlite`, via the still-unimplemented `Params` trait from issue #44)
  is a natural follow-up, not blocked by anything decided here.

## Consequences

- Closes issue #25 (`Statement::raw_bind_parameter`/`clear_bindings`).
- Unblocks (but doesn't itself implement) issue #44's `BindIndex`/
  `Params`/`Name` traits, issue #42's `params!`/`named_params!` macros,
  and the `params_from_iter` piece of issue #43 — all can now be built as
  ergonomic layers over `Statement::raw_bind_parameter`.
- `Insert::rows`'s element type change is the one deliberate breaking
  change this ADR authorizes; every direct construction/pattern-match of
  `Insert { rows: ... }` in this crate (tests included) was updated in
  the same PR that implements this ADR.
- `Statement::expanded_sql` can now do real value substitution (it
  previously just returned the original SQL text, since nothing could be
  bound); updated in the same PR.
- Still not attempted: `?`/`:name` markers as *result-column* expressions
  (`SELECT ? FROM t` binding a literal projection) — the `SELECT`
  column-list grammar (`SelectColumns::Named`, a plain name list) doesn't
  support arbitrary expressions in projection position at all yet, a
  separate, pre-existing gap unrelated to parameter binding.
