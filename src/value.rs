//! The SQLite value/type model: the five storage classes every column value
//! belongs to. See <https://www.sqlite.org/datatype3.html>.

/// A dynamically-typed SQL value, corresponding to one of SQLite's five
/// storage classes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// The `NULL` storage class.
    Null,
    /// A signed integer, stored in as few bytes as the value needs (up to
    /// 8 bytes on the wire; represented here as `i64`).
    Integer(i64),
    /// A floating-point value, stored as an IEEE 754 8-byte double.
    Real(f64),
    /// A text string.
    Text(String),
    /// A blob of binary data, stored exactly as given.
    Blob(Vec<u8>),
}

/// The storage class of a [`Value`], independent of its actual data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Null,
    Integer,
    Real,
    Text,
    Blob,
}

impl Value {
    /// Returns this value's storage class.
    pub fn value_type(&self) -> Type {
        match self {
            Value::Null => Type::Null,
            Value::Integer(_) => Type::Integer,
            Value::Real(_) => Type::Real,
            Value::Text(_) => Type::Text,
            Value::Blob(_) => Type::Blob,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_type_matches_variant() {
        assert_eq!(Value::Null.value_type(), Type::Null);
        assert_eq!(Value::Integer(42).value_type(), Type::Integer);
        assert_eq!(Value::Real(1.5).value_type(), Type::Real);
        assert_eq!(Value::Text("hi".into()).value_type(), Type::Text);
        assert_eq!(Value::Blob(vec![1, 2, 3]).value_type(), Type::Blob);
    }

    #[test]
    fn equal_variants_compare_equal() {
        assert_eq!(Value::Integer(7), Value::Integer(7));
        assert_ne!(Value::Integer(7), Value::Integer(8));
        assert_ne!(Value::Integer(0), Value::Null);
    }
}
