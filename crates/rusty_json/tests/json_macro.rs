use rusty_json::{json, Map, Number, Value};

#[test]
fn scalars() {
    assert_eq!(json!(null), Value::Null);
    assert_eq!(json!(true), Value::Bool(true));
    assert_eq!(json!(false), Value::Bool(false));
    assert_eq!(json!(42), Value::Number(Number::from(42i32)));
    assert_eq!(json!("hi"), Value::String(alloc_string("hi")));
}

fn alloc_string(s: &str) -> String {
    String::from(s)
}

#[test]
fn empty_array_and_object() {
    assert_eq!(json!([]), Value::Array(Vec::new()));
    assert_eq!(json!({}), Value::Object(Map::new()));
}

#[test]
fn array_of_scalars() {
    assert_eq!(
        json!([1, 2, 3]),
        Value::Array(vec![
            Value::Number(Number::from(1i32)),
            Value::Number(Number::from(2i32)),
            Value::Number(Number::from(3i32)),
        ])
    );
}

#[test]
fn array_with_trailing_comma() {
    assert_eq!(
        json!([1, 2,]),
        Value::Array(vec![
            Value::Number(Number::from(1i32)),
            Value::Number(Number::from(2i32)),
        ])
    );
}

#[test]
fn nested_array() {
    assert_eq!(
        json!([1, [2, 3], null]),
        Value::Array(vec![
            Value::Number(Number::from(1i32)),
            Value::Array(vec![
                Value::Number(Number::from(2i32)),
                Value::Number(Number::from(3i32)),
            ]),
            Value::Null,
        ])
    );
}

#[test]
fn object_with_entries() {
    let v = json!({
        "a": 1,
        "b": true,
        "c": null,
    });
    assert_eq!(v["a"], Value::Number(Number::from(1i32)));
    assert_eq!(v["b"], Value::Bool(true));
    assert_eq!(v["c"], Value::Null);
}

#[test]
fn nested_object_and_array() {
    let v = json!({
        "code": 200,
        "success": true,
        "payload": {
            "features": ["serde", "json", null]
        }
    });
    assert_eq!(v["code"], 200);
    assert_eq!(v["success"], true);
    assert_eq!(v["payload"]["features"][0], "serde");
    assert_eq!(v["payload"]["features"][1], "json");
    assert_eq!(v["payload"]["features"][2], Value::Null);
}

#[test]
fn interpolated_expression() {
    let x = 5;
    let v = json!({ "doubled": x * 2 });
    assert_eq!(v["doubled"], 10);
}

#[test]
fn object_without_trailing_comma() {
    let v = json!({ "a": 1, "b": 2 });
    assert_eq!(v["a"], 1);
    assert_eq!(v["b"], 2);
}
