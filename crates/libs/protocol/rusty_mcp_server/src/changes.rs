//! Publishing "this changed" to the clients listening for it.
//!
//! The application owns a [`ChangeBroadcaster`], hands a clone to
//! [`ServerBuilder::notify_changes`](crate::ServerBuilder::notify_changes),
//! and calls [`ChangeBroadcaster::resource_updated`] (and friends) wherever
//! the change happens. Every open `subscriptions/listen` that opted into the
//! category gets a notification. The events are re-fetch signals, not data, so
//! a bounded buffer per listener is the right size: a listener that falls
//! behind is told to re-check everything instead of failing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

/// Events buffered per listener unless [`ChangeBroadcaster::with_capacity`]
/// says otherwise.
pub const DEFAULT_CAPACITY: usize = 64;

/// Something changed that listening clients should know about.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Which changes a server announces, and so which capabilities it
/// advertises (`tools.listChanged`, `prompts.listChanged`,
/// `resources.listChanged`, `resources.subscribe`). A category a server does
/// not announce is dropped from every subscription, with no error, so
/// declare everything you intend to publish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChangeKinds {
    /// The tool list may change.
    pub tools_list: bool,
    /// The prompt list may change.
    pub prompts_list: bool,
    /// The resource list may change.
    pub resources_list: bool,
    /// Single resources' contents may change.
    pub resource_updates: bool,
}

impl ChangeKinds {
    /// Both kinds of resource change (a server with resources but no tools
    /// or prompts announces these).
    pub const fn all_resources() -> Self {
        Self {
            tools_list: false,
            prompts_list: false,
            resources_list: true,
            resource_updates: true,
        }
    }

    /// Every category.
    pub const fn all() -> Self {
        Self {
            tools_list: true,
            prompts_list: true,
            resources_list: true,
            resource_updates: true,
        }
    }
}

/// One listener's end of the broadcaster. Dropping it deregisters the
/// listener, so a quiet broadcaster does not collect dead ones.
pub(crate) struct Subscription {
    pub(crate) events: Receiver<ChangeEvent>,
    /// Set when events were dropped because this listener fell behind.
    pub(crate) lagged: Arc<AtomicBool>,
    id: u64,
    owner: Arc<Inner>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.owner
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|slot| slot.id != self.id);
    }
}

struct Slot {
    id: u64,
    tx: SyncSender<ChangeEvent>,
    lagged: Arc<AtomicBool>,
}

struct Inner {
    slots: Mutex<Vec<Slot>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    capacity: usize,
}

/// Fans change events out to every live `subscriptions/listen`. Clone it
/// into whatever needs to signal a change; all clones share the listeners.
#[derive(Clone)]
pub struct ChangeBroadcaster {
    inner: Arc<Inner>,
}

impl Default for ChangeBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ChangeBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeBroadcaster")
            .field("listeners", &self.listeners())
            .finish()
    }
}

impl ChangeBroadcaster {
    /// A broadcaster buffering [`DEFAULT_CAPACITY`] events per listener.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// A broadcaster buffering `capacity` events per listener (at least 1).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                slots: Mutex::new(Vec::new()),
                next_id: AtomicU64::new(0),
                closed: AtomicBool::new(false),
                capacity: capacity.max(1),
            }),
        }
    }

    /// Publish `event` to every live listener. Infallible on purpose: having
    /// no listeners is the normal state, not an error.
    pub fn notify(&self, event: ChangeEvent) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slots.retain(|slot| match slot.tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                slot.lagged.store(true, Ordering::SeqCst);
                true
            }
            // The listener ended.
            Err(TrySendError::Disconnected(_)) => false,
        });
    }

    /// The tool list changed.
    pub fn tools_changed(&self) {
        self.notify(ChangeEvent::ToolsListChanged);
    }

    /// The prompt list changed.
    pub fn prompts_changed(&self) {
        self.notify(ChangeEvent::PromptsListChanged);
    }

    /// The resource list changed.
    pub fn resources_changed(&self) {
        self.notify(ChangeEvent::ResourcesListChanged);
    }

    /// One resource's contents changed.
    pub fn resource_updated(&self, uri: impl Into<String>) {
        self.notify(ChangeEvent::ResourceUpdated { uri: uri.into() });
    }

    /// End every listener gracefully (each sends its final result), and make
    /// later listeners end at once. For server shutdown.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        // Dropping the senders disconnects every receiver.
        self.inner
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// How many listeners are live.
    pub fn listeners(&self) -> usize {
        self.inner
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Start listening. After [`close`](Self::close) the returned
    /// subscription is already disconnected.
    pub(crate) fn subscribe(&self) -> Subscription {
        let (tx, events) = sync_channel(self.inner.capacity);
        let lagged = Arc::new(AtomicBool::new(false));
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        if !self.inner.closed.load(Ordering::SeqCst) {
            self.inner
                .slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Slot {
                    id,
                    tx,
                    lagged: Arc::clone(&lagged),
                });
        }
        Subscription {
            events,
            lagged,
            id,
            owner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;

    #[test]
    fn every_listener_gets_every_event_in_order() {
        let b = ChangeBroadcaster::new();
        let (one, two) = (b.subscribe(), b.subscribe());
        assert_eq!(b.listeners(), 2);
        b.tools_changed();
        b.resource_updated("mem://a");
        for sub in [&one, &two] {
            assert_eq!(
                sub.events.try_recv().unwrap(),
                ChangeEvent::ToolsListChanged
            );
            assert_eq!(
                sub.events.try_recv().unwrap(),
                ChangeEvent::ResourceUpdated {
                    uri: "mem://a".into()
                }
            );
            assert_eq!(sub.events.try_recv().unwrap_err(), TryRecvError::Empty);
        }
    }

    #[test]
    fn publishing_with_no_listeners_is_fine_and_dead_listeners_are_dropped() {
        let b = ChangeBroadcaster::new();
        b.resources_changed();
        let gone = b.subscribe();
        let kept = b.subscribe();
        drop(gone);
        assert_eq!(
            b.listeners(),
            1,
            "an ended listener leaves at once, without a publish"
        );
        b.prompts_changed();
        assert_eq!(b.listeners(), 1);
        assert_eq!(
            kept.events.try_recv().unwrap(),
            ChangeEvent::PromptsListChanged
        );
    }

    #[test]
    fn a_slow_listener_is_marked_lagged_without_blocking_the_publisher() {
        let b = ChangeBroadcaster::with_capacity(2);
        let slow = b.subscribe();
        for _ in 0..5 {
            b.resources_changed();
        }
        assert!(slow.lagged.load(Ordering::SeqCst));
        assert_eq!(
            slow.events.try_iter().count(),
            2,
            "only the buffer survives"
        );
        // The fast listener of a different broadcaster is unaffected.
        let other = ChangeBroadcaster::with_capacity(2);
        let fast = other.subscribe();
        other.resources_changed();
        assert!(!fast.lagged.load(Ordering::SeqCst));
    }

    #[test]
    fn close_disconnects_listeners_now_and_later() {
        let b = ChangeBroadcaster::new();
        let early = b.subscribe();
        b.close();
        assert_eq!(
            early.events.try_recv().unwrap_err(),
            TryRecvError::Disconnected
        );
        let late = b.subscribe();
        assert_eq!(
            late.events.try_recv().unwrap_err(),
            TryRecvError::Disconnected
        );
        assert_eq!(b.listeners(), 0);
        b.tools_changed(); // still harmless
    }

    #[test]
    fn clones_share_the_same_listeners() {
        let a = ChangeBroadcaster::new();
        let b = a.clone();
        let sub = a.subscribe();
        b.tools_changed();
        assert_eq!(
            sub.events.try_recv().unwrap(),
            ChangeEvent::ToolsListChanged
        );
    }
}
