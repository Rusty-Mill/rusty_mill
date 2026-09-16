//! [`Toolset`] — a group of tools resolved at run time.
//!
//! An agent's tool list is fixed at construction; a toolset is the escape
//! hatch for cases where it should not be. MCP servers are the canonical
//! example: the tool list comes from the server at connect time, and can be
//! filtered per invocation.

use adk_core::{InvocationContext, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

use crate::tool::{SharedTool, Tool};

/// Supplies tools to an agent, possibly varying by invocation.
#[async_trait]
pub trait Toolset: Send + Sync {
    /// The tools available for this invocation.
    ///
    /// Called once per agent turn, before the model request is built, so the
    /// returned list may depend on session state.
    async fn tools(&self, ctx: &InvocationContext) -> Result<Vec<SharedTool>>;

    /// Releases any resources held by the toolset, such as an MCP connection.
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// A fixed set of tools, optionally filtered by name.
pub struct StaticToolset {
    tools: Vec<SharedTool>,
    filter: Option<HashSet<String>>,
}

impl StaticToolset {
    /// Builds a toolset over a fixed list.
    pub fn new(tools: Vec<SharedTool>) -> Self {
        Self {
            tools,
            filter: None,
        }
    }

    /// Exposes only the named tools, dropping the rest.
    pub fn with_filter<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.filter = Some(names.into_iter().map(Into::into).collect());
        self
    }
}

#[async_trait]
impl Toolset for StaticToolset {
    async fn tools(&self, _ctx: &InvocationContext) -> Result<Vec<SharedTool>> {
        Ok(match &self.filter {
            None => self.tools.clone(),
            Some(allowed) => self
                .tools
                .iter()
                .filter(|t| allowed.contains(t.name()))
                .cloned()
                .collect(),
        })
    }
}

/// Anything an agent will accept in its tool list.
///
/// Lets a single builder method take both individual tools and whole toolsets
/// without the caller sorting them into separate lists.
pub enum ToolSource {
    /// One tool.
    Tool(SharedTool),
    /// A group resolved per invocation.
    Toolset(Arc<dyn Toolset>),
}

impl ToolSource {
    /// Resolves this source to concrete tools for one invocation.
    pub async fn resolve(&self, ctx: &InvocationContext) -> Result<Vec<SharedTool>> {
        match self {
            ToolSource::Tool(tool) => Ok(vec![Arc::clone(tool)]),
            ToolSource::Toolset(set) => set.tools(ctx).await,
        }
    }
}

impl From<SharedTool> for ToolSource {
    fn from(tool: SharedTool) -> Self {
        ToolSource::Tool(tool)
    }
}

impl From<Arc<dyn Toolset>> for ToolSource {
    fn from(set: Arc<dyn Toolset>) -> Self {
        ToolSource::Toolset(set)
    }
}

impl<T: Tool + 'static> From<T> for ToolSource {
    fn from(tool: T) -> Self {
        ToolSource::Tool(Arc::new(tool))
    }
}

/// Resolves every source into a flat tool list, rejecting duplicate names.
///
/// Duplicates are an error rather than a silent last-wins because a model
/// cannot disambiguate two tools with the same name, and the resulting
/// misdispatch is very hard to diagnose from the outside.
pub async fn resolve_tools(
    sources: &[ToolSource],
    ctx: &InvocationContext,
) -> Result<Vec<SharedTool>> {
    let mut resolved: Vec<SharedTool> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for source in sources {
        for tool in source.resolve(ctx).await? {
            if !seen.insert(tool.name().to_string()) {
                return Err(adk_core::AdkError::Config(format!(
                    "duplicate tool name '{}' for agent '{}'",
                    tool.name(),
                    ctx.agent_name
                )));
            }
            resolved.push(tool);
        }
    }
    Ok(resolved)
}
