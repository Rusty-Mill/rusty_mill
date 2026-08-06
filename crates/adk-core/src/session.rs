//! The [`Session`] — one conversation thread, its state, and its event history.

use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::state::State;

/// A single conversation thread between a user and an app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Unique id for this thread.
    pub id: String,
    /// The agent application this thread belongs to.
    pub app_name: String,
    /// The user this thread belongs to.
    pub user_id: String,
    /// The conversation scratchpad.
    #[serde(default)]
    pub state: State,
    /// Every event so far, oldest first.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Seconds since the Unix epoch at the last append.
    pub last_update_time: f64,
}

impl Session {
    /// Builds an empty session.
    pub fn new(
        id: impl Into<String>,
        app_name: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            app_name: app_name.into(),
            user_id: user_id.into(),
            state: State::new(),
            events: Vec::new(),
            last_update_time: crate::now_seconds(),
        }
    }

    /// Sets the starting state.
    pub fn with_state(mut self, state: State) -> Self {
        self.state = state;
        self
    }

    /// The most recent event, if any.
    pub fn last_event(&self) -> Option<&Event> {
        self.events.last()
    }

    /// Events belonging to one invocation, in order.
    pub fn events_for_invocation<'a>(
        &'a self,
        invocation_id: &'a str,
    ) -> impl Iterator<Item = &'a Event> {
        self.events
            .iter()
            .filter(move |e| e.invocation_id == invocation_id)
    }

    /// Conversation history as model-ready content, skipping events that carry
    /// no content and streaming chunks that a later event supersedes.
    pub fn contents(&self) -> Vec<crate::content::Content> {
        self.events
            .iter()
            .filter(|e| !e.is_partial())
            .filter_map(|e| e.content.clone())
            .filter(|c| !c.parts.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;

    #[test]
    fn contents_skips_partial_and_empty_events() {
        let mut s = Session::new("s1", "app", "u1");
        s.events.push(Event::new("inv", "user").with_content(Content::user_text("hi")));
        s.events.push(Event::new("inv", "agent").with_text("Par").as_partial());
        s.events.push(Event::new("inv", "agent")); // no content
        s.events.push(Event::new("inv", "agent").with_text("Paris"));

        let contents = s.contents();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[1].text(), "Paris");
    }

    #[test]
    fn events_filter_by_invocation() {
        let mut s = Session::new("s1", "app", "u1");
        s.events.push(Event::new("inv-1", "agent").with_text("a"));
        s.events.push(Event::new("inv-2", "agent").with_text("b"));
        assert_eq!(s.events_for_invocation("inv-2").count(), 1);
    }
}
