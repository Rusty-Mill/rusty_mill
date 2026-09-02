//! [`ScenarioBuilder`] -- the Rust port of
//! `meshed.domains.generators.scenario.ScenarioBuilder` (DOM-026..036):
//! a builder that generates causally ordered manpower domain event
//! scenarios, enforcing prerequisite ordering by construction --
//! a person must be activated (`add_status_change(.., "ACTIVE", ..)`)
//! before `add_assignment()`; a position must be authorized
//! (`add_position_authorization()`) before `add_position_fill()` --
//! and linking derived events back to their causal source via
//! `source_event_ids`.
//!
//! # `ScenarioEvent` replaces the source's `list[BaseEvent]`
//!
//! The source's `build()` returns `list[BaseEvent]`, relying on
//! Python's duck typing (`isinstance(event, PersonnelAssigned)`) for
//! callers to discriminate by concrete type. Rust has no common base
//! type the six event structs this builder ever appends can share
//! (`DomainEvent` isn't object-safe -- `EVENT_NAME`/`avro_schema` are
//! associated consts/`Self`-returning), so [`ScenarioEvent`] is a
//! closed enum over exactly those six types, giving callers `match`
//! instead of `isinstance`: exhaustive and compiler-checked, strictly
//! stronger than the source's runtime type check.
//!
//! # `add_*` methods consume and return `Self`, not `&mut Self`
//!
//! The source's `add_*()` methods mutate `self` in place and return it,
//! supporting both a fluent one-expression chain
//! (`ScenarioBuilder().add_x().add_y()`) and calling a method as a bare
//! statement on a named variable, reusing that same variable on the
//! next line. Rust can't support both idioms with one signature: a
//! `&mut self -> &mut Self` method chains fluently only within a
//! single statement (the receiver's temporary doesn't outlive it), so
//! `let builder = ScenarioBuilder::new().add_x(...);` -- binding the
//! *builder itself*, not a call *within* one statement -- won't
//! compile. This port picks `mut self -> Self` (or, for the two
//! fallible methods, `Result<Self, ScenarioError>`): a plain owned
//! value flows through the whole chain, which is what every test
//! capturing the built object in a `let` needs; tests that called a
//! method as a bare statement on a reused variable are rewritten as
//! `let builder = builder.add_x(...);` -- Rust's ownership-respecting
//! shadow-reassignment, not a behavior change.
//!
//! Only [`add_assignment`](ScenarioBuilder::add_assignment) and
//! [`add_position_fill`](ScenarioBuilder::add_position_fill) can fail
//! (matching the source's own `ValueError`s); every other `add_*`
//! method is infallible, so a chain reads
//! `.add_x(...).add_y(...).add_assignment(...)?.add_z(...)` --  `?`
//! unwraps a fallible step back to a plain `Self` the chain continues
//! from.

use crate::events::{
    PersonnelAssigned, PersonnelPromoted, PersonnelSeparated, PositionAuthorizationChanged,
    PositionFilled, StatusChanged,
};
use rusty_err::Error;
use rusty_meshed_core::BaseEvent;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// One event a [`ScenarioBuilder`] can append -- see the module doc for
/// why this exists instead of the source's `list[BaseEvent]`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioEvent {
    StatusChanged(StatusChanged),
    PositionAuthorizationChanged(PositionAuthorizationChanged),
    PersonnelAssigned(PersonnelAssigned),
    PositionFilled(PositionFilled),
    PersonnelPromoted(PersonnelPromoted),
    PersonnelSeparated(PersonnelSeparated),
}

impl ScenarioEvent {
    /// The embedded lineage contract shared by whichever concrete event
    /// this variant carries.
    pub fn base(&self) -> &BaseEvent {
        match self {
            ScenarioEvent::StatusChanged(e) => &e.base,
            ScenarioEvent::PositionAuthorizationChanged(e) => &e.base,
            ScenarioEvent::PersonnelAssigned(e) => &e.base,
            ScenarioEvent::PositionFilled(e) => &e.base,
            ScenarioEvent::PersonnelPromoted(e) => &e.base,
            ScenarioEvent::PersonnelSeparated(e) => &e.base,
        }
    }

    /// The concrete event type's name -- `type(event).__name__` in the
    /// source. What the demo generators' `_EVENT_TOPIC_MAP` (DOM-042)
    /// key on.
    pub fn event_name(&self) -> &'static str {
        match self {
            ScenarioEvent::StatusChanged(_) => "StatusChanged",
            ScenarioEvent::PositionAuthorizationChanged(_) => "PositionAuthorizationChanged",
            ScenarioEvent::PersonnelAssigned(_) => "PersonnelAssigned",
            ScenarioEvent::PositionFilled(_) => "PositionFilled",
            ScenarioEvent::PersonnelPromoted(_) => "PersonnelPromoted",
            ScenarioEvent::PersonnelSeparated(_) => "PersonnelSeparated",
        }
    }

    /// This event's `person_id`, or (only [`PositionAuthorizationChanged`]
    /// lacks one) its `position_id` -- the source's
    /// `getattr(event, "person_id", None) or getattr(event,
    /// "position_id", None)` (`run_scenario.py`'s per-event summary
    /// line, DOM-047). Every other variant has a `person_id`, even
    /// [`PositionFilled`] (which also has a `position_id`) -- Python's
    /// `getattr` checks `person_id` first, so that's what it returns
    /// there too.
    pub fn person_or_position_id(&self) -> &str {
        match self {
            ScenarioEvent::StatusChanged(e) => &e.person_id,
            ScenarioEvent::PositionAuthorizationChanged(e) => &e.position_id,
            ScenarioEvent::PersonnelAssigned(e) => &e.person_id,
            ScenarioEvent::PositionFilled(e) => &e.person_id,
            ScenarioEvent::PersonnelPromoted(e) => &e.person_id,
            ScenarioEvent::PersonnelSeparated(e) => &e.person_id,
        }
    }
}

/// Errors from [`ScenarioBuilder::add_assignment`]/
/// [`ScenarioBuilder::add_position_fill`] -- the source's two
/// `ValueError`s, raised when a causal prerequisite wasn't met.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScenarioError {
    /// `add_assignment` called for a person with no prior
    /// `add_status_change(person_id, "ACTIVE")`.
    #[error(
        "Cannot assign '{0}': person has no prior StatusChanged to ACTIVE. \
         Call add_status_change(person_id, 'ACTIVE') before add_assignment()."
    )]
    PersonNotActive(String),
    /// `add_position_fill` called for a position with no prior
    /// `add_position_authorization`.
    #[error(
        "Cannot fill '{0}': position has no prior PositionAuthorizationChanged. \
         Call add_position_authorization(position_id, ...) before add_position_fill()."
    )]
    PositionNotAuthorized(String),
}

/// Builder for causally ordered manpower domain event scenarios
/// (DOM-026). See the module doc for the chaining/fallibility shape
/// and why [`ScenarioEvent`] replaces the source's `list[BaseEvent]`.
#[derive(Debug)]
pub struct ScenarioBuilder {
    correlation_id: String,
    /// Wall-clock anchor for timestamp generation, UTC whole seconds
    /// since the epoch (the source's `datetime.replace(microsecond=0)`
    /// truncation).
    base_time_epoch_secs: i64,
    events: Vec<ScenarioEvent>,
    time_offset_days: i64,
    active_persons: HashSet<String>,
    authorized_positions: HashSet<String>,
    /// `person_id` -> the `event_id` of their most recent
    /// `PersonnelAssigned`, used to populate `PositionFilled`'s (and a
    /// retroactive correction's) `source_event_ids`.
    assigned_persons: HashMap<String, String>,
}

impl Default for ScenarioBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScenarioBuilder {
    /// A builder with a fresh, random `correlation_id` (UUID v4) and
    /// `base_time` set to now.
    pub fn new() -> Self {
        Self::with_correlation_id(rusty_uuid::Uuid::new_v4().to_string())
    }

    /// A builder with a caller-supplied `correlation_id`.
    pub fn with_correlation_id(correlation_id: impl Into<String>) -> Self {
        let base_time_epoch_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        ScenarioBuilder {
            correlation_id: correlation_id.into(),
            base_time_epoch_secs,
            events: Vec::new(),
            time_offset_days: 0,
            active_persons: HashSet::new(),
            authorized_positions: HashSet::new(),
            assigned_persons: HashMap::new(),
        }
    }

    /// This scenario's shared `correlation_id` (DOM-036).
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// This scenario's wall-clock anchor, as an ISO-8601 UTC string.
    pub fn base_time(&self) -> String {
        format_iso_from_epoch_secs(self.base_time_epoch_secs)
    }

    /// Adds a `StatusChanged` event. If `new_status` is `"ACTIVE"`, the
    /// person becomes eligible for [`add_assignment`](Self::add_assignment)
    /// (DOM-027).
    pub fn add_status_change(
        mut self,
        person_id: impl Into<String>,
        new_status: impl Into<String>,
        previous_status: impl Into<String>,
        days_forward: i64,
    ) -> Self {
        let person_id = person_id.into();
        let new_status = new_status.into();
        let ts = self.next_timestamp(days_forward);
        let event = StatusChanged::new(
            self.correlation_id.clone(),
            person_id.clone(),
            previous_status,
            new_status.clone(),
            ts.clone(),
            ts,
        );
        self.events.push(ScenarioEvent::StatusChanged(event));
        if new_status == "ACTIVE" {
            self.active_persons.insert(person_id);
        }
        self
    }

    /// Adds a `PositionAuthorizationChanged` event
    /// (`authorization_status = "AUTHORIZED"`), marking the position
    /// eligible for [`add_position_fill`](Self::add_position_fill)
    /// (DOM-028).
    pub fn add_position_authorization(
        mut self,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        authorized_grade: impl Into<String>,
        duty_title: impl Into<String>,
        days_forward: i64,
    ) -> Self {
        let position_id = position_id.into();
        let ts = self.next_timestamp(days_forward);
        let event = PositionAuthorizationChanged::new(
            self.correlation_id.clone(),
            position_id.clone(),
            unit_uic,
            authorized_grade,
            duty_title,
            "AUTHORIZED",
            ts.clone(),
            ts,
        );
        self.events
            .push(ScenarioEvent::PositionAuthorizationChanged(event));
        self.authorized_positions.insert(position_id);
        self
    }

    /// Adds a `PersonnelAssigned` event (DOM-029).
    ///
    /// # Errors
    ///
    /// [`ScenarioError::PersonNotActive`] if `person_id` was never
    /// activated via `add_status_change(person_id, "ACTIVE", ..)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_assignment(
        mut self,
        person_id: impl Into<String>,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        duty_title: impl Into<String>,
        grade: impl Into<String>,
        days_forward: i64,
    ) -> Result<Self, ScenarioError> {
        let person_id = person_id.into();
        if !self.active_persons.contains(&person_id) {
            return Err(ScenarioError::PersonNotActive(person_id));
        }
        let ts = self.next_timestamp(days_forward);
        let event = PersonnelAssigned::new(
            self.correlation_id.clone(),
            person_id.clone(),
            position_id,
            unit_uic,
            duty_title,
            grade,
            ts.clone(),
            ts,
        );
        let event_id = event.base.event_id.clone();
        self.events.push(ScenarioEvent::PersonnelAssigned(event));
        self.assigned_persons.insert(person_id, event_id);
        Ok(self)
    }

    /// Adds a `PositionFilled` event, linking `source_event_ids` back
    /// to `person_id`'s most recent `PersonnelAssigned` if one exists
    /// (DOM-030).
    ///
    /// # Errors
    ///
    /// [`ScenarioError::PositionNotAuthorized`] if `position_id` was
    /// never authorized via `add_position_authorization`.
    pub fn add_position_fill(
        mut self,
        position_id: impl Into<String>,
        person_id: impl Into<String>,
        unit_uic: impl Into<String>,
        days_forward: i64,
    ) -> Result<Self, ScenarioError> {
        let position_id = position_id.into();
        if !self.authorized_positions.contains(&position_id) {
            return Err(ScenarioError::PositionNotAuthorized(position_id));
        }
        let person_id = person_id.into();
        let ts = self.next_timestamp(days_forward);
        let mut event = PositionFilled::new(
            self.correlation_id.clone(),
            position_id,
            person_id.clone(),
            unit_uic,
            ts.clone(),
            ts,
        );
        if let Some(source_event_id) = self.assigned_persons.get(&person_id) {
            event.base.source_event_ids = vec![source_event_id.clone()];
        }
        self.events.push(ScenarioEvent::PositionFilled(event));
        Ok(self)
    }

    /// Adds a `PersonnelPromoted` event -- no prerequisite check
    /// (DOM-031).
    pub fn add_promotion(
        mut self,
        person_id: impl Into<String>,
        from_grade: impl Into<String>,
        to_grade: impl Into<String>,
        days_forward: i64,
    ) -> Self {
        let ts = self.next_timestamp(days_forward);
        let event = PersonnelPromoted::new(
            self.correlation_id.clone(),
            person_id,
            from_grade,
            to_grade,
            ts.clone(),
            ts,
        );
        self.events.push(ScenarioEvent::PersonnelPromoted(event));
        self
    }

    /// Adds a `PersonnelSeparated` event -- no prerequisite check
    /// (DOM-032).
    pub fn add_separation(
        mut self,
        person_id: impl Into<String>,
        separation_reason: impl Into<String>,
        days_forward: i64,
    ) -> Self {
        let ts = self.next_timestamp(days_forward);
        let event = PersonnelSeparated::new(
            self.correlation_id.clone(),
            person_id,
            separation_reason,
            ts.clone(),
            ts,
        );
        self.events.push(ScenarioEvent::PersonnelSeparated(event));
        self
    }

    /// Adds a retroactive `PersonnelAssigned` correction:
    /// `effective_date` is `effective_days_ago` days *before*
    /// `base_time`, while `transaction_date` advances forward as
    /// usual -- demonstrating `effective_date < transaction_date`
    /// (DOM-033). Links `source_event_ids` to `person_id`'s prior
    /// assignment if one exists, and becomes the new "most recent
    /// assignment" for `person_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_retroactive_correction(
        mut self,
        person_id: impl Into<String>,
        position_id: impl Into<String>,
        unit_uic: impl Into<String>,
        duty_title: impl Into<String>,
        grade: impl Into<String>,
        effective_days_ago: i64,
        days_forward: i64,
    ) -> Self {
        let person_id = person_id.into();
        let transaction_ts = self.next_timestamp(days_forward);
        let effective_ts =
            format_iso_from_epoch_secs(self.base_time_epoch_secs - effective_days_ago * 86_400);

        let mut event = PersonnelAssigned::new(
            self.correlation_id.clone(),
            person_id.clone(),
            position_id,
            unit_uic,
            duty_title,
            grade,
            effective_ts,
            transaction_ts,
        );
        if let Some(source_event_id) = self.assigned_persons.get(&person_id) {
            event.base.source_event_ids = vec![source_event_id.clone()];
        }
        let event_id = event.base.event_id.clone();
        self.events.push(ScenarioEvent::PersonnelAssigned(event));
        self.assigned_persons.insert(person_id, event_id);
        self
    }

    /// Returns a copy of the event sequence in causal (insertion)
    /// order (DOM-034). Owned and independent of `self` -- mutating it
    /// can't affect the builder's own state (trivially true of any
    /// `Vec<T: Clone>` returned by value in Rust, unlike the source,
    /// which calls this out explicitly because Python needs a
    /// deliberate `list(self._events)` copy to get the same guarantee).
    pub fn build(&self) -> Vec<ScenarioEvent> {
        self.events.clone()
    }

    /// Advances `time_offset_days` by `days_forward` *before* computing
    /// the new timestamp, guaranteeing monotonically non-decreasing
    /// timestamps across calls (DOM-035).
    fn next_timestamp(&mut self, days_forward: i64) -> String {
        self.time_offset_days += days_forward;
        format_iso_from_epoch_secs(self.base_time_epoch_secs + self.time_offset_days * 86_400)
    }
}

/// Formats a Unix epoch-seconds instant as an ISO-8601 UTC string
/// (`YYYY-MM-DDTHH:MM:SSZ`) -- the same hand-rolled civil-from-days
/// algorithm `rusty_meshed_core::BaseEvent`'s own `now_iso()` uses
/// (duplicated here since that one is private and only ever formats
/// "now"), extended to accept an arbitrary instant via
/// [`i64::div_euclid`]/[`i64::rem_euclid`] rather than an
/// always-non-negative `u64`, since [`ScenarioBuilder::add_retroactive_correction`]
/// needs a *past* instant, not just "now".
fn format_iso_from_epoch_secs(total_secs: i64) -> String {
    let mut days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The simplest complete scenario: status_change -> authorize ->
    /// assign -> fill.
    fn build_minimal() -> (ScenarioBuilder, Vec<ScenarioEvent>) {
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_position_authorization("POS1", "UNIT-A", "E5", "Rifleman", 1)
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap()
            .add_position_fill("POS1", "P1", "UNIT-A", 1)
            .unwrap();
        let events = builder.build();
        (builder, events)
    }

    // -- Causal ordering ---------------------------------------------

    #[test]
    fn status_change_before_assignment() {
        let (_, events) = build_minimal();
        let names: Vec<&str> = events.iter().map(ScenarioEvent::event_name).collect();
        let status_idx = names.iter().position(|n| *n == "StatusChanged").unwrap();
        let assigned_idx = names
            .iter()
            .position(|n| *n == "PersonnelAssigned")
            .unwrap();
        assert!(status_idx < assigned_idx);
    }

    #[test]
    fn authorization_before_fill() {
        let (_, events) = build_minimal();
        let names: Vec<&str> = events.iter().map(ScenarioEvent::event_name).collect();
        let auth_idx = names
            .iter()
            .position(|n| *n == "PositionAuthorizationChanged")
            .unwrap();
        let fill_idx = names.iter().position(|n| *n == "PositionFilled").unwrap();
        assert!(auth_idx < fill_idx);
    }

    #[test]
    fn assignment_before_fill() {
        let (_, events) = build_minimal();
        let names: Vec<&str> = events.iter().map(ScenarioEvent::event_name).collect();
        let assigned_idx = names
            .iter()
            .position(|n| *n == "PersonnelAssigned")
            .unwrap();
        let fill_idx = names.iter().position(|n| *n == "PositionFilled").unwrap();
        assert!(assigned_idx < fill_idx);
    }

    #[test]
    fn complete_causal_chain_order() {
        let (_, events) = build_minimal();
        let names: Vec<&str> = events.iter().map(ScenarioEvent::event_name).collect();
        assert_eq!(
            names,
            vec![
                "StatusChanged",
                "PositionAuthorizationChanged",
                "PersonnelAssigned",
                "PositionFilled",
            ]
        );
    }

    // -- correlation_id -------------------------------------------------

    #[test]
    fn all_events_share_correlation_id() {
        let (_, events) = build_minimal();
        assert!(!events.is_empty());
        let expected = &events[0].base().correlation_id;
        for event in &events {
            assert_eq!(&event.base().correlation_id, expected);
        }
    }

    #[test]
    fn custom_correlation_id_propagated() {
        let custom_id = "test-correlation-id-12345";
        let builder = ScenarioBuilder::with_correlation_id(custom_id)
            .add_status_change("P1", "ACTIVE", "NONE", 1);
        for event in builder.build() {
            assert_eq!(event.base().correlation_id, custom_id);
        }
    }

    #[test]
    fn different_scenarios_different_correlation_ids() {
        let b1 = ScenarioBuilder::new();
        let b2 = ScenarioBuilder::new();
        assert_ne!(b1.correlation_id(), b2.correlation_id());
    }

    // -- source_event_ids linkage ----------------------------------------

    #[test]
    fn position_filled_sources_personnel_assigned() {
        let (_, events) = build_minimal();
        let assigned = events
            .iter()
            .find_map(|e| match e {
                ScenarioEvent::PersonnelAssigned(a) => Some(a),
                _ => None,
            })
            .unwrap();
        let filled = events
            .iter()
            .find_map(|e| match e {
                ScenarioEvent::PositionFilled(f) => Some(f),
                _ => None,
            })
            .unwrap();
        assert!(filled
            .base
            .source_event_ids
            .contains(&assigned.base.event_id));
    }

    #[test]
    fn root_events_have_empty_source_event_ids() {
        let (_, events) = build_minimal();
        let status = events
            .iter()
            .find(|e| matches!(e, ScenarioEvent::StatusChanged(_)))
            .unwrap();
        assert!(status.base().source_event_ids.is_empty());
    }

    #[test]
    fn authorization_event_has_empty_source_event_ids() {
        let (_, events) = build_minimal();
        let auth = events
            .iter()
            .find(|e| matches!(e, ScenarioEvent::PositionAuthorizationChanged(_)))
            .unwrap();
        assert!(auth.base().source_event_ids.is_empty());
    }

    // -- Timestamp monotonicity -------------------------------------------

    #[test]
    fn timestamps_monotonically_non_decreasing() {
        let (_, events) = build_minimal();
        for pair in events.windows(2) {
            assert!(pair[1].base().timestamp >= pair[0].base().timestamp);
        }
    }

    #[test]
    fn timestamps_strictly_increasing_with_forward_days() {
        // `BaseEvent.timestamp` (checked by the previous test) is a
        // real wall-clock instant with only whole-second resolution in
        // this port (`now_iso()`'s own design) -- unreliable for a
        // *strict* inequality between two events built microseconds
        // apart in a test. `effective_date` is what `days_forward`
        // actually advances (`_next_timestamp`, DOM-035), so it's what
        // this test needs to check for a deterministic result; the
        // source's own test happens to pass via microsecond-resolution
        // wall-clock timestamps, but the property being verified is
        // `_next_timestamp`'s own monotonic advance, not wall-clock
        // construction order.
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_status_change("P1", "TDY", "ACTIVE", 5);
        let events = builder.build();
        let effective_date = |event: &ScenarioEvent| match event {
            ScenarioEvent::StatusChanged(e) => e.effective_date.clone(),
            other => panic!("expected StatusChanged, got {other:?}"),
        };
        assert!(effective_date(&events[0]) < effective_date(&events[1]));
    }

    // -- Retroactive correction -------------------------------------------

    #[test]
    fn retroactive_correction_effective_before_transaction() {
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_position_authorization("POS1", "UNIT-A", "E5", "Rifleman", 1)
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap()
            .add_retroactive_correction("P1", "POS1", "UNIT-A", "Rifleman", "E5", 30, 1);
        let events = builder.build();
        let retro = events.last().unwrap();
        match retro {
            ScenarioEvent::PersonnelAssigned(a) => {
                assert!(a.effective_date < a.transaction_date);
            }
            other => panic!("expected PersonnelAssigned, got {other:?}"),
        }
    }

    #[test]
    fn retroactive_effective_date_is_in_the_past() {
        let mut builder = ScenarioBuilder::new();
        builder = builder.add_status_change("P1", "ACTIVE", "NONE", 1);
        builder = builder.add_position_authorization("POS1", "UNIT-A", "E5", "Rifleman", 1);
        builder = builder
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap();
        let base_time = builder.base_time();
        builder =
            builder.add_retroactive_correction("P1", "POS1", "UNIT-A", "Rifleman", "E5", 10, 1);
        let events = builder.build();
        let retro = events.last().unwrap();
        match retro {
            ScenarioEvent::PersonnelAssigned(a) => assert!(a.effective_date < base_time),
            other => panic!("expected PersonnelAssigned, got {other:?}"),
        }
    }

    // -- Precondition enforcement -----------------------------------------

    #[test]
    fn assignment_without_prior_status_change_raises() {
        let builder = ScenarioBuilder::new();
        let err = builder
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap_err();
        assert_eq!(err, ScenarioError::PersonNotActive("P1".to_string()));
    }

    #[test]
    fn position_fill_without_prior_authorization_raises() {
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_position_authorization("POS1", "UNIT-A", "E5", "Rifleman", 1)
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap();
        let err = builder
            .add_position_fill("POS2", "P1", "UNIT-A", 1)
            .unwrap_err();
        assert_eq!(
            err,
            ScenarioError::PositionNotAuthorized("POS2".to_string())
        );
    }

    #[test]
    fn assignment_requires_active_status_specifically() {
        // A StatusChanged to a non-ACTIVE new_status must not unlock
        // assignment.
        let builder = ScenarioBuilder::new().add_status_change("P1", "TDY", "NONE", 1);
        let err = builder
            .add_assignment("P1", "POS1", "UNIT-A", "Rifleman", "E5", 1)
            .unwrap_err();
        assert_eq!(err, ScenarioError::PersonNotActive("P1".to_string()));
    }

    // -- Richer scenarios --------------------------------------------------

    #[test]
    fn multi_person_scenario() {
        let mut builder = ScenarioBuilder::new();
        for i in 1..=3 {
            let person = format!("P{i}");
            let position = format!("POS{i}");
            builder = builder.add_status_change(&person, "ACTIVE", "NONE", 1);
            builder = builder.add_position_authorization(&position, "UNIT-A", "E5", "Rifleman", 1);
            builder = builder
                .add_assignment(&person, &position, "UNIT-A", "Rifleman", "E5", 1)
                .unwrap();
            builder = builder
                .add_position_fill(&position, &person, "UNIT-A", 1)
                .unwrap();
        }
        let events = builder.build();
        assert_eq!(events.len(), 12);
        let corr_ids: HashSet<&str> = events
            .iter()
            .map(|e| e.base().correlation_id.as_str())
            .collect();
        assert_eq!(corr_ids.len(), 1);
    }

    #[test]
    fn promotion_event_included() {
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_promotion("P1", "E4", "E5", 1);
        let events = builder.build();
        assert!(events
            .iter()
            .any(|e| matches!(e, ScenarioEvent::PersonnelPromoted(_))));
    }

    #[test]
    fn separation_event_included() {
        let builder = ScenarioBuilder::new()
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_separation("P1", "ETS", 1);
        let events = builder.build();
        assert!(events
            .iter()
            .any(|e| matches!(e, ScenarioEvent::PersonnelSeparated(_))));
    }

    #[test]
    fn build_returns_a_fresh_copy_each_call() {
        let builder = ScenarioBuilder::new().add_status_change("P1", "ACTIVE", "NONE", 1);
        let events1 = builder.build();
        let events2 = builder.build();
        assert_eq!(events1.len(), events2.len());
        assert_eq!(events1, events2);
    }

    #[test]
    fn promotion_shares_correlation_id() {
        let cid = "fixed-corr-id";
        let builder = ScenarioBuilder::with_correlation_id(cid)
            .add_status_change("P1", "ACTIVE", "NONE", 1)
            .add_promotion("P1", "E4", "E5", 1);
        for event in builder.build() {
            assert_eq!(event.base().correlation_id, cid);
        }
    }
}
