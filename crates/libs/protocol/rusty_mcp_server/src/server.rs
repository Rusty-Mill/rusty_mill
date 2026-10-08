//! The immutable description of a server: who it is, which protocol
//! revisions it speaks and which tools it offers. Build one with
//! [`Server::builder`], share it between connections with an `Arc`.

use crate::connection::CallContext;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ErrorData, Implementation, ProtocolVersion, Tool,
};
use std::fmt;

/// A tool's implementation. It gets the call's context (cancellation,
/// progress, who is calling) and the parameters, and answers with a result
/// or a protocol-level error. A tool that merely *fails* should return a
/// result with `is_error: Some(true)`, which the model can read and react
/// to; return `Err` only for a malformed call.
pub type ToolHandler =
    dyn Fn(&CallContext, CallToolParams) -> Result<CallToolResult, ErrorData> + Send + Sync;

/// Entries per `tools/list` page unless [`ServerBuilder::page_size`] says
/// otherwise.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// Requests in flight per connection unless
/// [`ServerBuilder::max_in_flight`] says otherwise.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 32;

/// The revisions a server speaks unless [`ServerBuilder::versions`] says
/// otherwise: every one this crate knows.
pub const DEFAULT_VERSIONS: [&str; 5] = [
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2026_07_28,
];

pub(crate) struct RegisteredTool {
    pub(crate) tool: Tool,
    pub(crate) handler: Box<ToolHandler>,
}

/// Why [`ServerBuilder::build`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// Two tools share a name.
    DuplicateTool(String),
    /// A page size or in-flight limit of zero would serve nothing.
    ZeroLimit(&'static str),
    /// No protocol revision to speak.
    NoVersions,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::DuplicateTool(name) => write!(f, "tool {name:?} is registered twice"),
            BuildError::ZeroLimit(what) => write!(f, "{what} must be at least 1"),
            BuildError::NoVersions => f.write_str("a server needs at least one protocol version"),
        }
    }
}

impl std::error::Error for BuildError {}

/// A server's fixed configuration. Cheap to share, no per-connection state.
pub struct Server {
    pub(crate) info: Implementation,
    pub(crate) instructions: Option<String>,
    pub(crate) versions: Vec<ProtocolVersion>,
    pub(crate) tools: Vec<RegisteredTool>,
    pub(crate) page_size: usize,
    pub(crate) max_in_flight: usize,
}

impl Server {
    /// Start describing a server called `name` at `version`.
    pub fn builder(name: impl Into<String>, version: impl Into<String>) -> ServerBuilder {
        ServerBuilder {
            info: Implementation::new(name, version),
            instructions: None,
            versions: DEFAULT_VERSIONS
                .iter()
                .map(|v| ProtocolVersion::new(*v))
                .collect(),
            tools: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
        }
    }

    /// Whether this server speaks `version`.
    pub(crate) fn supports(&self, version: &ProtocolVersion) -> bool {
        self.versions.contains(version)
    }

    /// The revisions with an `initialize` handshake.
    pub(crate) fn classic_versions(&self) -> Vec<ProtocolVersion> {
        self.versions
            .iter()
            .filter(|v| !v.is_stateless())
            .cloned()
            .collect()
    }
}

/// Collects a [`Server`]'s description; see [`Server::builder`].
pub struct ServerBuilder {
    info: Implementation,
    instructions: Option<String>,
    versions: Vec<ProtocolVersion>,
    tools: Vec<RegisteredTool>,
    page_size: usize,
    max_in_flight: usize,
}

impl ServerBuilder {
    /// Guidance clients may show a model about how to use the server.
    #[must_use]
    pub fn instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = Some(text.into());
        self
    }

    /// Speak only these revisions (default: [`DEFAULT_VERSIONS`]).
    #[must_use]
    pub fn versions(mut self, versions: Vec<ProtocolVersion>) -> Self {
        self.versions = versions;
        self
    }

    /// Tools per `tools/list` page (default [`DEFAULT_PAGE_SIZE`]).
    #[must_use]
    pub fn page_size(mut self, size: usize) -> Self {
        self.page_size = size;
        self
    }

    /// Requests a connection may have in flight at once (default
    /// [`DEFAULT_MAX_IN_FLIGHT`]); one more is answered with an error.
    #[must_use]
    pub fn max_in_flight(mut self, max: usize) -> Self {
        self.max_in_flight = max;
        self
    }

    /// Offer `tool`, answered by `handler`. The tool's `input_schema` is
    /// advertised as given and not enforced: the handler validates its own
    /// arguments.
    #[must_use]
    pub fn tool(
        mut self,
        tool: Tool,
        handler: impl Fn(&CallContext, CallToolParams) -> Result<CallToolResult, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.tools.push(RegisteredTool {
            tool,
            handler: Box::new(handler),
        });
        self
    }

    /// Finish.
    ///
    /// # Errors
    /// [`BuildError`] for a duplicate tool name, a zero limit or no versions.
    pub fn build(self) -> Result<Server, BuildError> {
        if self.versions.is_empty() {
            return Err(BuildError::NoVersions);
        }
        if self.page_size == 0 {
            return Err(BuildError::ZeroLimit("page size"));
        }
        if self.max_in_flight == 0 {
            return Err(BuildError::ZeroLimit("in-flight limit"));
        }
        for (i, t) in self.tools.iter().enumerate() {
            if self.tools[..i].iter().any(|u| u.tool.name == t.tool.name) {
                return Err(BuildError::DuplicateTool(t.tool.name.clone()));
            }
        }
        Ok(Server {
            info: self.info,
            instructions: self.instructions,
            versions: self.versions,
            tools: self.tools,
            page_size: self.page_size,
            max_in_flight: self.max_in_flight,
        })
    }
}
