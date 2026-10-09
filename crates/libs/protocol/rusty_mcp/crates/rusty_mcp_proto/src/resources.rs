//! Resources: `resources/list`, `resources/templates/list`, `resources/read`.

use crate::util::{expect_object, field, opt_array, opt_string, opt_u64, string, Obj};
use crate::{Error, Result};
use rusty_json::Value;

/// A resource a server can read out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// Its URI.
    pub uri: String,
    /// Programmatic name.
    pub name: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
    /// What it is, if described.
    pub description: Option<String>,
    /// Its media type, if known.
    pub mime_type: Option<String>,
    /// Its size in bytes, if known.
    pub size: Option<u64>,
}

impl Resource {
    /// A resource with only the required members.
    pub fn new(uri: impl Into<String>, name: impl Into<String>) -> Self {
        Resource {
            uri: uri.into(),
            name: name.into(),
            title: None,
            description: None,
            mime_type: None,
            size: None,
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("uri", self.uri.as_str())
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .opt("description", self.description.as_deref())
            .opt("mimeType", self.mime_type.as_deref())
            .opt("size", self.size)
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "Resource")?;
        Ok(Resource {
            uri: string(v, "Resource", "uri")?,
            name: string(v, "Resource", "name")?,
            title: opt_string(v, "Resource", "title")?,
            description: opt_string(v, "Resource", "description")?,
            mime_type: opt_string(v, "Resource", "mimeType")?,
            size: opt_u64(v, "Resource", "size")?,
        })
    }
}

/// A parameterised family of resources (an RFC 6570 URI template).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceTemplate {
    /// The URI template.
    pub uri_template: String,
    /// Programmatic name.
    pub name: String,
    /// Human-readable name, if any.
    pub title: Option<String>,
    /// What it is, if described.
    pub description: Option<String>,
    /// The media type of every match, if uniform.
    pub mime_type: Option<String>,
}

impl ResourceTemplate {
    /// A template with only the required members.
    pub fn new(uri_template: impl Into<String>, name: impl Into<String>) -> Self {
        ResourceTemplate {
            uri_template: uri_template.into(),
            name: name.into(),
            title: None,
            description: None,
            mime_type: None,
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set("uriTemplate", self.uri_template.as_str())
            .set("name", self.name.as_str())
            .opt("title", self.title.as_deref())
            .opt("description", self.description.as_deref())
            .opt("mimeType", self.mime_type.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ResourceTemplate")?;
        Ok(ResourceTemplate {
            uri_template: string(v, "ResourceTemplate", "uriTemplate")?,
            name: string(v, "ResourceTemplate", "name")?,
            title: opt_string(v, "ResourceTemplate", "title")?,
            description: opt_string(v, "ResourceTemplate", "description")?,
            mime_type: opt_string(v, "ResourceTemplate", "mimeType")?,
        })
    }
}

/// The content of one resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceContents {
    /// Text content.
    Text {
        /// Its URI.
        uri: String,
        /// Its media type, if known.
        mime_type: Option<String>,
        /// The text.
        text: String,
    },
    /// Binary content, base64 encoded.
    Blob {
        /// Its URI.
        uri: String,
        /// Its media type, if known.
        mime_type: Option<String>,
        /// The base64 data.
        blob: String,
    },
}

impl ResourceContents {
    /// Text content with no media type.
    pub fn text(uri: impl Into<String>, text: impl Into<String>) -> Self {
        ResourceContents::Text {
            uri: uri.into(),
            mime_type: None,
            text: text.into(),
        }
    }

    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        match self {
            ResourceContents::Text {
                uri,
                mime_type,
                text,
            } => Obj::new()
                .set("uri", uri.as_str())
                .opt("mimeType", mime_type.as_deref())
                .set("text", text.as_str())
                .finish(),
            ResourceContents::Blob {
                uri,
                mime_type,
                blob,
            } => Obj::new()
                .set("uri", uri.as_str())
                .opt("mimeType", mime_type.as_deref())
                .set("blob", blob.as_str())
                .finish(),
        }
    }

    /// Decode from a JSON value: `text` makes it text, `blob` makes it binary.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ResourceContents")?;
        let uri = string(v, "ResourceContents", "uri")?;
        let mime_type = opt_string(v, "ResourceContents", "mimeType")?;
        if v.get("text").is_some() {
            return Ok(ResourceContents::Text {
                uri,
                mime_type,
                text: string(v, "ResourceContents", "text")?,
            });
        }
        if v.get("blob").is_some() {
            return Ok(ResourceContents::Blob {
                uri,
                mime_type,
                blob: string(v, "ResourceContents", "blob")?,
            });
        }
        Err(Error::decode(
            "ResourceContents",
            "neither \"text\" nor \"blob\"",
        ))
    }
}

/// The result of `resources/list`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListResourcesResult {
    /// This page of resources.
    pub resources: Vec<Resource>,
    /// Pass back as `cursor` for the next page.
    pub next_cursor: Option<String>,
}

impl ListResourcesResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "resources",
                Value::Array(self.resources.iter().map(Resource::to_value).collect()),
            )
            .opt("nextCursor", self.next_cursor.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ListResourcesResult")?;
        Ok(ListResourcesResult {
            resources: opt_array(v, "ListResourcesResult", "resources")?
                .iter()
                .map(Resource::from_value)
                .collect::<Result<_>>()?,
            next_cursor: opt_string(v, "ListResourcesResult", "nextCursor")?,
        })
    }
}

/// The result of `resources/templates/list`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListResourceTemplatesResult {
    /// This page of templates.
    pub resource_templates: Vec<ResourceTemplate>,
    /// Pass back as `cursor` for the next page.
    pub next_cursor: Option<String>,
}

impl ListResourceTemplatesResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "resourceTemplates",
                Value::Array(
                    self.resource_templates
                        .iter()
                        .map(ResourceTemplate::to_value)
                        .collect(),
                ),
            )
            .opt("nextCursor", self.next_cursor.as_deref())
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ListResourceTemplatesResult")?;
        Ok(ListResourceTemplatesResult {
            resource_templates: opt_array(v, "ListResourceTemplatesResult", "resourceTemplates")?
                .iter()
                .map(ResourceTemplate::from_value)
                .collect::<Result<_>>()?,
            next_cursor: opt_string(v, "ListResourceTemplatesResult", "nextCursor")?,
        })
    }
}

/// The parameters of `resources/read`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadResourceParams {
    /// Which resource.
    pub uri: String,
}

impl ReadResourceParams {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new().set("uri", self.uri.as_str()).finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ReadResourceParams")?;
        Ok(ReadResourceParams {
            uri: string(v, "ReadResourceParams", "uri")?,
        })
    }
}

/// The result of `resources/read`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadResourceResult {
    /// The contents; a directory-like URI may return several.
    pub contents: Vec<ResourceContents>,
}

impl ReadResourceResult {
    /// This as a JSON value.
    pub fn to_value(&self) -> Value {
        Obj::new()
            .set(
                "contents",
                Value::Array(
                    self.contents
                        .iter()
                        .map(ResourceContents::to_value)
                        .collect(),
                ),
            )
            .finish()
    }

    /// Decode from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self> {
        expect_object(v, "ReadResourceResult")?;
        let _ = field(v, "ReadResourceResult", "contents")?;
        Ok(ReadResourceResult {
            contents: opt_array(v, "ReadResourceResult", "contents")?
                .iter()
                .map(ResourceContents::from_value)
                .collect::<Result<_>>()?,
        })
    }
}
