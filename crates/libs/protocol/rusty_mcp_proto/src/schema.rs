//! A small builder for the JSON Schema a tool's `inputSchema` needs.
//!
//! It covers what tool arguments use in practice: an object of named
//! properties that are strings, integers, numbers, booleans, string enums or
//! arrays of strings, each optionally described and required. Anything richer
//! is built as a [`Value`] and passed to [`Schema::property`].

use rusty_json::Value;

/// The type of one property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `"type": "string"`.
    String,
    /// `"type": "integer"`.
    Integer,
    /// `"type": "number"`.
    Number,
    /// `"type": "boolean"`.
    Boolean,
    /// `"type": "array"` of strings.
    StringArray,
    /// A string restricted to these values.
    OneOf(Vec<String>),
}

/// An object schema under construction.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    properties: Vec<(String, Value)>,
    required: Vec<String>,
}

impl Schema {
    /// An object schema with no properties.
    pub fn object() -> Self {
        Schema::default()
    }

    /// Add a property of a simple kind.
    pub fn field(self, name: &str, kind: Kind, description: &str, required: bool) -> Self {
        let mut prop = Value::object();
        match kind {
            Kind::String => {
                prop.insert("type", "string");
            }
            Kind::Integer => {
                prop.insert("type", "integer");
            }
            Kind::Number => {
                prop.insert("type", "number");
            }
            Kind::Boolean => {
                prop.insert("type", "boolean");
            }
            Kind::StringArray => {
                prop.insert("type", "array");
                let mut items = Value::object();
                items.insert("type", "string");
                prop.insert("items", items);
            }
            Kind::OneOf(values) => {
                prop.insert("type", "string");
                prop.insert(
                    "enum",
                    Value::Array(values.iter().map(|v| Value::from(v.as_str())).collect()),
                );
            }
        }
        if !description.is_empty() {
            prop.insert("description", description);
        }
        self.property(name, prop, required)
    }

    /// Add a property with a hand-built schema.
    pub fn property(mut self, name: &str, schema: Value, required: bool) -> Self {
        self.properties.retain(|(n, _)| n != name);
        self.required.retain(|n| n != name);
        self.properties.push((name.to_string(), schema));
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// The finished schema. `required` is left out when empty, as the spec examples do.
    pub fn build(&self) -> Value {
        let mut props = Value::object();
        for (name, schema) in &self.properties {
            props.insert(name, schema.clone());
        }
        let mut out = Value::object();
        out.insert("type", "object");
        out.insert("properties", props);
        if !self.required.is_empty() {
            out.insert(
                "required",
                Value::Array(
                    self.required
                        .iter()
                        .map(|n| Value::from(n.as_str()))
                        .collect(),
                ),
            );
        }
        out
    }
}
