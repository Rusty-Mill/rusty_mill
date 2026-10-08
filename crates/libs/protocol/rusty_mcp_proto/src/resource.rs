//! Resources: the thing a server can read and the body it returns.

use crate::codec::{object, opt_string, opt_u64, opt_value, string, Obj, Wire};
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
