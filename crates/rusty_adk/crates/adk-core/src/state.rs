//! Session state with ADK's prefix-scoped persistence semantics.
//!
//! State is a flat string-keyed JSON map. A key's prefix decides how far the
//! value travels and how long it survives:
//!
//! | Prefix   | Scope                                   | Persisted |
//! |----------|-----------------------------------------|-----------|
//! | `app:`   | all users, all sessions of an app       | yes       |
//! | `user:`  | one user, across their sessions         | yes       |
//! | `temp:`  | the current invocation only             | no        |
//! | *(none)* | the current session                     | yes       |
//!
//! Writes go to a pending delta rather than to the committed map. The delta
//! rides out on an [`Event`](crate::Event)'s
//! [`EventActions::state_delta`](crate::EventActions::state_delta) and only
//! lands when the runner processes that event. Reads see pending writes
//! immediately — ADK calls these "dirty reads", and they are what lets a tool
//! and a callback coordinate within a single step.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Prefix marking state shared by every user of an app.
pub const APP_PREFIX: &str = "app:";
/// Prefix marking state shared across one user's sessions.
pub const USER_PREFIX: &str = "user:";
/// Prefix marking state discarded when the invocation ends.
pub const TEMP_PREFIX: &str = "temp:";

/// How far a state key's value travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateScope {
    /// Shared by every user and session of the app (`app:`).
    App,
    /// Shared across one user's sessions (`user:`).
    User,
    /// Confined to the current session (no prefix).
    Session,
    /// Discarded when the invocation ends (`temp:`).
    Temp,
}

impl StateScope {
    /// Classifies a key by its prefix.
    pub fn of(key: &str) -> Self {
        if key.starts_with(APP_PREFIX) {
            StateScope::App
        } else if key.starts_with(USER_PREFIX) {
            StateScope::User
        } else if key.starts_with(TEMP_PREFIX) {
            StateScope::Temp
        } else {
            StateScope::Session
        }
    }

    /// Whether values in this scope outlive the invocation.
    pub fn is_persistent(&self) -> bool {
        !matches!(self, StateScope::Temp)
    }
}

/// A session's key/value scratchpad, with a pending write delta.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Values already committed by the session service.
    #[serde(default, flatten)]
    base: BTreeMap<String, Value>,
    /// Writes not yet carried out on an event.
    #[serde(default, skip)]
    delta: BTreeMap<String, Value>,
}

impl State {
    /// An empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds committed state from an existing map, with no pending writes.
    pub fn from_map(base: BTreeMap<String, Value>) -> Self {
        Self {
            base,
            delta: BTreeMap::new(),
        }
    }

    /// Reads a key, preferring a pending write over the committed value.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.delta.get(key).or_else(|| self.base.get(key))
    }

    /// Reads a key and deserializes it into `T`.
    ///
    /// Returns `None` when the key is absent or the value does not fit `T`.
    pub fn get_as<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Option<T> {
        self.get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Reads a key, falling back to `default` when absent or ill-typed.
    pub fn get_or<T: for<'de> Deserialize<'de>>(&self, key: &str, default: T) -> T {
        self.get_as(key).unwrap_or(default)
    }

    /// Stages a write. The value is readable immediately but is not persisted
    /// until the delta is carried out on an event and processed by the runner.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.delta.insert(key.into(), value.into());
    }

    /// Stages a write of any serializable value.
    ///
    /// Returns an error if `value` cannot be represented as JSON — state must
    /// stay serializable to survive a persistent session store.
    pub fn set_json<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: &T,
    ) -> Result<(), serde_json::Error> {
        self.delta.insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    /// True when the key is present in either the delta or the committed map.
    pub fn contains_key(&self, key: &str) -> bool {
        self.delta.contains_key(key) || self.base.contains_key(key)
    }

    /// Every key visible right now, committed and pending, without duplicates.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.base
            .keys()
            .filter(|k| !self.delta.contains_key(*k))
            .chain(self.delta.keys())
    }

    /// The effective state: committed values overlaid with pending writes.
    pub fn to_map(&self) -> BTreeMap<String, Value> {
        let mut merged = self.base.clone();
        merged.extend(self.delta.iter().map(|(k, v)| (k.clone(), v.clone())));
        merged
    }

    /// The pending writes, without clearing them.
    pub fn delta(&self) -> &BTreeMap<String, Value> {
        &self.delta
    }

    /// True when there is nothing staged.
    pub fn has_delta(&self) -> bool {
        !self.delta.is_empty()
    }

    /// Removes and returns the pending writes, for attaching to an event.
    pub fn take_delta(&mut self) -> BTreeMap<String, Value> {
        std::mem::take(&mut self.delta)
    }

    /// Applies a delta to the committed map, dropping `temp:` keys.
    ///
    /// This is what a session service calls when it processes an event.
    /// Temporary keys are filtered here rather than at write time so that a
    /// tool can still read its own `temp:` scratch values within the
    /// invocation that wrote them.
    pub fn commit(&mut self, delta: impl IntoIterator<Item = (String, Value)>) {
        for (key, value) in delta {
            if StateScope::of(&key) == StateScope::Temp {
                continue;
            }
            self.base.insert(key, value);
        }
    }

    /// Folds pending writes into the committed map and returns what was applied.
    ///
    /// The returned delta includes `temp:` keys — they belong on the event for
    /// observability even though [`State::commit`] refuses to persist them.
    pub fn commit_pending(&mut self) -> BTreeMap<String, Value> {
        let delta = self.take_delta();
        self.commit(delta.clone());
        delta
    }

    /// Drops every `temp:` key. Called when an invocation ends.
    pub fn clear_temp(&mut self) {
        self.base
            .retain(|k, _| StateScope::of(k) != StateScope::Temp);
        self.delta
            .retain(|k, _| StateScope::of(k) != StateScope::Temp);
    }

    /// The committed entries in one scope, keyed without their prefix.
    pub fn scoped(&self, scope: StateScope) -> BTreeMap<String, Value> {
        let prefix = match scope {
            StateScope::App => APP_PREFIX,
            StateScope::User => USER_PREFIX,
            StateScope::Temp => TEMP_PREFIX,
            StateScope::Session => "",
        };
        self.to_map()
            .into_iter()
            .filter(|(k, _)| StateScope::of(k) == scope)
            .map(|(k, v)| (k[prefix.len()..].to_string(), v))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_is_read_from_the_prefix() {
        assert_eq!(StateScope::of("app:discount"), StateScope::App);
        assert_eq!(StateScope::of("user:lang"), StateScope::User);
        assert_eq!(StateScope::of("temp:scratch"), StateScope::Temp);
        assert_eq!(StateScope::of("step"), StateScope::Session);
    }

    #[test]
    fn pending_writes_are_readable_before_commit() {
        let mut s = State::new();
        s.set("status", "processing");
        assert_eq!(s.get("status"), Some(&json!("processing")));
        assert!(s.has_delta());
    }

    #[test]
    fn delta_overrides_committed_value() {
        let mut s = State::from_map([("k".into(), json!(1))].into());
        s.set("k", 2);
        assert_eq!(s.get("k"), Some(&json!(2)));
        s.commit_pending();
        assert_eq!(s.get("k"), Some(&json!(2)));
        assert!(!s.has_delta());
    }

    #[test]
    fn temp_keys_are_reported_but_never_committed() {
        let mut s = State::new();
        s.set("temp:scratch", 42);
        s.set("keep", 1);
        let delta = s.commit_pending();

        // Both appear on the event...
        assert_eq!(delta.len(), 2);
        // ...but only the durable one survives in committed state.
        assert_eq!(s.get("keep"), Some(&json!(1)));
        assert_eq!(s.get("temp:scratch"), None);
    }

    #[test]
    fn clear_temp_drops_pending_and_committed_temp_keys() {
        let mut s = State::from_map([("temp:a".into(), json!(1)), ("b".into(), json!(2))].into());
        s.set("temp:c", 3);
        s.clear_temp();
        assert!(!s.contains_key("temp:a"));
        assert!(!s.contains_key("temp:c"));
        assert!(s.contains_key("b"));
    }

    #[test]
    fn scoped_strips_the_prefix() {
        let s = State::from_map(
            [
                ("user:login_count".into(), json!(5)),
                ("app:flag".into(), json!(true)),
                ("plain".into(), json!("x")),
            ]
            .into(),
        );
        assert_eq!(
            s.scoped(StateScope::User),
            [("login_count".into(), json!(5))].into()
        );
        assert_eq!(
            s.scoped(StateScope::Session),
            [("plain".into(), json!("x"))].into()
        );
    }

    #[test]
    fn keys_does_not_duplicate_overridden_entries() {
        let mut s = State::from_map([("k".into(), json!(1))].into());
        s.set("k", 2);
        s.set("j", 3);
        let mut keys: Vec<_> = s.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec!["j".to_string(), "k".to_string()]);
    }

    #[test]
    fn get_or_falls_back_on_missing_and_mistyped() {
        let s = State::from_map([("n".into(), json!("not a number"))].into());
        assert_eq!(s.get_or::<i64>("missing", 7), 7);
        assert_eq!(s.get_or::<i64>("n", 7), 7);
    }
}
