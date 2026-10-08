//! The immutable description of a server: who it is, which protocol
//! revisions it speaks and which tools it offers. Build one with
//! [`Server::builder`], share it between connections with an `Arc`.

use crate::ask::ToolOutcome;
use crate::changes::{ChangeBroadcaster, ChangeKinds};
use crate::connection::CallContext;
#[cfg(feature = "request-state")]
use crate::state::StateCodec;
use crate::uri_template::{TemplateError, UriTemplate, UriVars};
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, CompleteParams, CompletionInfo, ErrorData, GetPromptParams,
    GetPromptResult, Implementation, Prompt, ProtocolVersion, ReadResourceParams,
    ReadResourceResult, Resource, ResourceTemplate, Tool,
};
use std::fmt;
#[cfg(feature = "request-state")]
use std::time::Duration;

/// A tool's implementation. It gets the call's context (cancellation,
/// progress, who is calling) and the parameters, and answers with a result
/// or a protocol-level error. A tool that merely *fails* should return a
/// result with `is_error: Some(true)`, which the model can read and react
/// to; return `Err` only for a malformed call.
pub type ToolHandler =
    dyn Fn(&CallContext, CallToolParams) -> Result<ToolOutcome, ErrorData> + Send + Sync;

/// A prompt's implementation: the arguments are already checked against the
/// prompt's declared required ones.
pub type PromptHandler =
    dyn Fn(&CallContext, GetPromptParams) -> Result<GetPromptResult, ErrorData> + Send + Sync;

/// A resource's implementation, called for exactly its URI.
pub type ResourceHandler =
    dyn Fn(&CallContext, ReadResourceParams) -> Result<ReadResourceResult, ErrorData> + Send + Sync;

/// A resource template's implementation, called for any URI the template
/// matches, with the variables it matched.
pub type TemplateHandler = dyn Fn(&CallContext, &UriVars, ReadResourceParams) -> Result<ReadResourceResult, ErrorData>
    + Send
    + Sync;

/// Argument completion for every prompt and resource template. The reference
/// is already checked to name a registered prompt (and one of its arguments)
/// or a registered template; at most 100 values are sent.
pub type CompletionHandler =
    dyn Fn(&CallContext, CompleteParams) -> Result<CompletionInfo, ErrorData> + Send + Sync;

/// The most completion values a response carries, per the spec.
pub(crate) const MAX_COMPLETION_VALUES: usize = 100;

/// Entries per list page unless [`ServerBuilder::page_size`] says
/// otherwise.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// How long a sealed `requestState` lasts unless
/// [`ServerBuilder::state_ttl`] says otherwise.
#[cfg(feature = "request-state")]
pub const DEFAULT_STATE_TTL: Duration = Duration::from_secs(600);

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

pub(crate) struct RegisteredPrompt {
    pub(crate) prompt: Prompt,
    pub(crate) handler: Box<PromptHandler>,
}

pub(crate) struct RegisteredResource {
    pub(crate) resource: Resource,
    pub(crate) handler: Box<ResourceHandler>,
}

pub(crate) struct RegisteredTemplate {
    pub(crate) template: ResourceTemplate,
    pub(crate) matcher: UriTemplate,
    pub(crate) handler: Box<TemplateHandler>,
}

/// Why [`ServerBuilder::build`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// Two tools share a name.
    DuplicateTool(String),
    /// Two prompts share a name.
    DuplicatePrompt(String),
    /// Two resources share a URI.
    DuplicateResource(String),
    /// Two resource templates share a URI template.
    DuplicateTemplate(String),
    /// A resource template this crate cannot match.
    BadTemplate(TemplateError),
    /// [`ServerBuilder::notify_changes`] announces changes to something the
    /// server does not offer (`"tools"`, `"prompts"` or `"resources"`).
    ChangesWithoutFeature(&'static str),
    /// A page size or in-flight limit of zero would serve nothing.
    ZeroLimit(&'static str),
    /// No protocol revision to speak.
    NoVersions,
    /// [`ServerBuilder::state_key`] was given fewer than 32 bytes.
    WeakStateKey,
    /// The operating system could not supply a random state key.
    NoRandomKey,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::DuplicateTool(name) => write!(f, "tool {name:?} is registered twice"),
            BuildError::DuplicatePrompt(name) => write!(f, "prompt {name:?} is registered twice"),
            BuildError::DuplicateResource(uri) => write!(f, "resource {uri:?} is registered twice"),
            BuildError::DuplicateTemplate(t) => write!(f, "template {t:?} is registered twice"),
            BuildError::BadTemplate(e) => write!(f, "{e}"),
            BuildError::ChangesWithoutFeature(what) => {
                write!(
                    f,
                    "changes to {what} are announced but the server offers none"
                )
            }
            BuildError::ZeroLimit(what) => write!(f, "{what} must be at least 1"),
            BuildError::NoVersions => f.write_str("a server needs at least one protocol version"),
            BuildError::WeakStateKey => {
                f.write_str("the request-state key needs at least 32 bytes")
            }
            BuildError::NoRandomKey => {
                f.write_str("could not get random bytes for the request-state key")
            }
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
    pub(crate) prompts: Vec<RegisteredPrompt>,
    pub(crate) resources: Vec<RegisteredResource>,
    pub(crate) templates: Vec<RegisteredTemplate>,
    pub(crate) completer: Option<Box<CompletionHandler>>,
    pub(crate) changes: Option<(ChangeBroadcaster, ChangeKinds)>,
    pub(crate) page_size: usize,
    pub(crate) max_in_flight: usize,
    #[cfg(feature = "request-state")]
    pub(crate) state: StateCodec,
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
            prompts: Vec::new(),
            resources: Vec::new(),
            templates: Vec::new(),
            completer: None,
            changes: None,
            deferred: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            #[cfg(feature = "request-state")]
            state_key: None,
            #[cfg(feature = "request-state")]
            state_ttl: DEFAULT_STATE_TTL,
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
    prompts: Vec<RegisteredPrompt>,
    resources: Vec<RegisteredResource>,
    templates: Vec<RegisteredTemplate>,
    completer: Option<Box<CompletionHandler>>,
    changes: Option<(ChangeBroadcaster, ChangeKinds)>,
    /// Registration mistakes, reported by [`ServerBuilder::build`].
    deferred: Vec<BuildError>,
    page_size: usize,
    max_in_flight: usize,
    #[cfg(feature = "request-state")]
    state_key: Option<Vec<u8>>,
    #[cfg(feature = "request-state")]
    state_ttl: Duration,
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
        self,
        tool: Tool,
        handler: impl Fn(&CallContext, CallToolParams) -> Result<CallToolResult, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.interactive_tool(tool, move |ctx, call| {
            handler(ctx, call).map(ToolOutcome::Done)
        })
    }

    /// Offer a tool that may need the user's input mid-call (2026-07-28
    /// clients only): `handler` answers [`ToolOutcome::Ask`] and is called
    /// again with the client's answers. See [`Ask`](crate::Ask).
    #[must_use]
    pub fn interactive_tool(
        mut self,
        tool: Tool,
        handler: impl Fn(&CallContext, CallToolParams) -> Result<ToolOutcome, ErrorData>
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

    /// The key that seals `requestState` (at least 32 bytes). Servers that
    /// answer from several processes must all share one; the default is a
    /// random key per build, which makes a state from one process invalid
    /// at another (the client sees a refused `requestState`).
    #[cfg(feature = "request-state")]
    #[must_use]
    pub fn state_key(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.state_key = Some(key.into());
        self
    }

    /// How long a sealed `requestState` stays valid (default 10 minutes).
    #[cfg(feature = "request-state")]
    #[must_use]
    pub fn state_ttl(mut self, ttl: Duration) -> Self {
        self.state_ttl = ttl;
        self
    }

    /// Offer `prompt`, answered by `handler`. A call missing an argument the
    /// prompt declares `required` is refused before the handler runs.
    #[must_use]
    pub fn prompt(
        mut self,
        prompt: Prompt,
        handler: impl Fn(&CallContext, GetPromptParams) -> Result<GetPromptResult, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.prompts.push(RegisteredPrompt {
            prompt,
            handler: Box::new(handler),
        });
        self
    }

    /// Offer `resource` at its URI, read by `handler`.
    #[must_use]
    pub fn resource(
        mut self,
        resource: Resource,
        handler: impl Fn(&CallContext, ReadResourceParams) -> Result<ReadResourceResult, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.resources.push(RegisteredResource {
            resource,
            handler: Box::new(handler),
        });
        self
    }

    /// Offer resources by URI template (`{name}` for a path segment,
    /// `{+name}` for anything), read by `handler` with the variables the URI
    /// matched. A URI that is also a registered [`resource`](Self::resource)
    /// goes to that resource; among templates, the first registered wins.
    #[must_use]
    pub fn resource_template(
        mut self,
        template: ResourceTemplate,
        handler: impl Fn(&CallContext, &UriVars, ReadResourceParams) -> Result<ReadResourceResult, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        match UriTemplate::compile(&template.uri_template) {
            Ok(matcher) => self.templates.push(RegisteredTemplate {
                template,
                matcher,
                handler: Box::new(handler),
            }),
            Err(e) => self.deferred.push(BuildError::BadTemplate(e)),
        }
        self
    }

    /// Complete the arguments of prompts and the variables of templates. The
    /// server advertises the `completions` capability only when this is set.
    #[must_use]
    pub fn completer(
        mut self,
        handler: impl Fn(&CallContext, CompleteParams) -> Result<CompletionInfo, ErrorData>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.completer = Some(Box::new(handler));
        self
    }

    /// Announce the changes in `kinds` and let clients listen for them with
    /// `subscriptions/listen`; publish them through (a clone of)
    /// `broadcaster`. The matching capabilities are advertised, and
    /// a category left out of `kinds` is never sent. Only 2026-07-28 clients
    /// can listen.
    #[must_use]
    pub fn notify_changes(mut self, broadcaster: &ChangeBroadcaster, kinds: ChangeKinds) -> Self {
        self.changes = Some((broadcaster.clone(), kinds));
        self
    }

    /// Finish.
    ///
    /// # Errors
    /// [`BuildError`] for a duplicate name, URI or template, a template that
    /// cannot be matched, changes announced for a feature the server lacks, a
    /// zero limit or no versions.
    pub fn build(self) -> Result<Server, BuildError> {
        if let Some(e) = self.deferred.into_iter().next() {
            return Err(e);
        }
        if self.versions.is_empty() {
            return Err(BuildError::NoVersions);
        }
        if let Some((_, k)) = &self.changes {
            let has_resources = !self.resources.is_empty() || !self.templates.is_empty();
            for (announced, offered, what) in [
                (k.tools_list, !self.tools.is_empty(), "tools"),
                (k.prompts_list, !self.prompts.is_empty(), "prompts"),
                (
                    k.resources_list || k.resource_updates,
                    has_resources,
                    "resources",
                ),
            ] {
                if announced && !offered {
                    return Err(BuildError::ChangesWithoutFeature(what));
                }
            }
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
        if let Some(d) = first_duplicate(self.prompts.iter().map(|p| p.prompt.name.as_str())) {
            return Err(BuildError::DuplicatePrompt(d.to_owned()));
        }
        if let Some(d) = first_duplicate(self.resources.iter().map(|r| r.resource.uri.as_str())) {
            return Err(BuildError::DuplicateResource(d.to_owned()));
        }
        if let Some(d) = first_duplicate(
            self.templates
                .iter()
                .map(|t| t.template.uri_template.as_str()),
        ) {
            return Err(BuildError::DuplicateTemplate(d.to_owned()));
        }
        #[cfg(feature = "request-state")]
        let state = {
            if self.state_ttl.as_secs() == 0 {
                return Err(BuildError::ZeroLimit("state lifetime (seconds)"));
            }
            let key = match self.state_key {
                Some(k) if k.len() < crate::state::MIN_KEY_BYTES => {
                    return Err(BuildError::WeakStateKey)
                }
                Some(k) => k,
                None => rusty_rand::bytes(crate::state::MIN_KEY_BYTES)
                    .map_err(|_| BuildError::NoRandomKey)?,
            };
            StateCodec::new(key, self.state_ttl)
        };
        Ok(Server {
            info: self.info,
            instructions: self.instructions,
            versions: self.versions,
            tools: self.tools,
            prompts: self.prompts,
            resources: self.resources,
            templates: self.templates,
            completer: self.completer,
            changes: self.changes,
            page_size: self.page_size,
            max_in_flight: self.max_in_flight,
            #[cfg(feature = "request-state")]
            state,
        })
    }
}

/// The first item that appears twice.
fn first_duplicate<'a>(items: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen: Vec<&str> = Vec::new();
    for item in items {
        if seen.contains(&item) {
            return Some(item);
        }
        seen.push(item);
    }
    None
}

impl Server {
    /// Whether `uri` names something `resources/read` can serve.
    pub(crate) fn can_read(&self, uri: &str) -> bool {
        self.resources.iter().any(|r| r.resource.uri == uri)
            || self
                .templates
                .iter()
                .any(|t| t.matcher.matches(uri).is_some())
    }
}
