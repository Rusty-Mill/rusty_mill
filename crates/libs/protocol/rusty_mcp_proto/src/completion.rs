//! `completion/complete`: argument autocompletion for prompts and
//! resource templates.

use crate::codec::{
    array, field, object, opt_bool, opt_string, opt_u64, opt_value, string, Obj, Wire,
};
use crate::page::ResultType;
use crate::{Error, Result};
use rusty_json::Value;

/// Method name for completion.
pub mod method {
    /// Ask for completions of one argument.
    pub const COMPLETE: &str = "completion/complete";
}

/// What is being completed.
#[derive(Clone, Debug, PartialEq)]
pub enum Reference {
    /// `ref/prompt`: an argument of the named prompt.
    Prompt {
        /// The prompt's name.
        name: String,
        /// The prompt's title.
        title: Option<String>,
    },
    /// `ref/resource`: a variable of a resource template.
    Resource {
        /// The template's URI.
        uri: String,
    },
}

impl Wire for Reference {
    fn to_value(&self) -> Value {
        match self {
            Reference::Prompt { name, title } => Obj::new()
                .set("type", "ref/prompt")
                .set("name", name.as_str())
                .opt("title", title.clone()),
            Reference::Resource { uri } => Obj::new()
                .set("type", "ref/resource")
                .set("uri", uri.as_str()),
        }
        .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "Reference";
        object(v, W)?;
        match string(v, W, "type")?.as_str() {
            "ref/prompt" => Ok(Reference::Prompt {
                name: string(v, W, "name")?,
                title: opt_string(v, W, "title")?,
            }),
            "ref/resource" => Ok(Reference::Resource {
                uri: string(v, W, "uri")?,
            }),
            other => Err(Error::decode(W, format!("unknown type {other:?}"))),
        }
    }
}

/// The argument being completed and what has been typed so far.
#[derive(Clone, Debug, PartialEq)]
pub struct ArgumentInfo {
    /// The argument's name.
    pub name: String,
    /// The text so far.
    pub value: String,
}

impl Wire for ArgumentInfo {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("name", self.name.as_str())
            .set("value", self.value.as_str())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ArgumentInfo";
        object(v, W)?;
        Ok(Self {
            name: string(v, W, "name")?,
            value: string(v, W, "value")?,
        })
    }
}

/// Parameters of `completion/complete`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteParams {
    /// The prompt or template.
    pub reference: Reference,
    /// The argument.
    pub argument: ArgumentInfo,
    /// Arguments already resolved, a JSON object of strings.
    pub context: Option<Value>,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for CompleteParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("ref", self.reference.to_value())
            .set("argument", self.argument.to_value())
            .opt_value("context", &self.context)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CompleteParams";
        object(v, W)?;
        let context = opt_value(v, "context");
        if let Some(c) = &context {
            object(c, "CompleteParams context")?;
        }
        Ok(Self {
            reference: Reference::from_value(field(v, W, "ref")?)?,
            argument: ArgumentInfo::from_value(field(v, W, "argument")?)?,
            context,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// The completions themselves. The spec caps `values` at 100; enforcing it
/// is the server's policy, not the codec's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionInfo {
    /// The suggestions, best first.
    pub values: Vec<String>,
    /// How many exist in all.
    pub total: Option<u64>,
    /// Whether more exist than are listed.
    pub has_more: Option<bool>,
}

impl Wire for CompletionInfo {
    fn to_value(&self) -> Value {
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
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CompletionInfo";
        object(v, W)?;
        let values = array(v, W, "values")?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(String::from)
                    .ok_or_else(|| Error::decode(W, "a value is not a string"))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            values,
            total: opt_u64(v, W, "total")?,
            has_more: opt_bool(v, W, "hasMore")?,
        })
    }
}

/// The result of `completion/complete`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompleteResult {
    /// `resultType`.
    pub result_type: Option<ResultType>,
    /// The completions.
    pub completion: CompletionInfo,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for CompleteResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("resultType", self.result_type.as_ref().map(|t| t.0.clone()))
            .set("completion", self.completion.to_value())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CompleteResult";
        object(v, W)?;
        Ok(Self {
            result_type: opt_string(v, W, "resultType")?.map(ResultType),
            completion: CompletionInfo::from_value(field(v, W, "completion")?)?,
            meta: opt_value(v, "_meta"),
        })
    }
}
