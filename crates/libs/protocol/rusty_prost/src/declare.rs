//! Declaring a schema as a table: [`message!`](crate::message), [`oneof!`](crate::oneof) and
//! [`open_enum!`](crate::open_enum) write the struct and the [`Message`](crate::Message) impl
//! for a proto message from one line per field, instead of an `encode` and a `merge_field` to
//! keep in step by hand.
//!
//! A field is `NUMBER name: Type = kind`, and the *kind* says how it goes on the wire:
//!
//! | kind | Rust type | on the wire |
//! |---|---|---|
//! | `string`, `bytes`, `bool`, `int32` | `String`, `Vec<u8>`, `bool`, `i32` | omitted at the default |
//! | `enumv` | `i32` | as `int32`; the enum is an [`open_enum!`](crate::open_enum) |
//! | `opt_string`, `opt_bytes`, `opt_int32`, `opt_bool` | `Option<_>` | written when `Some`, even if zero |
//! | `msg` | `Option<M>` | written when `Some` |
//! | `rep_string`, `rep_msg` | `Vec<String>`, `Vec<M>` | one occurrence per element |
//! | `map_str`, `map_msg` | `BTreeMap<String, String>`, `BTreeMap<String, M>` | one entry per key, in key order, key and value both written |
//!
//! A `oneof` is declared by [`oneof!`](crate::oneof) as an enum of its members, and held in the
//! message as `oneof name: Type;` (at most one per message, written before the numbered fields,
//! which is field-number order when the oneof's numbers are the lowest).
//!
//! ```
//! use rusty_prost::{message, Message};
//!
//! message! {
//!     /// `message Greeting { string who = 1; optional int32 times = 2; }`
//!     Greeting {
//!         1 who: String = string,
//!         2 times: Option<i32> = opt_int32,
//!     }
//! }
//!
//! let bytes = Greeting { who: "hi".into(), times: Some(0) }.encode_to_vec().unwrap();
//! assert_eq!(bytes, [0x0a, 2, b'h', b'i', 0x10, 0]);
//! assert_eq!(Greeting::decode(&bytes).unwrap().times, Some(0));
//! ```
//!
//! Every kind is checked against `prost` on the same schema (`tests/declare.rs`). One byte-level
//! difference is deliberate: a map entry always carries its key and value, as the reference
//! implementation writes it, where `prost` leaves out a key or value that is the default. Both
//! read either form.

use crate::{Field, Result, Writer};

/// A value the schema has no enum member for (see the `TryFrom<i32>` of an
/// [`open_enum!`](crate::open_enum)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownEnumValue(pub i32);

impl std::fmt::Display for UnknownEnumValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown enumeration value {}", self.0)
    }
}

impl std::error::Error for UnknownEnumValue {}

/// The members of a proto `oneof`, as declared by [`oneof!`](crate::oneof).
pub trait Oneof: Sized {
    /// Writes the member that is set.
    fn encode_into(&self, w: &mut Writer);

    /// Reads `f` into `slot` if it is a member of this oneof (the last member wins; a message
    /// member that repeats merges into the previous occurrence). Any other field is skipped.
    fn merge_into(slot: &mut Option<Self>, f: &Field<'_>) -> Result<()>;
}

/// How one field is written (see the module docs for the kinds).
#[doc(hidden)]
#[macro_export]
macro_rules! __put {
    (string, $w:ident, $n:literal, $v:expr) => {
        if !$v.is_empty() {
            $w.string($n, $v);
        }
    };
    (bytes, $w:ident, $n:literal, $v:expr) => {
        if !$v.is_empty() {
            $w.bytes($n, $v);
        }
    };
    (bool, $w:ident, $n:literal, $v:expr) => {
        if *$v {
            $w.bool($n, true);
        }
    };
    (int32, $w:ident, $n:literal, $v:expr) => {
        if *$v != 0 {
            $w.int32($n, *$v);
        }
    };
    (enumv, $w:ident, $n:literal, $v:expr) => {
        $crate::__put!(int32, $w, $n, $v)
    };
    (opt_string, $w:ident, $n:literal, $v:expr) => {
        if let Some(s) = $v {
            $w.string($n, s);
        }
    };
    (opt_bytes, $w:ident, $n:literal, $v:expr) => {
        if let Some(b) = $v {
            $w.bytes($n, b);
        }
    };
    (opt_int32, $w:ident, $n:literal, $v:expr) => {
        if let Some(i) = $v {
            $w.int32($n, *i);
        }
    };
    (opt_bool, $w:ident, $n:literal, $v:expr) => {
        if let Some(b) = $v {
            $w.bool($n, *b);
        }
    };
    (msg, $w:ident, $n:literal, $v:expr) => {
        if let Some(m) = $v {
            $w.message($n, |w| $crate::Message::encode(m, w));
        }
    };
    (rep_string, $w:ident, $n:literal, $v:expr) => {
        for s in $v {
            $w.string($n, s);
        }
    };
    (rep_msg, $w:ident, $n:literal, $v:expr) => {
        for m in $v {
            $w.message($n, |w| $crate::Message::encode(m, w));
        }
    };
    (map_str, $w:ident, $n:literal, $v:expr) => {
        for (k, v) in $v {
            $w.message($n, |w| {
                w.string(1, k);
                w.string(2, v);
            });
        }
    };
    (map_msg, $w:ident, $n:literal, $v:expr) => {
        for (k, v) in $v {
            $w.message($n, |w| {
                w.string(1, k);
                w.message(2, |w| $crate::Message::encode(v, w));
            });
        }
    };
}

/// How one occurrence of a field is read into its slot.
#[doc(hidden)]
#[macro_export]
macro_rules! __take {
    (string, $slot:expr, $f:ident) => {
        *$slot = $f.str()?.to_owned()
    };
    (bytes, $slot:expr, $f:ident) => {
        *$slot = $f.bytes()?.to_vec()
    };
    (bool, $slot:expr, $f:ident) => {
        *$slot = $f.bool()?
    };
    (int32, $slot:expr, $f:ident) => {
        *$slot = $f.int32()?
    };
    (enumv, $slot:expr, $f:ident) => {
        $crate::__take!(int32, $slot, $f)
    };
    (opt_string, $slot:expr, $f:ident) => {
        *$slot = Some($f.str()?.to_owned())
    };
    (opt_bytes, $slot:expr, $f:ident) => {
        *$slot = Some($f.bytes()?.to_vec())
    };
    (opt_int32, $slot:expr, $f:ident) => {
        *$slot = Some($f.int32()?)
    };
    (opt_bool, $slot:expr, $f:ident) => {
        *$slot = Some($f.bool()?)
    };
    (msg, $slot:expr, $f:ident) => {
        $crate::field::message($slot, &$f)?
    };
    (rep_string, $slot:expr, $f:ident) => {
        $slot.push($f.str()?.to_owned())
    };
    (rep_msg, $slot:expr, $f:ident) => {
        $crate::field::push_message($slot, &$f)?
    };
    (map_str, $slot:expr, $f:ident) => {
        $crate::field::map_entry(
            $slot,
            &$f,
            |k, f| {
                *k = f.str()?.to_owned();
                Ok(())
            },
            |v, f| {
                *v = f.str()?.to_owned();
                Ok(())
            },
        )?
    };
    (map_msg, $slot:expr, $f:ident) => {
        $crate::field::map_entry(
            $slot,
            &$f,
            |k, f| {
                *k = f.str()?.to_owned();
                Ok(())
            },
            |v, f| $crate::field::merge_message(v, f),
        )?
    };
}

/// A message: the struct, and the [`Message`](crate::Message) impl that reads and writes it.
/// `oneof name: Type;` (at most one, first) holds a `oneof` declared with
/// [`oneof!`](crate::oneof) as `Option<Type>`. See the [module docs](crate::declare) for
/// the field kinds.
#[macro_export]
macro_rules! message {
    (
        $(#[$meta:meta])*
        $name:ident {
            $(oneof $of:ident : $oty:ty;)?
            $($num:literal $field:ident : $ty:ty = $kind:ident),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default, Clone, PartialEq)]
        pub struct $name {
            $(
                #[allow(missing_docs)]
                pub $of: Option<$oty>,
            )?
            $(
                #[allow(missing_docs)]
                pub $field: $ty,
            )*
        }

        impl $crate::Message for $name {
            #[allow(unused_variables)]
            fn encode(&self, w: &mut $crate::Writer) {
                $(
                    if let Some(v) = &self.$of {
                        $crate::Oneof::encode_into(v, w);
                    }
                )?
                $( $crate::__put!($kind, w, $num, &self.$field); )*
            }

            #[allow(unused_variables)]
            fn merge_field(&mut self, f: $crate::Field<'_>) -> $crate::Result<()> {
                match f.number {
                    $( $num => { $crate::__take!($kind, &mut self.$field, f); } )*
                    _ => {
                        $(
                            <$oty as $crate::Oneof>::merge_into(&mut self.$of, &f)?;
                        )?
                    }
                }
                Ok(())
            }
        }
    };
}

/// How one member of a oneof is written.
#[doc(hidden)]
#[macro_export]
macro_rules! __oput {
    (string, $w:ident, $n:literal, $v:expr) => {
        $w.string($n, $v)
    };
    (bytes, $w:ident, $n:literal, $v:expr) => {
        $w.bytes($n, $v)
    };
    (msg, $w:ident, $n:literal, $v:expr) => {
        $w.message($n, |w| $crate::Message::encode($v, w))
    };
}

/// How one member of a oneof is read. A message member that repeats merges into the previous
/// occurrence when it is the same member, as the reference does.
#[doc(hidden)]
#[macro_export]
macro_rules! __otake {
    (string, $slot:ident, $var:path, $f:ident) => {
        *$slot = Some($var($f.str()?.to_owned()))
    };
    (bytes, $slot:ident, $var:path, $f:ident) => {
        *$slot = Some($var($f.bytes()?.to_vec()))
    };
    (msg, $slot:ident, $var:path, $f:ident) => {{
        let mut m = match $slot.take() {
            Some($var(existing)) => existing,
            _ => Default::default(),
        };
        $crate::field::merge_message(&mut m, $f)?;
        *$slot = Some($var(m));
    }};
}

/// The members of a proto `oneof`: an enum, with the [`Oneof`] impl that [`message!`] calls.
/// A member is `NUMBER Variant(Type) = kind`, with kind `string`, `bytes` or `msg`.
#[macro_export]
macro_rules! oneof {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($num:literal $var:ident($ty:ty) = $kind:ident),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $name {
            $(
                #[allow(missing_docs)]
                $var($ty),
            )*
        }

        impl $crate::Oneof for $name {
            fn encode_into(&self, w: &mut $crate::Writer) {
                match self {
                    $( Self::$var(v) => $crate::__oput!($kind, w, $num, v), )*
                }
            }

            fn merge_into(
                slot: &mut Option<Self>,
                f: &$crate::Field<'_>,
            ) -> $crate::Result<()> {
                match f.number {
                    $( $num => { $crate::__otake!($kind, slot, Self::$var, f); } )*
                    _ => {}
                }
                Ok(())
            }
        }
    };
}

/// A proto enum, held in message fields as its `i32` (so a member added by a newer peer
/// survives); `TryFrom<i32>` tells the known ones apart. A member is
/// `Variant = NUMBER: "PROTO_NAME"`.
#[macro_export]
macro_rules! open_enum {
    ($(#[$meta:meta])* $name:ident { $($var:ident = $num:literal : $str:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(i32)]
        pub enum $name {
            $(
                #[allow(missing_docs)]
                $var = $num,
            )*
        }

        impl $name {
            /// The name in the `.proto` (`TASK_STATE_WORKING`).
            pub fn as_str_name(&self) -> &'static str {
                match self {
                    $( Self::$var => $str, )*
                }
            }

            /// The member named `name` in the `.proto`.
            pub fn from_str_name(name: &str) -> Option<Self> {
                match name {
                    $( $str => Some(Self::$var), )*
                    _ => None,
                }
            }
        }

        impl TryFrom<i32> for $name {
            type Error = $crate::UnknownEnumValue;

            fn try_from(value: i32) -> Result<Self, $crate::UnknownEnumValue> {
                match value {
                    $( $num => Ok(Self::$var), )*
                    other => Err($crate::UnknownEnumValue(other)),
                }
            }
        }
    };
}
