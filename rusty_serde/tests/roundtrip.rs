use std::collections::BTreeMap;

use rusty_serde::{json, Deserialize, Serialize};

fn roundtrip<T>(value: T, expected_json: &str)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let encoded = json::to_string(&value).unwrap();
    assert_eq!(encoded, expected_json);
    let decoded: T = json::from_str(&encoded).unwrap();
    assert_eq!(decoded, value);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

#[test]
fn named_struct() {
    roundtrip(Point { x: 1, y: -2 }, r#"{"x":1,"y":-2}"#);
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
    roundtrip(Unit, "null");
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
        r#"{"point":{"x":0,"y":0},"items":[1,2,3],"tag":null}"#,
    );
    roundtrip(
        Nested {
            point: Point { x: 5, y: 6 },
            items: vec![],
            tag: Some("hi".into()),
        },
        r#"{"point":{"x":5,"y":6},"items":[],"tag":"hi"}"#,
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
    roundtrip(Shape::Circle, r#""Circle""#);
}

#[test]
fn enum_newtype_variant() {
    roundtrip(Shape::Square(2.5), r#"{"Square":2.5}"#);
}

#[test]
fn enum_tuple_variant() {
    roundtrip(Shape::Rectangle(1.0, 2.0), r#"{"Rectangle":[1.0,2.0]}"#);
}

#[test]
fn enum_struct_variant() {
    roundtrip(
        Shape::Named {
            label: "tri".into(),
            sides: 3,
        },
        r#"{"Named":{"label":"tri","sides":3}}"#,
    );
}

#[test]
fn primitives_and_collections() {
    roundtrip(true, "true");
    roundtrip(-7i64, "-7");
    roundtrip(7u64, "7");
    roundtrip("hello".to_string(), r#""hello""#);
    roundtrip(vec![1, 2, 3], "[1,2,3]");
    roundtrip((1, "two".to_string(), 3.5), r#"[1,"two",3.5]"#);

    let mut map = BTreeMap::new();
    map.insert("a".to_string(), 1);
    map.insert("b".to_string(), 2);
    roundtrip(map, r#"{"a":1,"b":2}"#);
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
    let decoded: Point = json::from_str(r#"{"x":1,"y":2,"z":99}"#).unwrap();
    assert_eq!(decoded, Point { x: 1, y: 2 });
}

#[test]
fn missing_field_is_an_error() {
    let err = json::from_str::<Point>(r#"{"x":1}"#).unwrap_err();
    assert!(err.to_string().contains("missing field"));
}

#[test]
fn type_mismatch_is_an_error() {
    let err = json::from_str::<Point>(r#"{"x":"nope","y":2}"#).unwrap_err();
    assert!(err.to_string().contains("invalid number"));
}

#[test]
fn trailing_garbage_is_an_error() {
    let err = json::from_str::<i32>("1 2").unwrap_err();
    assert!(err.to_string().contains("trailing"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct GenericPoint<T> {
    x: T,
    y: T,
}

#[test]
fn generic_named_struct() {
    roundtrip(GenericPoint { x: 1, y: 2 }, r#"{"x":1,"y":2}"#);
    roundtrip(
        GenericPoint {
            x: "a".to_string(),
            y: "b".to_string(),
        },
        r#"{"x":"a","y":"b"}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct GenericWrapper<T>(T);

#[test]
fn generic_newtype_struct() {
    roundtrip(GenericWrapper(vec![1, 2, 3]), "[1,2,3]");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct GenericPair<A, B>(A, B);

#[test]
fn generic_tuple_struct() {
    roundtrip(GenericPair(1, "two".to_string()), r#"[1,"two"]"#);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum GenericEither<L, R> {
    Left(L),
    Right(R),
    Neither,
    Both { left: L, right: R },
}

#[test]
fn generic_enum() {
    roundtrip(GenericEither::<i32, String>::Left(1), r#"{"Left":1}"#);
    roundtrip(
        GenericEither::<i32, String>::Right("hi".to_string()),
        r#"{"Right":"hi"}"#,
    );
    roundtrip(GenericEither::<i32, String>::Neither, r#""Neither""#);
    roundtrip(
        GenericEither::Both {
            left: 1,
            right: "hi".to_string(),
        },
        r#"{"Both":{"left":1,"right":"hi"}}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Bounded<T: Clone + std::fmt::Debug>(T);

#[test]
fn generic_struct_with_preexisting_bound() {
    roundtrip(Bounded(42), "42");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Attributed {
    #[rusty_serde(rename = "n")]
    name: String,
    #[rusty_serde(default)]
    count: i32,
    #[rusty_serde(skip)]
    cache: i32,
    plain: bool,
}

#[test]
fn field_rename() {
    let value = Attributed {
        name: "hi".into(),
        count: 5,
        cache: 0,
        plain: true,
    };
    let json = json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"n":"hi","count":5,"plain":true}"#);
}

#[test]
fn field_default_when_missing() {
    let decoded: Attributed = json::from_str(r#"{"n":"hi","plain":false}"#).unwrap();
    assert_eq!(
        decoded,
        Attributed {
            name: "hi".into(),
            count: 0,
            cache: 0,
            plain: false,
        }
    );
}

#[test]
fn field_default_when_present() {
    let decoded: Attributed = json::from_str(r#"{"n":"hi","count":9,"plain":false}"#).unwrap();
    assert_eq!(decoded.count, 9);
}

#[test]
fn field_skip_never_serialized_and_always_defaulted() {
    let value = Attributed {
        name: "hi".into(),
        count: 1,
        cache: 999,
        plain: true,
    };
    let json = json::to_string(&value).unwrap();
    assert!(!json.contains("cache"));

    // Even if a "cache" key is present on the wire, it's ignored - the
    // field is never read, only ever defaulted.
    let decoded: Attributed =
        json::from_str(r#"{"n":"hi","count":1,"cache":999,"plain":true}"#).unwrap();
    assert_eq!(decoded.cache, 0);
}

#[test]
fn missing_required_field_still_errors_alongside_defaults() {
    let err = json::from_str::<Attributed>(r#"{"count":1,"plain":true}"#).unwrap_err();
    assert!(err.to_string().contains("missing field `n`"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Renamed {
    #[rusty_serde(rename = "circle")]
    Circle,
    #[rusty_serde(rename = "square")]
    Square { side: f64 },
}

#[test]
fn variant_rename() {
    roundtrip(Renamed::Circle, r#""circle""#);
    roundtrip(Renamed::Square { side: 2.0 }, r#"{"square":{"side":2.0}}"#);
}

#[test]
fn pretty_enough_whitespace_tolerance() {
    let decoded: Point = json::from_str("{\n  \"x\": 1,\n  \"y\": 2\n}\n").unwrap();
    assert_eq!(decoded, Point { x: 1, y: 2 });
}
