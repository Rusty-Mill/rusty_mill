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
