//! Aggregate SQL functions (Part B gap row "Connection + functions module:
//! aggregate/window functions + collations" — the aggregate slice; window
//! functions and collations are covered by [`crate::connection`] and this
//! issue's tracking comment, not here).
//!
//! **Design deviation, stated plainly:** unlike `rusqlite::functions::Aggregate<A,
//! T>` (a generic trait with an associated `Aggregator` state type,
//! designed to be implemented once per aggregate and driven by SQLite's C
//! step/finalize callbacks), [`Aggregate`] here is a plain value-holding
//! struct: a starting accumulator [`crate::Value`], a `step` closure that
//! folds one row's argument into the accumulator, and a `finalize`
//! closure that turns the accumulator into the result. Simpler, but it
//! constrains accumulator state to whatever fits in a single `Value` —
//! fine for `COUNT`/`SUM`/`MIN`/`MAX`, not expressive enough for
//! something like `AVG` (needs a running sum *and* count) or
//! `GROUP_CONCAT`'s separator handling, which aren't provided as
//! built-ins here.
//!
//! Also unlike real SQLite, whole-table aggregation is all that's
//! supported — there's no `GROUP BY` (see [`crate::dml_select::SelectColumns::Aggregates`]),
//! so an aggregate `SELECT` always produces exactly one output row.

use crate::error::Result;
use crate::eval::compare_values;
use crate::value::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

type StepFn = dyn Fn(&Value, &[Value]) -> Result<Value>;
type FinalizeFn = dyn Fn(Value) -> Result<Value>;

/// A registered aggregate SQL function, usable in an aggregate select list
/// (e.g. `SELECT COUNT(*) FROM t`) via [`crate::Connection::create_aggregate_function`].
pub struct Aggregate {
    pub(crate) init: Value,
    pub(crate) step: Box<StepFn>,
    pub(crate) finalize: Box<FinalizeFn>,
}

impl Aggregate {
    /// Builds a custom aggregate from a starting accumulator, a `step`
    /// closure (accumulator, this row's evaluated argument) → new
    /// accumulator, and a `finalize` closure that converts the final
    /// accumulator into the result value.
    pub fn new<S, F>(init: Value, step: S, finalize: F) -> Aggregate
    where
        S: Fn(&Value, &[Value]) -> Result<Value> + 'static,
        F: Fn(Value) -> Result<Value> + 'static,
    {
        Aggregate {
            init,
            step: Box::new(step),
            finalize: Box::new(finalize),
        }
    }

    /// Like [`Aggregate::new`], but `finalize` is the identity function —
    /// for aggregates (like `SUM`/`MIN`/`MAX`) where the accumulator
    /// itself is already the result.
    pub fn simple<S>(init: Value, step: S) -> Aggregate
    where
        S: Fn(&Value, &[Value]) -> Result<Value> + 'static,
    {
        Aggregate::new(init, step, Ok)
    }

    fn count() -> Aggregate {
        Aggregate::simple(Value::Integer(0), |acc, args| {
            let n = match acc {
                Value::Integer(n) => *n,
                _ => 0,
            };
            let skip_null = matches!(args.first(), Some(Value::Null));
            Ok(Value::Integer(if skip_null { n } else { n + 1 }))
        })
    }

    fn sum() -> Aggregate {
        Aggregate::simple(Value::Null, |acc, args| {
            let Some(value) = args.first() else {
                return Ok(acc.clone());
            };
            if matches!(value, Value::Null) {
                return Ok(acc.clone());
            }
            Ok(match (acc, value) {
                (Value::Null, v) => v.clone(),
                (Value::Integer(a), Value::Integer(b)) => Value::Integer(a + b),
                (Value::Integer(a), Value::Real(b)) => Value::Real(*a as f64 + b),
                (Value::Real(a), Value::Integer(b)) => Value::Real(a + *b as f64),
                (Value::Real(a), Value::Real(b)) => Value::Real(a + b),
                (other, _) => other.clone(),
            })
        })
    }

    fn min() -> Aggregate {
        Aggregate::simple(Value::Null, |acc, args| extreme(acc, args, Ordering::Less))
    }

    fn max() -> Aggregate {
        Aggregate::simple(Value::Null, |acc, args| {
            extreme(acc, args, Ordering::Greater)
        })
    }
}

/// Shared step logic for `MIN`/`MAX`: keeps `acc` unless `args[0]` is
/// non-null and compares as `keep_when` relative to `acc`.
fn extreme(acc: &Value, args: &[Value], keep_when: Ordering) -> Result<Value> {
    let Some(value) = args.first() else {
        return Ok(acc.clone());
    };
    if matches!(value, Value::Null) {
        return Ok(acc.clone());
    }
    Ok(match acc {
        Value::Null => value.clone(),
        _ if compare_values(value, acc) == keep_when => value.clone(),
        _ => acc.clone(),
    })
}

/// The aggregates every [`crate::Connection`] starts with — real SQLite's
/// `COUNT`/`SUM`/`MIN`/`MAX` are built into the engine core rather than
/// registered like a user aggregate, so this crate seeds the same names
/// by default rather than requiring `create_aggregate_function` calls
/// just to run `SELECT COUNT(*) FROM t`.
pub(crate) fn builtins() -> HashMap<String, Aggregate> {
    let mut m = HashMap::new();
    m.insert("COUNT".to_string(), Aggregate::count());
    m.insert("SUM".to_string(), Aggregate::sum());
    m.insert("MIN".to_string(), Aggregate::min());
    m.insert("MAX".to_string(), Aggregate::max());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_skips_null_but_not_star_placeholder() {
        let count = Aggregate::count();
        let mut acc = count.init.clone();
        for args in [
            vec![Value::Integer(1)],
            vec![Value::Null],
            vec![Value::Integer(1)],
        ] {
            acc = (count.step)(&acc, &args).unwrap();
        }
        assert_eq!(acc, Value::Integer(2));
    }

    #[test]
    fn sum_ignores_nulls_and_promotes_to_real() {
        let sum = Aggregate::sum();
        let mut acc = sum.init.clone();
        for args in [
            vec![Value::Integer(1)],
            vec![Value::Null],
            vec![Value::Real(1.5)],
        ] {
            acc = (sum.step)(&acc, &args).unwrap();
        }
        assert_eq!(acc, Value::Real(2.5));
    }

    #[test]
    fn sum_over_no_rows_is_null() {
        let sum = Aggregate::sum();
        assert_eq!(sum.init, Value::Null);
    }

    #[test]
    fn min_and_max_track_extremes_and_skip_nulls() {
        let min = Aggregate::min();
        let max = Aggregate::max();
        let mut min_acc = min.init.clone();
        let mut max_acc = max.init.clone();
        for args in [
            vec![Value::Integer(5)],
            vec![Value::Null],
            vec![Value::Integer(1)],
            vec![Value::Integer(9)],
        ] {
            min_acc = (min.step)(&min_acc, &args).unwrap();
            max_acc = (max.step)(&max_acc, &args).unwrap();
        }
        assert_eq!(min_acc, Value::Integer(1));
        assert_eq!(max_acc, Value::Integer(9));
    }

    #[test]
    fn custom_aggregate_with_finalize() {
        let avg_like = Aggregate::new(
            Value::Integer(0),
            |acc, args| match (acc, args.first()) {
                (Value::Integer(n), Some(Value::Integer(v))) => Ok(Value::Integer(n + v)),
                (acc, _) => Ok(acc.clone()),
            },
            |acc| match acc {
                Value::Integer(n) => Ok(Value::Real(n as f64 / 2.0)),
                other => Ok(other),
            },
        );
        let mut acc = avg_like.init.clone();
        acc = (avg_like.step)(&acc, &[Value::Integer(4)]).unwrap();
        acc = (avg_like.step)(&acc, &[Value::Integer(6)]).unwrap();
        assert_eq!((avg_like.finalize)(acc).unwrap(), Value::Real(5.0));
    }
}
