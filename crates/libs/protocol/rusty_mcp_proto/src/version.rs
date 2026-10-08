//! Protocol revisions and the `Implementation` peers exchange.

use crate::codec::{object, Wire};
use crate::{Error, Result};
use rusty_json::Value;

/// An MCP protocol revision, a `YYYY-MM-DD` string. Unknown revisions are
/// representable on purpose: negotiation needs to carry what a peer sent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(String);

impl ProtocolVersion {
    /// The first revision, with the `initialize` handshake.
    pub const V_2024_11_05: &'static str = "2024-11-05";
    /// Adds Streamable HTTP.
    pub const V_2025_03_26: &'static str = "2025-03-26";
    /// Adds structured tool output and elicitation.
    pub const V_2025_06_18: &'static str = "2025-06-18";
    /// Adds tasks.
    pub const V_2025_11_25: &'static str = "2025-11-25";
    /// Stateless: no `initialize`, `server/discover`, per-request metadata.
    pub const V_2026_07_28: &'static str = "2026-07-28";

    /// Wrap a revision string as sent by a peer.
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// The revision as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this revision is stateless (2026-07-28 or later). Revisions
    /// are ISO dates, so text order is date order.
    pub fn is_stateless(&self) -> bool {
        self.0.as_str() >= Self::V_2026_07_28
    }
}

impl Wire for ProtocolVersion {
    fn to_value(&self) -> Value {
        Value::from(self.0.as_str())
    }

    fn from_value(v: &Value) -> Result<Self> {
        v.as_str()
            .map(Self::new)
            .ok_or_else(|| Error::decode("protocol version", "not a string"))
    }
}

/// Name and version of a client or server, as exchanged on connect.
#[derive(Clone, Debug, PartialEq)]
pub struct Implementation {
    /// Programmatic name.
    pub name: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// Version string.
    pub version: String,
    /// What it is.
    pub description: Option<String>,
    /// Icons, kept as raw JSON.
    pub icons: Option<Value>,
    /// Home page.
    pub website_url: Option<String>,
}

impl Implementation {
    /// An implementation with only the required members.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: None,
            version: version.into(),
            description: None,
            icons: None,
            website_url: None,
        }
    }
}

impl Wire for Implementation {
    fn to_value(&self) -> Value {
        crate::codec::Obj::new()
            .set("name", self.name.as_str())
            .opt("title", self.title.clone())
            .set("version", self.version.as_str())
            .opt("description", self.description.clone())
            .opt_value("icons", &self.icons)
            .opt("websiteUrl", self.website_url.clone())
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        use crate::codec::{opt_string, opt_value, string};
        const W: &str = "Implementation";
        object(v, W)?;
        Ok(Self {
            name: string(v, W, "name")?,
            title: opt_string(v, W, "title")?,
            version: string(v, W, "version")?,
            description: opt_string(v, W, "description")?,
            icons: opt_value(v, "icons"),
            website_url: opt_string(v, W, "websiteUrl")?,
        })
    }
}
