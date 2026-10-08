//! Resources: the thing a server can read and the body it returns.

use crate::codec::{
    array, decode_all, encode_all, object, opt_string, opt_u64, opt_value, string, Obj, Wire,
};
use crate::page::{
    decode_cache_scope, decode_list, decode_ttl_ms, encode_list, CacheScope, Paging, ResultType,
};
use crate::{Error, Result};
use rusty_json::Value;

/// A resource a server can read (also the `resource_link` content block).
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    /// Where it lives.
    pub uri: String,
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// What it is.
    pub description: Option<String>,
    /// Media type.
    pub mime_type: Option<String>,
    /// Size in bytes.
    pub size: Option<u64>,
    /// Icons, raw JSON.
    pub icons: Option<Value>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
    /// Annotations, raw JSON.
    pub annotations: Option<Value>,
}

impl Resource {
    /// A resource with only the required members.
    pub fn new(uri: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            name: name.into(),
            title: None,
            description: None,
            mime_type: None,
            size: None,
            icons: None,
            meta: None,
            annotations: None,
        }
    }
}

impl Wire for Resource {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("uri", self.uri.as_str())
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .opt("description", self.description.clone())
            .opt("mimeType", self.mime_type.clone())
            .opt("size", self.size)
            .opt_value("icons", &self.icons)
            .opt_value("_meta", &self.meta)
            .opt_value("annotations", &self.annotations)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "Resource";
        object(v, W)?;
        Ok(Self {
            uri: string(v, W, "uri")?,
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            description: opt_string(v, W, "description")?,
            mime_type: opt_string(v, W, "mimeType")?,
            size: opt_u64(v, W, "size")?,
            icons: opt_value(v, "icons"),
            meta: opt_value(v, "_meta"),
            annotations: opt_value(v, "annotations"),
        })
    }
}

/// The body of a resource: text or base64 bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceContents {
    /// Text, told apart on the wire by a `text` member.
    Text {
        /// Where it lives.
        uri: String,
        /// Media type.
        mime_type: Option<String>,
        /// The text.
        text: String,
        /// `_meta`, raw JSON.
        meta: Option<Value>,
    },
    /// Bytes, told apart by a `blob` member.
    Blob {
        /// Where it lives.
        uri: String,
        /// Media type.
        mime_type: Option<String>,
        /// Base64 bytes.
        blob: String,
        /// `_meta`, raw JSON.
        meta: Option<Value>,
    },
}

impl Wire for ResourceContents {
    fn to_value(&self) -> Value {
        let (uri, mime_type, key, body, meta) = match self {
            ResourceContents::Text {
                uri,
                mime_type,
                text,
                meta,
            } => (uri, mime_type, "text", text, meta),
            ResourceContents::Blob {
                uri,
                mime_type,
                blob,
                meta,
            } => (uri, mime_type, "blob", blob, meta),
        };
        Obj::new()
            .set("uri", uri.as_str())
            .opt("mimeType", mime_type.clone())
            .set(key, body.as_str())
            .opt_value("_meta", meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ResourceContents";
        object(v, W)?;
        let uri = string(v, W, "uri")?;
        let mime_type = opt_string(v, W, "mimeType")?;
        let meta = opt_value(v, "_meta");
        if v.get("text").is_some() {
            let text = string(v, W, "text")?;
            Ok(ResourceContents::Text {
                uri,
                mime_type,
                text,
                meta,
            })
        } else if v.get("blob").is_some() {
            let blob = string(v, W, "blob")?;
            Ok(ResourceContents::Blob {
                uri,
                mime_type,
                blob,
                meta,
            })
        } else {
            Err(Error::decode(W, "neither \"text\" nor \"blob\""))
        }
    }
}

/// Method names for resources.
pub mod method {
    /// List the resources a server offers.
    pub const LIST: &str = "resources/list";
    /// List the resource templates a server offers.
    pub const TEMPLATES_LIST: &str = "resources/templates/list";
    /// Read one resource.
    pub const READ: &str = "resources/read";
}

/// A URI template for resources a server computes (RFC 6570).
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceTemplate {
    /// The template.
    pub uri_template: String,
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// What it is.
    pub description: Option<String>,
    /// Media type of what it produces.
    pub mime_type: Option<String>,
    /// Icons, raw JSON.
    pub icons: Option<Value>,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
    /// Annotations, raw JSON.
    pub annotations: Option<Value>,
}

impl ResourceTemplate {
    /// A template with only the required members.
    pub fn new(uri_template: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            uri_template: uri_template.into(),
            name: name.into(),
            title: None,
            description: None,
            mime_type: None,
            icons: None,
            meta: None,
            annotations: None,
        }
    }
}

impl Wire for ResourceTemplate {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("uriTemplate", self.uri_template.as_str())
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .opt("description", self.description.clone())
            .opt("mimeType", self.mime_type.clone())
            .opt_value("icons", &self.icons)
            .opt_value("_meta", &self.meta)
            .opt_value("annotations", &self.annotations)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ResourceTemplate";
        object(v, W)?;
        Ok(Self {
            uri_template: string(v, W, "uriTemplate")?,
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            description: opt_string(v, W, "description")?,
            mime_type: opt_string(v, W, "mimeType")?,
            icons: opt_value(v, "icons"),
            meta: opt_value(v, "_meta"),
            annotations: opt_value(v, "annotations"),
        })
    }
}

/// A page of `resources/list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListResourcesResult {
    /// The resources on this page.
    pub resources: Vec<Resource>,
    /// Cursor, cache hints and `_meta`.
    pub paging: Paging,
}

impl Wire for ListResourcesResult {
    fn to_value(&self) -> Value {
        encode_list("resources", &self.resources, &self.paging)
    }

    fn from_value(v: &Value) -> Result<Self> {
        let (resources, paging) = decode_list(v, "ListResourcesResult", "resources")?;
        Ok(Self { resources, paging })
    }
}

/// A page of `resources/templates/list`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListResourceTemplatesResult {
    /// The templates on this page.
    pub resource_templates: Vec<ResourceTemplate>,
    /// Cursor, cache hints and `_meta`.
    pub paging: Paging,
}

impl Wire for ListResourceTemplatesResult {
    fn to_value(&self) -> Value {
        encode_list("resourceTemplates", &self.resource_templates, &self.paging)
    }

    fn from_value(v: &Value) -> Result<Self> {
        let (resource_templates, paging) =
            decode_list(v, "ListResourceTemplatesResult", "resourceTemplates")?;
        Ok(Self {
            resource_templates,
            paging,
        })
    }
}

/// Parameters of `resources/read`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadResourceParams {
    /// Which resource.
    pub uri: String,
    /// 2026-07-28: answers to an earlier `input_required` result.
    pub input_responses: Option<Value>,
    /// 2026-07-28: the opaque state the server asked to have echoed.
    pub request_state: Option<String>,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl ReadResourceParams {
    /// A plain read.
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            input_responses: None,
            request_state: None,
            meta: None,
        }
    }
}

impl Wire for ReadResourceParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("uri", self.uri.as_str())
            .opt_value("inputResponses", &self.input_responses)
            .opt("requestState", self.request_state.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ReadResourceParams";
        object(v, W)?;
        Ok(Self {
            uri: string(v, W, "uri")?,
            input_responses: opt_value(v, "inputResponses"),
            request_state: opt_string(v, W, "requestState")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// The result of a finished `resources/read`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReadResourceResult {
    /// `resultType`.
    pub result_type: Option<ResultType>,
    /// 2026-07-28: how long the contents stay fresh, in milliseconds.
    pub ttl_ms: Option<u64>,
    /// 2026-07-28: who may cache it.
    pub cache_scope: Option<CacheScope>,
    /// The contents (more than one for a directory-like resource).
    pub contents: Vec<ResourceContents>,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for ReadResourceResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .opt("resultType", self.result_type.as_ref().map(|t| t.0.clone()))
            .opt("ttlMs", self.ttl_ms)
            .opt("cacheScope", CacheScope::encode(self.cache_scope))
            .set("contents", encode_all(&self.contents))
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ReadResourceResult";
        object(v, W)?;
        Ok(Self {
            result_type: opt_string(v, W, "resultType")?.map(ResultType),
            ttl_ms: decode_ttl_ms(v, W)?,
            cache_scope: decode_cache_scope(v)?,
            contents: decode_all(array(v, W, "contents")?)?,
            meta: opt_value(v, "_meta"),
        })
    }
}
