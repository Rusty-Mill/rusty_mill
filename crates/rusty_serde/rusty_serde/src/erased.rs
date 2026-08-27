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
//! The bottom half of this module is the same idea applied to
//! [`Deserializer`]/[`Visitor`](crate::de::Visitor), for `deserialize_with`.
//! The role [`Out`] plays flips accordingly: [`Deserializer`] itself has no
//! per-format "Ok" associated type to smuggle out (every method's result
//! type is `V::Value`, chosen by whichever [`Visitor`](crate::de::Visitor)
//! the *caller* supplies) - so it's erasing a caller-supplied `Visitor`
//! ([`VisitorToErased`]) that needs `Out`, not erasing the deserializer
//! itself ([`DeserializerToErased`]).

use rusty_serde_erased::Out;

use crate::de::{Deserializer, Visitor};
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

/// What `#[rusty_serde(serialize_with = "path")]`'s generated code wraps a
/// field's value in: `SerializeStruct::serialize_field`/
/// `SerializeMap::serialize_entry` need a `T: Serialize` value, and this
/// crate's derive macro never parses field types, so there's no way for it
/// to write a matching `Serialize` impl for the field's own (unknown) type
/// directly - `With` is the one, reusable such impl every `serialize_with`
/// field's generated code shares, its own [`Serialize::serialize`] just
/// calling back into [`call_with`].
#[doc(hidden)]
pub struct With<'a, T: ?Sized> {
    pub value: &'a T,
    pub with_fn: fn(&T, ErasedAsSerializer<'_>) -> Result<(), ErasedError>,
}

impl<T: ?Sized> Serialize for With<'_, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        call_with(self.value, serializer, self.with_fn)
    }
}

fn unsupported_compound_shape_de() -> ErasedError {
    ErasedError::custom(
        "a `deserialize_with` function can't deserialize a sequence/tuple/map/struct/enum \
         shape (yet) - only scalars, `Option`, and unit are supported",
    )
}

/// Object-safe stand-in for [`Visitor`], mirroring [`ErasedSerializer`] on
/// the deserialize side - see the module's "Deserialize" doc section for
/// why it's [`VisitorToErased`] (not [`DeserializerToErased`]) that carries
/// [`Out`] here.
#[doc(hidden)]
pub trait ErasedVisitor {
    fn erased_visit_bool(self: Box<Self>, v: bool) -> Result<(), ErasedError>;
    fn erased_visit_i64(self: Box<Self>, v: i64) -> Result<(), ErasedError>;
    fn erased_visit_u64(self: Box<Self>, v: u64) -> Result<(), ErasedError>;
    fn erased_visit_f64(self: Box<Self>, v: f64) -> Result<(), ErasedError>;
    fn erased_visit_str(self: Box<Self>, v: &str) -> Result<(), ErasedError>;
    fn erased_visit_bytes(self: Box<Self>, v: &[u8]) -> Result<(), ErasedError>;
    fn erased_visit_none(self: Box<Self>) -> Result<(), ErasedError>;
    fn erased_visit_some(
        self: Box<Self>,
        deserializer: Box<dyn ErasedDeserializer + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_visit_unit(self: Box<Self>) -> Result<(), ErasedError>;
}

/// Object-safe stand-in for [`Deserializer`], scoped the same way
/// [`ErasedSerializer`] is on the serialize side - scalars/`Option`/unit
/// only, everything else (`erased_deserialize_*` for a compound shape)
/// simply isn't part of this trait, since [`ErasedAsDeserializer`] returns
/// a clear error for those before ever reaching it.
#[doc(hidden)]
pub trait ErasedDeserializer {
    fn erased_is_human_readable(&self) -> bool;
    fn erased_deserialize_any(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_bool(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_i8(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_i16(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_i32(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_i64(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_u8(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_u16(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_u32(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_u64(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_f32(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_f64(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_char(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_str(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_string(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_bytes(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_byte_buf(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_option(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
    fn erased_deserialize_unit(
        self: Box<Self>,
        visitor: Box<dyn ErasedVisitor + '_>,
    ) -> Result<(), ErasedError>;
}

/// Wraps a real, concrete `V: Visitor<'de>` to implement the object-safe
/// [`ErasedVisitor`] trait - the "concrete-to-erased" half of the deserialize
/// adapter pair (see the module's "Deserialize" doc section for why `Out`
/// lives here rather than on [`DeserializerToErased`]).
struct VisitorToErased<'out, V> {
    inner: V,
    out: Out<'out>,
}

/// Generates one [`ErasedVisitor`] method that forwards to the
/// identically-shaped real [`Visitor`] method (always instantiated at
/// `E = ErasedError`, since `Visitor`'s methods are themselves generic over
/// the error type), stashing the result via `Out`.
macro_rules! forward_visit {
    ($erased:ident, $real:ident($($arg:ident: $ty:ty),*)) => {
        fn $erased(self: Box<Self>, $($arg: $ty),*) -> Result<(), ErasedError> {
            let this = *self;
            match this.inner.$real($($arg),*) {
                // SAFETY: `this.out` was constructed (in
                // `ErasedAsDeserializer`'s own generic methods, the only
                // caller) from an `Option<V::Value>` for this exact `V` -
                // `val`'s type here is that same `V::Value`.
                Ok(val) => {
                    unsafe { this.out.set(val) };
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    };
}

impl<'de, 'out, V: Visitor<'de>> ErasedVisitor for VisitorToErased<'out, V> {
    forward_visit!(erased_visit_bool, visit_bool(v: bool));
    forward_visit!(erased_visit_i64, visit_i64(v: i64));
    forward_visit!(erased_visit_u64, visit_u64(v: u64));
    forward_visit!(erased_visit_f64, visit_f64(v: f64));
    forward_visit!(erased_visit_str, visit_str(v: &str));
    forward_visit!(erased_visit_bytes, visit_bytes(v: &[u8]));
    forward_visit!(erased_visit_none, visit_none());
    forward_visit!(erased_visit_unit, visit_unit());

    fn erased_visit_some(
        self: Box<Self>,
        deserializer: Box<dyn ErasedDeserializer + '_>,
    ) -> Result<(), ErasedError> {
        let this = *self;
        match this.inner.visit_some(ErasedAsDeserializer(deserializer)) {
            Ok(val) => {
                unsafe { this.out.set(val) };
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Wraps a real, concrete `D: Deserializer<'de>` to implement the
/// object-safe [`ErasedDeserializer`] trait. Needs no [`Out`] of its own -
/// unlike [`Serializer`], [`Deserializer`] has no per-format "Ok" type of
/// its own to smuggle out (every method's result type comes from whichever
/// `Visitor` the *caller* supplies, which is what [`VisitorToErased`]
/// already handles).
struct DeserializerToErased<D> {
    inner: D,
}

/// Generates one [`ErasedDeserializer`] method that forwards to the
/// identically-shaped real [`Deserializer`] method, wrapping the erased
/// visitor as a real (synthetic-`Value = ()`) [`Visitor`] via
/// [`ErasedAsVisitor`].
macro_rules! forward_deserialize {
    ($erased:ident, $real:ident) => {
        fn $erased(
            self: Box<Self>,
            visitor: Box<dyn ErasedVisitor + '_>,
        ) -> Result<(), ErasedError> {
            self.inner
                .$real(ErasedAsVisitor(visitor))
                .map_err(ErasedError::custom)
        }
    };
}

impl<'de, D: Deserializer<'de>> ErasedDeserializer for DeserializerToErased<D> {
    fn erased_is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }

    forward_deserialize!(erased_deserialize_any, deserialize_any);
    forward_deserialize!(erased_deserialize_bool, deserialize_bool);
    forward_deserialize!(erased_deserialize_i8, deserialize_i8);
    forward_deserialize!(erased_deserialize_i16, deserialize_i16);
    forward_deserialize!(erased_deserialize_i32, deserialize_i32);
    forward_deserialize!(erased_deserialize_i64, deserialize_i64);
    forward_deserialize!(erased_deserialize_u8, deserialize_u8);
    forward_deserialize!(erased_deserialize_u16, deserialize_u16);
    forward_deserialize!(erased_deserialize_u32, deserialize_u32);
    forward_deserialize!(erased_deserialize_u64, deserialize_u64);
    forward_deserialize!(erased_deserialize_f32, deserialize_f32);
    forward_deserialize!(erased_deserialize_f64, deserialize_f64);
    forward_deserialize!(erased_deserialize_char, deserialize_char);
    forward_deserialize!(erased_deserialize_str, deserialize_str);
    forward_deserialize!(erased_deserialize_string, deserialize_string);
    forward_deserialize!(erased_deserialize_bytes, deserialize_bytes);
    forward_deserialize!(erased_deserialize_byte_buf, deserialize_byte_buf);
    forward_deserialize!(erased_deserialize_option, deserialize_option);
    forward_deserialize!(erased_deserialize_unit, deserialize_unit);
}

/// The concrete `Value = ()` [`Visitor`] every erased-deserializer method
/// hands the *real* format, so the format's own `deserialize_*` can call an
/// ordinary (non-object-safe) `Visitor` method - which then forwards
/// through the boxed [`ErasedVisitor`] this wraps.
struct ErasedAsVisitor<'a>(Box<dyn ErasedVisitor + 'a>);

impl<'de> Visitor<'de> for ErasedAsVisitor<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a scalar, option, or unit value")
    }

    fn visit_bool<E: ErrorTrait>(self, v: bool) -> Result<(), E> {
        self.0.erased_visit_bool(v).map_err(E::custom)
    }
    fn visit_i64<E: ErrorTrait>(self, v: i64) -> Result<(), E> {
        self.0.erased_visit_i64(v).map_err(E::custom)
    }
    fn visit_u64<E: ErrorTrait>(self, v: u64) -> Result<(), E> {
        self.0.erased_visit_u64(v).map_err(E::custom)
    }
    fn visit_f64<E: ErrorTrait>(self, v: f64) -> Result<(), E> {
        self.0.erased_visit_f64(v).map_err(E::custom)
    }
    fn visit_str<E: ErrorTrait>(self, v: &str) -> Result<(), E> {
        self.0.erased_visit_str(v).map_err(E::custom)
    }
    fn visit_bytes<E: ErrorTrait>(self, v: &[u8]) -> Result<(), E> {
        self.0.erased_visit_bytes(v).map_err(E::custom)
    }
    fn visit_none<E: ErrorTrait>(self) -> Result<(), E> {
        self.0.erased_visit_none().map_err(E::custom)
    }
    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let boxed: Box<dyn ErasedDeserializer + '_> = Box::new(DeserializerToErased {
            inner: deserializer,
        });
        self.0.erased_visit_some(boxed).map_err(D::Error::custom)
    }
    fn visit_unit<E: ErrorTrait>(self) -> Result<(), E> {
        self.0.erased_visit_unit().map_err(E::custom)
    }
}

/// The concrete type a `deserialize_with` function actually gets
/// monomorphized against - mirrors [`ErasedAsSerializer`], see its docs.
pub struct ErasedAsDeserializer<'a>(Box<dyn ErasedDeserializer + 'a>);

/// Generates one [`Deserializer`] method that erases `visitor`, drives it
/// through the boxed [`ErasedDeserializer`], and unstashes the real
/// `V::Value` via `Out`.
macro_rules! forward_erased_deserializer {
    ($real:ident, $erased:ident) => {
        fn $real<V>(self, visitor: V) -> Result<V::Value, ErasedError>
        where
            V: Visitor<'de>,
        {
            let mut ok: Option<V::Value> = None;
            let boxed: Box<dyn ErasedVisitor + '_> = Box::new(VisitorToErased {
                inner: visitor,
                out: Out::new(&mut ok),
            });
            self.0.$erased(boxed)?;
            Ok(ok.expect(
                "VisitorToErased's one terminal call always populates `out` on the `Ok` path",
            ))
        }
    };
}

impl<'de> Deserializer<'de> for ErasedAsDeserializer<'_> {
    type Error = ErasedError;

    fn is_human_readable(&self) -> bool {
        self.0.erased_is_human_readable()
    }

    forward_erased_deserializer!(deserialize_any, erased_deserialize_any);
    forward_erased_deserializer!(deserialize_bool, erased_deserialize_bool);
    forward_erased_deserializer!(deserialize_i8, erased_deserialize_i8);
    forward_erased_deserializer!(deserialize_i16, erased_deserialize_i16);
    forward_erased_deserializer!(deserialize_i32, erased_deserialize_i32);
    forward_erased_deserializer!(deserialize_i64, erased_deserialize_i64);
    forward_erased_deserializer!(deserialize_u8, erased_deserialize_u8);
    forward_erased_deserializer!(deserialize_u16, erased_deserialize_u16);
    forward_erased_deserializer!(deserialize_u32, erased_deserialize_u32);
    forward_erased_deserializer!(deserialize_u64, erased_deserialize_u64);
    forward_erased_deserializer!(deserialize_f32, erased_deserialize_f32);
    forward_erased_deserializer!(deserialize_f64, erased_deserialize_f64);
    forward_erased_deserializer!(deserialize_char, erased_deserialize_char);
    forward_erased_deserializer!(deserialize_str, erased_deserialize_str);
    forward_erased_deserializer!(deserialize_string, erased_deserialize_string);
    forward_erased_deserializer!(deserialize_bytes, erased_deserialize_bytes);
    forward_erased_deserializer!(deserialize_byte_buf, erased_deserialize_byte_buf);
    forward_erased_deserializer!(deserialize_option, erased_deserialize_option);
    forward_erased_deserializer!(deserialize_unit, erased_deserialize_unit);

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_seq<V>(self, _visitor: V) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_tuple<V>(self, _len: usize, _visitor: V) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_map<V>(self, _visitor: V) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_identifier<V>(self, _visitor: V) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    fn deserialize_ignored_any<V>(self, _visitor: V) -> Result<V::Value, ErasedError>
    where
        V: Visitor<'de>,
    {
        Err(unsupported_compound_shape_de())
    }
    // `deserialize_internally_tagged_enum` inherits the trait's own
    // default ("this deserializer does not support internally tagged
    // enums") - already exactly the error this scope calls for.
}

/// Drives `with_fn` (an ordinary `fn my_with<'de, D: Deserializer<'de>>(D)
/// -> Result<T, D::Error>`, monomorphized at `D = ErasedAsDeserializer` -
/// see [`ErasedAsDeserializer`]'s docs) against a real `deserializer`,
/// translating the erased result back into a genuine `Result<T,
/// D::Error>`. `T` needs no `Out`-based hand-off here - unlike
/// `serialize_with`'s `S::Ok` (unknown until called), `T` is exactly
/// `with_fn`'s own, already-concrete return type. This is the entry point
/// `rusty_serde_derive`'s generated code calls for a `deserialize_with`
/// field.
#[doc(hidden)]
pub fn call_with_deserialize<'de, T, D>(
    deserializer: D,
    with_fn: fn(ErasedAsDeserializer<'_>) -> Result<T, ErasedError>,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    let boxed: Box<dyn ErasedDeserializer + '_> = Box::new(DeserializerToErased {
        inner: deserializer,
    });
    with_fn(ErasedAsDeserializer(boxed)).map_err(D::Error::custom)
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

    /// Stands in for what `#[rusty_serde(serialize_with = "...")]`'s
    /// generated code produces: a plain struct field's value wrapped in
    /// `With` at the `SerializeStruct::serialize_field` call site.
    struct Event {
        at: std::time::Duration,
    }

    impl Serialize for Event {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            use crate::ser::SerializeStruct;
            let mut state = serializer.serialize_struct("Event", 1)?;
            state.serialize_field(
                "at",
                &With {
                    value: &self.at,
                    with_fn: |v, s| serialize_seconds(v, s),
                },
            )?;
            state.end()
        }
    }

    #[test]
    fn with_wraps_a_field_for_serialize_field() {
        let value = Event {
            at: std::time::Duration::from_secs(5),
        };
        assert_eq!(json::to_string(&value).unwrap(), r#"{"at":5}"#);
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

    fn deserialize_seconds<'de, D>(deserializer: D) -> Result<std::time::Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = deserializer.deserialize_u64(U64Visitor)?;
        Ok(std::time::Duration::from_secs(secs))
    }

    struct U64Visitor;
    impl<'de> Visitor<'de> for U64Visitor {
        type Value = u64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a u64")
        }
        fn visit_u64<E: ErrorTrait>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }
    }

    #[test]
    fn call_with_deserialize_reformats_a_scalar_through_json() {
        let mut de = json::Deserializer::from_str("42");
        let value: std::time::Duration =
            call_with_deserialize(&mut de, |d| deserialize_seconds(d)).unwrap();
        assert_eq!(value, std::time::Duration::from_secs(42));
    }

    fn deserialize_label<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OptVisitor;
        impl<'de> Visitor<'de> for OptVisitor {
            type Value = Option<String>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an optional string")
            }
            fn visit_none<E: ErrorTrait>(self) -> Result<Option<String>, E> {
                Ok(None)
            }
            fn visit_some<D2>(self, deserializer: D2) -> Result<Option<String>, D2::Error>
            where
                D2: Deserializer<'de>,
            {
                struct StrVisitor;
                impl<'de> Visitor<'de> for StrVisitor {
                    type Value = String;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("a string")
                    }
                    fn visit_str<E: ErrorTrait>(self, v: &str) -> Result<String, E> {
                        Ok(v.to_string())
                    }
                }
                deserializer.deserialize_str(StrVisitor).map(Some)
            }
        }
        deserializer.deserialize_option(OptVisitor)
    }

    #[test]
    fn call_with_deserialize_forwards_an_option_and_erases_its_inner_value_too() {
        let mut de = json::Deserializer::from_str(r#""hi""#);
        let some: Option<String> =
            call_with_deserialize(&mut de, |d| deserialize_label(d)).unwrap();
        assert_eq!(some.as_deref(), Some("hi"));

        let mut de = json::Deserializer::from_str("null");
        let none: Option<String> =
            call_with_deserialize(&mut de, |d| deserialize_label(d)).unwrap();
        assert_eq!(none, None);
    }

    fn deserialize_as_seq<'de, D>(deserializer: D) -> Result<i32, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SeqVisitor;
        impl<'de> Visitor<'de> for SeqVisitor {
            type Value = i32;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a sequence")
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<i32, A::Error>
            where
                A: crate::de::SeqAccess<'de>,
            {
                Ok(seq.next_element::<i32>()?.unwrap_or_default())
            }
        }
        deserializer.deserialize_seq(SeqVisitor)
    }

    #[test]
    fn deserialize_compound_shapes_are_a_clear_error_not_a_silent_miscompile() {
        let mut de = json::Deserializer::from_str("[1]");
        let err: Result<i32, _> = call_with_deserialize(&mut de, |d| deserialize_as_seq(d));
        let err = err.unwrap_err();
        assert!(err.to_string().contains("sequence/tuple/map/struct/enum"));
    }
}
