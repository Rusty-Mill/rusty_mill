//! Expression evaluator (foundation-tier `A6`). Evaluates the [`Expr`]
//! tree the `SELECT` parser produces against a single row, without
//! knowledge of scanning or storage — those are `A7`.

use crate::dml_select::{BinaryOp, Expr};
use crate::error::{Error, Result};
use crate::value::Value;
use std::cmp::Ordering;

/// Evaluates an expression against one row, given the row's column names
/// in the same order as its values.
pub fn evaluate(expr: &Expr, column_names: &[String], row: &[Value]) -> Result<Value> {
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
            let l = evaluate(left, column_names, row)?;
            let r = evaluate(right, column_names, row)?;
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
    }
}

/// Evaluates an expression as a boolean filter (SQLite's convention: any
/// non-zero, non-null value is true).
pub fn evaluate_bool(expr: &Expr, column_names: &[String], row: &[Value]) -> Result<bool> {
    Ok(match evaluate(expr, column_names, row)? {
        Value::Null => false,
        Value::Integer(n) => n != 0,
        Value::Real(f) => f != 0.0,
        Value::Text(s) => !s.is_empty(),
        Value::Blob(b) => !b.is_empty(),
    })
}

/// Orders two values per SQLite's storage-class ordering (`NULL` <
/// `INTEGER`/`REAL` < `TEXT` < `BLOB`), comparing within a class when both
/// sides share it. See <https://www.sqlite.org/datatype3.html#sort_order>.
fn compare_values(a: &Value, b: &Value) -> Ordering {
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
}
