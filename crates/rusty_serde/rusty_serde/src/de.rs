//! The deserialization half of the data model.
//!
//! Deserialization is push-based rather than pull-based: a [`Deserializer`]
//! inspects the input and calls the matching method on a [`Visitor`], which
//! builds the target value. This is what lets a self-describing format like
//! JSON hand back whatever shape it actually finds (`deserialize_any`) while
//! a non-self-describing format could instead trust the `deserialize_*` hint
//! the caller asked for.

use std::fmt;

use crate::error::Error;

/// A type that can be built by visiting a [`Deserializer`].
pub trait Deserialize<'de>: Sized {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}

/// A format capable of driving a [`Visitor`] over its input.
pub trait Deserializer<'de>: Sized {
    type Error: Error;

    /// The deserialize-side counterpart to [`Serializer::is_human_readable`](crate::ser::Serializer::is_human_readable) -
    /// whether the input is text-based/human-editable (JSON, this crate's
    /// RON-like format) or not. Defaults to `true`; a binary format would
    /// override it to `false`.
    fn is_human_readable(&self) -> bool {
        true
    }

    /// Figure out what's in the input without a type hint (JSON, for
    /// example, can always do this - the token itself tells you the shape).
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_unit_struct<V>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_newtype_struct<V>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_tuple_struct<V>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_struct<V>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_enum<V>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;

    /// Deserializes an "internally tagged" enum: `{"<tag>": "Variant",
    /// ...fields}` rather than `{"Variant": ...fields}`. The variant tag
    /// can appear anywhere in the object, which - unlike every other method
    /// on this trait - generally requires buffering the whole value before
    /// any of it can be handed to `visitor`. Formats that can't support
    /// that (or haven't implemented it) can leave this at its default,
    /// which just reports it as unsupported; `rusty_serde`'s JSON format
    /// overrides it.
    fn deserialize_internally_tagged_enum<V>(
        self,
        name: &'static str,
        tag: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let _ = (name, tag, variants, visitor);
        Err(Self::Error::custom(
            "this deserializer does not support internally tagged enums",
        ))
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
}

/// A "builder" that a [`Deserializer`] calls into once it knows what shape
/// the input actually is. Every method has a default that reports a type
/// mismatch, so a `Visitor` only needs to implement the handful it expects.
pub trait Visitor<'de>: Sized {
    type Value;

    /// A human-readable description of what this visitor accepts, used in
    /// "invalid type" error messages.
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result;

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Bool(v), &self))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Signed(v), &self))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Unsigned(v), &self))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Float(v), &self))
    }

    fn visit_char<E>(self, v: char) -> Result<Self::Value, E>
    where
        E: Error,
    {
        let mut buf = [0u8; 4];
        self.visit_str(v.encode_utf8(&mut buf))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Str(v), &self))
    }

    /// Like [`visit_str`](Self::visit_str), but `v` is borrowed from the
    /// input the [`Deserializer`] is reading rather than a temporary owned
    /// by the deserializer itself - a format calls this instead of
    /// `visit_str`/`visit_string` when it can hand back a slice that lives
    /// as long as `'de` (e.g. a JSON string with no escapes). Defaults to
    /// `visit_str`, so a visitor that doesn't care about zero-copy doesn't
    /// need to implement this separately; `&'de str`/`Cow<'de, str>`
    /// override it to actually borrow instead of allocating.
    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        self.visit_str(v)
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: Error,
    {
        self.visit_str(&v)
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Bytes(v), &self))
    }

    /// Borrowed-input counterpart to [`visit_bytes`](Self::visit_bytes), the
    /// same way [`visit_borrowed_str`](Self::visit_borrowed_str) is to
    /// `visit_str`.
    fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
    where
        E: Error,
    {
        self.visit_bytes(v)
    }

    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
    where
        E: Error,
    {
        self.visit_bytes(&v)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Option, &self))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = deserializer;
        Err(Error::custom("invalid type: some, expected a value"))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Err(invalid_type(&Unexpected::Unit, &self))
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = deserializer;
        Err(Error::custom("invalid type: newtype struct"))
    }

    fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let _ = seq;
        Err(Error::custom("invalid type: sequence"))
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let _ = map;
        Err(Error::custom("invalid type: map"))
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let _ = data;
        Err(Error::custom("invalid type: enum"))
    }
}

/// Accepts any single value and throws it away. Used by generated
/// `#[derive(Deserialize)]` code (and formats) to skip over fields/entries
/// the target type doesn't care about.
pub struct IgnoredAny;

impl<'de> Deserialize<'de> for IgnoredAny {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IgnoredAnyVisitor;

        impl<'de> Visitor<'de> for IgnoredAnyVisitor {
            type Value = IgnoredAny;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "anything at all")
            }
            fn visit_bool<E>(self, _v: bool) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_i64<E>(self, _v: i64) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_u64<E>(self, _v: u64) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_f64<E>(self, _v: f64) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_str<E>(self, _v: &str) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_bytes<E>(self, _v: &[u8]) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_none<E>(self) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_some<D>(self, deserializer: D) -> Result<IgnoredAny, D::Error>
            where
                D: Deserializer<'de>,
            {
                IgnoredAny::deserialize(deserializer)
            }
            fn visit_unit<E>(self) -> Result<IgnoredAny, E> {
                Ok(IgnoredAny)
            }
            fn visit_newtype_struct<D>(self, deserializer: D) -> Result<IgnoredAny, D::Error>
            where
                D: Deserializer<'de>,
            {
                IgnoredAny::deserialize(deserializer)
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<IgnoredAny, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(IgnoredAny)
            }
            fn visit_map<A>(self, mut map: A) -> Result<IgnoredAny, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(IgnoredAny)
            }
        }

        deserializer.deserialize_ignored_any(IgnoredAnyVisitor)
    }
}

pub trait SeqAccess<'de> {
    type Error: Error;

    fn next_element<T>(&mut self) -> Result<Option<T>, Self::Error>
    where
        T: Deserialize<'de>;

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

pub trait MapAccess<'de> {
    type Error: Error;

    fn next_key<K>(&mut self) -> Result<Option<K>, Self::Error>
    where
        K: Deserialize<'de>;

    fn next_value<V>(&mut self) -> Result<V, Self::Error>
    where
        V: Deserialize<'de>;

    fn next_entry<K, V>(&mut self) -> Result<Option<(K, V)>, Self::Error>
    where
        K: Deserialize<'de>,
        V: Deserialize<'de>,
    {
        match self.next_key::<K>()? {
            Some(key) => Ok(Some((key, self.next_value::<V>()?))),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

pub trait EnumAccess<'de>: Sized {
    type Error: Error;
    type Variant: VariantAccess<'de, Error = Self::Error>;

    fn variant<V>(self) -> Result<(V, Self::Variant), Self::Error>
    where
        V: Deserialize<'de>;
}

pub trait VariantAccess<'de>: Sized {
    type Error: Error;

    fn unit_variant(self) -> Result<(), Self::Error>;
    fn newtype_variant<T>(self) -> Result<T, Self::Error>
    where
        T: Deserialize<'de>;
    fn tuple_variant<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
    fn struct_variant<V>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>;
}

/// Value actually found, used to build "invalid type" error messages.
enum Unexpected<'a> {
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Str(&'a str),
    Bytes(&'a [u8]),
    Unit,
    Option,
}

impl fmt::Display for Unexpected<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Unexpected::Bool(v) => write!(f, "boolean `{v}`"),
            Unexpected::Signed(v) => write!(f, "integer `{v}`"),
            Unexpected::Unsigned(v) => write!(f, "integer `{v}`"),
            Unexpected::Float(v) => write!(f, "float `{v}`"),
            Unexpected::Str(v) => write!(f, "string {v:?}"),
            Unexpected::Bytes(v) => write!(f, "bytes {v:?}"),
            Unexpected::Unit => write!(f, "unit value"),
            Unexpected::Option => write!(f, "option value"),
        }
    }
}

struct WithExpecting<'a, V: ?Sized>(&'a V);

impl<'a, 'de, V> fmt::Display for WithExpecting<'a, V>
where
    V: Visitor<'de>,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.expecting(f)
    }
}

/// Implements a batch of `Deserializer` methods by forwarding straight to
/// `deserialize_any`, for formats/helpers where every shape collapses to
/// the same handling (e.g. a deserializer that only ever hands back a
/// single string, used to feed a generated field/variant identifier enum).
#[macro_export]
macro_rules! forward_to_deserialize_any {
    ($($method:ident)*) => {
        $(
            $crate::forward_to_deserialize_any!(@method $method);
        )*
    };
    (@method deserialize_unit_struct) => {
        fn deserialize_unit_struct<V>(
            self,
            _name: &'static str,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method deserialize_newtype_struct) => {
        fn deserialize_newtype_struct<V>(
            self,
            _name: &'static str,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method deserialize_tuple) => {
        fn deserialize_tuple<V>(
            self,
            _len: usize,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method deserialize_tuple_struct) => {
        fn deserialize_tuple_struct<V>(
            self,
            _name: &'static str,
            _len: usize,
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method deserialize_struct) => {
        fn deserialize_struct<V>(
            self,
            _name: &'static str,
            _fields: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method deserialize_enum) => {
        fn deserialize_enum<V>(
            self,
            _name: &'static str,
            _variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
    (@method $method:ident) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: $crate::de::Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
}

fn invalid_type<'de, E, V>(unexpected: &Unexpected, visitor: &V) -> E
where
    E: Error,
    V: Visitor<'de>,
{
    E::custom(format_args!(
        "invalid type: {unexpected}, expected {}",
        WithExpecting(visitor)
    ))
}
