//! `api list` and `api describe`: the catalog, with each method's
//! parameters as a JSON Schema, for callers that discover methods at run
//! time.

use rusty_json::{Map, Value};
use rusty_mux_proto::schema::{self, Method, Param, Presence, Returns, Ty};

/// One line per method: its name and summary, or a JSON object of them.
pub(crate) fn list(json: bool) -> String {
    schema::CATALOG
        .iter()
        .map(|m| match json {
            true => {
                object([("name", m.name.into()), ("description", m.doc.into())]).to_json_string()
            }
            false => format!("{:<16} {}", m.name, m.doc),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `method` as one JSON object: its name, summary, parameters as a JSON
/// Schema of the arguments object, and what it returns.
pub(crate) fn describe(method: &Method) -> String {
    object([
        ("name", method.name.into()),
        ("description", method.doc.into()),
        ("params", input_schema(method)),
        ("returns", returns(method.returns).into()),
    ])
    .to_json_string()
}

/// `method`'s arguments as the JSON Schema of one object, as `api describe`
/// prints it. An id parameter also takes what an id flag does: a name, a
/// position or an id suffix.
pub fn input_schema(method: &Method) -> Value {
    let params = method.params;
    let properties: Map = params
        .iter()
        .map(|p| (p.name.to_owned(), property(p)))
        .collect();
    let required: Vec<Value> = params
        .iter()
        .filter(|p| p.presence == Presence::Required)
        .map(|p| p.name.into())
        .collect();
    object([
        ("type", "object".into()),
        ("properties", properties.into()),
        ("required", required.into()),
        ("additionalProperties", false.into()),
    ])
}

fn property(param: &Param) -> Value {
    let mut fields: Vec<(&str, Value)> = match param.ty {
        Ty::Id(_) => vec![("type", "string".into()), ("format", "uuid".into())],
        Ty::Text | Ty::Bytes => vec![("type", "string".into())],
        Ty::Cells => vec![
            ("type", "integer".into()),
            ("minimum", 1.into()),
            ("maximum", u16::MAX.into()),
        ],
        Ty::Lines => vec![
            ("type", "integer".into()),
            ("minimum", 0.into()),
            ("maximum", u32::MAX.into()),
        ],
        Ty::Words => vec![
            ("type", "array".into()),
            ("items", object([("type", "string".into())])),
        ],
        Ty::Flag => vec![("type", "boolean".into())],
    };
    fields.push(("description", param.doc.into()));
    object(fields)
}

fn returns(returns: Returns) -> String {
    match returns {
        Returns::Nothing => "nothing".into(),
        Returns::Unanswered => "nothing; only a failure is answered".into(),
        Returns::Created(entity) => format!("{} id", entity.name()),
        Returns::Text => "text".into(),
        Returns::Entries => "entries".into(),
    }
}

fn object<'a>(fields: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        Value::from_json_str(text).expect("valid JSON")
    }

    #[test]
    fn every_method_is_listed_once() {
        let text = list(false);
        assert_eq!(text.lines().count(), schema::CATALOG.len());
        for (line, method) in list(true).lines().zip(schema::CATALOG) {
            let Value::Object(map) = parse(line) else {
                panic!("{line}");
            };
            assert_eq!(map.get("name"), Some(&Value::from(method.name)));
        }
    }

    #[test]
    fn describe_gives_a_json_schema_of_the_arguments() {
        let method = schema::method("terminal.title").expect("title");
        let expected = parse(
            r#"{
                "name": "terminal.title",
                "description": "Read the title the terminal's program last set.",
                "params": {
                    "type": "object",
                    "properties": {
                        "block": {"type": "string", "format": "uuid", "description": "The block to act on."}
                    },
                    "required": ["block"],
                    "additionalProperties": false
                },
                "returns": "text"
            }"#,
        );
        assert_eq!(parse(&describe(method)), expected);
    }

    #[test]
    fn parameter_types_map_to_json_schema_types() {
        let format = schema::method("terminal.format").expect("format");
        let described = parse(&describe(format));
        let property = |name: &str| {
            described
                .get("params")
                .and_then(|p| p.get("properties"))
                .and_then(|p| p.get(name))
                .and_then(|p| p.get("type"))
                .cloned()
        };
        assert_eq!(property("history"), Some(Value::from("integer")));
        assert_eq!(property("trim"), Some(Value::from("boolean")));
        assert_eq!(property("vt"), Some(Value::from("boolean")));
    }

    #[test]
    fn every_method_describes_as_json() {
        for method in schema::CATALOG {
            let Value::Object(map) = parse(&describe(method)) else {
                panic!("{}", method.name);
            };
            assert!(map.contains_key("params"), "{}", method.name);
        }
    }
}
