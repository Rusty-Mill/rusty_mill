use std::collections::BTreeMap;

use rusty_serde::{json, ron, Deserialize, Serialize};

fn roundtrip<T>(value: T, expected: &str)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let encoded = ron::to_string(&value).unwrap();
    assert_eq!(encoded, expected);
    let decoded: T = ron::from_str(&encoded).unwrap();
    assert_eq!(decoded, value);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

#[test]
fn named_struct() {
    roundtrip(Point { x: 1, y: -2 }, "{x:1,y:-2}");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Wrapper(i32);

#[test]
fn newtype_struct() {
    roundtrip(Wrapper(42), "42");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Pair(i32, String);

#[test]
fn tuple_struct() {
    roundtrip(Pair(1, "two".into()), r#"[1,"two"]"#);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Unit;

#[test]
fn unit_struct() {
    roundtrip(Unit, "()");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Nested {
    point: Point,
    items: Vec<i32>,
    tag: Option<String>,
}

#[test]
fn nested_struct() {
    roundtrip(
        Nested {
            point: Point { x: 0, y: 0 },
            items: vec![1, 2, 3],
            tag: None,
        },
        "{point:{x:0,y:0},items:[1,2,3],tag:None}",
    );
    roundtrip(
        Nested {
            point: Point { x: 5, y: 6 },
            items: vec![],
            tag: Some("hi".into()),
        },
        r#"{point:{x:5,y:6},items:[],tag:Some("hi")}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Shape {
    Circle,
    Square(f64),
    Rectangle(f64, f64),
    Named { label: String, sides: u32 },
}

#[test]
fn enum_unit_variant() {
    roundtrip(Shape::Circle, "Circle");
}

#[test]
fn enum_newtype_variant() {
    roundtrip(Shape::Square(2.5), "Square(2.5)");
}

#[test]
fn enum_tuple_variant() {
    roundtrip(Shape::Rectangle(1.0, 2.0), "Rectangle[1.0,2.0]");
}

#[test]
fn enum_struct_variant() {
    roundtrip(
        Shape::Named {
            label: "tri".into(),
            sides: 3,
        },
        r#"Named{label:"tri",sides:3}"#,
    );
}

#[test]
fn primitives_and_collections() {
    roundtrip(true, "true");
    roundtrip(-7i64, "-7");
    roundtrip(7u64, "7");
    roundtrip("hello".to_string(), r#""hello""#);
    roundtrip('h', "'h'");
    roundtrip(vec![1, 2, 3], "[1,2,3]");
    roundtrip((1, "two".to_string(), 3.5), r#"[1,"two",3.5]"#);
}

#[test]
fn map_keys_are_not_restricted_to_strings() {
    // Unlike JSON, this format's map keys can be any serializable shape -
    // exercised here with integer keys, which JSON's data model would have
    // to coerce to strings.
    let mut map = BTreeMap::new();
    map.insert(1, "a".to_string());
    map.insert(2, "b".to_string());
    roundtrip(map, r#"{1:"a",2:"b"}"#);
}

#[test]
fn string_escaping() {
    roundtrip(
        "line1\nline2\t\"quoted\"\\".to_string(),
        r#""line1\nline2\t\"quoted\"\\""#,
    );
}

#[test]
fn unknown_fields_are_ignored() {
    let decoded: Point = ron::from_str("{x:1,y:2,z:99}").unwrap();
    assert_eq!(decoded, Point { x: 1, y: 2 });
    // Unknown fields can themselves be arbitrarily-shaped values.
    let decoded: Point = ron::from_str("{x:1,y:2,z:[1,Some(2),Named(2,3),{a:1}]}").unwrap();
    assert_eq!(decoded, Point { x: 1, y: 2 });
}

#[test]
fn missing_field_is_an_error() {
    let err = ron::from_str::<Point>("{x:1}").unwrap_err();
    assert!(err.to_string().contains("missing field"));
}

#[test]
fn type_mismatch_is_an_error() {
    let err = ron::from_str::<Point>(r#"{x:"nope",y:2}"#).unwrap_err();
    assert!(err.to_string().contains("invalid number"));
}

#[test]
fn trailing_garbage_is_an_error() {
    let err = ron::from_str::<i32>("1 2").unwrap_err();
    assert!(err.to_string().contains("trailing"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct GenericPoint<T> {
    x: T,
    y: T,
}

#[test]
fn generic_named_struct() {
    roundtrip(GenericPoint { x: 1, y: 2 }, "{x:1,y:2}");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum GenericEither<L, R> {
    Left(L),
    Right(R),
}

#[test]
fn generic_enum() {
    roundtrip(GenericEither::<i32, String>::Left(1), "Left(1)");
    roundtrip(
        GenericEither::<i32, String>::Right("hi".to_string()),
        r#"Right("hi")"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Attributed {
    #[rusty_serde(rename = "n")]
    name: String,
    #[rusty_serde(default)]
    count: i32,
    #[rusty_serde(skip)]
    cache: i32,
}

#[test]
fn field_attributes() {
    let value = Attributed {
        name: "hi".into(),
        count: 5,
        cache: 0,
    };
    assert_eq!(ron::to_string(&value).unwrap(), r#"{n:"hi",count:5}"#);

    let decoded: Attributed = ron::from_str(r#"{n:"hi"}"#).unwrap();
    assert_eq!(
        decoded,
        Attributed {
            name: "hi".into(),
            count: 0,
            cache: 0,
        }
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(untagged)]
enum UntaggedShape {
    Named(String),
    Point(i32, i32),
}

#[test]
fn untagged_enum() {
    roundtrip(UntaggedShape::Named("x".into()), r#""x""#);
    roundtrip(UntaggedShape::Point(1, 2), "[1,2]");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Meta {
    id: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    name: String,
    #[rusty_serde(flatten)]
    meta: Meta,
}

#[test]
fn flatten() {
    // Flattening switches the container to the generic map-entry path for
    // *all* its fields (not just the flattened one), so - unlike a plain
    // struct's bare field names - the keys come out quoted here.
    roundtrip(
        Record {
            name: "x".into(),
            meta: Meta { id: 1 },
        },
        r#"{"name":"x","id":1}"#,
    );
}

/// The same derived type, with the same value, round-trips through both
/// formats - proving `#[derive(Serialize, Deserialize)]` really doesn't
/// know or care which one it's talking to, even though the two wire
/// representations look nothing alike.
#[test]
fn same_derived_type_round_trips_through_both_formats() {
    let value = Nested {
        point: Point { x: 5, y: 6 },
        items: vec![1, 2, 3],
        tag: Some("hi".into()),
    };

    let as_json = json::to_string(&value).unwrap();
    assert_eq!(
        as_json,
        r#"{"point":{"x":5,"y":6},"items":[1,2,3],"tag":"hi"}"#
    );
    assert_eq!(json::from_str::<Nested>(&as_json).unwrap(), value);

    let as_ron = ron::to_string(&value).unwrap();
    assert_eq!(as_ron, r#"{point:{x:5,y:6},items:[1,2,3],tag:Some("hi")}"#);
    assert_eq!(ron::from_str::<Nested>(&as_ron).unwrap(), value);
}

/// Both of this crate's formats are text-based, so `is_human_readable()`
/// is `true` for each of them (the default every `Serializer`/
/// `Deserializer` impl inherits unless it opts into a compact binary
/// representation instead) - exercised here with a hand-written impl that
/// branches on it, the same way a real-world type (a timestamp, say)
/// would pick a human-editable representation vs. a compact one.
#[derive(Debug, PartialEq)]
struct HumanReadableProbe(bool);

impl Serialize for HumanReadableProbe {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: rusty_serde::Serializer,
    {
        serializer.is_human_readable().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HumanReadableProbe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: rusty_serde::Deserializer<'de>,
    {
        let human_readable = deserializer.is_human_readable();
        bool::deserialize(deserializer).map(|_| HumanReadableProbe(human_readable))
    }
}

#[test]
fn is_human_readable_is_true_for_both_of_this_crates_formats() {
    assert_eq!(json::to_string(&HumanReadableProbe(true)).unwrap(), "true");
    assert_eq!(
        json::from_str::<HumanReadableProbe>("true").unwrap(),
        HumanReadableProbe(true)
    );

    assert_eq!(ron::to_string(&HumanReadableProbe(true)).unwrap(), "true");
    assert_eq!(
        ron::from_str::<HumanReadableProbe>("true").unwrap(),
        HumanReadableProbe(true)
    );
}

/// Serializes straight from an iterator/`Display` value via
/// `collect_seq`/`collect_map`/`collect_str`, rather than collecting into
/// a `Vec`/`HashMap`/`String` first - proving those default trait methods
/// actually produce the same output `serialize_seq`/`serialize_map`/
/// `serialize_str` would.
struct Collected {
    doubled: Vec<i32>,
    named: BTreeMap<i32, i32>,
}

impl Serialize for Collected {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: rusty_serde::Serializer,
    {
        use rusty_serde::ser::SerializeTuple;
        let mut tuple = serializer.serialize_tuple(2)?;
        struct SeqField<'a>(&'a Vec<i32>);
        impl<'a> Serialize for SeqField<'a> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: rusty_serde::Serializer,
            {
                serializer.collect_seq(self.0.iter().map(|v| v * 2))
            }
        }
        struct MapField<'a>(&'a BTreeMap<i32, i32>);
        impl<'a> Serialize for MapField<'a> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: rusty_serde::Serializer,
            {
                serializer.collect_map(self.0.iter().map(|(k, v)| (*k, *v)))
            }
        }
        SerializeTuple::serialize_element(&mut tuple, &SeqField(&self.doubled))?;
        SerializeTuple::serialize_element(&mut tuple, &MapField(&self.named))?;
        SerializeTuple::end(tuple)
    }
}

#[test]
fn collect_seq_and_collect_map_match_the_manual_equivalents() {
    let mut named = BTreeMap::new();
    named.insert(1, 10);
    named.insert(2, 20);
    let value = Collected {
        doubled: vec![1, 2, 3],
        named,
    };
    assert_eq!(
        json::to_string(&value).unwrap(),
        r#"[[2,4,6],{"1":10,"2":20}]"#
    );
    assert_eq!(ron::to_string(&value).unwrap(), "[[2,4,6],{1:10,2:20}]");
}

struct Display3;
impl std::fmt::Display for Display3 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "three")
    }
}
struct CollectStr;
impl Serialize for CollectStr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: rusty_serde::Serializer,
    {
        serializer.collect_str(&Display3)
    }
}

#[test]
fn collect_str_matches_serialize_str() {
    assert_eq!(json::to_string(&CollectStr).unwrap(), r#""three""#);
    assert_eq!(ron::to_string(&CollectStr).unwrap(), r#""three""#);
}

mod remote_target {
    pub struct ForeignPoint {
        pub x: i32,
        pub y: i32,
    }
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
#[rusty_serde(remote = "remote_target::ForeignPoint")]
struct ForeignPointMirror {
    x: i32,
    y: i32,
}

#[test]
fn remote_derive_round_trips_through_ron_too() {
    let value = remote_target::ForeignPoint { x: 1, y: 2 };
    let encoded = ron::to_string(&value).unwrap();
    assert_eq!(encoded, "{x:1,y:2}");
    let decoded: remote_target::ForeignPoint = ron::from_str(&encoded).unwrap();
    assert_eq!((decoded.x, decoded.y), (value.x, value.y));
}

mod as_seconds {
    use rusty_serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Event {
    #[rusty_serde(
        serialize_with = "as_seconds::serialize",
        deserialize_with = "as_seconds::deserialize"
    )]
    elapsed: std::time::Duration,
}

#[test]
fn serialize_with_reformats_a_field_through_ron_too() {
    let value = Event {
        elapsed: std::time::Duration::from_secs(5),
    };
    assert_eq!(ron::to_string(&value).unwrap(), "{elapsed:5}");
}

#[test]
fn with_round_trips_through_ron_too() {
    let value = Event {
        elapsed: std::time::Duration::from_secs(5),
    };
    let encoded = ron::to_string(&value).unwrap();
    let decoded: Event = ron::from_str(&encoded).unwrap();
    assert_eq!(decoded, value);
}
