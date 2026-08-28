//! A tool that signals a change, so `subscriptions/listen` has something to
//! carry.
//!
//! Real servers publish from wherever the change actually happens — a file
//! watcher, a database trigger, a config reload. This stands in for that so the
//! subscription path is exercisable end to end.

use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// Which resource to mark as changed.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TouchArgs {
    /// URI of the resource to announce as updated.
    pub uri: String,
}

#[tool_router(router = notify_tools, vis = "pub(crate)")]
impl crate::server::DemoServer {
    /// Announce that a resource changed.
    #[tool(
        name = "touch_resource",
        description = "Announce that a resource changed, notifying subscribed clients."
    )]
    pub async fn touch_resource(
        &self,
        Parameters(TouchArgs { uri }): Parameters<TouchArgs>,
    ) -> String {
        // Two distinct signals: the resource itself changed, and the list may
        // have too. A client subscribed to only one still gets what it asked
        // for.
        self.changes.resource_updated(&uri);
        self.changes.resources_changed();

        format!(
            "announced {uri} to {} listener(s)",
            self.changes.listener_count()
        )
    }
}
