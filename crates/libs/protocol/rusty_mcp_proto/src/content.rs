//! Content blocks, the output of tools and prompts.

use crate::codec::{object, opt_value, string, Obj, Wire};
use crate::resource::{Resource, ResourceContents};
use crate::{Error, Result};
use rusty_json::Value;

/// The members every content block may carry besides its payload, kept as
/// raw JSON so a gateway forwards them untouched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// `annotations`.
    pub annotations: Option<Value>,
    /// `_meta`.
    pub meta: Option<Value>,
}

impl Extras {
    fn encode(&self, obj: Obj) -> Obj {
        obj.opt_value("annotations", &self.annotations)
            .opt_value("_meta", &self.meta)
    }

    fn decode(v: &Value) -> Self {
        Self {
            annotations: opt_value(v, "annotations"),
            meta: opt_value(v, "_meta"),
        }
    }
}

/// One block of tool or prompt output, told apart by `type`.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentBlock {
    /// `text`.
    Text {
        /// The text.
        text: String,
        /// Annotations and `_meta`.
        extras: Extras,
    },
    /// `image`.
    Image {
        /// Base64 bytes.
        data: String,
        /// Media type.
        mime_type: String,
        /// Annotations and `_meta`.
        extras: Extras,
    },
    /// `audio`.
    Audio {
        /// Base64 bytes.
        data: String,
        /// Media type.
        mime_type: String,
        /// Annotations and `_meta`.
        extras: Extras,
    },
    /// `resource`: a resource body embedded in the output.
    Resource {
        /// The embedded body.
        resource: ResourceContents,
        /// Annotations and `_meta`.
        extras: Extras,
    },
    /// `resource_link`: a reference the client may read.
    ResourceLink(Resource),
}

impl ContentBlock {
    /// A plain text block.
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text {
            text: text.into(),
            extras: Extras::default(),
        }
    }
}

impl Wire for ContentBlock {
    fn to_value(&self) -> Value {
        match self {
            ContentBlock::Text { text, extras } => {
                extras.encode(Obj::new().set("type", "text").set("text", text.as_str()))
            }
            ContentBlock::Image {
                data,
                mime_type,
                extras,
            } => extras.encode(media("image", data, mime_type)),
            ContentBlock::Audio {
                data,
                mime_type,
                extras,
            } => extras.encode(media("audio", data, mime_type)),
            ContentBlock::Resource { resource, extras } => extras.encode(
                Obj::new()
                    .set("type", "resource")
                    .set("resource", resource.to_value()),
            ),
            ContentBlock::ResourceLink(link) => {
                let mut v = link.to_value();
                v.insert("type", "resource_link");
                return v;
            }
        }
        .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "ContentBlock";
        object(v, W)?;
        let extras = Extras::decode(v);
        match string(v, W, "type")?.as_str() {
            "text" => Ok(ContentBlock::Text {
                text: string(v, W, "text")?,
                extras,
            }),
            "image" => Ok(ContentBlock::Image {
                data: string(v, W, "data")?,
                mime_type: string(v, W, "mimeType")?,
                extras,
            }),
            "audio" => Ok(ContentBlock::Audio {
                data: string(v, W, "data")?,
                mime_type: string(v, W, "mimeType")?,
                extras,
            }),
            "resource" => Ok(ContentBlock::Resource {
                resource: ResourceContents::from_value(crate::codec::field(v, W, "resource")?)?,
                extras,
            }),
            "resource_link" => Ok(ContentBlock::ResourceLink(Resource::from_value(v)?)),
            other => Err(Error::decode(W, format!("unknown type {other:?}"))),
        }
    }
}

fn media(kind: &str, data: &str, mime_type: &str) -> Obj {
    Obj::new()
        .set("type", kind)
        .set("data", data)
        .set("mimeType", mime_type)
}
