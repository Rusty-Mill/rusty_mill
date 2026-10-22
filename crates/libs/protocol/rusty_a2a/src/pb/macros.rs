//! The two declaration macros behind [`super`]: a message is a struct plus the `Message` impl
//! that `spec/a2a.proto` dictates, written once as a field table. A field's *kind* names how it
//! goes on the wire (`string`, `msg`, `rep_msg`, `map_msg`, ...); the table is the proto.

/// How one field is written. Proto3 implicit-presence scalars are omitted at their default;
/// `opt_*` and `msg` are written when `Some`; map entries always write both key and value.
macro_rules! put {
    (string, $w:ident, $n:literal, $v:expr) => {
        if !$v.is_empty() {
            $w.string($n, $v);
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
        put!(int32, $w, $n, $v)
    };
    (opt_string, $w:ident, $n:literal, $v:expr) => {
        if let Some(s) = $v {
            $w.string($n, s);
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
            $w.message($n, |w| rusty_prost::Message::encode(m, w));
        }
    };
    (rep_string, $w:ident, $n:literal, $v:expr) => {
        for s in $v {
            $w.string($n, s);
        }
    };
    (rep_msg, $w:ident, $n:literal, $v:expr) => {
        for m in $v {
            $w.message($n, |w| rusty_prost::Message::encode(m, w));
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
                w.message(2, |w| rusty_prost::Message::encode(v, w));
            });
        }
    };
}

/// How one occurrence of a field is read into its slot.
macro_rules! take {
    (string, $slot:expr, $f:ident) => {
        *$slot = $f.str()?.to_owned()
    };
    (bool, $slot:expr, $f:ident) => {
        *$slot = $f.bool()?
    };
    (int32, $slot:expr, $f:ident) => {
        *$slot = $f.int32()?
    };
    (enumv, $slot:expr, $f:ident) => {
        take!(int32, $slot, $f)
    };
    (opt_string, $slot:expr, $f:ident) => {
        *$slot = Some($f.str()?.to_owned())
    };
    (opt_int32, $slot:expr, $f:ident) => {
        *$slot = Some($f.int32()?)
    };
    (opt_bool, $slot:expr, $f:ident) => {
        *$slot = Some($f.bool()?)
    };
    (msg, $slot:expr, $f:ident) => {
        rusty_prost::field::message($slot, &$f)?
    };
    (rep_string, $slot:expr, $f:ident) => {
        $slot.push($f.str()?.to_owned())
    };
    (rep_msg, $slot:expr, $f:ident) => {
        rusty_prost::field::push_message($slot, &$f)?
    };
    (map_str, $slot:expr, $f:ident) => {
        rusty_prost::field::map_entry(
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
        rusty_prost::field::map_entry(
            $slot,
            &$f,
            |k, f| {
                *k = f.str()?.to_owned();
                Ok(())
            },
            |v, f| rusty_prost::field::merge_message(v, f),
        )?
    };
}

/// A message: the struct, and the `Message` impl that reads and writes it. `oneof name: Type;`
/// (at most one, first) is a proto `oneof`, held as `Option<Type>`, with `Type` from [`oneof!`].
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

        impl rusty_prost::Message for $name {
            #[allow(unused_variables)]
            fn encode(&self, w: &mut rusty_prost::Writer) {
                $(
                    if let Some(v) = &self.$of {
                        v.encode_into(w);
                    }
                )?
                $( put!($kind, w, $num, &self.$field); )*
            }

            #[allow(unused_variables)]
            fn merge_field(&mut self, f: rusty_prost::Field<'_>) -> rusty_prost::Result<()> {
                match f.number {
                    $( $num => { take!($kind, &mut self.$field, f); } )*
                    _ => {
                        $(
                            <$oty>::merge_into(&mut self.$of, &f)?;
                        )?
                    }
                }
                Ok(())
            }
        }
    };
}

/// How one member of a oneof is written and read. A repeated message member merges into the
/// previous occurrence when it is the same member, as the reference does.
macro_rules! oput {
    (string, $w:ident, $n:literal, $v:expr) => {
        $w.string($n, $v)
    };
    (bytes, $w:ident, $n:literal, $v:expr) => {
        $w.bytes($n, $v)
    };
    (msg, $w:ident, $n:literal, $v:expr) => {
        $w.message($n, |w| rusty_prost::Message::encode($v, w))
    };
}

macro_rules! otake {
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
        rusty_prost::field::merge_message(&mut m, $f)?;
        *$slot = Some($var(m));
    }};
}

/// A oneof's members; the enum, and what [`message!`] calls to write and read it.
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

        impl $name {
            pub(super) fn encode_into(&self, w: &mut rusty_prost::Writer) {
                match self {
                    $( Self::$var(v) => oput!($kind, w, $num, v), )*
                }
            }

            /// Reads `f` if it is a member of this oneof; any other field is skipped.
            pub(super) fn merge_into(
                slot: &mut Option<Self>,
                f: &rusty_prost::Field<'_>,
            ) -> rusty_prost::Result<()> {
                match f.number {
                    $( $num => { otake!($kind, slot, Self::$var, f); } )*
                    _ => {}
                }
                Ok(())
            }
        }
    };
}
