//! Expression evaluator (foundation-tier `A6`). Evaluates the [`Expr`]
//! tree the `SELECT` parser produces against a single row, without
//! knowledge of scanning or storage — those are `A7`.
//!
//! Also home to scalar-function-call evaluation (Part B gap row
//! "Connection + functions module: scalar SQL functions") via
//! [`evaluate_with_functions`], added without changing [`evaluate`]'s
//! signature — [`evaluate`] is now defined in terms of it (an empty
//! registry), so both stay in sync from one implementation.

use crate::dml_select::{BinaryOp, Expr};
use crate::error::{Error, Result};
use crate::value::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

/// A registered scalar SQL function: takes evaluated argument values,
/// returns the function's result.
pub type ScalarFn = dyn Fn(&[Value]) -> Result<Value>;

/// Evaluates an expression against one row, given the row's column names
/// in the same order as its values. Errors on [`Expr::FunctionCall`] —
/// use [`evaluate_with_functions`] for expressions that may contain one.
pub fn evaluate(expr: &Expr, column_names: &[String], row: &[Value]) -> Result<Value> {
    evaluate_with_functions(expr, column_names, row, &HashMap::new())
}

/// Like [`evaluate`], but resolves [`Expr::FunctionCall`] against
/// `functions` (name → implementation).
pub fn evaluate_with_functions(
    expr: &Expr,
    column_names: &[String],
    row: &[Value],
    functions: &HashMap<String, Box<ScalarFn>>,
) -> Result<Value> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Column(name) => {
            let idx = column_names
                .iter()
                .position(|c| c == name)
                .ok_or_else(|| Error::UnknownColumn(name.clone()))?;
            Ok(row[idx].clone())
        }
        Expr::BinaryOp { op, left, right } => {
            let l = evaluate_with_functions(left, column_names, row, functions)?;
            let r = evaluate_with_functions(right, column_names, row, functions)?;
            let ord = compare_values(&l, &r);
            let result = match op {
                BinaryOp::Eq => ord == Ordering::Equal,
                BinaryOp::NotEq => ord != Ordering::Equal,
                BinaryOp::Lt => ord == Ordering::Less,
                BinaryOp::LtEq => ord != Ordering::Greater,
                BinaryOp::Gt => ord == Ordering::Greater,
                BinaryOp::GtEq => ord != Ordering::Less,
            };
            Ok(Value::Integer(result as i64))
        }
        Expr::FunctionCall { name, args } => {
            let f = functions
                .get(name)
                .ok_or_else(|| Error::FunctionNotFound(name.clone()))?;
            let arg_values = args
                .iter()
                .map(|a| evaluate_with_functions(a, column_names, row, functions))
                .collect::<Result<Vec<Value>>>()?;
            f(&arg_values)
        }
        Expr::And(left, right) => {
            let l = to_bool3(evaluate_with_functions(left, column_names, row, functions)?)?;
            let r = to_bool3(evaluate_with_functions(
                right,
                column_names,
                row,
                functions,
            )?)?;
            let result = match (l, r) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            };
            Ok(bool3_to_value(result))
        }
        Expr::Or(left, right) => {
            let l = to_bool3(evaluate_with_functions(left, column_names, row, functions)?)?;
            let r = to_bool3(evaluate_with_functions(
                right,
                column_names,
                row,
                functions,
            )?)?;
            let result = match (l, r) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            };
            Ok(bool3_to_value(result))
        }
        Expr::Not(inner) => {
            let v = to_bool3(evaluate_with_functions(
                inner,
                column_names,
                row,
                functions,
            )?)?;
            Ok(bool3_to_value(v.map(|b| !b)))
        }
        Expr::Like {
            left,
            pattern,
            escape,
            negate,
        } => {
            let l = evaluate_with_functions(left, column_names, row, functions)?;
            let p = evaluate_with_functions(pattern, column_names, row, functions)?;
            let esc = match escape {
                Some(e) => match evaluate_with_functions(e, column_names, row, functions)? {
                    Value::Null => return Ok(Value::Null),
                    other => Some(value_as_text(&other)?.chars().next().ok_or_else(|| {
                        Error::UnrecognizedStatement("ESCAPE must be one character".to_string())
                    })?),
                },
                None => None,
            };
            Ok(match (value_as_text_opt(&l), value_as_text_opt(&p)) {
                (Some(text), Some(pat)) => {
                    let matched = crate::like::like_match(&text, &pat, esc);
                    bool3_to_value(Some(matched != *negate))
                }
                _ => Value::Null,
            })
        }
        Expr::Glob {
            left,
            pattern,
            negate,
        } => {
            let l = evaluate_with_functions(left, column_names, row, functions)?;
            let p = evaluate_with_functions(pattern, column_names, row, functions)?;
            Ok(match (value_as_text_opt(&l), value_as_text_opt(&p)) {
                (Some(text), Some(pat)) => {
                    let matched = crate::like::glob_match(&text, &pat);
                    bool3_to_value(Some(matched != *negate))
                }
                _ => Value::Null,
            })
        }
        Expr::Between {
            expr,
            low,
            high,
            negate,
        } => {
            let v = evaluate_with_functions(expr, column_names, row, functions)?;
            let lo = evaluate_with_functions(low, column_names, row, functions)?;
            let hi = evaluate_with_functions(high, column_names, row, functions)?;
            if v == Value::Null || lo == Value::Null || hi == Value::Null {
                return Ok(Value::Null);
            }
            let matched = compare_values(&v, &lo) != Ordering::Less
                && compare_values(&v, &hi) != Ordering::Greater;
            Ok(bool3_to_value(Some(matched != *negate)))
        }
        Expr::InList { expr, list, negate } => {
            let x = evaluate_with_functions(expr, column_names, row, functions)?;
            if x == Value::Null {
                return Ok(Value::Null);
            }
            let mut found = false;
            let mut saw_null = false;
            for item in list {
                let v = evaluate_with_functions(item, column_names, row, functions)?;
                if v == Value::Null {
                    saw_null = true;
                    continue;
                }
                if compare_values(&x, &v) == Ordering::Equal {
                    found = true;
                    break;
                }
            }
            let result = if found {
                Some(true)
            } else if saw_null {
                None
            } else {
                Some(false)
            };
            Ok(bool3_to_value(result.map(|b| b != *negate)))
        }
        Expr::Case {
            operand,
            branches,
            else_result,
        } => {
            let operand_value = match operand {
                Some(o) => Some(evaluate_with_functions(o, column_names, row, functions)?),
                None => None,
            };
            for (cond, result) in branches {
                let matched = match &operand_value {
                    Some(ov) => {
                        let cv = evaluate_with_functions(cond, column_names, row, functions)?;
                        // Simple-form matching is `=` comparison; a NULL
                        // operand or NULL WHEN value never matches (same
                        // "unknown isn't true" rule as every other
                        // comparison here), falling through to ELSE.
                        *ov != Value::Null
                            && cv != Value::Null
                            && compare_values(ov, &cv) == Ordering::Equal
                    }
                    None => {
                        let cv = evaluate_with_functions(cond, column_names, row, functions)?;
                        to_bool3(cv)?.unwrap_or(false)
                    }
                };
                if matched {
                    return evaluate_with_functions(result, column_names, row, functions);
                }
            }
            match else_result {
                Some(e) => evaluate_with_functions(e, column_names, row, functions),
                None => Ok(Value::Null),
            }
        }
        // No bindings are available here — `crate::Statement` resolves
        // `Parameter` nodes to a concrete `Literal` (bound value, or
        // `Value::Null` if unbound) before this ever runs; a caller that
        // reaches this some other way (e.g. `Connection::query_map` given
        // SQL text with a literal `?`) gets the same `Value::Null` real
        // SQLite would give an unbound parameter.
        Expr::Parameter(_) => Ok(Value::Null),
    }
}

/// Evaluates an expression as a boolean filter (SQLite's convention: any
/// non-zero, non-null value is true).
pub fn evaluate_bool(expr: &Expr, column_names: &[String], row: &[Value]) -> Result<bool> {
    to_bool(evaluate(expr, column_names, row)?)
}

/// Like [`evaluate_bool`], but resolves function calls against
/// `functions` — see [`evaluate_with_functions`].
pub fn evaluate_bool_with_functions(
    expr: &Expr,
    column_names: &[String],
    row: &[Value],
    functions: &HashMap<String, Box<ScalarFn>>,
) -> Result<bool> {
    to_bool(evaluate_with_functions(expr, column_names, row, functions)?)
}

fn to_bool(value: Value) -> Result<bool> {
    Ok(to_bool3(value)?.unwrap_or(false))
}

/// Like [`to_bool`], but distinguishes `NULL` (`None`) from a `FALSE`-ish
/// value (`Some(false)`) instead of collapsing both to `false` — needed
/// so [`Expr::And`]/[`Expr::Or`]/[`Expr::Not`] (issue #112) can implement
/// SQLite's actual three-valued boolean logic rather than plain two-valued
/// `&&`/`||`/`!`.
fn to_bool3(value: Value) -> Result<Option<bool>> {
    Ok(match value {
        Value::Null => None,
        Value::Integer(n) => Some(n != 0),
        Value::Real(f) => Some(f != 0.0),
        Value::Text(s) => Some(!s.is_empty()),
        Value::Blob(b) => Some(!b.is_empty()),
    })
}

/// The inverse of [`to_bool3`]: SQLite represents boolean results as
/// `INTEGER` `0`/`1`, with `NULL` staying `NULL`.
fn bool3_to_value(value: Option<bool>) -> Value {
    value.map_or(Value::Null, |b| Value::Integer(b as i64))
}

/// Coerces `value` to text for [`Expr::Like`]/[`Expr::Glob`] matching,
/// the same numeric-to-text coercion SQLite applies to these operators'
/// operands. `None` for `NULL` (propagates as a `NULL` match result) and
/// `Blob` (has no textual reading — never matches, same as real SQLite).
fn value_as_text_opt(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Blob(_) => None,
        Value::Integer(n) => Some(n.to_string()),
        Value::Real(f) => Some(f.to_string()),
        Value::Text(s) => Some(s.clone()),
    }
}

fn value_as_text(value: &Value) -> Result<String> {
    value_as_text_opt(value)
        .ok_or_else(|| Error::UnrecognizedStatement(format!("expected text, got {value:?}")))
}

/// Orders two values per SQLite's storage-class ordering (`NULL` <
/// `INTEGER`/`REAL` < `TEXT` < `BLOB`), comparing within a class when both
/// sides share it. See <https://www.sqlite.org/datatype3.html#sort_order>.
pub(crate) fn compare_values(a: &Value, b: &Value) -> Ordering {
    fn class_rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Integer(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }

    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Integer(x), Value::Real(y)) => {
            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (Value::Real(x), Value::Integer(y)) => {
            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
        }
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => class_rank(a).cmp(&class_rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<String> {
        vec!["a".into(), "b".into()]
    }

    #[test]
    fn evaluates_literal() {
        let v = evaluate(&Expr::Literal(Value::Integer(5)), &cols(), &[]).unwrap();
        assert_eq!(v, Value::Integer(5));
    }

    #[test]
    fn evaluates_column_reference() {
        let row = vec![Value::Integer(1), Value::Text("x".into())];
        let v = evaluate(&Expr::Column("b".into()), &cols(), &row).unwrap();
        assert_eq!(v, Value::Text("x".into()));
    }

    #[test]
    fn unknown_column_is_an_error() {
        let row = vec![Value::Integer(1), Value::Text("x".into())];
        assert_eq!(
            evaluate(&Expr::Column("z".into()), &cols(), &row),
            Err(Error::UnknownColumn("z".into()))
        );
    }

    #[test]
    fn evaluates_equality() {
        let row = vec![Value::Integer(1), Value::Text("x".into())];
        let expr = Expr::BinaryOp {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Column("a".into())),
            right: Box::new(Expr::Literal(Value::Integer(1))),
        };
        assert!(evaluate_bool(&expr, &cols(), &row).unwrap());
    }

    #[test]
    fn evaluates_integer_real_comparison_across_variants() {
        let row = vec![Value::Integer(2), Value::Text("x".into())];
        let expr = Expr::BinaryOp {
            op: BinaryOp::Lt,
            left: Box::new(Expr::Column("a".into())),
            right: Box::new(Expr::Literal(Value::Real(2.5))),
        };
        assert!(evaluate_bool(&expr, &cols(), &row).unwrap());
    }

    #[test]
    fn null_is_falsy() {
        assert!(!evaluate_bool(&Expr::Literal(Value::Null), &cols(), &[]).unwrap());
    }

    fn lit(n: i64) -> Expr {
        Expr::Literal(Value::Integer(n))
    }

    fn null() -> Expr {
        Expr::Literal(Value::Null)
    }

    #[test]
    fn and_is_true_only_if_both_sides_are_true() {
        assert_eq!(
            evaluate(&Expr::And(Box::new(lit(1)), Box::new(lit(1))), &cols(), &[]).unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            evaluate(&Expr::And(Box::new(lit(1)), Box::new(lit(0))), &cols(), &[]).unwrap(),
            Value::Integer(0)
        );
    }

    #[test]
    fn or_is_true_if_either_side_is_true() {
        assert_eq!(
            evaluate(&Expr::Or(Box::new(lit(0)), Box::new(lit(1))), &cols(), &[]).unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            evaluate(&Expr::Or(Box::new(lit(0)), Box::new(lit(0))), &cols(), &[]).unwrap(),
            Value::Integer(0)
        );
    }

    #[test]
    fn not_inverts_a_truthy_value() {
        assert_eq!(
            evaluate(&Expr::Not(Box::new(lit(0))), &cols(), &[]).unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            evaluate(&Expr::Not(Box::new(lit(1))), &cols(), &[]).unwrap(),
            Value::Integer(0)
        );
    }

    #[test]
    fn not_null_is_null() {
        assert_eq!(
            evaluate(&Expr::Not(Box::new(null())), &cols(), &[]).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn false_and_null_is_false_not_null() {
        // SQLite's three-valued logic: FALSE wins over NULL in AND.
        assert_eq!(
            evaluate(&Expr::And(Box::new(lit(0)), Box::new(null())), &cols(), &[]).unwrap(),
            Value::Integer(0)
        );
    }

    #[test]
    fn true_and_null_is_null() {
        assert_eq!(
            evaluate(&Expr::And(Box::new(lit(1)), Box::new(null())), &cols(), &[]).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn true_or_null_is_true_not_null() {
        // SQLite's three-valued logic: TRUE wins over NULL in OR.
        assert_eq!(
            evaluate(&Expr::Or(Box::new(lit(1)), Box::new(null())), &cols(), &[]).unwrap(),
            Value::Integer(1)
        );
    }

    #[test]
    fn false_or_null_is_null() {
        assert_eq!(
            evaluate(&Expr::Or(Box::new(lit(0)), Box::new(null())), &cols(), &[]).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn and_or_not_as_a_where_filter_treat_null_as_non_matching() {
        // Consistent with evaluate_bool's existing "NULL is falsy" rule
        // for a top-level WHERE result.
        assert!(
            !evaluate_bool(&Expr::And(Box::new(lit(1)), Box::new(null())), &cols(), &[]).unwrap()
        );
        assert!(
            !evaluate_bool(&Expr::Or(Box::new(lit(0)), Box::new(null())), &cols(), &[]).unwrap()
        );
    }

    fn text(s: &str) -> Expr {
        Expr::Literal(Value::Text(s.to_string()))
    }

    #[test]
    fn like_matches_with_percent_wildcard() {
        let expr = Expr::Like {
            left: Box::new(text("hello world")),
            pattern: Box::new(text("hello%")),
            escape: None,
            negate: false,
        };
        assert!(evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn not_like_negates_the_match() {
        let expr = Expr::Like {
            left: Box::new(text("hello world")),
            pattern: Box::new(text("hello%")),
            escape: None,
            negate: true,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn like_with_null_operand_is_null() {
        let expr = Expr::Like {
            left: Box::new(null()),
            pattern: Box::new(text("a%")),
            escape: None,
            negate: false,
        };
        assert_eq!(evaluate(&expr, &cols(), &[]).unwrap(), Value::Null);
    }

    #[test]
    fn glob_matches_with_star_wildcard() {
        let expr = Expr::Glob {
            left: Box::new(text("hello.txt")),
            pattern: Box::new(text("*.txt")),
            negate: false,
        };
        assert!(evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn glob_is_case_sensitive_unlike_like() {
        let expr = Expr::Glob {
            left: Box::new(text("HELLO")),
            pattern: Box::new(text("hello")),
            negate: false,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn between_matches_inclusive_range() {
        let expr = Expr::Between {
            expr: Box::new(lit(5)),
            low: Box::new(lit(1)),
            high: Box::new(lit(10)),
            negate: false,
        };
        assert!(evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn between_boundaries_are_inclusive() {
        let at_low = Expr::Between {
            expr: Box::new(lit(1)),
            low: Box::new(lit(1)),
            high: Box::new(lit(10)),
            negate: false,
        };
        let at_high = Expr::Between {
            expr: Box::new(lit(10)),
            low: Box::new(lit(1)),
            high: Box::new(lit(10)),
            negate: false,
        };
        assert!(evaluate_bool(&at_low, &cols(), &[]).unwrap());
        assert!(evaluate_bool(&at_high, &cols(), &[]).unwrap());
    }

    #[test]
    fn not_between_negates_the_match() {
        let expr = Expr::Between {
            expr: Box::new(lit(5)),
            low: Box::new(lit(1)),
            high: Box::new(lit(10)),
            negate: true,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn between_with_null_operand_is_null() {
        let expr = Expr::Between {
            expr: Box::new(null()),
            low: Box::new(lit(1)),
            high: Box::new(lit(10)),
            negate: false,
        };
        assert_eq!(evaluate(&expr, &cols(), &[]).unwrap(), Value::Null);
    }

    #[test]
    fn in_list_matches_a_member() {
        let expr = Expr::InList {
            expr: Box::new(lit(2)),
            list: vec![lit(1), lit(2), lit(3)],
            negate: false,
        };
        assert!(evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn in_list_does_not_match_a_non_member() {
        let expr = Expr::InList {
            expr: Box::new(lit(5)),
            list: vec![lit(1), lit(2), lit(3)],
            negate: false,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn not_in_list_negates_the_match() {
        let expr = Expr::InList {
            expr: Box::new(lit(2)),
            list: vec![lit(1), lit(2), lit(3)],
            negate: true,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn null_in_list_is_null() {
        let expr = Expr::InList {
            expr: Box::new(null()),
            list: vec![lit(1), lit(2)],
            negate: false,
        };
        assert_eq!(evaluate(&expr, &cols(), &[]).unwrap(), Value::Null);
    }

    #[test]
    fn not_in_list_with_a_null_member_and_no_match_is_null() {
        // x NOT IN (1, NULL) is NULL, not TRUE, for any non-matching x --
        // real SQL's NULL-aware IN semantics: x could equal the unknown
        // NULL value, so "definitely not in the list" can't be asserted.
        let expr = Expr::InList {
            expr: Box::new(lit(5)),
            list: vec![lit(1), null()],
            negate: true,
        };
        assert_eq!(evaluate(&expr, &cols(), &[]).unwrap(), Value::Null);
    }

    #[test]
    fn in_list_with_a_null_member_and_a_real_match_is_true() {
        let expr = Expr::InList {
            expr: Box::new(lit(1)),
            list: vec![lit(1), null()],
            negate: false,
        };
        assert!(evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn empty_in_list_never_matches() {
        let expr = Expr::InList {
            expr: Box::new(lit(1)),
            list: vec![],
            negate: false,
        };
        assert!(!evaluate_bool(&expr, &cols(), &[]).unwrap());
    }

    #[test]
    fn searched_case_returns_the_first_matching_branch() {
        let expr = Expr::Case {
            operand: None,
            branches: vec![
                (
                    Expr::BinaryOp {
                        op: BinaryOp::Eq,
                        left: Box::new(lit(1)),
                        right: Box::new(lit(2)),
                    },
                    text("no"),
                ),
                (
                    Expr::BinaryOp {
                        op: BinaryOp::Eq,
                        left: Box::new(lit(1)),
                        right: Box::new(lit(1)),
                    },
                    text("yes"),
                ),
            ],
            else_result: Some(Box::new(text("else"))),
        };
        assert_eq!(
            evaluate(&expr, &cols(), &[]).unwrap(),
            Value::Text("yes".to_string())
        );
    }

    #[test]
    fn searched_case_falls_through_to_else_when_nothing_matches() {
        let expr = Expr::Case {
            operand: None,
            branches: vec![(
                Expr::BinaryOp {
                    op: BinaryOp::Eq,
                    left: Box::new(lit(1)),
                    right: Box::new(lit(2)),
                },
                text("no"),
            )],
            else_result: Some(Box::new(text("else"))),
        };
        assert_eq!(
            evaluate(&expr, &cols(), &[]).unwrap(),
            Value::Text("else".to_string())
        );
    }

    #[test]
    fn case_with_no_else_and_no_match_is_null() {
        let expr = Expr::Case {
            operand: None,
            branches: vec![(
                Expr::BinaryOp {
                    op: BinaryOp::Eq,
                    left: Box::new(lit(1)),
                    right: Box::new(lit(2)),
                },
                text("no"),
            )],
            else_result: None,
        };
        assert_eq!(evaluate(&expr, &cols(), &[]).unwrap(), Value::Null);
    }

    #[test]
    fn simple_case_matches_operand_by_equality() {
        let expr = Expr::Case {
            operand: Some(Box::new(lit(2))),
            branches: vec![(lit(1), text("one")), (lit(2), text("two"))],
            else_result: Some(Box::new(text("other"))),
        };
        assert_eq!(
            evaluate(&expr, &cols(), &[]).unwrap(),
            Value::Text("two".to_string())
        );
    }

    #[test]
    fn simple_case_with_null_operand_never_matches() {
        let expr = Expr::Case {
            operand: Some(Box::new(null())),
            branches: vec![(null(), text("matched-null"))],
            else_result: Some(Box::new(text("else"))),
        };
        assert_eq!(
            evaluate(&expr, &cols(), &[]).unwrap(),
            Value::Text("else".to_string())
        );
    }

    #[test]
    fn plain_evaluate_errors_on_function_call() {
        let expr = Expr::FunctionCall {
            name: "UPPER".into(),
            args: vec![],
        };
        assert_eq!(
            evaluate(&expr, &cols(), &[]),
            Err(Error::FunctionNotFound("UPPER".into()))
        );
    }

    #[test]
    fn evaluate_with_functions_calls_registered_function() {
        let mut functions: HashMap<String, Box<ScalarFn>> = HashMap::new();
        functions.insert(
            "DOUBLE".to_string(),
            Box::new(|args: &[Value]| match args {
                [Value::Integer(n)] => Ok(Value::Integer(n * 2)),
                _ => Err(Error::FunctionNotFound("DOUBLE".into())),
            }),
        );
        let expr = Expr::FunctionCall {
            name: "DOUBLE".into(),
            args: vec![Expr::Literal(Value::Integer(21))],
        };
        let result = evaluate_with_functions(&expr, &cols(), &[], &functions).unwrap();
        assert_eq!(result, Value::Integer(42));
    }

    #[test]
    fn evaluate_with_functions_errors_on_unregistered_function() {
        let functions: HashMap<String, Box<ScalarFn>> = HashMap::new();
        let expr = Expr::FunctionCall {
            name: "MISSING".into(),
            args: vec![],
        };
        assert_eq!(
            evaluate_with_functions(&expr, &cols(), &[], &functions),
            Err(Error::FunctionNotFound("MISSING".into()))
        );
    }
}
