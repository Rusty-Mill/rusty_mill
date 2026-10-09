//! Argument completion: `completion/complete`.

use crate::util::{
    expect_object, field, map_to_value, opt_array, opt_bool, opt_string_map, opt_u64, string, Obj,
};
use crate::{Error, Result};
use rusty_json::Value;
use std::collections::BTreeMap;

/// What is being completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    /// An argument of the named prompt (`ref/prompt`).
    Prompt {
        /// The prompt's name.
        name: String,
    },
    /// A variable of the resource template with this URI (`ref/resource`).
    Resource {
        /// The template's URI.
        uri: String,
    },
}

impl Reference {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        match self {
            Reference::Prompt { name } => Obj::new()
                .set("type", "ref/prompt")
                .set("name", name.as_str())
                .finish(),
            Reference::Resource { uri } => Obj::new()
                .set("type", "ref/resource")
                .set("uri", uri.as_str())
                .finish(),
        }
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Reference")?;
        match string(v, "Reference", "type")?.as_str() {
            "ref/prompt" => Ok(Reference::Prompt {
                name: string(v, "Reference", "name")?,
            }),
            "ref/resource" => Ok(Reference::Resource {
                uri: string(v, "Reference", "uri")?,
            }),
            other => Err(Error::decode(
                "Reference",
                format!("unknown type {other:?}"),
            )),
        }
    }
}

/// The argument being completed and what has been typed so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentInfo {
    /// The argument's name.
    pub name: String,
    /// The text typed so far.
    pub value: String,
}

/// The parameters of `completion/complete`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteParams {
    /// What is being completed.
    pub reference: Reference,
    /// The argument and its current value.
    pub argument: ArgumentInfo,
    /// Arguments already resolved; left out of the wire form when empty.
    pub context: BTreeMap<String, String>,
}

impl CompleteParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        let mut argument = Value::object();
        argument.insert("name", self.argument.name.as_str());
        argument.insert("value", self.argument.value.as_str());
        let mut obj = Obj::new()
            .set("ref", self.reference.to_value())
            .set("argument", argument);
        if !self.context.is_empty() {
            obj = obj.set(
                "context",
                Obj::new()
                    .set("arguments", map_to_value(&self.context))
                    .finish(),
            );
        }
        obj.finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "CompleteParams")?;
        let argument = field(v, "CompleteParams", "argument")?;
        Ok(CompleteParams {
            reference: Reference::from_value(field(v, "CompleteParams", "ref")?)?,
            argument: ArgumentInfo {
                name: string(argument, "CompleteParams.argument", "name")?,
                value: string(argument, "CompleteParams.argument", "value")?,
            },
            context: match v.get("context") {
                None | Some(Value::Null) => BTreeMap::new(),
                Some(ctx) => opt_string_map(ctx, "CompleteParams.context", "arguments")?,
            },
        })
    }
}

/// The suggestions for one completion request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// The suggested values; the spec allows at most 100.
    pub values: Vec<String>,
    /// How many matches exist in total, if known.
    pub total: Option<u32>,
    /// Whether more matches exist than were returned.
    pub has_more: Option<bool>,
}

impl Completion {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "values",
                Value::Array(
                    self.values
                        .iter()
                        .map(|s| Value::from(s.as_str()))
                        .collect(),
                ),
            )
            .opt("total", self.total)
            .opt("hasMore", self.has_more)
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Completion")?;
        let values = opt_array(v, "Completion", "values")?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(String::from)
                    .ok_or_else(|| Error::decode("Completion", "a value is not a string"))
            })
            .collect::<Result<_>>()?;
        let total = opt_u64(v, "Completion", "total")?
            .map(|n| {
                u32::try_from(n).map_err(|_| Error::decode("Completion", "\"total\" is too large"))
            })
            .transpose()?;
        Ok(Completion {
            values,
            total,
            has_more: opt_bool(v, "Completion", "hasMore")?,
        })
    }
}

/// The result of `completion/complete`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompleteResult {
    /// The suggestions.
    pub completion: Completion,
}

impl CompleteResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("completion", self.completion.to_value())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "CompleteResult")?;
        Ok(CompleteResult {
            completion: Completion::from_value(field(v, "CompleteResult", "completion")?)?,
        })
    }
}
