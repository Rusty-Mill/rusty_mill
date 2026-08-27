//! `ToSql`: converts a Rust value into a [`Value`] for binding into a
//! statement parameter. Part B gap row "types module: ToSql trait +
//! blanket impls".
//!
//! Unlike `rusqlite::ToSql`, this trait's `to_sql` isn't fallible — every
//! impl here (primitives, `String`, `Vec<u8>`, `Option<T>`) has no failure
//! case, so wrapping the return in `Result` would be error handling for a
//! scenario that can't happen. A future impl that genuinely can fail (e.g.
//! a timestamp type with a representable range) can introduce a fallible
//! variant then, without forcing every existing impl to pretend it can
//! fail today.

use crate::value::Value;

/// Converts `self` into a [`Value`] for binding into a statement
/// parameter.
pub trait ToSql {
    fn to_sql(&self) -> Value;
}

impl ToSql for Value {
    fn to_sql(&self) -> Value {
        self.clone()
    }
}

impl ToSql for i64 {
    fn to_sql(&self) -> Value {
        Value::Integer(*self)
    }
}

impl ToSql for i32 {
    fn to_sql(&self) -> Value {
        Value::Integer(*self as i64)
    }
}

impl ToSql for f64 {
    fn to_sql(&self) -> Value {
        Value::Real(*self)
    }
}

impl ToSql for bool {
    fn to_sql(&self) -> Value {
        Value::Integer(*self as i64)
    }
}

impl ToSql for String {
    fn to_sql(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl ToSql for str {
    fn to_sql(&self) -> Value {
        Value::Text(self.to_string())
    }
}

impl ToSql for &str {
    /// A separate impl from [`ToSql for str`] (the unsized one), rather
    /// than relying on it alone: a bound like `T: ToSql` (used by e.g.
    /// [`crate::Statement::raw_bind_parameter`]) implicitly requires
    /// `T: Sized`, which only `&str` — not `str` itself — satisfies. This
    /// is what lets `stmt.raw_bind_parameter(1, "hi")` work directly.
    fn to_sql(&self) -> Value {
        Value::Text(self.to_string())
    }
}

impl ToSql for Vec<u8> {
    fn to_sql(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_sql(&self) -> Value {
        match self {
            Some(v) => v.to_sql(),
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_primitives() {
        assert_eq!(42i64.to_sql(), Value::Integer(42));
        assert_eq!(7i32.to_sql(), Value::Integer(7));
        assert_eq!(1.5f64.to_sql(), Value::Real(1.5));
        assert_eq!(true.to_sql(), Value::Integer(1));
        assert_eq!(false.to_sql(), Value::Integer(0));
    }

    #[test]
    fn converts_string_and_bytes() {
        assert_eq!("hi".to_sql(), Value::Text("hi".into()));
        assert_eq!(String::from("hi").to_sql(), Value::Text("hi".into()));
        assert_eq!(vec![1u8, 2, 3].to_sql(), Value::Blob(vec![1, 2, 3]));
    }

    #[test]
    fn converts_option() {
        assert_eq!(Some(5i64).to_sql(), Value::Integer(5));
        assert_eq!(None::<i64>.to_sql(), Value::Null);
    }
}
