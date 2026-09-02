//! Tool declarations and the JSON Schema subset used to describe them.
//!
//! ADK sends every tool to the model as a `FunctionDeclaration`: a name, a
//! description, and a JSON Schema for its parameters. The schema subset here
//! is the intersection the major providers accept — enough to describe ADK's
//! recommended "simple types, few parameters" tool signatures without emitting
//! constructs that some providers reject.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The JSON Schema primitive types usable in a tool parameter schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SchemaType {
    /// A JSON string.
    String,
    /// A JSON number with a fractional part allowed.
    Number,
    /// A JSON integer.
    Integer,
    /// A JSON boolean.
    Boolean,
    /// A JSON array; see [`Schema::items`].
    Array,
    /// A JSON object; see [`Schema::properties`].
    Object,
}

/// A JSON Schema node describing a tool parameter or return value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// The value's type.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub schema_type: Option<SchemaType>,

    /// What this value means. Surfaced to the model, so it should read as
    /// guidance, not as implementation detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Allowed values, for closed sets.
    #[serde(rename = "enum", default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,

    /// Field schemas, when [`Schema::schema_type`] is
    /// [`SchemaType::Object`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Schema>,

    /// Names of required properties.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,

    /// Element schema, when [`Schema::schema_type`] is [`SchemaType::Array`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Schema>>,

    /// Whether the value may be null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,

    /// Semantic hint such as `date-time`, passed through to the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl Schema {
    /// A schema for a value of the given type.
    pub fn of(schema_type: SchemaType) -> Self {
        Self {
            schema_type: Some(schema_type),
            ..Default::default()
        }
    }

    /// A string schema.
    pub fn string() -> Self {
        Self::of(SchemaType::String)
    }

    /// An integer schema.
    pub fn integer() -> Self {
        Self::of(SchemaType::Integer)
    }

    /// A number schema.
    pub fn number() -> Self {
        Self::of(SchemaType::Number)
    }

    /// A boolean schema.
    pub fn boolean() -> Self {
        Self::of(SchemaType::Boolean)
    }

    /// An array schema with the given element type.
    pub fn array(items: Schema) -> Self {
        Self {
            schema_type: Some(SchemaType::Array),
            items: Some(Box::new(items)),
            ..Default::default()
        }
    }

    /// An empty object schema. Add fields with [`Schema::property`].
    pub fn object() -> Self {
        Self::of(SchemaType::Object)
    }

    /// Adds a required property.
    pub fn property(mut self, name: impl Into<String>, schema: Schema) -> Self {
        let name = name.into();
        self.required.push(name.clone());
        self.properties.insert(name, schema);
        self
    }

    /// Adds an optional property.
    pub fn optional_property(mut self, name: impl Into<String>, schema: Schema) -> Self {
        self.properties.insert(name.into(), schema);
        self
    }

    /// Sets the description.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Restricts the value to a closed set.
    pub fn with_enum<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.enum_values = values.into_iter().map(Into::into).collect();
        self
    }

    /// Marks the value nullable.
    pub fn nullable(mut self) -> Self {
        self.nullable = Some(true);
        self
    }

    /// Checks a value against this schema.
    ///
    /// Deliberately shallow: it verifies types, required properties, and enum
    /// membership. That is enough to turn a model's malformed tool call into a
    /// useful error message before the tool body runs, which is the only thing
    /// validation is for here — this is not a general JSON Schema validator.
    pub fn validate(&self, value: &Value, path: &str) -> crate::Result<()> {
        use crate::AdkError;

        if value.is_null() {
            if self.nullable.unwrap_or(false) {
                return Ok(());
            }
            if self.schema_type.is_none() {
                return Ok(());
            }
            return Err(AdkError::validation(path, "expected a value, found null"));
        }

        match self.schema_type {
            Some(SchemaType::String) => {
                let s = value.as_str().ok_or_else(|| {
                    AdkError::validation(path, format!("expected string, found {}", kind(value)))
                })?;
                if !self.enum_values.is_empty()
                    && !self.enum_values.iter().any(|allowed| allowed == s)
                {
                    return Err(AdkError::validation(
                        path,
                        format!("'{s}' is not one of {:?}", self.enum_values),
                    ));
                }
            }
            Some(SchemaType::Integer) => {
                if !value.is_i64() && !value.is_u64() {
                    return Err(AdkError::validation(
                        path,
                        format!("expected integer, found {}", kind(value)),
                    ));
                }
            }
            Some(SchemaType::Number) => {
                if !value.is_number() {
                    return Err(AdkError::validation(
                        path,
                        format!("expected number, found {}", kind(value)),
                    ));
                }
            }
            Some(SchemaType::Boolean) => {
                if !value.is_boolean() {
                    return Err(AdkError::validation(
                        path,
                        format!("expected boolean, found {}", kind(value)),
                    ));
                }
            }
            Some(SchemaType::Array) => {
                let arr = value.as_array().ok_or_else(|| {
                    AdkError::validation(path, format!("expected array, found {}", kind(value)))
                })?;
                if let Some(items) = &self.items {
                    for (i, element) in arr.iter().enumerate() {
                        items.validate(element, &format!("{path}[{i}]"))?;
                    }
                }
            }
            Some(SchemaType::Object) => {
                let obj = value.as_object().ok_or_else(|| {
                    AdkError::validation(path, format!("expected object, found {}", kind(value)))
                })?;
                for name in &self.required {
                    if !obj.contains_key(name) {
                        return Err(AdkError::validation(
                            join(path, name),
                            "required property is missing",
                        ));
                    }
                }
                for (name, schema) in &self.properties {
                    if let Some(field) = obj.get(name) {
                        schema.validate(field, &join(path, name))?;
                    }
                }
            }
            None => {}
        }
        Ok(())
    }
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A tool as presented to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    /// The tool's name, as the model must spell it in a function call.
    pub name: String,

    /// What the tool does and when to reach for it.
    ///
    /// ADK derives this from the function's doc comment, and it is the single
    /// biggest influence on whether the model calls the tool correctly.
    pub description: String,

    /// Schema for the tool's arguments — an object schema, or `None` for a
    /// tool that takes no arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Schema>,

    /// Schema for the tool's return value, where the provider supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Schema>,
}

impl FunctionDeclaration {
    /// Builds a declaration with no parameters.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
            response: None,
        }
    }

    /// Sets the parameter schema.
    pub fn with_parameters(mut self, parameters: Schema) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Sets the response schema.
    pub fn with_response(mut self, response: Schema) -> Self {
        self.response = Some(response);
        self
    }

    /// Validates a set of call arguments against the parameter schema.
    pub fn validate_args(&self, args: &Value) -> crate::Result<()> {
        match &self.parameters {
            Some(schema) => schema.validate(args, ""),
            None => Ok(()),
        }
    }
}

/// Maps a Rust type to the [`Schema`] describing it.
///
/// Implemented for the types a tool parameter can reasonably be. The
/// `#[adk_tool]` macro uses this to derive a declaration from a function
/// signature, so adding an impl here makes that type usable as a parameter.
pub trait HasSchema {
    /// The schema describing this type.
    fn schema() -> Schema;
}

macro_rules! impl_schema_type {
    ($($ty:ty => $ctor:expr),* $(,)?) => {
        $(impl HasSchema for $ty {
            fn schema() -> Schema {
                $ctor
            }
        })*
    };
}

impl_schema_type! {
    String => Schema::string(),
    bool => Schema::boolean(),
    i8 => Schema::integer(),
    i16 => Schema::integer(),
    i32 => Schema::integer(),
    i64 => Schema::integer(),
    u8 => Schema::integer(),
    u16 => Schema::integer(),
    u32 => Schema::integer(),
    u64 => Schema::integer(),
    usize => Schema::integer(),
    isize => Schema::integer(),
    f32 => Schema::number(),
    f64 => Schema::number(),
    Value => Schema::default(),
}

impl<T: HasSchema> HasSchema for Vec<T> {
    fn schema() -> Schema {
        Schema::array(T::schema())
    }
}

impl<T: HasSchema> HasSchema for Option<T> {
    fn schema() -> Schema {
        T::schema().nullable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn weather_schema() -> Schema {
        Schema::object()
            .property("city", Schema::string().describe("The city name."))
            .optional_property(
                "unit",
                Schema::string().with_enum(["Celsius", "Fahrenheit"]),
            )
    }

    #[test]
    fn accepts_valid_args() {
        assert!(weather_schema()
            .validate(&json!({"city": "Paris", "unit": "Celsius"}), "")
            .is_ok());
    }

    #[test]
    fn optional_property_may_be_omitted() {
        assert!(weather_schema()
            .validate(&json!({"city": "Paris"}), "")
            .is_ok());
    }

    #[test]
    fn missing_required_property_is_reported_by_name() {
        let err = weather_schema().validate(&json!({}), "").unwrap_err();
        assert!(err.to_string().contains("city"), "got: {err}");
    }

    #[test]
    fn enum_violation_is_rejected() {
        let err = weather_schema()
            .validate(&json!({"city": "Paris", "unit": "Kelvin"}), "")
            .unwrap_err();
        assert!(err.to_string().contains("Kelvin"), "got: {err}");
    }

    #[test]
    fn wrong_type_is_rejected_with_a_path() {
        let err = weather_schema()
            .validate(&json!({"city": 42}), "")
            .unwrap_err();
        assert!(err.to_string().contains("city"), "got: {err}");
        assert!(err.to_string().contains("expected string"), "got: {err}");
    }

    #[test]
    fn array_elements_are_validated_positionally() {
        let schema = Schema::array(Schema::integer());
        let err = schema.validate(&json!([1, 2, "three"]), "ids").unwrap_err();
        assert!(err.to_string().contains("ids[2]"), "got: {err}");
    }

    #[test]
    fn null_is_rejected_unless_nullable() {
        assert!(Schema::string().validate(&Value::Null, "x").is_err());
        assert!(Schema::string()
            .nullable()
            .validate(&Value::Null, "x")
            .is_ok());
    }

    #[test]
    fn rust_types_map_to_the_expected_schema_types() {
        assert_eq!(String::schema().schema_type, Some(SchemaType::String));
        assert_eq!(i64::schema().schema_type, Some(SchemaType::Integer));
        assert_eq!(f64::schema().schema_type, Some(SchemaType::Number));
        assert_eq!(bool::schema().schema_type, Some(SchemaType::Boolean));
        let list = <Vec<String>>::schema();
        assert_eq!(list.schema_type, Some(SchemaType::Array));
        assert_eq!(list.items.unwrap().schema_type, Some(SchemaType::String));
        assert_eq!(<Option<i64>>::schema().nullable, Some(true));
    }

    #[test]
    fn declaration_serializes_with_json_schema_key_names() {
        let decl = FunctionDeclaration::new("get_weather", "Retrieves weather for a city.")
            .with_parameters(weather_schema());
        let v = serde_json::to_value(&decl).unwrap();
        assert_eq!(v["parameters"]["type"], "OBJECT");
        assert_eq!(v["parameters"]["properties"]["city"]["type"], "STRING");
        assert_eq!(v["parameters"]["required"], json!(["city"]));
        assert_eq!(
            v["parameters"]["properties"]["unit"]["enum"],
            json!(["Celsius", "Fahrenheit"])
        );
    }
}
