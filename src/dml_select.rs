//! SQL parser: `SELECT` (DML subset, foundation-tier `A4b`, extended by
//! many later issues — aggregates, `GROUP BY`/`HAVING`, window
//! functions, compound `SELECT`, `WITH`, and (issue #130) `INNER`/
//! `LEFT`/`CROSS JOIN`; subqueries remain unsupported, tracked as issue
//! #131). Parses `WHERE` into an [`Expr`] tree but does not evaluate it
//! — evaluation is `A6`. Grammar reference:
//! <https://www.sqlite.org/lang_select.html>.

use crate::ddl::ParseError;
use crate::token::Token;
use crate::value::Value;

/// The result-column list of a `SELECT`.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumns {
    /// `SELECT * FROM ...`
    All,
    /// `SELECT a, b FROM ...`
    Named(Vec<String>),
    /// A select list that's entirely aggregate-function calls, e.g.
    /// `SELECT COUNT(*), SUM(a) FROM t`. Evaluated over every row matching
    /// `filter`, bucketed by [`Select::group_by`] into one output row per
    /// distinct group (issue #125) — an empty `group_by` is one implicit
    /// whole-table group, same as this crate's pre-#125 behavior.
    ///
    /// **Scope, stated plainly:** the `GROUP BY` column(s) themselves
    /// aren't added to the output row — this select-list variant is
    /// still calls-only (no mixing a bare grouped column into the list
    /// alongside aggregate calls, e.g. `SELECT category, COUNT(*) ...`
    /// isn't parseable). `GROUP BY`/`HAVING` still work correctly for
    /// bucketing and filtering; the grouped value just isn't projected.
    /// Extending the select-list grammar to mix plain columns with
    /// aggregate calls is a larger, separate change than this issue's
    /// own "extends the existing whole-table aggregation path" scope.
    Aggregates(Vec<AggregateCall>),
    /// A select list that's entirely window-function calls, e.g.
    /// `SELECT SUM(a) OVER (PARTITION BY b) FROM t`. See [`WindowCall`]'s
    /// doc comment for this crate's "whole partition, no frame" scope.
    Window(Vec<WindowCall>),
}

/// One window-function call in a window select list, e.g. the
/// `SUM(a) OVER (PARTITION BY b)` in
/// `SELECT SUM(a) OVER (PARTITION BY b) FROM t`.
///
/// **Scope, stated plainly:** real SQLite window functions support an
/// `ORDER BY` inside `OVER (...)` plus an explicit frame clause
/// (`ROWS`/`RANGE BETWEEN ...`), which together let a window function's
/// result differ row-by-row within a partition (a running total, a rank,
/// `LAG`/`LEAD`, ...). Building that needs real per-partition ordering
/// and frame-boundary machinery — a comparable amount of new grammar and
/// execution logic to the vtab epic (#38), not a small addition. This
/// crate only supports `PARTITION BY` with no `ORDER BY`/frame: every
/// row in a partition gets the same whole-partition aggregate value
/// (built on the same [`crate::Aggregate`] used for whole-table
/// aggregation — see [`crate::Connection::create_window_function`]).
/// `ROW_NUMBER`/`RANK`/`DENSE_RANK`/`NTILE`/`LAG`/`LEAD` (which are
/// inherently row-position-dependent, not whole-partition aggregates)
/// aren't supported for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowCall {
    pub name: String,
    pub arg: AggregateArg,
    pub partition_by: Vec<String>,
}

/// One aggregate-function call in an aggregate select list, e.g. the
/// `COUNT(*)` in `SELECT COUNT(*) FROM t`.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateCall {
    pub name: String,
    pub arg: AggregateArg,
}

/// An aggregate call's argument.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateArg {
    /// `*`, as in `COUNT(*)`. The parser only accepts this for `COUNT` —
    /// real SQLite doesn't allow `SUM(*)`/`MIN(*)`/etc. either.
    Star,
    Expr(Box<Expr>),
}

/// A result-column name for an aggregate call, e.g. `COUNT(*)` or
/// `SUM(a)`. Simplified relative to real SQLite's full result-column-name
/// inference: any non-column expression argument is just shown as `expr`.
/// `pub(crate)` so `engine.rs`/`Statement` (different modules) can reuse
/// it for `Statement::column_names` on an aggregate `SELECT`, and so the
/// parser itself can reuse it for `HAVING`'s aggregate-reference syntax
/// (issue #125 — see [`SelectParser::parse_operand`]'s own doc comment).
pub(crate) fn describe_aggregate_call(call: &AggregateCall) -> String {
    let arg = match &call.arg {
        AggregateArg::Star => "*".to_string(),
        AggregateArg::Expr(expr) => match expr.as_ref() {
            Expr::Column(name) => name.clone(),
            _ => "expr".to_string(),
        },
    };
    format!("{}({arg})", call.name)
}

/// `INNER`/`LEFT [OUTER]`/`CROSS JOIN` (issue #130). A bare `JOIN` with
/// no leading keyword parses as `Inner`, matching real SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Cross,
}

/// A join's `ON`/`USING` condition (issue #130). `CROSS JOIN` takes
/// neither, hence `None` — the parser enforces that `Inner`/`Left`
/// always carry `On`/`Using` and `Cross` always carries `None`.
#[derive(Debug, Clone, PartialEq)]
pub enum JoinCondition {
    On(Expr),
    Using(Vec<String>),
    None,
}

/// A `FROM`/`JOIN` table reference: a table name plus an optional alias
/// (issue #130) — e.g. `orders o` / `orders AS o` parses as `TableRef {
/// name: "orders", alias: Some("o") }`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub name: String,
    pub alias: Option<String>,
}

/// One join in a `FROM` clause's join chain (issue #130). See
/// [`Select::joins`]'s own doc comment for how a chain of these becomes
/// one combined row source.
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub kind: JoinKind,
    pub table: TableRef,
    pub condition: JoinCondition,
}

/// A parsed `SELECT` statement — single-table, or (issue #130)
/// multi-table via `joins`.
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub columns: SelectColumns,
    pub table_name: String,
    /// The primary (`FROM`) table's alias, if given (issue #130) — e.g.
    /// `o` in `FROM orders o` / `FROM orders AS o`. Qualified-column
    /// resolution (`engine::scan_joined`) uses this (falling back to
    /// `table_name` itself when `None`) as `table_name`'s qualifier.
    pub table_alias: Option<String>,
    /// Additional tables joined onto `table_name`, in `FROM`-clause
    /// order (issue #130) — empty for a plain single-table `SELECT`,
    /// this crate's only supported shape before this issue and still
    /// the overwhelmingly common case. A non-empty chain is folded,
    /// left to right, into one combined row source with `"qualifier.
    /// column"`-named columns (see `engine::scan_joined`) before
    /// `filter`/projection run — `eval::resolve_column_index` then lets
    /// both the qualified form and (when unambiguous) the bare column
    /// name resolve.
    pub joins: Vec<Join>,
    pub filter: Option<Expr>,
    /// Whether `DISTINCT` followed `SELECT` (issue #116) — the engine
    /// dedups the final output rows, preserving first-occurrence order.
    pub distinct: bool,
    /// `GROUP BY col, ...` (issue #125) — only meaningful (and only
    /// checked) for [`SelectColumns::Aggregates`]; empty means "one
    /// implicit whole-table group", matching this crate's pre-#125
    /// aggregate behavior exactly (see `engine::execute_select_with_aggregates`).
    pub group_by: Vec<String>,
    /// `HAVING expr` (issue #125) — a post-aggregation filter, evaluated
    /// once per group rather than once per row like `WHERE`. Parsed with
    /// [`SelectParser::parse_operand`]'s aggregate-reference extension,
    /// so it can reference an aggregate result (e.g. `HAVING COUNT(*) >
    /// 1`), which plain `WHERE` cannot (grouping/aggregation hasn't
    /// happened yet when `WHERE` runs).
    pub having: Option<Expr>,
}

/// `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` (issue #126) — how two
/// `SELECT`s' result sets combine in a [`CompoundSelect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    /// Concatenates both sides, then dedups the combined rows.
    Union,
    /// Concatenates both sides with no deduplication.
    UnionAll,
    /// Only rows present on both sides, deduped.
    Intersect,
    /// Only the left side's rows absent from the right side, deduped.
    Except,
}

/// A parsed compound `SELECT`: `select-core (UNION [ALL] | INTERSECT |
/// EXCEPT select-core)*` (issue #126) — a plain, non-compound `SELECT`
/// is `rest: vec![]`. Each side executes independently through the same
/// path a standalone [`Select`] would (`first`, then each `rest` entry,
/// left-associative); combining is a pure Rust-side `Vec` operation over
/// the two sides' already-materialized rows — no new execution model.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelect {
    pub first: Select,
    pub rest: Vec<(CompoundOp, Select)>,
}

/// One `name AS (SELECT ...)` common table expression in a `WITH`
/// clause (issue #127). `WITH RECURSIVE` is explicitly deferred (epic
/// #111's own Part 3 note) — a genuinely different execution shape
/// (iterate to a fixed point) rather than a small addition.
#[derive(Debug, Clone, PartialEq)]
pub struct Cte {
    pub name: String,
    pub select: CompoundSelect,
}

/// A parsed `WITH name AS (SELECT ...) [, ...] <select-stmt>` statement
/// (issue #127). Each `cte` is executed once, in order, and its result
/// materialized as an ephemeral table visible by name to every `cte`
/// after it and to `body` — see `Connection::run_with_select`'s doc
/// comment for exactly how (no new subsystem: an ephemeral use of the
/// existing `TableSource`/virtual-table machinery). **Decided and
/// documented (this issue's own "decide and document" acceptance
/// point):** a CTE name shadows a real table of the same name for the
/// duration of this statement, matching real SQLite's own precedence.
#[derive(Debug, Clone, PartialEq)]
pub struct WithSelect {
    pub ctes: Vec<Cte>,
    pub body: CompoundSelect,
}

/// A minimal expression tree — enough to represent `WHERE` filters.
/// Evaluated by `A6`, not here.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(String),
    Literal(Value),
    BinaryOp {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `left AND right` (issue #112). Three-valued: `eval.rs` combines
    /// per SQLite's own `NULL`-propagation rule (`FALSE AND NULL` is
    /// `FALSE`, `TRUE AND NULL` is `NULL`), not plain two-valued boolean
    /// `&&`.
    And(Box<Expr>, Box<Expr>),
    /// `left OR right` (issue #112). Three-valued, mirroring [`Expr::And`]
    /// (`TRUE OR NULL` is `TRUE`, `FALSE OR NULL` is `NULL`).
    Or(Box<Expr>, Box<Expr>),
    /// `NOT expr` (issue #112). `NOT NULL` is `NULL`.
    Not(Box<Expr>),
    /// `left LIKE pattern [ESCAPE escape]` (`NOT LIKE` if `negate`) —
    /// issue #113. `%` matches any run of characters, `_` matches one,
    /// ASCII-case-insensitive.
    Like {
        left: Box<Expr>,
        pattern: Box<Expr>,
        escape: Option<Box<Expr>>,
        negate: bool,
    },
    /// `left GLOB pattern` (`NOT GLOB` if `negate`) — issue #113. Unix
    /// glob syntax (`*`/`?`/`[...]`), case-sensitive, no escape.
    Glob {
        left: Box<Expr>,
        pattern: Box<Expr>,
        negate: bool,
    },
    /// `expr BETWEEN low AND high` (`NOT BETWEEN` if `negate`) — issue
    /// #113. Sugar for `expr >= low AND expr <= high`.
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negate: bool,
    },
    /// `CASE [operand] WHEN cond1 THEN result1 ... [ELSE else_result] END`
    /// — issue #115. `operand` is `Some` for the simple form (`CASE
    /// operand WHEN val THEN ...`, matched by equality) and `None` for
    /// the searched form (`CASE WHEN cond THEN ...`, each `cond`
    /// evaluated as a boolean). The first matching branch's result wins;
    /// `else_result` (or `NULL` if absent) if none match.
    Case {
        operand: Option<Box<Expr>>,
        branches: Vec<(Expr, Expr)>,
        else_result: Option<Box<Expr>>,
    },
    /// `expr IN (v1, v2, ...)` (`NOT IN` if `negate`) — issue #114,
    /// literal-list form only. `IN (SELECT ...)` (subquery form) needs
    /// nested query execution, which this crate's `Expr`/`eval.rs`
    /// layering doesn't support yet — tracked separately as a
    /// new-subsystem gap alongside subqueries in general (issue #131).
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negate: bool,
    },
    /// A scalar function call, e.g. `UPPER(name)`. Evaluated only by
    /// `eval::evaluate_with_functions` — plain `evaluate`/`evaluate_bool`
    /// (which predate function-call support) error on this variant rather
    /// than silently treating it as something else.
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
    /// A `?`/`?N`/`:name`/`@name`/`$name` bound-parameter marker — see
    /// `docs/adr/0002-parameter-markers.md`. `crate::Statement` resolves
    /// these to a bound value (or `Value::Null`, for an unbound one)
    /// before evaluation; the plain `eval`/`engine` functions (given no
    /// bindings to consult) also treat every `Parameter` as `Value::Null`,
    /// matching real SQLite's own unbound-parameter default.
    Parameter(ParamMarker),
}

/// A `?`/`?N`/`:name`/`@name`/`$name`-style bound-parameter marker, as
/// parsed. Not yet resolved to a concrete index — see
/// `docs/adr/0002-parameter-markers.md`'s index-resolution rule, applied
/// by `crate::Statement` at prepare time.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamMarker {
    /// Bare `?` — positional, auto-numbered by left-to-right occurrence
    /// order starting at 1 (SQLite's convention).
    Anonymous,
    /// `?N` — an explicit 1-based positional index.
    Numbered(usize),
    /// `:name`/`@name`/`$name`, sigil included — SQLite treats these as
    /// distinct namespaces even for matching name text (`:foo` and
    /// `@foo` are different parameters).
    Named(String),
}

/// Parses a `Token::Param`'s stored spec text into a [`ParamMarker`].
pub(crate) fn parse_param_marker(spec: &str) -> ParamMarker {
    if spec.is_empty() {
        ParamMarker::Anonymous
    } else if let Ok(n) = spec.parse::<usize>() {
        ParamMarker::Numbered(n)
    } else {
        ParamMarker::Named(spec.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// Parses a single-table `SELECT` statement from a token stream (as
/// produced by [`crate::tokenize`]). Any trailing tokens past the parsed
/// `SELECT` (e.g. a `UNION`/`INTERSECT`/`EXCEPT` clause) are left
/// unconsumed rather than erroring — see [`parse_select_at`], which
/// [`parse_compound_select`] (issue #126) uses to chain multiple
/// `SELECT`s from one token stream instead of erroring on the first
/// one's trailing tokens.
pub fn parse_select(tokens: &[Token]) -> Result<Select, ParseError> {
    let (select, _) = parse_select_at(tokens, 0)?;
    Ok(select)
}

/// Parses a single-table `SELECT` statement starting at `tokens[pos]`.
/// Returns the parsed [`Select`] and the index of the first token past
/// it — the same "parse at an offset, report where it stopped" shape as
/// [`parse_expr_at`]/[`parse_operand_at`], letting [`parse_compound_select`]
/// chain multiple `SELECT`s from one token stream.
pub(crate) fn parse_select_at(tokens: &[Token], pos: usize) -> Result<(Select, usize), ParseError> {
    let mut p = SelectParser {
        tokens,
        pos,
        in_having: false,
    };
    p.expect_ident("SELECT")?;

    let distinct = if p.peek_ident("DISTINCT") {
        p.advance();
        true
    } else {
        false
    };

    let columns = if p.peek_punct("*") {
        p.advance();
        SelectColumns::All
    } else if p.starts_aggregate_call() {
        let first = p.parse_aggregate_call()?;
        if p.peek_ident("OVER") {
            let mut windows = vec![p.parse_window_call(first)?];
            while p.peek_punct(",") {
                p.advance();
                let call = p.parse_aggregate_call()?;
                windows.push(p.parse_window_call(call)?);
            }
            SelectColumns::Window(windows)
        } else {
            let mut calls = vec![first];
            while p.peek_punct(",") {
                p.advance();
                calls.push(p.parse_aggregate_call()?);
            }
            SelectColumns::Aggregates(calls)
        }
    } else {
        let mut cols = vec![p.parse_qualified_ident()?];
        while p.peek_punct(",") {
            p.advance();
            cols.push(p.parse_qualified_ident()?);
        }
        SelectColumns::Named(cols)
    };

    p.expect_ident("FROM")?;
    let table_name = p.expect_any_ident()?;
    let table_alias = p.parse_table_alias()?;
    let joins = p.parse_joins()?;

    let filter = if p.peek_ident("WHERE") {
        p.advance();
        Some(p.parse_or_expr()?)
    } else {
        None
    };

    let group_by = if p.peek_ident("GROUP") {
        p.advance();
        p.expect_ident("BY")?;
        let mut cols = vec![p.expect_any_ident()?];
        while p.peek_punct(",") {
            p.advance();
            cols.push(p.expect_any_ident()?);
        }
        cols
    } else {
        Vec::new()
    };

    let having = if p.peek_ident("HAVING") {
        p.advance();
        Some(p.parse_having_expr()?)
    } else {
        None
    };

    Ok((
        Select {
            columns,
            table_name,
            table_alias,
            joins,
            filter,
            distinct,
            group_by,
            having,
        },
        p.pos,
    ))
}

/// Parses a compound `SELECT` — `select-core (UNION [ALL] | INTERSECT |
/// EXCEPT select-core)*` (issue #126) — from a token stream (as produced
/// by [`crate::tokenize`]). A plain, non-compound `SELECT` parses fine
/// here too (`rest` is just empty) — this is the entry point every
/// actual statement-dispatch site (`Statement::prepare`,
/// `Connection::query_row`/`query_map`/`query_one`) should use instead
/// of [`parse_select`] directly, so a `UNION`/`INTERSECT`/`EXCEPT`
/// clause is never silently dropped as unconsumed trailing tokens.
pub fn parse_compound_select(tokens: &[Token]) -> Result<CompoundSelect, ParseError> {
    let (compound, _) = parse_compound_select_at(tokens, 0)?;
    Ok(compound)
}

/// Parses a compound `SELECT` starting at `tokens[pos]`. Returns the
/// parsed [`CompoundSelect`] and the index of the first token past it —
/// the same "parse at an offset, report where it stopped" shape as
/// [`parse_select_at`], letting [`parse_with_select`] (issue #127) parse
/// a `WITH` clause's parenthesized CTE bodies (which may themselves be
/// compound `SELECT`s) from the middle of a larger token stream.
pub(crate) fn parse_compound_select_at(
    tokens: &[Token],
    pos: usize,
) -> Result<(CompoundSelect, usize), ParseError> {
    let (first, mut pos) = parse_select_at(tokens, pos)?;

    let mut rest = Vec::new();
    loop {
        let op = if token_is_ident(tokens, pos, "UNION") {
            pos += 1;
            if token_is_ident(tokens, pos, "ALL") {
                pos += 1;
                CompoundOp::UnionAll
            } else {
                CompoundOp::Union
            }
        } else if token_is_ident(tokens, pos, "INTERSECT") {
            pos += 1;
            CompoundOp::Intersect
        } else if token_is_ident(tokens, pos, "EXCEPT") {
            pos += 1;
            CompoundOp::Except
        } else {
            break;
        };

        let (next_select, next_pos) = parse_select_at(tokens, pos)?;
        pos = next_pos;
        rest.push((op, next_select));
    }

    Ok((CompoundSelect { first, rest }, pos))
}

/// Parses a `WITH name AS (SELECT ...) [, ...] <select-stmt>` statement
/// (issue #127) from a token stream (as produced by [`crate::tokenize`]).
pub fn parse_with_select(tokens: &[Token]) -> Result<WithSelect, ParseError> {
    let mut p = SelectParser {
        tokens,
        pos: 0,
        in_having: false,
    };
    p.expect_ident("WITH")?;

    let mut ctes = Vec::new();
    loop {
        let name = p.expect_any_ident()?;
        p.expect_ident("AS")?;
        p.expect_punct("(")?;
        let (select, new_pos) = parse_compound_select_at(p.tokens, p.pos)?;
        p.pos = new_pos;
        p.expect_punct(")")?;
        ctes.push(Cte { name, select });

        if p.peek_punct(",") {
            p.advance();
            continue;
        }
        break;
    }

    let (body, _) = parse_compound_select_at(p.tokens, p.pos)?;

    Ok(WithSelect { ctes, body })
}

fn token_is_ident(tokens: &[Token], pos: usize, keyword: &str) -> bool {
    matches!(tokens.get(pos), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
}

/// Parses a single expression (the full boolean/comparison precedence
/// chain used by `WHERE`) starting at `tokens[pos]`. Returns the parsed
/// [`Expr`] and the index of the first token past it.
///
/// Reused by `ddl.rs` for `CHECK`/`DEFAULT` column constraints (issue
/// #117), which need the same expression grammar without a full `SELECT`
/// wrapped around it — rather than duplicating `parse_or_expr` and its
/// whole precedence chain in a second parser.
pub(crate) fn parse_expr_at(tokens: &[Token], pos: usize) -> Result<(Expr, usize), ParseError> {
    let mut p = SelectParser {
        tokens,
        pos,
        in_having: false,
    };
    let expr = p.parse_or_expr()?;
    Ok((expr, p.pos))
}

/// Parses a single primary expression (a literal, column, function call,
/// `CASE`, or parameter — no comparison operator required) starting at
/// `tokens[pos]`. Returns the parsed [`Expr`] and the index of the first
/// token past it.
///
/// Reused by `ddl.rs` for `DEFAULT` column-constraint values (issue
/// #117): [`parse_expr_at`] goes through `parse_comparison`, which
/// *requires* a binary operator after its left operand (it models
/// `WHERE`, where a bare value isn't itself a filter) — wrong for
/// `DEFAULT 1`, a bare value with no operator at all.
pub(crate) fn parse_operand_at(tokens: &[Token], pos: usize) -> Result<(Expr, usize), ParseError> {
    let mut p = SelectParser {
        tokens,
        pos,
        in_having: false,
    };
    let expr = p.parse_operand()?;
    Ok((expr, p.pos))
}

struct SelectParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Set only while parsing a `HAVING` clause (issue #125) — see
    /// [`SelectParser::parse_operand`]'s aggregate-reference branch.
    in_having: bool,
}

impl<'a> SelectParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    /// Whether the select list starting at the current position looks like
    /// `IDENT(` — an aggregate call — rather than a plain column name.
    fn starts_aggregate_call(&self) -> bool {
        matches!(self.peek(), Some(Token::Ident(_)))
            && matches!(self.peek_at(1), Some(Token::Punct(s)) if *s == "(")
    }

    fn parse_aggregate_call(&mut self) -> Result<AggregateCall, ParseError> {
        let name = self.expect_any_ident()?;
        self.expect_punct("(")?;
        let arg = if self.peek_punct("*") {
            if !name.eq_ignore_ascii_case("COUNT") {
                return Err(ParseError::UnexpectedToken(
                    "'*' is only a valid argument to COUNT".to_string(),
                ));
            }
            self.advance();
            AggregateArg::Star
        } else {
            AggregateArg::Expr(Box::new(self.parse_operand()?))
        };
        self.expect_punct(")")?;
        Ok(AggregateCall { name, arg })
    }

    /// Parses `OVER (PARTITION BY col1, col2, ...)` (or `OVER ()`, no
    /// partitioning) following an already-parsed [`AggregateCall`], into
    /// a [`WindowCall`] — see that type's doc comment for the "no
    /// `ORDER BY`/frame clause" scope limit.
    fn parse_window_call(&mut self, call: AggregateCall) -> Result<WindowCall, ParseError> {
        self.expect_ident("OVER")?;
        self.expect_punct("(")?;
        let mut partition_by = Vec::new();
        if self.peek_ident("PARTITION") {
            self.advance();
            self.expect_ident("BY")?;
            partition_by.push(self.expect_any_ident()?);
            while self.peek_punct(",") {
                self.advance();
                partition_by.push(self.expect_any_ident()?);
            }
        }
        self.expect_punct(")")?;
        Ok(WindowCall {
            name: call.name,
            arg: call.arg,
            partition_by,
        })
    }

    fn peek_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Token::Punct(s)) if *s == p)
    }

    fn peek_ident(&self, keyword: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos);
        self.pos += 1;
        tok
    }

    fn expect_ident(&mut self, keyword: &str) -> Result<(), ParseError> {
        match self.advance() {
            Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword) => Ok(()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    fn expect_any_ident(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    /// Like [`SelectParser::expect_any_ident`], but also accepts a
    /// `table.column`-qualified form (issue #130), returned as one
    /// dotted string (`"table.column"`) — [`eval::resolve_column_index`]
    /// (a different module) is what actually understands that shape at
    /// resolution time; the parser here only recognizes and joins the
    /// two tokens.
    fn parse_qualified_ident(&mut self) -> Result<String, ParseError> {
        let first = self.expect_any_ident()?;
        if self.peek_punct(".") {
            self.advance();
            let second = self.expect_any_ident()?;
            Ok(format!("{first}.{second}"))
        } else {
            Ok(first)
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<(), ParseError> {
        match self.advance() {
            Some(Token::Punct(s)) if *s == p => Ok(()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    /// Parses an optional table alias following a `FROM`/`JOIN` table
    /// reference (issue #130): `[AS] ident`, or nothing. A bare (no
    /// `AS`) alias is only accepted when the next identifier isn't one
    /// of the keywords that can legitimately follow a table reference
    /// here (`WHERE`, a join keyword, `ON`/`USING`, `GROUP`, `HAVING`,
    /// or a compound-`SELECT` operator) — otherwise e.g. `FROM t WHERE
    /// ...` would misparse `WHERE` itself as `t`'s alias.
    fn parse_table_alias(&mut self) -> Result<Option<String>, ParseError> {
        if self.peek_ident("AS") {
            self.advance();
            return Ok(Some(self.expect_any_ident()?));
        }
        const RESERVED: &[&str] = &[
            "WHERE",
            "INNER",
            "LEFT",
            "CROSS",
            "JOIN",
            "ON",
            "USING",
            "GROUP",
            "HAVING",
            "UNION",
            "INTERSECT",
            "EXCEPT",
        ];
        if let Some(Token::Ident(s)) = self.peek() {
            if !RESERVED.iter().any(|kw| s.eq_ignore_ascii_case(kw)) {
                let alias = s.clone();
                self.advance();
                return Ok(Some(alias));
            }
        }
        Ok(None)
    }

    /// Parses a `FROM` clause's join chain (issue #130): zero or more
    /// `[INNER | LEFT [OUTER] | CROSS] JOIN table [[AS] alias] (ON expr
    /// | USING (col, ...))`, in order. A bare `JOIN` (no leading
    /// keyword) is `INNER`. `CROSS JOIN` takes no `ON`/`USING` — any
    /// other kind requires exactly one of them.
    fn parse_joins(&mut self) -> Result<Vec<Join>, ParseError> {
        let mut joins = Vec::new();
        loop {
            let kind = if self.peek_ident("INNER") {
                self.advance();
                self.expect_ident("JOIN")?;
                JoinKind::Inner
            } else if self.peek_ident("LEFT") {
                self.advance();
                if self.peek_ident("OUTER") {
                    self.advance();
                }
                self.expect_ident("JOIN")?;
                JoinKind::Left
            } else if self.peek_ident("CROSS") {
                self.advance();
                self.expect_ident("JOIN")?;
                JoinKind::Cross
            } else if self.peek_ident("JOIN") {
                self.advance();
                JoinKind::Inner
            } else {
                break;
            };

            let name = self.expect_any_ident()?;
            let alias = self.parse_table_alias()?;

            let condition = if kind == JoinKind::Cross {
                JoinCondition::None
            } else if self.peek_ident("ON") {
                self.advance();
                JoinCondition::On(self.parse_or_expr()?)
            } else if self.peek_ident("USING") {
                self.advance();
                self.expect_punct("(")?;
                let mut cols = vec![self.expect_any_ident()?];
                while self.peek_punct(",") {
                    self.advance();
                    cols.push(self.expect_any_ident()?);
                }
                self.expect_punct(")")?;
                JoinCondition::Using(cols)
            } else {
                return Err(ParseError::UnexpectedToken(
                    "expected ON or USING after JOIN".to_string(),
                ));
            };

            joins.push(Join {
                kind,
                table: TableRef { name, alias },
                condition,
            });
        }
        Ok(joins)
    }

    /// `expr (OR expr)*`, left-associative — the lowest-precedence level
    /// of the `WHERE` boolean grammar (issue #112).
    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.peek_ident("OR") {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Parses a `HAVING` clause's expression — the same boolean grammar
    /// as `WHERE` ([`SelectParser::parse_or_expr`]), but with
    /// [`SelectParser::in_having`] set for its duration so
    /// [`SelectParser::parse_operand`] treats `IDENT(...)` as an
    /// aggregate-call reference (issue #125). Scoped to exactly this
    /// call — reset afterward, so `WHERE`/`CHECK`/`DEFAULT` parsing
    /// elsewhere is unaffected.
    fn parse_having_expr(&mut self) -> Result<Expr, ParseError> {
        self.in_having = true;
        let result = self.parse_or_expr();
        self.in_having = false;
        result
    }

    /// `expr (AND expr)*`, left-associative — binds tighter than `OR`,
    /// looser than `NOT`, matching SQLite's own precedence.
    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not_expr()?;
        while self.peek_ident("AND") {
            self.advance();
            let right = self.parse_not_expr()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `NOT expr` (right-recursive, so `NOT NOT x` parses) or a plain
    /// boolean primary.
    fn parse_not_expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek_ident("NOT") {
            self.advance();
            let inner = self.parse_not_expr()?;
            Ok(Expr::Not(Box::new(inner)))
        } else {
            self.parse_bool_primary()
        }
    }

    /// A parenthesized boolean sub-expression (`(a = 1 OR b = 2)`) or a
    /// single comparison. Grouping parens are scoped to the boolean
    /// grammar only here — this crate has no general arithmetic/operand
    /// grouping to extend (`parse_operand` never accepted `(` either).
    fn parse_bool_primary(&mut self) -> Result<Expr, ParseError> {
        if self.peek_punct("(") {
            self.advance();
            let inner = self.parse_or_expr()?;
            self.expect_punct(")")?;
            Ok(inner)
        } else {
            self.parse_comparison()
        }
    }

    fn peek_ident_at(&self, offset: usize, keyword: &str) -> bool {
        matches!(self.peek_at(offset), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_operand()?;

        // `NOT LIKE`/`NOT GLOB`/`NOT BETWEEN`/`NOT IN` -- infix negation,
        // distinct from the prefix `NOT` `parse_not_expr` already handles
        // (that one wraps a whole boolean primary; this one only fires
        // mid-comparison, so there's no ambiguity between the two).
        let negate = self.peek_ident("NOT")
            && (self.peek_ident_at(1, "LIKE")
                || self.peek_ident_at(1, "GLOB")
                || self.peek_ident_at(1, "BETWEEN")
                || self.peek_ident_at(1, "IN"));
        if negate {
            self.advance();
        }

        if self.peek_ident("LIKE") {
            self.advance();
            let pattern = self.parse_operand()?;
            let escape = if self.peek_ident("ESCAPE") {
                self.advance();
                Some(Box::new(self.parse_operand()?))
            } else {
                None
            };
            return Ok(Expr::Like {
                left: Box::new(left),
                pattern: Box::new(pattern),
                escape,
                negate,
            });
        }
        if self.peek_ident("GLOB") {
            self.advance();
            let pattern = self.parse_operand()?;
            return Ok(Expr::Glob {
                left: Box::new(left),
                pattern: Box::new(pattern),
                negate,
            });
        }
        if self.peek_ident("BETWEEN") {
            self.advance();
            let low = self.parse_operand()?;
            self.expect_ident("AND")?;
            let high = self.parse_operand()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
                negate,
            });
        }
        if self.peek_ident("IN") {
            self.advance();
            self.expect_punct("(")?;
            let mut list = Vec::new();
            if !self.peek_punct(")") {
                loop {
                    list.push(self.parse_operand()?);
                    if self.peek_punct(",") {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            self.expect_punct(")")?;
            return Ok(Expr::InList {
                expr: Box::new(left),
                list,
                negate,
            });
        }

        let op = match self.advance() {
            Some(Token::Punct("=")) => BinaryOp::Eq,
            Some(Token::Punct("<>")) | Some(Token::Punct("!=")) => BinaryOp::NotEq,
            Some(Token::Punct("<")) => BinaryOp::Lt,
            Some(Token::Punct("<=")) => BinaryOp::LtEq,
            Some(Token::Punct(">")) => BinaryOp::Gt,
            Some(Token::Punct(">=")) => BinaryOp::GtEq,
            Some(Token::Eof) | None => return Err(ParseError::UnexpectedEof),
            Some(other) => return Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        };
        let right = self.parse_operand()?;
        Ok(Expr::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    /// Parses a primary expression. Beyond the usual literal/column/
    /// function-call/`CASE`/parameter cases, when [`SelectParser::
    /// in_having`] is set (i.e. only while parsing a `HAVING` clause —
    /// issue #125), an `IDENT(...)` shape is parsed as an aggregate-call
    /// reference (via [`SelectParser::parse_aggregate_call`], so
    /// `COUNT(*)` works) rather than a scalar [`Expr::FunctionCall`], and
    /// represented as `Expr::Column` under that call's display name
    /// (matching [`describe_aggregate_call`], the same name the select
    /// list's own aggregate calls surface as output columns under) —
    /// this lets `HAVING COUNT(*) > 1` evaluate against the group's
    /// already-finalized aggregate value with the ordinary boolean
    /// evaluator, no special-cased "is this an aggregate?" logic needed
    /// in `eval.rs`. Consistent with the select list's own established
    /// rule that `IDENT(...)` always means an aggregate call there too
    /// (never a plain scalar function call — scalar calls are a
    /// `WHERE`/`CHECK`/`DEFAULT`-context-only concept in this grammar).
    fn parse_operand(&mut self) -> Result<Expr, ParseError> {
        if self.in_having && self.starts_aggregate_call() {
            let call = self.parse_aggregate_call()?;
            return Ok(Expr::Column(describe_aggregate_call(&call)));
        }
        match self.peek().cloned() {
            Some(Token::Ident(name)) if name.eq_ignore_ascii_case("CASE") => {
                self.advance();
                self.parse_case_expr()
            }
            Some(Token::Ident(name)) => {
                self.advance();
                if self.peek_punct("(") {
                    self.advance();
                    let mut args = Vec::new();
                    if !self.peek_punct(")") {
                        loop {
                            args.push(self.parse_operand()?);
                            if self.peek_punct(",") {
                                self.advance();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_punct(")")?;
                    Ok(Expr::FunctionCall { name, args })
                } else if self.peek_punct(".") {
                    // `table.column` (issue #130) -- joined the qualifier
                    // and column into one dotted `Expr::Column` string;
                    // see `eval::resolve_column_index` for how that's
                    // resolved against a joined row source's qualified
                    // column names.
                    self.advance();
                    let column = self.expect_any_ident()?;
                    Ok(Expr::Column(format!("{name}.{column}")))
                } else {
                    Ok(Expr::Column(name))
                }
            }
            Some(Token::Integer(n)) => {
                self.advance();
                Ok(Expr::Literal(Value::Integer(n)))
            }
            Some(Token::Real(f)) => {
                self.advance();
                Ok(Expr::Literal(Value::Real(f)))
            }
            Some(Token::String(s)) => {
                self.advance();
                Ok(Expr::Literal(Value::Text(s)))
            }
            Some(Token::Blob(b)) => {
                self.advance();
                Ok(Expr::Literal(Value::Blob(b)))
            }
            Some(Token::Param(spec)) => {
                self.advance();
                Ok(Expr::Parameter(parse_param_marker(&spec)))
            }
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    /// Parses a `CASE` expression's body — the leading `CASE` keyword is
    /// already consumed. Simple form (`CASE operand WHEN val THEN
    /// ...`) if the next token isn't `WHEN`; searched form (`CASE WHEN
    /// cond THEN ...`, `cond` a full boolean expression) otherwise.
    fn parse_case_expr(&mut self) -> Result<Expr, ParseError> {
        let operand = if self.peek_ident("WHEN") {
            None
        } else {
            Some(Box::new(self.parse_operand()?))
        };

        let mut branches = Vec::new();
        while self.peek_ident("WHEN") {
            self.advance();
            let cond = if operand.is_some() {
                self.parse_operand()?
            } else {
                self.parse_or_expr()?
            };
            self.expect_ident("THEN")?;
            let result = self.parse_operand()?;
            branches.push((cond, result));
        }
        if branches.is_empty() {
            return Err(ParseError::UnexpectedToken(
                "CASE needs at least one WHEN branch".to_string(),
            ));
        }

        let else_result = if self.peek_ident("ELSE") {
            self.advance();
            Some(Box::new(self.parse_operand()?))
        } else {
            None
        };
        self.expect_ident("END")?;

        Ok(Expr::Case {
            operand,
            branches,
            else_result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    #[test]
    fn parses_select_star() {
        let tokens = tokenize("SELECT * FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.columns, SelectColumns::All);
        assert_eq!(select.table_name, "t");
        assert_eq!(select.filter, None);
        assert!(!select.distinct);
    }

    #[test]
    fn parses_select_distinct_star() {
        let tokens = tokenize("SELECT DISTINCT * FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.distinct);
        assert_eq!(select.columns, SelectColumns::All);
    }

    #[test]
    fn parses_select_distinct_named_columns() {
        let tokens = tokenize("SELECT DISTINCT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.distinct);
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_named_columns() {
        let tokens = tokenize("SELECT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_where_clause() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Column("a".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_not_eq_operator() {
        let tokens = tokenize("SELECT * FROM t WHERE a <> 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::NotEq,
                left: Box::new(Expr::Column("a".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn missing_from_keyword_is_an_error() {
        let tokens = tokenize("SELECT a t").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn parses_function_call_in_where() {
        let tokens = tokenize("SELECT * FROM t WHERE UPPER(name) = 'X'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::FunctionCall {
                    name: "UPPER".into(),
                    args: vec![Expr::Column("name".into())],
                }),
                right: Box::new(Expr::Literal(Value::Text("X".into()))),
            })
        );
    }

    #[test]
    fn parses_function_call_with_multiple_args() {
        let tokens = tokenize("SELECT * FROM t WHERE ADD(a, 1) = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *left,
            Expr::FunctionCall {
                name: "ADD".into(),
                args: vec![Expr::Column("a".into()), Expr::Literal(Value::Integer(1))],
            }
        );
    }

    #[test]
    fn parses_function_call_with_no_args() {
        let tokens = tokenize("SELECT * FROM t WHERE RANDOM() = 0").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *left,
            Expr::FunctionCall {
                name: "RANDOM".into(),
                args: vec![],
            }
        );
    }

    #[test]
    fn parses_count_star() {
        let tokens = tokenize("SELECT COUNT(*) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Aggregates(vec![AggregateCall {
                name: "COUNT".into(),
                arg: AggregateArg::Star,
            }])
        );
    }

    #[test]
    fn parses_multiple_aggregate_calls() {
        let tokens = tokenize("SELECT SUM(a), MAX(b) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Aggregates(vec![
                AggregateCall {
                    name: "SUM".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                },
                AggregateCall {
                    name: "MAX".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("b".into()))),
                },
            ])
        );
    }

    #[test]
    fn star_arg_only_valid_for_count() {
        let tokens = tokenize("SELECT SUM(*) FROM t").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn plain_column_list_still_parses_as_named() {
        let tokens = tokenize("SELECT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_window_call_with_partition_by() {
        let tokens = tokenize("SELECT SUM(a) OVER (PARTITION BY b) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![WindowCall {
                name: "SUM".into(),
                arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                partition_by: vec!["b".into()],
            }])
        );
    }

    #[test]
    fn parses_window_call_with_no_partition() {
        let tokens = tokenize("SELECT COUNT(*) OVER () FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![WindowCall {
                name: "COUNT".into(),
                arg: AggregateArg::Star,
                partition_by: vec![],
            }])
        );
    }

    #[test]
    fn parses_multiple_window_calls_and_multi_column_partition() {
        let tokens = tokenize(
            "SELECT SUM(a) OVER (PARTITION BY b, c), COUNT(*) OVER (PARTITION BY b) FROM t",
        )
        .unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![
                WindowCall {
                    name: "SUM".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                    partition_by: vec!["b".into(), "c".into()],
                },
                WindowCall {
                    name: "COUNT".into(),
                    arg: AggregateArg::Star,
                    partition_by: vec!["b".into()],
                },
            ])
        );
    }

    fn eq(col: &str, n: i64) -> Expr {
        Expr::BinaryOp {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Column(col.into())),
            right: Box::new(Expr::Literal(Value::Integer(n))),
        }
    }

    #[test]
    fn parses_and() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 AND b = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(Box::new(eq("a", 1)), Box::new(eq("b", 2))))
        );
    }

    #[test]
    fn parses_or() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 OR b = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Or(Box::new(eq("a", 1)), Box::new(eq("b", 2))))
        );
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // a=1 OR (b=2 AND c=3), not (a=1 OR b=2) AND c=3.
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 OR b = 2 AND c = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Or(
                Box::new(eq("a", 1)),
                Box::new(Expr::And(Box::new(eq("b", 2)), Box::new(eq("c", 3)))),
            ))
        );
    }

    #[test]
    fn parses_not() {
        let tokens = tokenize("SELECT * FROM t WHERE NOT a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.filter, Some(Expr::Not(Box::new(eq("a", 1)))));
    }

    #[test]
    fn parses_double_not() {
        let tokens = tokenize("SELECT * FROM t WHERE NOT NOT a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Not(Box::new(Expr::Not(Box::new(eq("a", 1))))))
        );
    }

    #[test]
    fn parses_parenthesized_grouping() {
        // (a=1 OR b=2) AND c=3 -- parens override AND-before-OR precedence.
        let tokens = tokenize("SELECT * FROM t WHERE (a = 1 OR b = 2) AND c = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::Or(Box::new(eq("a", 1)), Box::new(eq("b", 2)))),
                Box::new(eq("c", 3)),
            ))
        );
    }

    #[test]
    fn parses_nested_parens() {
        let tokens = tokenize("SELECT * FROM t WHERE ((a = 1))").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.filter, Some(eq("a", 1)));
    }

    #[test]
    fn missing_closing_paren_is_an_error() {
        let tokens = tokenize("SELECT * FROM t WHERE (a = 1").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedEof)
        ));
    }

    #[test]
    fn parses_like() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: false,
            })
        );
    }

    #[test]
    fn parses_like_with_escape() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a\\%' ESCAPE '\\'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a\\%".into()))),
                escape: Some(Box::new(Expr::Literal(Value::Text("\\".into())))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_like() {
        let tokens = tokenize("SELECT * FROM t WHERE name NOT LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: true,
            })
        );
    }

    #[test]
    fn parses_glob() {
        let tokens = tokenize("SELECT * FROM t WHERE name GLOB 'a*'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Glob {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a*".into()))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_glob() {
        let tokens = tokenize("SELECT * FROM t WHERE name NOT GLOB 'a*'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Glob {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a*".into()))),
                negate: true,
            })
        );
    }

    #[test]
    fn parses_between() {
        let tokens = tokenize("SELECT * FROM t WHERE a BETWEEN 1 AND 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Between {
                expr: Box::new(Expr::Column("a".into())),
                low: Box::new(Expr::Literal(Value::Integer(1))),
                high: Box::new(Expr::Literal(Value::Integer(10))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_between() {
        let tokens = tokenize("SELECT * FROM t WHERE a NOT BETWEEN 1 AND 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Between {
                expr: Box::new(Expr::Column("a".into())),
                low: Box::new(Expr::Literal(Value::Integer(1))),
                high: Box::new(Expr::Literal(Value::Integer(10))),
                negate: true,
            })
        );
    }

    #[test]
    fn like_combines_with_and() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a%' AND b = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::Like {
                    left: Box::new(Expr::Column("name".into())),
                    pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                    escape: None,
                    negate: false,
                }),
                Box::new(eq("b", 1)),
            ))
        );
    }

    #[test]
    fn prefix_not_still_wraps_a_like_comparison() {
        // `WHERE NOT name LIKE 'a%'` -- prefix NOT (boolean-level),
        // distinct from `NOT LIKE` (infix, per-operator negation).
        let tokens = tokenize("SELECT * FROM t WHERE NOT name LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Not(Box::new(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: false,
            })))
        );
    }

    #[test]
    fn parses_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN (1, 2, 3)").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![
                    Expr::Literal(Value::Integer(1)),
                    Expr::Literal(Value::Integer(2)),
                    Expr::Literal(Value::Integer(3)),
                ],
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a NOT IN (1, 2)").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![
                    Expr::Literal(Value::Integer(1)),
                    Expr::Literal(Value::Integer(2)),
                ],
                negate: true,
            })
        );
    }

    #[test]
    fn parses_empty_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN ()").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![],
                negate: false,
            })
        );
    }

    #[test]
    fn in_list_combines_with_and() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN (1, 2) AND b = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::InList {
                    expr: Box::new(Expr::Column("a".into())),
                    list: vec![
                        Expr::Literal(Value::Integer(1)),
                        Expr::Literal(Value::Integer(2)),
                    ],
                    negate: false,
                }),
                Box::new(eq("b", 3)),
            ))
        );
    }

    #[test]
    fn parses_searched_case_in_where() {
        let tokens =
            tokenize("SELECT * FROM t WHERE CASE WHEN a = 1 THEN 1 ELSE 0 END = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Case {
                    operand: None,
                    branches: vec![(eq("a", 1), Expr::Literal(Value::Integer(1)))],
                    else_result: Some(Box::new(Expr::Literal(Value::Integer(0)))),
                }),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_searched_case_with_multiple_when_branches() {
        let tokens = tokenize(
            "SELECT * FROM t WHERE CASE WHEN a = 1 THEN 10 WHEN a = 2 THEN 20 ELSE 0 END = 10",
        )
        .unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        let Expr::Case { branches, .. } = *left else {
            panic!("expected Case");
        };
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn parses_simple_case() {
        let tokens =
            tokenize("SELECT * FROM t WHERE CASE a WHEN 1 THEN 10 ELSE 0 END = 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Case {
                    operand: Some(Box::new(Expr::Column("a".into()))),
                    branches: vec![(
                        Expr::Literal(Value::Integer(1)),
                        Expr::Literal(Value::Integer(10))
                    )],
                    else_result: Some(Box::new(Expr::Literal(Value::Integer(0)))),
                }),
                right: Box::new(Expr::Literal(Value::Integer(10))),
            })
        );
    }

    #[test]
    fn parses_case_with_no_else() {
        let tokens = tokenize("SELECT * FROM t WHERE CASE WHEN a = 1 THEN 1 END = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        let Expr::Case { else_result, .. } = *left else {
            panic!("expected Case");
        };
        assert_eq!(else_result, None);
    }

    #[test]
    fn case_with_no_when_branches_is_an_error() {
        let tokens = tokenize("SELECT * FROM t WHERE CASE ELSE 1 END = 1").unwrap();
        assert!(parse_select(&tokens).is_err());
    }

    #[test]
    fn select_without_group_by_or_having_has_neither() {
        let tokens = tokenize("SELECT COUNT(*) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.group_by.is_empty());
        assert_eq!(select.having, None);
    }

    #[test]
    fn parses_group_by_single_column() {
        let tokens = tokenize("SELECT COUNT(*) FROM t GROUP BY category").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.group_by, vec!["category".to_string()]);
    }

    #[test]
    fn parses_group_by_multiple_columns() {
        let tokens = tokenize("SELECT COUNT(*) FROM t GROUP BY a, b").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.group_by, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn parses_having_as_a_plain_comparison() {
        let tokens = tokenize("SELECT COUNT(*) FROM t GROUP BY a HAVING a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.having,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Column("a".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_having_referencing_an_aggregate() {
        let tokens = tokenize("SELECT COUNT(*) FROM t GROUP BY a HAVING COUNT(*) > 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.having,
            Some(Expr::BinaryOp {
                op: BinaryOp::Gt,
                left: Box::new(Expr::Column("COUNT(*)".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_having_referencing_a_non_star_aggregate() {
        let tokens =
            tokenize("SELECT SUM(amount) FROM t GROUP BY a HAVING SUM(amount) > 100").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.having,
            Some(Expr::BinaryOp {
                op: BinaryOp::Gt,
                left: Box::new(Expr::Column("SUM(amount)".into())),
                right: Box::new(Expr::Literal(Value::Integer(100))),
            })
        );
    }

    #[test]
    fn having_without_group_by_is_allowed() {
        // Real SQLite allows this too -- it just filters the single
        // whole-table aggregate row.
        let tokens = tokenize("SELECT COUNT(*) FROM t HAVING COUNT(*) > 0").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.group_by.is_empty());
        assert!(select.having.is_some());
    }

    #[test]
    fn aggregate_reference_syntax_is_scoped_to_having_only() {
        // The same "IDENT(...)" shape in WHERE is a plain scalar
        // function call (Expr::FunctionCall), not an aggregate
        // reference -- the HAVING-only parsing extension doesn't leak
        // into WHERE.
        let tokens = tokenize("SELECT * FROM t WHERE UPPER(a) = 'X'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::FunctionCall {
                    name: "UPPER".into(),
                    args: vec![Expr::Column("a".into())],
                }),
                right: Box::new(Expr::Literal(Value::Text("X".into()))),
            })
        );
    }

    #[test]
    fn plain_select_is_a_compound_with_an_empty_rest() {
        let tokens = tokenize("SELECT * FROM t").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.first.table_name, "t");
        assert!(compound.rest.is_empty());
    }

    #[test]
    fn parses_union() {
        let tokens = tokenize("SELECT a FROM t UNION SELECT a FROM u").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.rest.len(), 1);
        assert_eq!(compound.rest[0].0, CompoundOp::Union);
        assert_eq!(compound.rest[0].1.table_name, "u");
    }

    #[test]
    fn parses_union_all() {
        let tokens = tokenize("SELECT a FROM t UNION ALL SELECT a FROM u").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.rest[0].0, CompoundOp::UnionAll);
    }

    #[test]
    fn parses_intersect() {
        let tokens = tokenize("SELECT a FROM t INTERSECT SELECT a FROM u").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.rest[0].0, CompoundOp::Intersect);
    }

    #[test]
    fn parses_except() {
        let tokens = tokenize("SELECT a FROM t EXCEPT SELECT a FROM u").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.rest[0].0, CompoundOp::Except);
    }

    #[test]
    fn parses_a_chain_of_three_selects() {
        let tokens =
            tokenize("SELECT a FROM t UNION SELECT a FROM u INTERSECT SELECT a FROM v").unwrap();
        let compound = parse_compound_select(&tokens).unwrap();
        assert_eq!(compound.rest.len(), 2);
        assert_eq!(compound.rest[0].0, CompoundOp::Union);
        assert_eq!(compound.rest[0].1.table_name, "u");
        assert_eq!(compound.rest[1].0, CompoundOp::Intersect);
        assert_eq!(compound.rest[1].1.table_name, "v");
    }

    #[test]
    fn plain_parse_select_leaves_a_trailing_union_unconsumed() {
        // Documents parse_select's own lenient trailing-token behavior
        // (see its doc comment) -- parse_compound_select is what actual
        // statement dispatch uses instead, precisely so this doesn't
        // silently drop the UNION clause.
        let tokens = tokenize("SELECT a FROM t UNION SELECT a FROM u").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.table_name, "t");
    }

    #[test]
    fn parses_a_single_cte() {
        let tokens = tokenize("WITH cte AS (SELECT a FROM t) SELECT a FROM cte").unwrap();
        let with_select = parse_with_select(&tokens).unwrap();
        assert_eq!(with_select.ctes.len(), 1);
        assert_eq!(with_select.ctes[0].name, "cte");
        assert_eq!(with_select.ctes[0].select.first.table_name, "t");
        assert_eq!(with_select.body.first.table_name, "cte");
    }

    #[test]
    fn parses_multiple_ctes_in_one_with_clause() {
        let tokens =
            tokenize("WITH a AS (SELECT x FROM t1), b AS (SELECT x FROM t2) SELECT x FROM a")
                .unwrap();
        let with_select = parse_with_select(&tokens).unwrap();
        assert_eq!(with_select.ctes.len(), 2);
        assert_eq!(with_select.ctes[0].name, "a");
        assert_eq!(with_select.ctes[0].select.first.table_name, "t1");
        assert_eq!(with_select.ctes[1].name, "b");
        assert_eq!(with_select.ctes[1].select.first.table_name, "t2");
    }

    #[test]
    fn a_cte_body_may_itself_be_a_compound_select() {
        let tokens =
            tokenize("WITH cte AS (SELECT a FROM t UNION SELECT a FROM u) SELECT a FROM cte")
                .unwrap();
        let with_select = parse_with_select(&tokens).unwrap();
        assert_eq!(with_select.ctes[0].select.rest.len(), 1);
        assert_eq!(with_select.ctes[0].select.rest[0].0, CompoundOp::Union);
    }

    #[test]
    fn a_later_cte_can_reference_an_earlier_one_by_name() {
        let tokens =
            tokenize("WITH a AS (SELECT x FROM t), b AS (SELECT x FROM a) SELECT x FROM b")
                .unwrap();
        let with_select = parse_with_select(&tokens).unwrap();
        assert_eq!(with_select.ctes[1].select.first.table_name, "a");
    }

    #[test]
    fn plain_select_has_no_alias_or_joins() {
        let tokens = tokenize("SELECT * FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.table_alias, None);
        assert!(select.joins.is_empty());
    }

    #[test]
    fn parses_from_table_alias_with_and_without_as() {
        let tokens = tokenize("SELECT * FROM t AS x").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.table_alias, Some("x".to_string()));

        let tokens = tokenize("SELECT * FROM t x").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.table_alias, Some("x".to_string()));
    }

    #[test]
    fn bare_join_is_inner_join() {
        let tokens = tokenize("SELECT * FROM t1 JOIN t2 ON t1.a = t2.a").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins.len(), 1);
        assert_eq!(select.joins[0].kind, JoinKind::Inner);
        assert_eq!(select.joins[0].table.name, "t2");
        assert_eq!(select.joins[0].table.alias, None);
        assert_eq!(
            select.joins[0].condition,
            JoinCondition::On(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Column("t1.a".to_string())),
                right: Box::new(Expr::Column("t2.a".to_string())),
            })
        );
    }

    #[test]
    fn parses_inner_join_with_alias() {
        let tokens = tokenize("SELECT * FROM t1 INNER JOIN t2 AS b ON t1.a = b.a").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins[0].kind, JoinKind::Inner);
        assert_eq!(select.joins[0].table.alias, Some("b".to_string()));
    }

    #[test]
    fn parses_left_join_with_optional_outer_keyword() {
        let tokens = tokenize("SELECT * FROM t1 LEFT JOIN t2 ON t1.a = t2.a").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins[0].kind, JoinKind::Left);

        let tokens = tokenize("SELECT * FROM t1 LEFT OUTER JOIN t2 ON t1.a = t2.a").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins[0].kind, JoinKind::Left);
    }

    #[test]
    fn parses_cross_join_with_no_condition() {
        let tokens = tokenize("SELECT * FROM t1 CROSS JOIN t2").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins[0].kind, JoinKind::Cross);
        assert_eq!(select.joins[0].condition, JoinCondition::None);
    }

    #[test]
    fn parses_join_using_clause() {
        let tokens = tokenize("SELECT * FROM t1 JOIN t2 USING (a, b)").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.joins[0].condition,
            JoinCondition::Using(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn parses_a_chain_of_two_joins() {
        let tokens =
            tokenize("SELECT * FROM t1 JOIN t2 ON t1.a = t2.a LEFT JOIN t3 ON t2.b = t3.b")
                .unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.joins.len(), 2);
        assert_eq!(select.joins[0].kind, JoinKind::Inner);
        assert_eq!(select.joins[0].table.name, "t2");
        assert_eq!(select.joins[1].kind, JoinKind::Left);
        assert_eq!(select.joins[1].table.name, "t3");
    }

    #[test]
    fn inner_or_left_join_without_on_or_using_is_an_error() {
        let tokens = tokenize("SELECT * FROM t1 JOIN t2").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn qualified_column_parses_as_a_single_dotted_expr_column() {
        let tokens = tokenize("SELECT t1.a FROM t1 JOIN t2 ON t1.id = t2.id").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["t1.a".to_string()])
        );
    }

    #[test]
    fn qualified_column_usable_in_where() {
        let tokens = tokenize("SELECT * FROM t1 JOIN t2 ON t1.id = t2.id WHERE t2.b = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Column("t2.b".to_string())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }
}
