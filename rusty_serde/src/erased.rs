//! An object-safe (`dyn`-compatible) stand-in for [`Serializer`]/[`Serialize`],
//! scoped to scalar/simple shapes - the second half of what
//! `with`/`serialize_with` (issue #12) needs, built on top of
//! `rusty_serde_erased`'s [`Out`] primitive.
//!
//! # Why this exists
//!
//! A `with`/`serialize_with` function is written once, generically, against
//! *any* `S: Serializer` - but `rusty_serde_derive` never parses field
//! types, so it has no name for the field's own concrete type to build a
//! matching generic wrapper around. The fix is to hand the with-function a
//! *concrete* type ([`ErasedAsSerializer`]) that itself implements the real
//! [`Serializer`] trait, so the with-function's own genericity is satisfied
//! by ordinary monomorphization (`S = ErasedAsSerializer`) instead of
//! needing the derive macro to name anything.
//!
//! [`ErasedAsSerializer`] just forwards each call through a boxed
//! [`ErasedSerializer`] trait object - and *that's* where [`Out`] earns its
//! keep: going the other direction (a real, concrete serializer wrapped as
//! [`ConcreteToErased`]) needs to hand the *original* caller back a real
//! `S::Ok` value, but an object-safe trait's methods can't mention `S::Ok`
//! in their own signatures. `Out` is the pointer-sized side channel that
//! lets `ConcreteToErased`'s impl (which, despite implementing an
//! object-safe trait, is itself a plain generic struct monomorphized once
//! per concrete `S`) write that value back to a slot the original caller
//! already knows the type of.
//!
//! # Scope
//!
//! Only scalars, `Option`, unit, and enum unit/newtype variants are
//! supported - a `with`/`serialize_with` function that tries to serialize a
//! sequence/tuple/map/struct shape gets a clear error instead of an
//! incomplete/silently-wrong implementation. Covers the overwhelming
//! majority of real-world `with` uses (reformatting one value: a
//! `Duration` as a number of seconds, a timestamp as an ISO-8601 string, an
//! enum as its wire tag, ...); nothing here stops a *format* (`json`/`ron`)
//! from using the ordinary, unerased [`Serializer`] trait for everything
//! else.
//!
//! # Deserialize
//!
//! `deserialize_with` needs the same treatment on the [`Deserializer`]/
//! [`Visitor`](crate::de::Visitor) side - not yet implemented; this module
//! only covers `with`'s serialize half so far.
//!
//! [`Deserializer`]: crate::Deserializer

use rusty_serde_erased::Out;

use crate::error::Error as ErrorTrait;
use crate::impossible::Impossible;
use crate::ser::{Serialize, Serializer};

/// The error type at the erased boundary - carries just a message, since an
/// object-safe trait can't stay generic over each concrete format's own
/// `Error` type.
#[derive(Debug)]
pub struct ErasedError(String);

impl std::fmt::Display for ErasedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ErasedError {}

impl ErrorTrait for ErasedError {
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        ErasedError(msg.to_string())
    }
}

/// Object-safe stand-in for [`Serialize`], needed to pass an `Option`'s or
/// a newtype variant's inner value across the erased boundary (its own
/// concrete type is exactly as unknown to us as the outer field's).
/// Blanket-implemented for every real `Serialize` type - never implement
/// this directly.
pub trait ErasedSerialize {
    #[doc(hidden)]
    fn erased_serialize(
        &self,
        serializer: Box<dyn ErasedSerializer + '_>,
    ) -> Result<(), ErasedError>;
}

impl<T: Serialize + ?Sized> ErasedSerialize for T {
    fn erased_serialize(
        &self,
        serializer: Box<dyn ErasedSerializer + '_>,
    ) -> Result<(), ErasedError> {
        self.serialize(ErasedAsSerializer(serializer))
    }
}

/// Object-safe stand-in for [`Serializer`], scoped to the shapes described
/// in the module docs. `#[doc(hidden)]` - meant for `rusty_serde_derive`'s
/// generated code and this module's own two adapters, not hand-written
/// against directly.
#[doc(hidden)]
pub trait ErasedSerializer {
    fn erased_is_human_readable(&self) -> bool;
    fn erased_serialize_bool(self: Box<Self>, v: bool) -> Result<(), ErasedError>;
    fn erased_serialize_i64(self: Box<Self>, v: i64) -> Result<(), ErasedError>;
    fn erased_serialize_u64(self: Box<Self>, v: u64) -> Result<(), ErasedError>;
    fn erased_serialize_f64(self: Box<Self>, v: f64) -> Result<(), ErasedError>;
    fn erased_serialize_str(self: Box<Self>, v: &str) -> Result<(), ErasedError>;
    fn erased_serialize_bytes(self: Box<Self>, v: &[u8]) -> Result<(), ErasedError>;
    fn erased_serialize_none(self: Box<Self>) -> Result<(), ErasedError>;
    fn erased_serialize_some(
        self: Box<Self>,
        value: &dyn ErasedSerialize,
    ) -> Result<(), ErasedError>;
    fn erased_serialize_unit(self: Box<Self>) -> Result<(), ErasedError>;
    fn erased_serialize_unit_variant(
        self: Box<Self>,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<(), ErasedError>;
    fn erased_serialize_newtype_variant(
        self: Box<Self>,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &dyn ErasedSerialize,
    ) -> Result<(), ErasedError>;
}

/// Wraps a real, concrete `S: Serializer` to implement the object-safe
/// [`ErasedSerializer`] trait - the "concrete-to-erased" half of the
/// adapter pair. `out` is where the real `S::Ok` value this struct's own
/// monomorphization produces gets stashed for the original (statically
/// `S`-aware) caller to read back; see the module docs.
struct ConcreteToErased<'out, S: Serializer> {
    inner: S,
    out: Out<'out>,
}

/// Generates one [`ErasedSerializer`] method that forwards straight to the
/// identically-shaped real [`Serializer`] method, stashing the result via
/// `Out` - every scalar/unit-ish method (everything except
/// `serialize_some`/`serialize_newtype_variant`, which also need to erase
/// their own inner value) follows this exact shape.
macro_rules! forward_scalar {
    ($erased:ident, $real:ident($($arg:ident: $ty:ty),*)) => {
        fn $erased(self: Box<Self>, $($arg: $ty),*) -> Result<(), ErasedError> {
            let this = *self;
            match this.inner.$real($($arg),*) {
                // SAFETY: `this.out` was constructed (in `call_with`/
                // `EraseAsSerialize::serialize`, the only two callers) from
                // an `Option<S::Ok>` for this exact `S` - `ok`'s type here
                // is that same `S::Ok`, so `set`'s type-matching contract
                // holds.
                Ok(ok) => {
                    unsafe { this.out.set(ok) };
                    Ok(())
                }
                Err(e) => Err(ErasedError::custom(e)),
            }
        }
    };
}

impl<'out, S: Serializer> ErasedSerializer for ConcreteToErased<'out, S> {
    fn erased_is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }

    forward_scalar!(erased_serialize_bool, serialize_bool(v: bool));
    forward_scalar!(erased_serialize_i64, serialize_i64(v: i64));
    forward_scalar!(erased_serialize_u64, serialize_u64(v: u64));
    forward_scalar!(erased_serialize_f64, serialize_f64(v: f64));
    forward_scalar!(erased_serialize_str, serialize_str(v: &str));
    forward_scalar!(erased_serialize_bytes, serialize_bytes(v: &[u8]));
    forward_scalar!(erased_serialize_none, serialize_none());
    forward_scalar!(erased_serialize_unit, serialize_unit());
    forward_scalar!(
        erased_serialize_unit_variant,
        serialize_unit_variant(name: &'static str, variant_index: u32, variant: &'static str)
    );

    fn erased_serialize_some(
        self: Box<Self>,
        value: &dyn ErasedSerialize,
    ) -> Result<(), ErasedError> {
        let this = *self;
        match this.inner.serialize_some(&EraseAsSerialize(value)) {
            Ok(ok) => {
                unsafe { this.out.set(ok) };
                Ok(())
            }
            Err(e) => Err(ErasedError::custom(e)),
        }
    }

    fn erased_serialize_newtype_variant(
        self: Box<Self>,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &dyn ErasedSerialize,
    ) -> Result<(), ErasedError> {
        let this = *self;
        match this.inner.serialize_newtype_variant(
            name,
            variant_index,
            variant,
            &EraseAsSerialize(value),
        ) {
            Ok(ok) => {
                unsafe { this.out.set(ok) };
                Ok(())
            }
            Err(e) => Err(ErasedError::custom(e)),
        }
    }
}

/// Wraps an already-erased `&dyn ErasedSerialize` to implement the real
/// [`Serialize`] trait - needed wherever a real `Serializer` method (e.g.
/// `serialize_some`) wants to call `value.serialize(some_concrete_S)`
/// itself. Drives the same erase/call/unstash dance as
/// [`ErasedAsSerializer`], just one level further in.
struct EraseAsSerialize<'a>(&'a dyn ErasedSerialize);

impl Serialize for EraseAsSerialize<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut ok: Option<S::Ok> = None;
        let boxed: Box<dyn ErasedSerializer + '_> = Box::new(ConcreteToErased {
            inner: serializer,
            out: Out::new(&mut ok),
        });
        self.0.erased_serialize(boxed).map_err(S::Error::custom)?;
        Ok(ok
            .expect("ConcreteToErased's one terminal call always populates `out` on the `Ok` path"))
    }
}

fn unsupported_compound_shape() -> ErasedError {
    ErasedError::custom(
        "a `with`/`serialize_with` function can't serialize a sequence/tuple/map/struct shape \
         (yet) - only scalars, `Option`, unit, and unit/newtype enum variants are supported",
    )
}

/// The concrete type a `with`/`serialize_with` function actually gets
/// monomorphized against - implements the real [`Serializer`] trait (so an
/// ordinary `fn my_with<S: Serializer>(value: &T, serializer: S) -> ...`
/// function, passed as a plain value, coerces to
/// `fn(&T, ErasedAsSerializer) -> Result<(), ErasedError>` without the
/// derive macro ever needing to write out `S` itself), by forwarding every
/// call through a boxed [`ErasedSerializer`].
pub struct ErasedAsSerializer<'a>(Box<dyn ErasedSerializer + 'a>);

impl Serializer for ErasedAsSerializer<'_> {
    type Ok = ();
    type Error = ErasedError;
    type SerializeSeq = Impossible<(), ErasedError>;
    type SerializeTuple = Impossible<(), ErasedError>;
    type SerializeTupleStruct = Impossible<(), ErasedError>;
    type SerializeTupleVariant = Impossible<(), ErasedError>;
    type SerializeMap = Impossible<(), ErasedError>;
    type SerializeStruct = Impossible<(), ErasedError>;
    type SerializeStructVariant = Impossible<(), ErasedError>;

    fn is_human_readable(&self) -> bool {
        self.0.erased_is_human_readable()
    }

    fn serialize_bool(self, v: bool) -> Result<(), ErasedError> {
        self.0.erased_serialize_bool(v)
    }
    fn serialize_i64(self, v: i64) -> Result<(), ErasedError> {
        self.0.erased_serialize_i64(v)
    }
    fn serialize_u64(self, v: u64) -> Result<(), ErasedError> {
        self.0.erased_serialize_u64(v)
    }
    fn serialize_f64(self, v: f64) -> Result<(), ErasedError> {
        self.0.erased_serialize_f64(v)
    }
    fn serialize_str(self, v: &str) -> Result<(), ErasedError> {
        self.0.erased_serialize_str(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), ErasedError> {
        self.0.erased_serialize_bytes(v)
    }
    fn serialize_none(self) -> Result<(), ErasedError> {
        self.0.erased_serialize_none()
    }
    fn serialize_some<T>(self, value: &T) -> Result<(), ErasedError>
    where
        T: Serialize + ?Sized,
    {
        // `&value` (not `value`): unsizing `&T -> &dyn ErasedSerialize`
        // needs `T: Sized`, which a `?Sized` field type doesn't guarantee -
        // `&T` itself is always `Sized` and gets the same blanket
        // `ErasedSerialize` impl (via the existing `Serialize for &T`).
        self.0.erased_serialize_some(&value)
    }
    fn serialize_unit(self) -> Result<(), ErasedError> {
        self.0.erased_serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<(), ErasedError> {
        self.0
            .erased_serialize_unit_variant(name, variant_index, variant)
    }
    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), ErasedError>
    where
        T: Serialize + ?Sized,
    {
        // See `serialize_some` above for why `&value` rather than `value`.
        self.0
            .erased_serialize_newtype_variant(name, variant_index, variant, &value)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(unsupported_compound_shape())
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(unsupported_compound_shape())
    }
}

/// Drives `with_fn` (an ordinary `fn my_with<S: Serializer>(value: &T,
/// serializer: S) -> Result<S::Ok, S::Error>`, monomorphized at `S =
/// ErasedAsSerializer` - see [`ErasedAsSerializer`]'s docs) against a real
/// `serializer`, translating the erased result back into a genuine
/// `Result<S::Ok, S::Error>`. This is the entry point
/// `rusty_serde_derive`'s generated code calls for a `with`/
/// `serialize_with` field.
#[doc(hidden)]
pub fn call_with<T, S>(
    value: &T,
    serializer: S,
    with_fn: fn(&T, ErasedAsSerializer<'_>) -> Result<(), ErasedError>,
) -> Result<S::Ok, S::Error>
where
    T: ?Sized,
    S: Serializer,
{
    let mut ok: Option<S::Ok> = None;
    let boxed: Box<dyn ErasedSerializer + '_> = Box::new(ConcreteToErased {
        inner: serializer,
        out: Out::new(&mut ok),
    });
    with_fn(value, ErasedAsSerializer(boxed)).map_err(S::Error::custom)?;
    Ok(ok.expect("ConcreteToErased's one terminal call always populates `out` on the `Ok` path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    struct Seconds(std::time::Duration);

    fn serialize_seconds<S: Serializer>(
        value: &std::time::Duration,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }

    impl Serialize for Seconds {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            call_with(&self.0, serializer, |v, s| serialize_seconds(v, s))
        }
    }

    #[test]
    fn call_with_reformats_a_scalar_through_json() {
        let value = Seconds(std::time::Duration::from_secs(42));
        assert_eq!(json::to_string(&value).unwrap(), "42");
    }

    #[test]
    fn call_with_reformats_a_scalar_through_ron() {
        let value = Seconds(std::time::Duration::from_secs(7));
        assert_eq!(crate::ron::to_string(&value).unwrap(), "7");
    }

    fn serialize_label<S: Serializer>(
        value: &Option<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(s) => serializer.serialize_some(s),
            None => serializer.serialize_none(),
        }
    }

    struct Label(Option<String>);

    impl Serialize for Label {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            call_with(&self.0, serializer, |v, s| serialize_label(v, s))
        }
    }

    #[test]
    fn call_with_forwards_an_option_and_erases_its_inner_value_too() {
        assert_eq!(
            json::to_string(&Label(Some("hi".to_string()))).unwrap(),
            r#""hi""#
        );
        assert_eq!(json::to_string(&Label(None)).unwrap(), "null");
    }

    fn serialize_as_seq<S: Serializer>(_value: &i32, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(1))?;
        seq.serialize_element(&1)?;
        seq.end()
    }

    struct Unsupported(i32);

    impl Serialize for Unsupported {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            call_with(&self.0, serializer, |v, s| serialize_as_seq(v, s))
        }
    }

    #[test]
    fn compound_shapes_are_a_clear_error_not_a_silent_miscompile() {
        let err = json::to_string(&Unsupported(1)).unwrap_err();
        assert!(err.to_string().contains("sequence/tuple/map/struct"));
    }
}
