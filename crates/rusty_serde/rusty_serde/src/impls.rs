//! `Serialize`/`Deserialize` impls for everything in `core`/`std` that the
//! data model can represent directly, so `#[derive(...)]` only has to worry
//! about the shape of the user's own type.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::Hash;

use crate::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use crate::ser::{Serialize, SerializeMap, SerializeSeq, SerializeTuple, Serializer};

macro_rules! impl_serialize_for_int {
    ($($ty:ty => $method:ident as $via:ty),* $(,)?) => {
        $(
            impl Serialize for $ty {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    serializer.$method(*self as $via)
                }
            }
        )*
    };
}

impl_serialize_for_int! {
    i8 => serialize_i8 as i8,
    i16 => serialize_i16 as i16,
    i32 => serialize_i32 as i32,
    i64 => serialize_i64 as i64,
    isize => serialize_i64 as i64,
    u8 => serialize_u8 as u8,
    u16 => serialize_u16 as u16,
    u32 => serialize_u32 as u32,
    u64 => serialize_u64 as u64,
    usize => serialize_u64 as u64,
}

impl Serialize for bool {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bool(*self)
    }
}

impl Serialize for f32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f32(*self)
    }
}

impl Serialize for f64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(*self)
    }
}

impl Serialize for char {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_char(*self)
    }
}

impl Serialize for str {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self)
    }
}

impl Serialize for String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self)
    }
}

impl Serialize for Cow<'_, str> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self)
    }
}

impl Serialize for () {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_unit()
    }
}

impl<T> Serialize for Option<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Some(value) => serializer.serialize_some(value),
            None => serializer.serialize_none(),
        }
    }
}

impl<T> Serialize for Box<T>
where
    T: Serialize + ?Sized,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (**self).serialize(serializer)
    }
}

impl<T> Serialize for [T]
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for element in self {
            seq.serialize_element(element)?;
        }
        seq.end()
    }
}

impl<T> Serialize for Vec<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<T> Serialize for &T
where
    T: Serialize + ?Sized,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (**self).serialize(serializer)
    }
}

impl<K, V> Serialize for HashMap<K, V>
where
    K: Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<K, V> Serialize for BTreeMap<K, V>
where
    K: Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

// ---- Deserialize ----

macro_rules! impl_deserialize_for_int {
    ($($ty:ty => $deserialize_method:ident),* $(,)?) => {
        $(
            impl<'de> Deserialize<'de> for $ty {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    struct IntVisitor;
                    impl<'de> Visitor<'de> for IntVisitor {
                        type Value = $ty;
                        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                            write!(f, "a {}", stringify!($ty))
                        }
                        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
                        where
                            E: crate::error::Error,
                        {
                            Ok(v as $ty)
                        }
                        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
                        where
                            E: crate::error::Error,
                        {
                            Ok(v as $ty)
                        }
                    }
                    deserializer.$deserialize_method(IntVisitor)
                }
            }
        )*
    };
}

impl_deserialize_for_int! {
    i8 => deserialize_i8,
    i16 => deserialize_i16,
    i32 => deserialize_i32,
    i64 => deserialize_i64,
    isize => deserialize_i64,
    u8 => deserialize_u8,
    u16 => deserialize_u16,
    u32 => deserialize_u32,
    u64 => deserialize_u64,
    usize => deserialize_u64,
}

impl<'de> Deserialize<'de> for bool {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BoolVisitor;
        impl<'de> Visitor<'de> for BoolVisitor {
            type Value = bool;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a boolean")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v)
            }
        }
        deserializer.deserialize_bool(BoolVisitor)
    }
}

impl<'de> Deserialize<'de> for f32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct F32Visitor;
        impl<'de> Visitor<'de> for F32Visitor {
            type Value = f32;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a f32")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v as f32)
            }
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v as f32)
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v as f32)
            }
        }
        deserializer.deserialize_f32(F32Visitor)
    }
}

impl<'de> Deserialize<'de> for f64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct F64Visitor;
        impl<'de> Visitor<'de> for F64Visitor {
            type Value = f64;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a f64")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v)
            }
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v as f64)
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v as f64)
            }
        }
        deserializer.deserialize_f64(F64Visitor)
    }
}

impl<'de> Deserialize<'de> for char {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CharVisitor;
        impl<'de> Visitor<'de> for CharVisitor {
            type Value = char;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a single character")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                let mut chars = v.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Ok(c),
                    _ => Err(E::custom("expected a single character")),
                }
            }
        }
        deserializer.deserialize_char(CharVisitor)
    }
}

impl<'de> Deserialize<'de> for String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StringVisitor;
        impl<'de> Visitor<'de> for StringVisitor {
            type Value = String;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a string")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v.to_owned())
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v)
            }
        }
        deserializer.deserialize_string(StringVisitor)
    }
}

// `'de: 'a` (not just `impl<'de> ... for &'de str`) is what lets this
// satisfy a field of type `&'a str` on a struct whose own lifetime `'a` is
// distinct from - but outlived by - the deserializer's `'de`, which is the
// shape `#[derive(Deserialize)]` generates for any type with its own
// lifetime parameter.
impl<'de: 'a, 'a> Deserialize<'de> for &'a str {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrVisitor;
        impl<'de> Visitor<'de> for StrVisitor {
            type Value = &'de str;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a borrowed string")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(v)
            }
        }
        deserializer.deserialize_str(StrVisitor)
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Cow<'a, str> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CowStrVisitor;
        impl<'de> Visitor<'de> for CowStrVisitor {
            type Value = Cow<'de, str>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a string")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(Cow::Borrowed(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(Cow::Owned(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(Cow::Owned(v))
            }
        }
        deserializer.deserialize_str(CowStrVisitor)
    }
}

impl<'de> Deserialize<'de> for () {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UnitVisitor;
        impl<'de> Visitor<'de> for UnitVisitor {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "unit")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(())
            }
        }
        deserializer.deserialize_unit(UnitVisitor)
    }
}

impl<'de, T> Deserialize<'de> for Option<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OptionVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T> Visitor<'de> for OptionVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = Option<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "an optional value")
            }
            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: crate::error::Error,
            {
                Ok(None)
            }
            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                T::deserialize(deserializer).map(Some)
            }
        }
        deserializer.deserialize_option(OptionVisitor(std::marker::PhantomData))
    }
}

impl<'de, T> Deserialize<'de> for Box<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Box::new)
    }
}

impl<'de, T> Deserialize<'de> for Vec<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VecVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T> Visitor<'de> for VecVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a sequence")
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(value) = seq.next_element::<T>()? {
                    vec.push(value);
                }
                Ok(vec)
            }
        }
        deserializer.deserialize_seq(VecVisitor(std::marker::PhantomData))
    }
}

impl<'de, K, V> Deserialize<'de> for HashMap<K, V>
where
    K: Deserialize<'de> + Eq + Hash,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor<K, V>(std::marker::PhantomData<(K, V)>);
        impl<'de, K, V> Visitor<'de> for MapVisitor<K, V>
        where
            K: Deserialize<'de> + Eq + Hash,
            V: Deserialize<'de>,
        {
            type Value = HashMap<K, V>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a map")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = HashMap::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((k, v)) = map.next_entry::<K, V>()? {
                    out.insert(k, v);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(MapVisitor(std::marker::PhantomData))
    }
}

impl<'de, K, V> Deserialize<'de> for BTreeMap<K, V>
where
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor<K, V>(std::marker::PhantomData<(K, V)>);
        impl<'de, K, V> Visitor<'de> for MapVisitor<K, V>
        where
            K: Deserialize<'de> + Ord,
            V: Deserialize<'de>,
        {
            type Value = BTreeMap<K, V>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a map")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some((k, v)) = map.next_entry::<K, V>()? {
                    out.insert(k, v);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(MapVisitor(std::marker::PhantomData))
    }
}

// ---- Tuples ----

macro_rules! tuple_impls {
    ($($len:expr => ($($n:tt $ty:ident),+))+) => {
        $(
            #[allow(non_snake_case)]
            impl<$($ty),+> Serialize for ($($ty,)+)
            where
                $($ty: Serialize,)+
            {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    let mut tuple = serializer.serialize_tuple($len)?;
                    $(tuple.serialize_element(&self.$n)?;)+
                    tuple.end()
                }
            }

            #[allow(non_snake_case)]
            impl<'de, $($ty),+> Deserialize<'de> for ($($ty,)+)
            where
                $($ty: Deserialize<'de>,)+
            {
                fn deserialize<De>(deserializer: De) -> Result<Self, De::Error>
                where
                    De: Deserializer<'de>,
                {
                    struct TupleVisitor<$($ty),+>(std::marker::PhantomData<($($ty,)+)>);
                    impl<'de, $($ty),+> Visitor<'de> for TupleVisitor<$($ty),+>
                    where
                        $($ty: Deserialize<'de>,)+
                    {
                        type Value = ($($ty,)+);
                        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                            write!(f, "a tuple of size {}", $len)
                        }
                        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                        where
                            A: SeqAccess<'de>,
                        {
                            $(
                                let $ty = seq
                                    .next_element::<$ty>()?
                                    .ok_or_else(|| crate::error::Error::custom("tuple had too few elements"))?;
                            )+
                            Ok(($($ty,)+))
                        }
                    }
                    deserializer.deserialize_tuple($len, TupleVisitor(std::marker::PhantomData))
                }
            }
        )+
    };
}

tuple_impls! {
    1 => (0 T0)
    2 => (0 T0, 1 T1)
    3 => (0 T0, 1 T1, 2 T2)
    4 => (0 T0, 1 T1, 2 T2, 3 T3)
    5 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4)
    6 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5)
    7 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6)
    8 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7)
}
