/// Builds a [`crate::Value`] using JSON literal syntax embedded in Rust
/// code: `null`, `true`/`false`, numbers, string literals, `[...]` arrays,
/// and `{ "key": value, ... }` objects (object keys must be string
/// literals). Any other expression is converted via `Value::from`.
///
/// ```
/// use rusty_json::json;
/// let v = json!({
///     "code": 200,
///     "success": true,
///     "payload": {
///         "features": ["serde", "json", null]
///     }
/// });
/// assert_eq!(v["code"], 200);
/// ```
#[macro_export]
macro_rules! json {
    ($($json:tt)+) => {
        $crate::__json_value!($($json)+)
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __json_value {
    //////////////////////////////////////////////////////////////////////
    // Array munching: accumulate parsed elements in `[$($elems:expr,)*]`,
    // consuming one JSON element (plus its trailing comma, if any) from
    // the remaining input on each recursive step.
    //////////////////////////////////////////////////////////////////////

    (@array [$($elems:expr,)*]) => {
        $crate::__json_vec![$($elems,)*]
    };

    (@array [$($elems:expr),*]) => {
        $crate::__json_vec![$($elems),*]
    };

    (@array [$($elems:expr,)*] null $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!(null)] $($rest)*)
    };

    (@array [$($elems:expr,)*] true $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!(true)] $($rest)*)
    };

    (@array [$($elems:expr,)*] false $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!(false)] $($rest)*)
    };

    (@array [$($elems:expr,)*] [$($array:tt)*] $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!([$($array)*])] $($rest)*)
    };

    (@array [$($elems:expr,)*] {$($object:tt)*} $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!({$($object)*})] $($rest)*)
    };

    (@array [$($elems:expr,)*] $next:expr, $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!($next),] $($rest)*)
    };

    (@array [$($elems:expr,)*] $last:expr) => {
        $crate::__json_value!(@array [$($elems,)* $crate::__json_value!($last)])
    };

    (@array [$($elems:expr),*] , $($rest:tt)*) => {
        $crate::__json_value!(@array [$($elems,)*] $($rest)*)
    };

    //////////////////////////////////////////////////////////////////////
    // Object munching: `$map` accumulates entries directly (via `insert`
    // statements). `($($key:tt)*)` holds the current key once found;
    // the two trailing `(...)` groups are "current remaining input" and
    // a copy of it (the copy is only used to report a nicer error token
    // on malformed input, mirroring serde_json's own implementation).
    //////////////////////////////////////////////////////////////////////

    (@object $map:ident () () ()) => {};

    (@object $map:ident [$($key:tt)+] ($value:expr) , $($rest:tt)*) => {
        let _ = $map.insert(($($key)+).into(), $value);
        $crate::__json_value!(@object $map () ($($rest)*) ($($rest)*));
    };

    (@object $map:ident [$($key:tt)+] ($value:expr)) => {
        let _ = $map.insert(($($key)+).into(), $value);
    };

    (@object $map:ident ($($key:tt)+) (: null $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!(null)) $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: true $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!(true)) $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: false $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!(false)) $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: [$($array:tt)*] $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!([$($array)*])) $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: {$($object:tt)*} $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!({$($object)*})) $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: $value:expr , $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!($value)) , $($rest)*);
    };

    (@object $map:ident ($($key:tt)+) (: $value:expr) $copy:tt) => {
        $crate::__json_value!(@object $map [$($key)+] ($crate::__json_value!($value)));
    };

    (@object $map:ident () ($key:literal : $($rest:tt)*) $copy:tt) => {
        $crate::__json_value!(@object $map ($key) (: $($rest)*) (: $($rest)*));
    };

    //////////////////////////////////////////////////////////////////////
    // Top level: no leading `@array`/`@object` munching tag.
    //////////////////////////////////////////////////////////////////////

    (null) => {
        $crate::Value::Null
    };

    (true) => {
        $crate::Value::Bool(true)
    };

    (false) => {
        $crate::Value::Bool(false)
    };

    ([]) => {
        $crate::Value::Array($crate::__json_vec![])
    };

    ([ $($tt:tt)+ ]) => {
        $crate::Value::Array($crate::__json_value!(@array [] $($tt)+))
    };

    ({}) => {
        $crate::Value::Object($crate::Map::new())
    };

    ({ $($tt:tt)+ }) => {
        $crate::Value::Object({
            let mut object = $crate::Map::new();
            $crate::__json_value!(@object object () ($($tt)+) ($($tt)+));
            object
        })
    };

    ($other:expr) => {
        $crate::Value::from($other)
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __json_vec {
    ($($content:tt)*) => {
        $crate::__private::vec![$($content)*]
    };
}
