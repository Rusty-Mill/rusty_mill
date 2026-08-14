//! `Rows`/`MappedRows`/`AndThenRows`: thin iterator wrappers over a
//! multi-row result set. Part B gap row "Rows/MappedRows/AndThenRows
//! iterator types".

use crate::error::Result;
use crate::row::Row;
use crate::value::Value;

/// An iterator over a query's result rows.
pub struct Rows<'a> {
    column_names: &'a [String],
    rows: std::slice::Iter<'a, Vec<Value>>,
}

impl<'a> Rows<'a> {
    /// Wraps a result set's column names and rows for iteration.
    pub fn new(column_names: &'a [String], rows: &'a [Vec<Value>]) -> Rows<'a> {
        Rows {
            column_names,
            rows: rows.iter(),
        }
    }

    /// Adapts this iterator to yield `f`'s output for each row instead of
    /// the [`Row`] itself.
    pub fn mapped<T, F>(self, f: F) -> MappedRows<'a, F>
    where
        F: FnMut(Row<'a>) -> Result<T>,
    {
        MappedRows { rows: self, f }
    }

    /// Like [`Rows::mapped`], but `f` may fail with any error type that
    /// [`crate::Error`] converts into.
    pub fn and_then<T, E, F>(self, f: F) -> AndThenRows<'a, F>
    where
        F: FnMut(Row<'a>) -> std::result::Result<T, E>,
        E: From<crate::Error>,
    {
        AndThenRows { rows: self, f }
    }
}

impl<'a> Iterator for Rows<'a> {
    type Item = Result<Row<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows
            .next()
            .map(|values| Ok(Row::new(self.column_names, values)))
    }
}

/// [`Rows`] adapted through a fallible mapping function.
pub struct MappedRows<'a, F> {
    rows: Rows<'a>,
    f: F,
}

impl<'a, T, F> Iterator for MappedRows<'a, F>
where
    F: FnMut(Row<'a>) -> Result<T>,
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|row| row.and_then(&mut self.f))
    }
}

/// [`Rows`] adapted through a mapping function whose error type is
/// generic (any `E: From<Error>`), rather than fixed to [`crate::Error`].
pub struct AndThenRows<'a, F> {
    rows: Rows<'a>,
    f: F,
}

impl<'a, T, E, F> Iterator for AndThenRows<'a, F>
where
    F: FnMut(Row<'a>) -> std::result::Result<T, E>,
    E: From<crate::Error>,
{
    type Item = std::result::Result<T, E>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|row| match row {
            Ok(row) => (self.f)(row),
            Err(e) => Err(E::from(e)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_set() -> (Vec<String>, Vec<Vec<Value>>) {
        (
            vec!["a".into()],
            vec![
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
                vec![Value::Integer(3)],
            ],
        )
    }

    #[test]
    fn iterates_rows() {
        let (cols, data) = result_set();
        let rows = Rows::new(&cols, &data);
        let collected: Result<Vec<i64>> =
            rows.map(|r| r.and_then(|row| row.get::<i64>(0))).collect();
        assert_eq!(collected.unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn mapped_applies_function_per_row() {
        let (cols, data) = result_set();
        let rows = Rows::new(&cols, &data);
        let doubled: Result<Vec<i64>> = rows
            .mapped(|row| row.get::<i64>(0).map(|n| n * 2))
            .collect();
        assert_eq!(doubled.unwrap(), vec![2, 4, 6]);
    }

    #[test]
    fn and_then_propagates_custom_error() {
        #[derive(Debug, PartialEq)]
        enum MyError {
            Inner(crate::Error),
            TooBig,
        }
        impl From<crate::Error> for MyError {
            fn from(e: crate::Error) -> MyError {
                MyError::Inner(e)
            }
        }

        let (cols, data) = result_set();
        let rows = Rows::new(&cols, &data);
        let result: std::result::Result<Vec<i64>, MyError> = rows
            .and_then(|row| {
                let n = row.get::<i64>(0)?;
                if n > 2 {
                    Err(MyError::TooBig)
                } else {
                    Ok(n)
                }
            })
            .collect();
        assert_eq!(result, Err(MyError::TooBig));
    }
}
