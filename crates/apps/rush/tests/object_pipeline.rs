use rush::value::Value;
use rush::vars;

#[test]
fn test_value_json_roundtrip() {
    let json_str = r#"{"name":"rush","version":1,"active":true,"tags":["shell","rust"]}"#;
    let parsed = Value::parse_json(json_str).expect("JSON parsing failed");

    match &parsed {
        Value::Object(map) => {
            assert_eq!(map.get("name"), Some(&Value::String("rush".to_string())));
            assert_eq!(map.get("version"), Some(&Value::Int(1)));
            assert_eq!(map.get("active"), Some(&Value::Bool(true)));
        }
        _ => panic!("Expected Object value"),
    }

    let emitted = parsed.to_json(false);
    assert!(emitted.contains("\"name\":\"rush\""));
    assert!(emitted.contains("\"version\":1"));
}

#[test]
fn test_object_property_path() {
    let json_str = r#"{"server":{"host":"localhost","port":8080}}"#;
    let parsed = Value::parse_json(json_str).expect("JSON parsing failed");

    assert_eq!(
        parsed.get_path("server.host"),
        Some(Value::String("localhost".to_string()))
    );
    assert_eq!(parsed.get_path("server.port"), Some(Value::Int(8080)));
    assert_eq!(parsed.get_path("server.missing"), None);
}

#[test]
fn test_object_variable_expansion() {
    let json_str = r#"{"user":{"id":42,"name":"Alice"}}"#;
    let parsed = Value::parse_json(json_str).expect("JSON parsing failed");

    vars::set_object("user_info", parsed);

    let retrieved = vars::get_object("user_info").expect("Object not found in vars");
    assert_eq!(
        retrieved.get_path("user.name"),
        Some(Value::String("Alice".to_string()))
    );
    assert_eq!(retrieved.get_path("user.id"), Some(Value::Int(42)));
}

#[test]
fn test_object_builtins_ls_obj() {
    let code = rush::builtins::try_run(&["ls-obj".to_string(), "src".to_string()]);
    assert_eq!(code, Some(0));

    let (items, has) = rush::value::take_pipeline_output();
    assert!(has);
    assert!(!items.is_empty());

    let first = &items[0];
    assert!(first.get_path("name").is_some());
    assert!(first.get_path("size").is_some());
}

#[test]
fn test_object_builtins_ps_obj() {
    let code = rush::builtins::try_run(&["ps-obj".to_string()]);
    assert_eq!(code, Some(0));

    let (items, has) = rush::value::take_pipeline_output();
    assert!(has);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get_path("name"),
        Some(Value::String("rush".to_string()))
    );
}

#[test]
fn test_object_builtins_where_select_sort() {
    // Populate input stream
    let items = vec![
        Value::parse_json(r#"{"name":"banana","count":5}"#).unwrap(),
        Value::parse_json(r#"{"name":"apple","count":12}"#).unwrap(),
        Value::parse_json(r#"{"name":"cherry","count":2}"#).unwrap(),
    ];

    // Stage 1: where count -gt 3
    rush::value::set_pipeline_input(items);
    let code = rush::builtins::try_run(&[
        "where".to_string(),
        "count".to_string(),
        "-gt".to_string(),
        "3".to_string(),
    ]);
    assert_eq!(code, Some(0));

    let (filtered, _) = rush::value::take_pipeline_output();
    assert_eq!(filtered.len(), 2); // banana (5) & apple (12)

    // Stage 2: sort-obj count
    rush::value::set_pipeline_input(filtered);
    let code = rush::builtins::try_run(&["sort-obj".to_string(), "count".to_string()]);
    assert_eq!(code, Some(0));

    let (sorted, _) = rush::value::take_pipeline_output();
    assert_eq!(sorted.len(), 2);
    assert_eq!(
        sorted[0].get_path("name"),
        Some(Value::String("banana".to_string()))
    );
    assert_eq!(
        sorted[1].get_path("name"),
        Some(Value::String("apple".to_string()))
    );

    // Stage 3: select name
    rush::value::set_pipeline_input(sorted);
    let code = rush::builtins::try_run(&["select".to_string(), "name".to_string()]);
    assert_eq!(code, Some(0));

    let (selected, _) = rush::value::take_pipeline_output();
    assert_eq!(selected.len(), 2);
    assert_eq!(
        selected[0].get_path("name"),
        Some(Value::String("banana".to_string()))
    );
    assert_eq!(selected[0].get_path("count"), None);
}
