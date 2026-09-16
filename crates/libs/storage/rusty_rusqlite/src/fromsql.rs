//! `FromSql`: converts a stored [`Value`] back into a Rust value. Part B
//! gap row "types module: FromSql trait + FromSqlError/FromSqlResult".

use crate::value::{Type, Value};

/// An error produced while converting a [`Value`] into a Rust type via
/// [`FromSql`].
#[derive(Debug, Clone, PartialEq)]
pub enum FromSqlError {
    /// The value's storage class doesn't match what the target type
    /// expects.
    InvalidType { expected: Type, actual: Type },
    /// The value's storage class matched, but the payload itself was out
    /// of range for the target type (e.g. an `Integer` too large for
    /// `i32`).
    OutOfRange,
}

pub type FromSqlResult<T> = Result<T, FromSqlError>;

/// Converts a stored [`Value`] into `Self`.
pub trait FromSql: Sized {
    fn column_result(value: &Value) -> FromSqlResult<Self>;
}

impl FromSql for Value {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        Ok(value.clone())
    }
}

impl FromSql for i64 {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        match value {
            Value::Integer(i) => Ok(*i),
            other => Err(FromSqlError::InvalidType {
                expected: Type::Integer,
                actual: other.value_type(),
            }),
        }
    }
}

impl FromSql for i32 {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        let i = i64::column_result(value)?;
        i32::try_from(i).map_err(|_| FromSqlError::OutOfRange)
    }
}

impl FromSql for f64 {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        match value {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            other => Err(FromSqlError::InvalidType {
                expected: Type::Real,
                actual: other.value_type(),
            }),
        }
    }
}

impl FromSql for bool {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        Ok(i64::column_result(value)? != 0)
    }
}

impl FromSql for String {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        match value {
            Value::Text(s) => Ok(s.clone()),
            other => Err(FromSqlError::InvalidType {
                expected: Type::Text,
                actual: other.value_type(),
            }),
        }
    }
}

impl FromSql for Vec<u8> {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        match value {
            Value::Blob(b) => Ok(b.clone()),
            other => Err(FromSqlError::InvalidType {
                expected: Type::Blob,
                actual: other.value_type(),
            }),
        }
    }
}

impl<T: FromSql> FromSql for Option<T> {
    fn column_result(value: &Value) -> FromSqlResult<Self> {
        match value {
            Value::Null => Ok(None),
            other => T::column_result(other).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_primitives() {
        assert_eq!(i64::column_result(&Value::Integer(42)), Ok(42));
        assert_eq!(f64::column_result(&Value::Real(1.5)), Ok(1.5));
        assert_eq!(f64::column_result(&Value::Integer(2)), Ok(2.0));
        assert_eq!(bool::column_result(&Value::Integer(1)), Ok(true));
    }

    #[test]
    fn converts_string_and_bytes() {
        assert_eq!(
            String::column_result(&Value::Text("hi".into())),
            Ok("hi".to_string())
        );
        assert_eq!(
            Vec::<u8>::column_result(&Value::Blob(vec![1, 2, 3])),
            Ok(vec![1, 2, 3])
        );
    }

    #[test]
    fn converts_option() {
        assert_eq!(Option::<i64>::column_result(&Value::Null), Ok(None));
        assert_eq!(
            Option::<i64>::column_result(&Value::Integer(5)),
            Ok(Some(5))
        );
    }

    #[test]
    fn wrong_type_is_an_error() {
        assert_eq!(
            i64::column_result(&Value::Text("x".into())),
            Err(FromSqlError::InvalidType {
                expected: Type::Integer,
                actual: Type::Text,
            })
        );
    }

    #[test]
    fn out_of_range_is_an_error() {
        assert_eq!(
            i32::column_result(&Value::Integer(i64::MAX)),
            Err(FromSqlError::OutOfRange)
        );
    }
}
