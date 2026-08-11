use std::collections::BTreeMap;

use rusty_serde::json::Value;
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

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(rename_all = "camelCase")]
struct RenameAllFields {
    user_name: String,
    is_active: bool,
    #[rusty_serde(rename = "override")]
    http_status: u32,
}

#[test]
fn container_rename_all_fields() {
    roundtrip(
        RenameAllFields {
            user_name: "ada".into(),
            is_active: true,
            http_status: 200,
        },
        r#"{"userName":"ada","isActive":true,"override":200}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum RenameAllVariants {
    FirstOption,
    SecondOption,
}

#[test]
fn container_rename_all_variants() {
    roundtrip(RenameAllVariants::FirstOption, r#""FIRST_OPTION""#);
    roundtrip(RenameAllVariants::SecondOption, r#""SECOND_OPTION""#);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(tag = "kind")]
enum Shape2 {
    Circle,
    Rectangle {
        width: f64,
        height: f64,
    },
    #[rusty_serde(rename = "tri")]
    Triangle {
        base: f64,
        height: f64,
    },
}

#[test]
fn internally_tagged_unit_variant() {
    roundtrip(Shape2::Circle, r#"{"kind":"Circle"}"#);
}

#[test]
fn internally_tagged_struct_variant() {
    roundtrip(
        Shape2::Rectangle {
            width: 2.0,
            height: 3.0,
        },
        r#"{"kind":"Rectangle","width":2.0,"height":3.0}"#,
    );
}

#[test]
fn internally_tagged_variant_rename() {
    roundtrip(
        Shape2::Triangle {
            base: 1.0,
            height: 2.0,
        },
        r#"{"kind":"tri","base":1.0,"height":2.0}"#,
    );
}

#[test]
fn internally_tagged_tag_can_appear_anywhere() {
    // The tag field doesn't have to come first on the wire - deserializing
    // it requires buffering the whole object either way.
    let decoded: Shape2 =
        json::from_str(r#"{"width":2.0,"height":3.0,"kind":"Rectangle"}"#).unwrap();
    assert_eq!(
        decoded,
        Shape2::Rectangle {
            width: 2.0,
            height: 3.0
        }
    );
}

#[test]
fn internally_tagged_missing_tag_is_an_error() {
    let err = json::from_str::<Shape2>(r#"{"width":2.0,"height":3.0}"#).unwrap_err();
    assert!(err.to_string().contains("missing tag field"));
}

#[test]
fn internally_tagged_unknown_variant_is_an_error() {
    let err = json::from_str::<Shape2>(r#"{"kind":"Hexagon"}"#).unwrap_err();
    assert!(err.to_string().contains("unknown variant"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(tag = "kind", content = "data")]
enum Shape3 {
    Circle,
    Radius(f64),
    Point(f64, f64),
    Rectangle {
        width: f64,
        height: f64,
    },
    #[rusty_serde(rename = "tri")]
    Triangle {
        base: f64,
        height: f64,
    },
}

#[test]
fn adjacently_tagged_unit_variant() {
    roundtrip(Shape3::Circle, r#"{"kind":"Circle"}"#);
}

#[test]
fn adjacently_tagged_newtype_variant() {
    roundtrip(Shape3::Radius(1.5), r#"{"kind":"Radius","data":1.5}"#);
}

#[test]
fn adjacently_tagged_tuple_variant() {
    // Adjacent tagging can represent every variant shape, including a
    // tuple variant - unlike internal tagging (`tag` alone), which rejects
    // tuple variants entirely since there's nowhere sound to splice their
    // data into the tag's own object.
    roundtrip(
        Shape3::Point(1.0, 2.0),
        r#"{"kind":"Point","data":[1.0,2.0]}"#,
    );
}

#[test]
fn adjacently_tagged_struct_variant() {
    roundtrip(
        Shape3::Rectangle {
            width: 2.0,
            height: 3.0,
        },
        r#"{"kind":"Rectangle","data":{"width":2.0,"height":3.0}}"#,
    );
}

#[test]
fn adjacently_tagged_variant_rename() {
    roundtrip(
        Shape3::Triangle {
            base: 1.0,
            height: 2.0,
        },
        r#"{"kind":"tri","data":{"base":1.0,"height":2.0}}"#,
    );
}

#[test]
fn adjacently_tagged_tag_can_appear_anywhere() {
    let decoded: Shape3 =
        json::from_str(r#"{"data":{"width":2.0,"height":3.0},"kind":"Rectangle"}"#).unwrap();
    assert_eq!(
        decoded,
        Shape3::Rectangle {
            width: 2.0,
            height: 3.0
        }
    );
}

#[test]
fn adjacently_tagged_missing_content_is_treated_as_null() {
    // A unit variant has no `content` key on the wire at all, so decoding
    // has to tolerate its absence the same way `Radius`/etc. would treat
    // an explicit `null`.
    let decoded: Shape3 = json::from_str(r#"{"kind":"Circle"}"#).unwrap();
    assert_eq!(decoded, Shape3::Circle);
}

#[test]
fn adjacently_tagged_missing_tag_is_an_error() {
    let err = json::from_str::<Shape3>(r#"{"data":1.5}"#).unwrap_err();
    assert!(err.to_string().contains("missing field `kind`"));
}

#[test]
fn adjacently_tagged_non_string_tag_is_an_error() {
    let err = json::from_str::<Shape3>(r#"{"kind":5}"#).unwrap_err();
    assert!(err.to_string().contains("tag must be a string"));
}

#[test]
fn adjacently_tagged_unknown_variant_is_an_error() {
    let err = json::from_str::<Shape3>(r#"{"kind":"Hexagon"}"#).unwrap_err();
    assert!(err.to_string().contains("unknown variant"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(tag = "kind", content = "data")]
enum Shape3WithOther {
    Circle,
    #[rusty_serde(other)]
    Unknown,
}

#[test]
fn adjacently_tagged_other_variant_catches_an_unrecognized_tag() {
    let decoded: Shape3WithOther = json::from_str(r#"{"kind":"Hexagon","data":123}"#).unwrap();
    assert_eq!(decoded, Shape3WithOther::Unknown);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct BoundedByWhere<T>
where
    T: Clone + std::fmt::Debug,
{
    value: T,
}

#[test]
fn where_clause_struct() {
    roundtrip(
        BoundedByWhere {
            value: "hi".to_string(),
        },
        r#"{"value":"hi"}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct TupleWhereClause<T>(T)
where
    T: Clone;

#[test]
fn where_clause_tuple_struct() {
    roundtrip(TupleWhereClause(9), "9");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum EitherWhere<L, R>
where
    L: Clone,
    R: Clone,
{
    Left(L),
    Right(R),
}

#[test]
fn where_clause_enum() {
    roundtrip(EitherWhere::<i32, String>::Left(1), r#"{"Left":1}"#);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SkipIfEmpty {
    name: String,
    #[rusty_serde(skip_serializing_if = "Option::is_none", default)]
    nickname: Option<String>,
    #[rusty_serde(skip_serializing_if = "Vec::is_empty", default)]
    tags: Vec<String>,
}

#[test]
fn skip_serializing_if_omits_when_true() {
    let value = SkipIfEmpty {
        name: "ada".into(),
        nickname: None,
        tags: vec![],
    };
    assert_eq!(json::to_string(&value).unwrap(), r#"{"name":"ada"}"#);
}

#[test]
fn skip_serializing_if_includes_when_false() {
    let value = SkipIfEmpty {
        name: "ada".into(),
        nickname: Some("countess".into()),
        tags: vec!["math".into()],
    };
    assert_eq!(
        json::to_string(&value).unwrap(),
        r#"{"name":"ada","nickname":"countess","tags":["math"]}"#
    );
}

#[test]
fn skip_serializing_if_round_trips_via_default() {
    // The field is missing entirely on the wire when skipped; `default`
    // is what lets deserialize recover the same value instead of erroring
    // with "missing field".
    roundtrip(
        SkipIfEmpty {
            name: "ada".into(),
            nickname: None,
            tags: vec![],
        },
        r#"{"name":"ada"}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum SkipIfVariant {
    Item {
        name: String,
        #[rusty_serde(skip_serializing_if = "Option::is_none", default)]
        note: Option<String>,
    },
}

#[test]
fn skip_serializing_if_in_struct_variant() {
    roundtrip(
        SkipIfVariant::Item {
            name: "x".into(),
            note: None,
        },
        r#"{"Item":{"name":"x"}}"#,
    );
    roundtrip(
        SkipIfVariant::Item {
            name: "x".into(),
            note: Some("y".into()),
        },
        r#"{"Item":{"name":"x","note":"y"}}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(tag = "kind")]
enum SkipIfTagged {
    Item {
        name: String,
        #[rusty_serde(skip_serializing_if = "Option::is_none", default)]
        note: Option<String>,
    },
}

#[test]
fn skip_serializing_if_in_internally_tagged_variant() {
    roundtrip(
        SkipIfTagged::Item {
            name: "x".into(),
            note: None,
        },
        r#"{"kind":"Item","name":"x"}"#,
    );
}

#[test]
fn value_parses_arbitrary_json() {
    let v: Value = json::from_str(r#"{"a":1,"b":[true,null,"x"],"c":{"d":2.5}}"#).unwrap();
    assert_eq!(v["a"].as_i64(), Some(1));
    assert_eq!(v["b"][0].as_bool(), Some(true));
    assert!(v["b"][1].is_null());
    assert_eq!(v["b"][2].as_str(), Some("x"));
    assert_eq!(v["c"]["d"].as_f64(), Some(2.5));
    assert!(v["missing"].is_null());
    assert!(v["b"][99].is_null());
}

#[test]
fn value_round_trips_and_displays_as_compact_json() {
    let original = r#"{"a":1,"b":[true,null,"x"]}"#;
    let v: Value = json::from_str(original).unwrap();
    let reencoded = json::to_string(&v).unwrap();
    assert_eq!(reencoded, original);
    assert_eq!(v.to_string(), original);
}

#[test]
fn value_from_conversions() {
    let v: Value = 42i32.into();
    assert_eq!(v.as_i64(), Some(42));
    let v: Value = "hi".into();
    assert_eq!(v.as_str(), Some("hi"));
    let v: Value = vec![1, 2, 3].into();
    assert_eq!(v.as_seq().unwrap().len(), 3);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct HasValueField {
    name: String,
    extra: Value,
}

#[test]
fn value_usable_as_a_derived_field() {
    let decoded: HasValueField =
        json::from_str(r#"{"name":"x","extra":{"whatever":[1,2]}}"#).unwrap();
    assert_eq!(decoded.name, "x");
    assert_eq!(decoded.extra["whatever"][1].as_i64(), Some(2));
    let json = json::to_string(&decoded).unwrap();
    assert_eq!(json, r#"{"name":"x","extra":{"whatever":[1,2]}}"#);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(untagged)]
enum UntaggedShape {
    Circle,
    Named(String),
    Point(i32, i32),
    Rect { width: f64, height: f64 },
}

#[test]
fn untagged_unit_variant() {
    roundtrip(UntaggedShape::Circle, "null");
}

#[test]
fn untagged_newtype_variant() {
    roundtrip(UntaggedShape::Named("x".into()), r#""x""#);
}

#[test]
fn untagged_tuple_variant() {
    roundtrip(UntaggedShape::Point(1, 2), "[1,2]");
}

#[test]
fn untagged_struct_variant() {
    roundtrip(
        UntaggedShape::Rect {
            width: 1.0,
            height: 2.0,
        },
        r#"{"width":1.0,"height":2.0}"#,
    );
}

#[test]
fn untagged_tries_variants_in_declaration_order() {
    // `Named(String)` comes before `Point(i32, i32)`, so a JSON array
    // never even reaches the newtype attempt; a JSON string never reaches
    // the tuple attempt.
    let decoded: UntaggedShape = json::from_str(r#""hi""#).unwrap();
    assert_eq!(decoded, UntaggedShape::Named("hi".into()));
    let decoded: UntaggedShape = json::from_str("[3,4]").unwrap();
    assert_eq!(decoded, UntaggedShape::Point(3, 4));
}

#[test]
fn untagged_no_matching_variant_is_an_error() {
    let err = json::from_str::<UntaggedShape>("true").unwrap_err();
    assert!(err.to_string().contains("did not match any variant"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(untagged)]
enum UntaggedGeneric<T> {
    Single(T),
    Pair(T, T),
}

#[test]
fn untagged_generic_enum() {
    roundtrip(UntaggedGeneric::Single(1), "1");
    roundtrip(UntaggedGeneric::Pair(1, 2), "[1,2]");
}

#[test]
fn pretty_enough_whitespace_tolerance() {
    let decoded: Point = json::from_str("{\n  \"x\": 1,\n  \"y\": 2\n}\n").unwrap();
    assert_eq!(decoded, Point { x: 1, y: 2 });
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Meta {
    id: i32,
    tag: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    name: String,
    #[rusty_serde(flatten)]
    meta: Meta,
}

#[test]
fn flatten_merges_a_nested_structs_fields_into_the_parent_object() {
    roundtrip(
        Record {
            name: "x".into(),
            meta: Meta {
                id: 1,
                tag: "t".into(),
            },
        },
        r#"{"name":"x","id":1,"tag":"t"}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct WithExtra {
    known: i32,
    #[rusty_serde(flatten)]
    extra: Value,
}

#[test]
fn flatten_into_value_captures_unknown_fields() {
    let decoded: WithExtra = json::from_str(r#"{"known":1,"a":2,"b":"x"}"#).unwrap();
    assert_eq!(decoded.known, 1);
    assert_eq!(decoded.extra["a"].as_i64(), Some(2));
    assert_eq!(decoded.extra["b"].as_str(), Some("x"));
    let encoded = json::to_string(&decoded).unwrap();
    assert_eq!(encoded, r#"{"known":1,"a":2,"b":"x"}"#);
}

#[test]
fn flatten_into_value_with_no_extra_fields_round_trips() {
    roundtrip(
        WithExtra {
            known: 1,
            extra: Value::Map(Vec::new()),
        },
        r#"{"known":1}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Borrowed<'a> {
    name: &'a str,
    age: i32,
}

#[test]
fn borrowed_str_field_round_trips() {
    let json = r#"{"name":"grace","age":30}"#;
    let decoded: Borrowed = json::from_str(json).unwrap();
    assert_eq!(
        decoded,
        Borrowed {
            name: "grace",
            age: 30
        }
    );
    assert_eq!(json::to_string(&decoded).unwrap(), json);
}

#[test]
fn borrowed_str_field_actually_borrows_from_the_input_without_escapes() {
    let json = r#"{"name":"grace","age":30}"#;
    let decoded: Borrowed = json::from_str(json).unwrap();
    // A real zero-copy borrow means `decoded.name` points *inside* `json`'s
    // buffer, not into some freshly allocated `String`.
    let input_range = json.as_ptr() as usize..(json.as_ptr() as usize + json.len());
    assert!(input_range.contains(&(decoded.name.as_ptr() as usize)));
}

#[test]
fn borrowed_str_field_falls_back_to_an_owned_string_when_the_input_has_escapes() {
    // `\n` forces the JSON deserializer down its escaped/owned path; `&str`
    // deserialization only supports the zero-copy case and reports a type
    // error rather than fabricating a borrow out of an owned buffer.
    let err = json::from_str::<Borrowed>(r#"{"name":"line\nbreak","age":1}"#).unwrap_err();
    assert!(err.to_string().contains("invalid type"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct CowField<'a> {
    name: std::borrow::Cow<'a, str>,
}

#[test]
fn cow_str_field_borrows_when_possible_and_owns_when_escaped() {
    let json = r#"{"name":"grace"}"#;
    let decoded: CowField = json::from_str(json).unwrap();
    assert!(matches!(decoded.name, std::borrow::Cow::Borrowed(_)));
    assert_eq!(decoded.name, "grace");

    let decoded: CowField = json::from_str(r#"{"name":"line\nbreak"}"#).unwrap();
    assert!(matches!(decoded.name, std::borrow::Cow::Owned(_)));
    assert_eq!(decoded.name, "line\nbreak");
}

fn default_retries() -> u32 {
    3
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct WithCustomDefault {
    name: String,
    #[rusty_serde(default = "default_retries")]
    retries: u32,
}

#[test]
fn field_default_path_used_when_missing() {
    let decoded: WithCustomDefault = json::from_str(r#"{"name":"x"}"#).unwrap();
    assert_eq!(
        decoded,
        WithCustomDefault {
            name: "x".into(),
            retries: 3,
        }
    );
}

#[test]
fn field_default_path_not_used_when_present() {
    let decoded: WithCustomDefault = json::from_str(r#"{"name":"x","retries":9}"#).unwrap();
    assert_eq!(decoded.retries, 9);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Aliased {
    #[rusty_serde(alias = "n", alias = "nm")]
    name: String,
    age: i32,
}

#[test]
fn field_alias_accepts_primary_name() {
    let decoded: Aliased = json::from_str(r#"{"name":"x","age":1}"#).unwrap();
    assert_eq!(
        decoded,
        Aliased {
            name: "x".into(),
            age: 1
        }
    );
}

#[test]
fn field_alias_accepts_any_alternate_name() {
    let decoded: Aliased = json::from_str(r#"{"n":"x","age":1}"#).unwrap();
    assert_eq!(decoded.name, "x");
    let decoded: Aliased = json::from_str(r#"{"nm":"x","age":1}"#).unwrap();
    assert_eq!(decoded.name, "x");
}

#[test]
fn field_alias_serializes_under_the_primary_name_only() {
    let value = Aliased {
        name: "x".into(),
        age: 1,
    };
    assert_eq!(json::to_string(&value).unwrap(), r#"{"name":"x","age":1}"#);
}

#[test]
fn field_alias_duplicate_via_alias_still_errors() {
    let err = json::from_str::<Aliased>(r#"{"name":"x","n":"y","age":1}"#).unwrap_err();
    assert!(err.to_string().contains("duplicate field"));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(deny_unknown_fields)]
struct Strict {
    x: i32,
}

#[test]
fn deny_unknown_fields_errors_on_an_unrecognized_key() {
    let err = json::from_str::<Strict>(r#"{"x":1,"y":2}"#).unwrap_err();
    assert!(err.to_string().contains("unknown field `y`"));
}

#[test]
fn deny_unknown_fields_still_accepts_known_fields() {
    let decoded: Strict = json::from_str(r#"{"x":1}"#).unwrap();
    assert_eq!(decoded, Strict { x: 1 });
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(deny_unknown_fields)]
enum StrictShape {
    Circle,
    Rect { width: f64, height: f64 },
}

#[test]
fn deny_unknown_fields_applies_to_struct_variants_too() {
    let err = json::from_str::<StrictShape>(r#"{"Rect":{"width":1.0,"height":2.0,"extra":true}}"#)
        .unwrap_err();
    assert!(err.to_string().contains("unknown field `extra`"));
    let decoded: StrictShape = json::from_str(r#"{"Rect":{"width":1.0,"height":2.0}}"#).unwrap();
    assert_eq!(
        decoded,
        StrictShape::Rect {
            width: 1.0,
            height: 2.0
        }
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct OneDirectional {
    name: String,
    #[rusty_serde(skip_serializing)]
    write_only_absent: i32,
    #[rusty_serde(skip_deserializing)]
    read_only_absent: i32,
}

#[test]
fn skip_serializing_omits_the_field_but_still_reads_it() {
    let value = OneDirectional {
        name: "x".into(),
        write_only_absent: 9,
        read_only_absent: 1,
    };
    let json = json::to_string(&value).unwrap();
    assert!(!json.contains("write_only_absent"));
    assert!(json.contains("read_only_absent"));

    // `write_only_absent` isn't on the wire (skip_serializing doesn't
    // affect deserialize), so it still has to be supplied to round-trip
    // through deserialize; `read_only_absent` is on the wire but always
    // defaults away regardless of what's supplied (skip_deserializing).
    let decoded: OneDirectional =
        json::from_str(r#"{"name":"x","write_only_absent":9,"read_only_absent":1}"#).unwrap();
    assert_eq!(decoded.name, "x");
    assert_eq!(decoded.write_only_absent, 9);
    assert_eq!(decoded.read_only_absent, 0);
}

#[test]
fn skip_deserializing_always_defaults_but_still_writes() {
    // `read_only_absent` is never read from the wire even if present - it's
    // always defaulted, same as `skip`'s read side.
    let decoded: OneDirectional =
        json::from_str(r#"{"name":"x","write_only_absent":9,"read_only_absent":999}"#).unwrap();
    assert_eq!(decoded.read_only_absent, 0);
    assert_eq!(decoded.write_only_absent, 9);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct DirectionRenamed {
    #[rusty_serde(rename(serialize = "out_name"))]
    serialize_only: String,
    #[rusty_serde(rename(deserialize = "in_name"))]
    deserialize_only: String,
    #[rusty_serde(rename(serialize = "o", deserialize = "i"))]
    both_directions: String,
}

#[test]
fn rename_serialize_only_uses_the_alternate_name_going_out() {
    let value = DirectionRenamed {
        serialize_only: "a".into(),
        deserialize_only: "b".into(),
        both_directions: "c".into(),
    };
    let json = json::to_string(&value).unwrap();
    // `serialize_only` uses its alternate name; `deserialize_only` falls
    // back to its own Rust name since `rename(deserialize = ..)` alone
    // doesn't affect the serialize direction.
    assert_eq!(json, r#"{"out_name":"a","deserialize_only":"b","o":"c"}"#);
}

#[test]
fn rename_deserialize_only_accepts_the_alternate_name_coming_in() {
    // `serialize_only` falls back to its own Rust name on the wire here,
    // since `rename(serialize = ..)` alone doesn't affect deserialize.
    let decoded: DirectionRenamed =
        json::from_str(r#"{"serialize_only":"a","in_name":"b","i":"c"}"#).unwrap();
    assert_eq!(
        decoded,
        DirectionRenamed {
            serialize_only: "a".into(),
            deserialize_only: "b".into(),
            both_directions: "c".into(),
        }
    );
}

#[test]
fn rename_bare_form_still_sets_both_directions() {
    // Regression: the pre-existing `rename = "x"` form (both directions at
    // once) still works alongside the new direction-specific form.
    roundtrip(
        Attributed {
            name: "hi".into(),
            count: 0,
            cache: 0,
            plain: false,
        },
        r#"{"n":"hi","count":0,"plain":false}"#,
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum ExternallyTaggedWithOther {
    Known,
    #[rusty_serde(other)]
    Unknown,
}

#[test]
fn other_variant_catches_an_unrecognized_external_tag() {
    let decoded: ExternallyTaggedWithOther = json::from_str(r#""Known""#).unwrap();
    assert_eq!(decoded, ExternallyTaggedWithOther::Known);

    let decoded: ExternallyTaggedWithOther = json::from_str(r#""SomethingElse""#).unwrap();
    assert_eq!(decoded, ExternallyTaggedWithOther::Unknown);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(tag = "kind")]
enum InternallyTaggedWithOther {
    Known {
        x: i32,
    },
    #[rusty_serde(other)]
    Unknown,
}

#[test]
fn other_variant_catches_an_unrecognized_internal_tag_and_discards_its_data() {
    let decoded: InternallyTaggedWithOther = json::from_str(r#"{"kind":"Known","x":1}"#).unwrap();
    assert_eq!(decoded, InternallyTaggedWithOther::Known { x: 1 });

    // Whatever fields came with the unrecognized tag are simply discarded.
    let decoded: InternallyTaggedWithOther =
        json::from_str(r#"{"kind":"SomethingElse","y":"whatever","z":[1,2,3]}"#).unwrap();
    assert_eq!(decoded, InternallyTaggedWithOther::Unknown);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(transparent)]
struct Meters {
    value: f64,
}

#[test]
fn transparent_named_struct_serializes_as_the_bare_inner_value() {
    roundtrip(Meters { value: 2.5 }, "2.5");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Distance {
    label: String,
    #[rusty_serde(rename = "m")]
    meters: Meters,
}

#[test]
fn transparent_named_struct_nests_the_same_way_a_newtype_struct_would() {
    roundtrip(
        Distance {
            label: "trail".into(),
            meters: Meters { value: 10.0 },
        },
        r#"{"label":"trail","m":10.0}"#,
    );
}

// A type that deliberately does NOT implement Serialize/Deserialize, to
// prove `#[rusty_serde(bound = "")]` below actually replaces the derive's
// auto-inferred `T: Serialize`/`T: Deserialize` bound rather than merely
// adding to it - without the override, the derive can't see that
// `PhantomData<T>` doesn't actually hold a `T` and would (wrongly) require
// `NotSerializable: Serialize`.
#[derive(Debug, PartialEq)]
struct NotSerializable;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(bound = "")]
struct Marker<T> {
    #[rusty_serde(skip)]
    _marker: std::marker::PhantomData<T>,
    value: i32,
}

#[test]
fn bound_override_lets_a_phantom_type_param_skip_the_auto_inferred_bound() {
    roundtrip(
        Marker::<NotSerializable> {
            _marker: std::marker::PhantomData,
            value: 7,
        },
        r#"{"value":7}"#,
    );
}

#[derive(Debug, Deserialize)]
struct RawPoint {
    x: i32,
    y: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(from = "RawPoint")]
struct FromPoint {
    x: i32,
    y: i32,
}

impl From<RawPoint> for FromPoint {
    fn from(r: RawPoint) -> Self {
        FromPoint { x: r.x, y: r.y }
    }
}

#[test]
fn from_deserializes_via_the_intermediate_type_then_converts() {
    let decoded: FromPoint = json::from_str(r#"{"x":1,"y":2}"#).unwrap();
    assert_eq!(decoded, FromPoint { x: 1, y: 2 });
    // Serialize is unaffected by `from` - still the normal, field-driven impl.
    assert_eq!(json::to_string(&decoded).unwrap(), r#"{"x":1,"y":2}"#);
}

#[derive(Debug, Deserialize)]
struct RawPositive {
    value: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[rusty_serde(try_from = "RawPositive")]
struct Positive {
    value: i32,
}

impl std::convert::TryFrom<RawPositive> for Positive {
    type Error = String;
    fn try_from(r: RawPositive) -> Result<Self, String> {
        if r.value > 0 {
            Ok(Positive { value: r.value })
        } else {
            Err(format!("{} is not positive", r.value))
        }
    }
}

#[test]
fn try_from_deserializes_via_the_intermediate_type_and_can_fail() {
    let decoded: Positive = json::from_str(r#"{"value":5}"#).unwrap();
    assert_eq!(decoded, Positive { value: 5 });

    let err = json::from_str::<Positive>(r#"{"value":-1}"#).unwrap_err();
    assert!(err.to_string().contains("is not positive"));
}

#[derive(Debug, Serialize)]
struct RawCelsius {
    degrees: f64,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[rusty_serde(into = "RawCelsius")]
struct Celsius {
    degrees: f64,
}

impl From<Celsius> for RawCelsius {
    fn from(c: Celsius) -> Self {
        RawCelsius { degrees: c.degrees }
    }
}

#[test]
fn into_serializes_by_cloning_into_the_intermediate_type() {
    let value = Celsius { degrees: 20.0 };
    assert_eq!(json::to_string(&value).unwrap(), r#"{"degrees":20.0}"#);
    // Deserialize is unaffected by `into` - still the normal, field-driven impl.
    let decoded: Celsius = json::from_str(r#"{"degrees":20.0}"#).unwrap();
    assert_eq!(decoded, value);
}

/// Stands in for a type from another crate: has its own (non-derived)
/// `Serialize`/`Deserialize` would-be target, all-public fields.
mod remote_target {
    #[derive(Debug, PartialEq)]
    pub struct ForeignPoint {
        pub x: i32,
        pub y: i32,
    }
}

// `remote` targets `remote_target::ForeignPoint` - this struct is never
// constructed directly, it's only a shape template for the derive macro.
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
#[rusty_serde(remote = "remote_target::ForeignPoint")]
struct ForeignPointMirror {
    x: i32,
    y: i32,
}

#[test]
fn remote_derive_targets_the_foreign_type_with_public_fields() {
    let value = remote_target::ForeignPoint { x: 1, y: 2 };
    let json = json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"x":1,"y":2}"#);
    let decoded: remote_target::ForeignPoint = json::from_str(&json).unwrap();
    assert_eq!(decoded, value);
}

// A private-field foreign type's mirror has to live in the same module as
// the type itself, same as real serde's own remote-derive examples - a
// struct literal needs its fields visible from wherever it's built, `derive`
// or not.
mod remote_getter_target {
    use rusty_serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq)]
    pub struct ForeignSecret {
        label: String,
        count: i32,
    }

    impl ForeignSecret {
        pub fn new(label: &str, count: i32) -> Self {
            ForeignSecret {
                label: label.to_string(),
                count,
            }
        }
        pub fn label(&self) -> String {
            self.label.clone()
        }
        pub fn count(&self) -> i32 {
            self.count
        }
    }

    // Never constructed directly - only a shape template for the derive
    // macro, which targets `ForeignSecret` via `remote`.
    #[allow(dead_code)]
    #[derive(Serialize, Deserialize)]
    #[rusty_serde(remote = "ForeignSecret")]
    pub struct ForeignSecretMirror {
        #[rusty_serde(getter = "ForeignSecret::label")]
        label: String,
        #[rusty_serde(getter = "ForeignSecret::count")]
        count: i32,
    }
}

#[test]
fn remote_derive_getter_reads_a_private_field_via_a_function() {
    let value = remote_getter_target::ForeignSecret::new("hi", 42);
    let json = json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"label":"hi","count":42}"#);
    let decoded: remote_getter_target::ForeignSecret = json::from_str(&json).unwrap();
    assert_eq!(decoded, value);
}

mod as_seconds {
    use rusty_serde::Serializer;
    use std::time::Duration;

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }
}

// `Duration` has no `Serialize` impl of its own in this crate - only
// `Serialize` is derived here (`serialize_with` doesn't touch deserialize;
// `deserialize_with` is a separate, not-yet-implemented attribute).
#[derive(Serialize)]
struct Event {
    name: String,
    #[rusty_serde(serialize_with = "as_seconds::serialize")]
    elapsed: std::time::Duration,
}

#[test]
fn serialize_with_reformats_a_field_via_a_function() {
    let value = Event {
        name: "boot".to_string(),
        elapsed: std::time::Duration::from_secs(90),
    };
    assert_eq!(
        json::to_string(&value).unwrap(),
        r#"{"name":"boot","elapsed":90}"#
    );
}

#[derive(Serialize)]
struct EventRenamed {
    #[rusty_serde(rename = "at", serialize_with = "as_seconds::serialize")]
    elapsed: std::time::Duration,
}

#[test]
fn serialize_with_combines_with_rename() {
    let value = EventRenamed {
        elapsed: std::time::Duration::from_secs(3),
    };
    assert_eq!(json::to_string(&value).unwrap(), r#"{"at":3}"#);
}
