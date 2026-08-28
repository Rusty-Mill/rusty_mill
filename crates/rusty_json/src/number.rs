use core::fmt;

/// A JSON number.
///
/// Stores whichever of unsigned integer, signed integer, or float
/// representation the value was constructed from, so integers up to 64 bits
/// round-trip exactly instead of being lossily converted through `f64`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number(Repr);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Repr {
    PosInt(u64),
    NegInt(i64),
    Float(f64),
}

impl Number {
    /// Returns the number as an `f64`, converting if necessary. This may
    /// lose precision for integers larger than 2^53.
    pub fn as_f64(&self) -> f64 {
        match self.0 {
            Repr::PosInt(n) => n as f64,
            Repr::NegInt(n) => n as f64,
            Repr::Float(n) => n,
        }
    }

    /// Returns the number as an `i64`, if it was constructed from an integer
    /// that fits without loss.
    pub fn as_i64(&self) -> Option<i64> {
        match self.0 {
            Repr::PosInt(n) => i64::try_from(n).ok(),
            Repr::NegInt(n) => Some(n),
            Repr::Float(_) => None,
        }
    }

    /// Returns the number as a `u64`, if it was constructed from a
    /// non-negative integer that fits without loss.
    pub fn as_u64(&self) -> Option<u64> {
        match self.0 {
            Repr::PosInt(n) => Some(n),
            Repr::NegInt(_) | Repr::Float(_) => None,
        }
    }

    /// Returns the number as an `i128`. Always exact for an integer-backed
    /// `Number` (64 bits always fits in 128), `None` for a float-backed one.
    pub fn as_i128(&self) -> Option<i128> {
        match self.0 {
            Repr::PosInt(n) => Some(i128::from(n)),
            Repr::NegInt(n) => Some(i128::from(n)),
            Repr::Float(_) => None,
        }
    }

    /// Returns the number as a `u128`. Always exact for a non-negative
    /// integer-backed `Number`, `None` otherwise.
    pub fn as_u128(&self) -> Option<u128> {
        match self.0 {
            Repr::PosInt(n) => Some(u128::from(n)),
            Repr::NegInt(_) | Repr::Float(_) => None,
        }
    }

    /// Builds a `Number` from an `i128`. Values that fit in `i64`/`u64`
    /// round-trip exactly; larger magnitudes fall back to a lossy `f64`
    /// representation, same as this crate's number parser does for
    /// out-of-range integer literals.
    pub fn from_i128(n: i128) -> Option<Number> {
        if let Ok(n) = i64::try_from(n) {
            Some(Number::from(n))
        } else if let Ok(n) = u64::try_from(n) {
            Some(Number::from(n))
        } else {
            Number::from_f64(n as f64)
        }
    }

    /// Builds a `Number` from a `u128`. Values that fit in `u64` round-trip
    /// exactly; larger magnitudes fall back to a lossy `f64` representation,
    /// same as this crate's number parser does for out-of-range integer
    /// literals.
    pub fn from_u128(n: u128) -> Option<Number> {
        if let Ok(n) = u64::try_from(n) {
            Some(Number::from(n))
        } else {
            Number::from_f64(n as f64)
        }
    }

    /// True if [`Number::as_i64`] would return `Some`.
    pub fn is_i64(&self) -> bool {
        self.as_i64().is_some()
    }

    /// True if [`Number::as_u64`] would return `Some`.
    pub fn is_u64(&self) -> bool {
        self.as_u64().is_some()
    }

    /// True if the number was constructed from a float (as opposed to an
    /// integer type).
    pub fn is_f64(&self) -> bool {
        matches!(self.0, Repr::Float(_))
    }

    /// Builds a `Number` from an `f64`, returning `None` for `NaN` or
    /// infinite values, which JSON's number grammar cannot represent.
    pub fn from_f64(f: f64) -> Option<Number> {
        if f.is_finite() {
            Some(Number(Repr::Float(f)))
        } else {
            None
        }
    }
}

macro_rules! impl_from_unsigned {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Number {
                fn from(n: $ty) -> Self {
                    Number(Repr::PosInt(n as u64))
                }
            }
        )*
    };
}

macro_rules! impl_from_signed {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Number {
                fn from(n: $ty) -> Self {
                    if n < 0 {
                        Number(Repr::NegInt(n as i64))
                    } else {
                        Number(Repr::PosInt(n as u64))
                    }
                }
            }
        )*
    };
}

impl_from_unsigned!(u8, u16, u32, u64, usize);
impl_from_signed!(i8, i16, i32, i64, isize);

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Repr::PosInt(n) => write!(f, "{n}"),
            Repr::NegInt(n) => write!(f, "{n}"),
            Repr::Float(n) => {
                // Rust's default f64 Display omits the decimal point for
                // whole numbers (`1.0` -> `"1"`); re-parsing that string
                // would then produce an integer `Repr`, losing the fact
                // that this value came from a float. Force a `.` (or
                // scientific notation) into the output so it always
                // round-trips back through `Repr::Float`.
                let mut buf = alloc::format!("{n}");
                if !buf.contains('.') && !buf.contains('e') && !buf.contains('E') {
                    buf.push_str(".0");
                }
                f.write_str(&buf)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn as_128_widens_exactly() {
        assert_eq!(Number::from(42u64).as_i128(), Some(42i128));
        assert_eq!(Number::from(42u64).as_u128(), Some(42u128));
        assert_eq!(Number::from(-7i64).as_i128(), Some(-7i128));
        assert_eq!(Number::from(-7i64).as_u128(), None);
        assert_eq!(Number::from_f64(1.5).unwrap().as_i128(), None);
    }

    #[test]
    fn from_128_round_trips_within_64_bits() {
        assert_eq!(Number::from_i128(42).unwrap().as_i64(), Some(42));
        assert_eq!(Number::from_i128(-7).unwrap().as_i64(), Some(-7));
        assert_eq!(Number::from_u128(42).unwrap().as_u64(), Some(42));
    }

    #[test]
    fn from_128_falls_back_to_f64_beyond_64_bits() {
        let huge = i128::from(u64::MAX) + 1;
        let n = Number::from_i128(huge).unwrap();
        assert!(n.is_f64());
        let huge_u = u128::from(u64::MAX) + 1;
        let n = Number::from_u128(huge_u).unwrap();
        assert!(n.is_f64());
    }

    #[test]
    fn integers_round_trip() {
        assert_eq!(Number::from(42u64).as_u64(), Some(42));
        assert_eq!(Number::from(42u64).as_i64(), Some(42));
        assert_eq!(Number::from(-7i64).as_i64(), Some(-7));
        assert_eq!(Number::from(-7i64).as_u64(), None);
        assert_eq!(Number::from(u64::MAX).as_i64(), None);
    }

    #[test]
    fn float_rejects_non_finite() {
        assert!(Number::from_f64(f64::NAN).is_none());
        assert!(Number::from_f64(f64::INFINITY).is_none());
        assert!(Number::from_f64(f64::NEG_INFINITY).is_none());
        assert!(Number::from_f64(1.5).is_some());
    }

    #[test]
    fn display_integers() {
        assert_eq!(Number::from(42u64).to_string(), "42");
        assert_eq!(Number::from(-7i64).to_string(), "-7");
    }

    #[test]
    fn display_floats_always_have_a_decimal_point() {
        assert_eq!(Number::from_f64(1.0).unwrap().to_string(), "1.0");
        assert_eq!(Number::from_f64(1.5).unwrap().to_string(), "1.5");
        assert_eq!(Number::from_f64(-0.0).unwrap().to_string(), "-0.0");
    }

    #[test]
    fn is_predicates() {
        assert!(Number::from(1u64).is_u64());
        assert!(Number::from(1u64).is_i64());
        assert!(!Number::from(1u64).is_f64());
        assert!(Number::from_f64(1.0).unwrap().is_f64());
    }
}
