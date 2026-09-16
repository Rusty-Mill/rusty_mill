//! Change notifications over `subscriptions/listen`.
//!
//! 2026-07-28 replaced the standalone HTTP GET stream and
//! `resources/subscribe`/`resources/unsubscribe` with a single long-lived
//! request: the client POSTs `subscriptions/listen`, opts in to the categories
//! it wants, and the server streams notifications on that request's response
//! until it ends.
//!
//! `rmcp` handles the protocol — dispatch, intersecting the requested filter
//! with what the server advertises, the acknowledgement, the final result. What
//! it leaves you is the loop: waiting for something in your application to
//! change and forwarding it to every client currently listening.
//!
//! [`ChangeBroadcaster`] is that loop. Application code calls
//! [`ChangeBroadcaster::resources_changed`] and every live subscription that
//! opted in gets the notification.
//!
//! ```no_run
//! use rusty_mcp::subscriptions::ChangeBroadcaster;
//!
//! # async fn example(changes: ChangeBroadcaster) {
//! // Anywhere in your application:
//! changes.resources_changed();
//! changes.resource_updated("config://demo");
//! # }
//! ```
//!
//! Wire it into the handler:
//!
//! ```ignore
//! impl ServerHandler for MyServer {
//!     fn get_info(&self) -> ServerInfo {
//!         rusty_mcp::server_info("my-server", "0.1.0",
//!             ServerCapabilities::builder()
//!                 .enable_resources()
//!                 // Without `list_changed`, the filter intersection drops the
//!                 // category and the client silently never hears about it.
//!                 .enable_resources_list_changed()
//!                 .enable_resources_subscribe()
//!                 .build())
//!     }
//!     rusty_mcp::forward_subscription_methods!(changes);
//! }
//! ```
//!
//! # Advertise what you intend to send
//!
//! `rmcp` intersects the client's requested filter with the capabilities from
//! `get_info`, so a category you forget to advertise is dropped without error —
//! the subscription succeeds and simply stays quiet. If notifications are not
//! arriving, check the capability flags first.

use std::sync::Arc;

use rmcp::{
    model::{ErrorData, SubscriptionFilter},
    service::{SubscriptionContext, SubscriptionSendError},
};
use tokio::sync::broadcast;

/// How many events a slow listener may fall behind before it starts dropping.
///
/// These are "re-fetch" signals rather than data, so a bounded buffer is right;
/// [`ChangeBroadcaster::run`] recovers from an overflow rather than failing.
const CHANNEL_CAPACITY: usize = 64;

/// Something changed that listening clients should know about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeEvent {
    /// The tool list changed.
    ToolsListChanged,
    /// The prompt list changed.
    PromptsListChanged,
    /// The resource list changed.
    ResourcesListChanged,
    /// One resource's contents changed.
    ResourceUpdated {
        /// URI of the resource.
        uri: String,
    },
}

/// Fans application change events out to every live subscription.
///
/// Clone it into whatever needs to signal a change; all clones share one
/// channel. Cheap to clone and safe to hold across await points.
#[derive(Debug, Clone)]
pub struct ChangeBroadcaster {
    tx: Arc<broadcast::Sender<ChangeEvent>>,
}

impl Default for ChangeBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangeBroadcaster {
    /// A broadcaster with the default buffer.
    pub fn new() -> Self {
        Self::with_capacity(CHANNEL_CAPACITY)
    }

    /// A broadcaster buffering `capacity` events per listener.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Self { tx: Arc::new(tx) }
    }

    /// Publish an event to every live subscription.
    ///
    /// Deliberately infallible: having no listeners is the normal state, not an
    /// error, and application code should not have to care whether anyone is
    /// currently subscribed.
    pub fn notify(&self, event: ChangeEvent) {
        // `send` errors only when there are no receivers.
        let _ = self.tx.send(event);
    }

    /// Signal that the tool list changed.
    pub fn tools_changed(&self) {
        self.notify(ChangeEvent::ToolsListChanged);
    }

    /// Signal that the prompt list changed.
    pub fn prompts_changed(&self) {
        self.notify(ChangeEvent::PromptsListChanged);
    }

    /// Signal that the resource list changed.
    pub fn resources_changed(&self) {
        self.notify(ChangeEvent::ResourcesListChanged);
    }

    /// Signal that one resource's contents changed.
    pub fn resource_updated(&self, uri: impl Into<String>) {
        self.notify(ChangeEvent::ResourceUpdated { uri: uri.into() });
    }

    /// How many subscriptions are currently listening.
    pub fn listener_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// The filter this server accepts, given what the client asked for.
    ///
    /// Everything requested, narrowed by `rmcp` to what `get_info` advertises.
    /// Override [`rmcp::ServerHandler::accepted_subscription_filter`] yourself
    /// if a subscription needs a per-client decision, such as restricting
    /// resource URIs to those the caller is authorized to see.
    pub fn accepted_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    /// Run one subscription until the client goes away.
    ///
    /// Forwards accepted events and returns `Ok(())` on cancellation, which is
    /// what tells `rmcp` to send the final result.
    pub async fn run(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut rx = self.tx.subscribe();
        let sink = context.sink();

        loop {
            tokio::select! {
                // Biased so a cancelled subscription stops promptly rather than
                // draining a backlog nobody will receive.
                biased;

                () = context.cancelled() => {
                    tracing::debug!(id = ?sink.id(), "subscription cancelled");
                    return Ok(());
                }

                received = rx.recv() => match received {
                    Ok(event) => {
                        if let Err(err) = dispatch(&context, event).await {
                            // The subscription ended between the select and the
                            // send; not an error worth failing the request over.
                            if matches!(err, SubscriptionSendError::SubscriptionClosed) {
                                return Ok(());
                            }
                            tracing::debug!(%err, "dropping a notification");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        // A slow listener fell behind. These are "re-fetch"
                        // signals, so the recovery is to re-announce each
                        // accepted list rather than to fail: the client ends up
                        // with fresh lists either way.
                        tracing::warn!(
                            missed,
                            "subscription lagged; re-announcing list changes"
                        );
                        resync(&context).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Every broadcaster was dropped, so nothing more can
                        // arrive. Ending gracefully beats holding the request
                        // open forever.
                        tracing::debug!("change broadcaster closed");
                        return Ok(());
                    }
                },
            }
        }
    }
}

/// Send one event, if the subscription opted in to it.
async fn dispatch(
    context: &SubscriptionContext,
    event: ChangeEvent,
) -> Result<(), SubscriptionSendError> {
    let accepted = context.accepted();
    let sink = context.sink();

    match event {
        ChangeEvent::ToolsListChanged if accepted.tools_list_changed == Some(true) => {
            sink.notify_tool_list_changed().await
        }
        ChangeEvent::PromptsListChanged if accepted.prompts_list_changed == Some(true) => {
            sink.notify_prompt_list_changed().await
        }
        ChangeEvent::ResourcesListChanged if accepted.resources_list_changed == Some(true) => {
            sink.notify_resource_list_changed().await
        }
        ChangeEvent::ResourceUpdated { uri }
            if accepted
                .resource_subscriptions
                .as_ref()
                .is_some_and(|uris| uris.contains(&uri)) =>
        {
            sink.notify_resource_updated(uri).await
        }
        // Not opted in. Skipped silently: the client chose not to receive this,
        // which is a normal outcome and not something to log per event.
        _ => Ok(()),
    }
}

/// Re-announce every accepted list-changed category after an overflow.
///
/// Per-resource updates cannot be recovered this way — the missed URIs are
/// gone — so a client that needs exact update events should keep up or use a
/// larger buffer.
async fn resync(context: &SubscriptionContext) {
    let accepted = context.accepted();
    let sink = context.sink();

    if accepted.tools_list_changed == Some(true) {
        let _ = sink.notify_tool_list_changed().await;
    }
    if accepted.prompts_list_changed == Some(true) {
        let _ = sink.notify_prompt_list_changed().await;
    }
    if accepted.resources_list_changed == Some(true) {
        let _ = sink.notify_resource_list_changed().await;
    }
}

/// Implement `accepted_subscription_filter` and `listen` by forwarding to a
/// [`ChangeBroadcaster`] field.
///
/// Expands inside an `impl ServerHandler` block:
///
/// ```ignore
/// impl ServerHandler for MyServer {
///     fn get_info(&self) -> ServerInfo { /* ... */ }
///     rusty_mcp::forward_subscription_methods!(changes);
/// }
/// ```
///
/// Remember to advertise the matching `list_changed` capabilities, or the
/// filter intersection drops the categories and the subscription stays quiet.
#[macro_export]
macro_rules! forward_subscription_methods {
    ($field:ident) => {
        fn accepted_subscription_filter(
            &self,
            requested: &$crate::__private::SubscriptionFilter,
        ) -> ::core::option::Option<$crate::__private::SubscriptionFilter> {
            self.$field.accepted_filter(requested)
        }

        async fn listen(
            &self,
            context: $crate::__private::SubscriptionContext,
        ) -> ::core::result::Result<(), $crate::__private::ErrorData> {
            self.$field.run(context).await
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifying_without_listeners_is_not_an_error() {
        // The normal state for a server nobody has subscribed to.
        let changes = ChangeBroadcaster::new();
        assert_eq!(changes.listener_count(), 0);

        changes.resources_changed();
        changes.resource_updated("config://demo");
    }

    #[tokio::test]
    async fn clones_share_one_channel() {
        let changes = ChangeBroadcaster::new();
        let mut rx = changes.tx.subscribe();

        // A clone handed to application code must reach the same listeners.
        let elsewhere = changes.clone();
        elsewhere.tools_changed();

        assert_eq!(
            rx.recv().await.expect("event"),
            ChangeEvent::ToolsListChanged
        );
    }

    #[tokio::test]
    async fn every_helper_maps_to_its_event() {
        let changes = ChangeBroadcaster::new();
        let mut rx = changes.tx.subscribe();

        changes.tools_changed();
        changes.prompts_changed();
        changes.resources_changed();
        changes.resource_updated("res://one");

        assert_eq!(
            rx.recv().await.expect("event"),
            ChangeEvent::ToolsListChanged
        );
        assert_eq!(
            rx.recv().await.expect("event"),
            ChangeEvent::PromptsListChanged
        );
        assert_eq!(
            rx.recv().await.expect("event"),
            ChangeEvent::ResourcesListChanged
        );
        assert_eq!(
            rx.recv().await.expect("event"),
            ChangeEvent::ResourceUpdated {
                uri: "res://one".to_string()
            }
        );
    }

    #[tokio::test]
    async fn a_slow_listener_lags_rather_than_blocking_the_publisher() {
        let changes = ChangeBroadcaster::with_capacity(2);
        let mut rx = changes.tx.subscribe();

        // Publishing past the buffer must not block or fail; the slow listener
        // takes the loss, and `run` re-announces on `Lagged`.
        for _ in 0..10 {
            changes.resources_changed();
        }

        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
    }

    #[test]
    fn listener_count_tracks_subscribers() {
        let changes = ChangeBroadcaster::new();
        let rx1 = changes.tx.subscribe();
        assert_eq!(changes.listener_count(), 1);

        let rx2 = changes.tx.subscribe();
        assert_eq!(changes.listener_count(), 2);

        drop(rx1);
        drop(rx2);
        assert_eq!(changes.listener_count(), 0);
    }

    #[test]
    fn the_accepted_filter_passes_the_request_through() {
        // Narrowing to advertised capabilities happens in rmcp; this only
        // declines to narrow further.
        let changes = ChangeBroadcaster::new();
        let requested = SubscriptionFilter::builder()
            .tools_list_changed()
            .resources_list_changed()
            .build();

        let accepted = changes.accepted_filter(&requested).expect("accepts");
        assert_eq!(accepted.tools_list_changed, Some(true));
        assert_eq!(accepted.resources_list_changed, Some(true));
        assert_eq!(accepted.prompts_list_changed, None);
    }

    #[test]
    fn capacity_is_never_zero() {
        // `broadcast::channel(0)` panics; clamp rather than propagate that.
        let changes = ChangeBroadcaster::with_capacity(0);
        changes.tools_changed();
    }
}
