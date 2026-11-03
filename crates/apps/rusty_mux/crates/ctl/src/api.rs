//! `api list`, `api describe` and `api llms`: the catalog, with each
//! method's parameters as a JSON Schema, for callers that discover methods
//! at run time, or as one Markdown document for a language model.

use rusty_json::{Map, Value};
use rusty_mux_proto::schema::{self, Method, Param, Presence, Returns, Ty};

use crate::cli;

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

/// The whole API as Markdown for a language model: how to call it, every
/// command that is not a method, then every method with its command line,
/// parameters and result. Generated from the catalog, so it never drifts.
pub(crate) fn llms() -> String {
    let mut text = String::from(LLMS_INTRO);
    text.push_str("\n## Commands\n\n");
    for (name, help) in cli::COMMANDS {
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        text.push_str(&format!("- `{name}`: {help}\n"));
    }
    text.push_str("\n## Methods\n");
    for method in schema::CATALOG {
        text.push_str(&llms_method(method));
    }
    text
}

const LLMS_INTRO: &str = "# rusty_mux API

`rusty-mux-ctl` drives a rusty_mux server: sessions hold windows, windows lay \
out blocks, and a block runs a program in a terminal. Every method below is a \
command and each of its parameters a `--flag`.

- Run `rusty-mux-ctl [--socket PATH] [--json] METHOD [--FLAG VALUE]...`. The \
socket defaults to `$RUSTY_MUX_SOCKET`.
- Join commands with ` + ` to run them in order on one connection, stopping at \
the first failure.
- An id flag takes a full id, a name, a position from 1, or the last four or \
more characters of an id, optionally as a path `SESSION/WINDOW/BLOCK`. Left out, it defaults to `$RUSTY_MUX_SESSION`, \
`$RUSTY_MUX_WINDOW` or `$RUSTY_MUX_BLOCK`, which are set inside a block.
- A flag parameter is given as `--NAME` alone. A text list repeats its flag.
- `--json` prints one JSON object per line. `api describe METHOD` gives one method's \
parameters as a JSON Schema.
- Exit status: 0 done, 1 refused by the server, 2 a wrong command line, 3 a \
target that names nothing or more than one thing, 4 no server, 124 a `wait` \
timeout. A failure prints a one-line reason on stderr.
";

fn llms_method(method: &Method) -> String {
    let mut text = format!(
        "\n### `{}`\n\n{}\n\n```sh\n{}\n```\n\n",
        method.name,
        method.doc,
        cli::synopsis(method)
    );
    if !method.params.is_empty() {
        text.push_str("| Flag | Type | Presence | Description |\n|---|---|---|---|\n");
        for p in method.params {
            let presence = match p.presence {
                Presence::Required => "required",
                Presence::Optional { .. } => "optional",
                Presence::Defaulted { .. } => "optional, server picks",
            };
            text.push_str(&format!(
                "| `--{}` | {} | {} | {} |\n",
                p.name,
                p.ty,
                presence,
                p.doc.replace('|', "\\|")
            ));
        }
        text.push('\n');
    }
    text.push_str(&format!("Returns: {}.\n", returns(method.returns)));
    text
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

    #[test]
    fn llms_has_every_method_and_command_once() {
        let text = llms();
        for method in schema::CATALOG {
            let heading = format!("\n### `{}`\n", method.name);
            assert_eq!(text.matches(&heading).count(), 1, "{}", method.name);
        }
        for (name, _) in cli::COMMANDS {
            assert_eq!(
                text.matches(&format!("\n- `{name}`: ")).count(),
                1,
                "{name}"
            );
        }
    }

    #[test]
    fn llms_gives_a_method_its_command_line_flags_and_result() {
        let text = llms();
        let start = text.find("\n### `terminal.format`\n").expect("format");
        let section = &text[start..];
        let section = &section[..section[1..].find("\n### ").map_or(section.len(), |i| i + 1)];
        assert!(
            section.contains("```sh\nrusty-mux-ctl terminal.format --block <block id>"),
            "{section}"
        );
        assert!(
            section.contains("| `--block` | block id | required | The block to act on. |"),
            "{section}"
        );
        assert!(
            section.contains("| `--trim` | flag (true or false) | optional |"),
            "{section}"
        );
        assert!(section.ends_with("Returns: text.\n"), "{section}");
    }

    #[test]
    fn llms_command_help_is_one_line_each() {
        for line in llms().lines().filter(|l| l.starts_with("- `")) {
            assert!(!line.contains("  "), "{line}");
        }
    }

    #[test]
    fn llms_table_cells_escape_pipes() {
        let text = llms();
        for row in text.lines().filter(|l| l.starts_with("| `--")) {
            assert_eq!(
                row.matches(" | ").count() - row.matches("\\|").count(),
                3,
                "{row}"
            );
        }
    }
}
