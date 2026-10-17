//! A `tracing` layer that turns spans into OpenTelemetry spans.
//!
//! [`SpanLayer`] is the whole of what `tracing-opentelemetry` was used for here:
//! it follows each `tracing` span from creation to close and hands the finished
//! span to a sink (the exporter's queue, in a running server).
//!
//! # What a span becomes
//!
//! - **Name:** the span's name, or its `otel.name` field.
//! - **Attributes:** every field, typed (`str`, `bool`, `i64`, `u64` when it
//!   fits, `f64`; anything else through `Debug`).
//! - **Kind, status:** the `otel.kind` (`server`, `client`, ...) and
//!   `otel.status_code` (`ok`, `error`) fields, with `otel.status_message`.
//! - **Events:** each `tracing` event inside the span, named by its message.
//! - **Timing:** creation to close, which is right however the span is entered.
//!
//! # Where it sits in a trace
//!
//! - A span carrying `trace_id` and `parent_span_id` fields, both valid hex as
//!   [`crate::trace::TraceContext::span`] records them, is a child of that
//!   **remote** span: it adopts the trace id and the parent id, follows the
//!   caller's `sampled` decision, and passes on `tracestate`. A remote parent
//!   wins over a local one, because the caller's trace is the one to join.
//! - Otherwise a span is a child of its `tracing` parent, in the same trace and
//!   with the same sampling decision.
//! - Otherwise it starts a trace, sampled by the configured ratio of trace ids.
//!
//! A span that is not sampled costs a map entry and nothing else: no
//! attributes, no events, nothing exported, and its children follow it.
//!
//! # Attributes set later
//!
//! A `tracing` field has to be declared when its span is created. For keys only
//! known afterwards, [`super::set_attribute`] reaches this layer through the
//! span's subscriber.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use tracing::{
    Event as TracingEvent, Id, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Record},
};
use tracing_subscriber::{Layer, layer::Context};

use super::span::{
    Attribute, AttributeValue, Event, FinishedSpan, Kind, MAX_EVENTS, SpanId, Status, TraceId,
    clip, now_nanos, parse_id, put, sampled_at,
};

/// Takes a finished span; returns whether it was accepted.
pub(crate) type Sink = Box<dyn Fn(FinishedSpan) -> bool + Send + Sync>;

/// Turns `tracing` spans into OpenTelemetry spans; see the [module docs](self).
pub struct SpanLayer {
    open: Mutex<HashMap<u64, Open>>,
    sink: Sink,
    sample_ratio: f64,
    dropped: AtomicU64,
    ids_failed: AtomicBool,
}

impl fmt::Debug for SpanLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanLayer")
            .field("sample_ratio", &self.sample_ratio)
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// A span that has started and not yet closed.
struct Open {
    /// Whether this span is exported. When not, the rest is unused.
    recording: bool,
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
    trace_state: Option<String>,
    name: String,
    kind: Kind,
    status: Status,
    status_message: Option<String>,
    start_wall: u64,
    start: Instant,
    attributes: Vec<Attribute>,
    events: Vec<Event>,
}

/// Where a new span sits in its trace.
struct Lineage {
    recording: bool,
    trace_id: TraceId,
    parent_span_id: Option<SpanId>,
    trace_state: Option<String>,
}

impl SpanLayer {
    /// A layer that samples root spans at `sample_ratio` and gives finished
    /// spans to `sink`.
    pub(crate) fn new(sample_ratio: f64, sink: Sink) -> Self {
        Self {
            open: Mutex::new(HashMap::new()),
            sink,
            sample_ratio,
            dropped: AtomicU64::new(0),
            ids_failed: AtomicBool::new(false),
        }
    }

    /// Finished spans the sink refused.
    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Set an attribute on an open span. Used by [`super::set_attribute`].
    pub(crate) fn set_attribute(&self, id: &Id, key: &str, value: AttributeValue) {
        if let Some(open) = self.lock().get_mut(&id.into_u64())
            && open.recording
        {
            put(&mut open.attributes, key, value);
        }
    }

    /// The open spans. A poisoned lock is recovered: a panic elsewhere must not
    /// turn every later span into another panic.
    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where a span with these `fields` and `local_parent` sits.
    fn lineage(&self, remote: Option<Remote>, local_parent: Option<&Id>) -> Option<Lineage> {
        if let Some(remote) = remote {
            return Some(Lineage {
                recording: remote.sampled,
                trace_id: remote.trace_id,
                parent_span_id: Some(remote.span_id),
                trace_state: remote.trace_state,
            });
        }
        if let Some(parent) = local_parent
            && let Some(parent) = self.lock().get(&parent.into_u64())
        {
            return Some(Lineage {
                recording: parent.recording,
                trace_id: parent.trace_id,
                parent_span_id: Some(parent.span_id),
                trace_state: parent.trace_state.clone(),
            });
        }
        // A root: the trace id decides whether it is recorded.
        let trace_id = random::<16>()?;
        Some(Lineage {
            recording: sampled_at(self.sample_ratio, &trace_id),
            trace_id,
            parent_span_id: None,
            trace_state: None,
        })
    }

    fn note_dropped(&self) {
        let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        // At 1, 2, 4, 8, ...: loud enough to notice, not a line per span.
        if dropped.is_power_of_two() {
            tracing::warn!(dropped, "dropping spans: the exporter is behind or stopped");
        }
    }

    fn note_ids_failed(&self) {
        if !self.ids_failed.swap(true, Ordering::Relaxed) {
            tracing::error!("could not get random span ids; spans will not be exported");
        }
    }
}

impl<S: Subscriber> Layer<S> for SpanLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        let remote = fields.take_remote();

        let local_parent = if attrs.is_contextual() {
            ctx.current_span().id().cloned()
        } else {
            attrs.parent().cloned()
        };

        let open = match self.lineage(remote, local_parent.as_ref()) {
            Some(lineage) if lineage.recording => match random::<8>() {
                Some(span_id) => {
                    let mut open = Open::new(attrs.metadata().name(), span_id, lineage);
                    open.apply(fields);
                    open
                }
                None => {
                    self.note_ids_failed();
                    Open::unrecorded()
                }
            },
            Some(_) => Open::unrecorded(),
            None => {
                self.note_ids_failed();
                Open::unrecorded()
            }
        };
        self.lock().insert(id.into_u64(), open);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut guard = self.lock();
        let Some(open) = guard.get_mut(&id.into_u64()).filter(|open| open.recording) else {
            return;
        };
        let mut fields = Fields::default();
        values.record(&mut fields);
        open.apply(fields);
    }

    fn on_event(&self, event: &TracingEvent<'_>, ctx: Context<'_, S>) {
        let parent = if event.is_contextual() {
            ctx.current_span().id().cloned()
        } else {
            event.parent().cloned()
        };
        let Some(parent) = parent else { return };

        let mut guard = self.lock();
        let Some(open) = guard
            .get_mut(&parent.into_u64())
            .filter(|open| open.recording)
        else {
            return;
        };
        if open.events.len() >= MAX_EVENTS {
            return;
        }
        let mut fields = Fields::for_event();
        event.record(&mut fields);
        open.events.push(Event {
            time_nanos: now_nanos(),
            name: fields
                .message
                .take()
                .unwrap_or_else(|| event.metadata().name().to_owned()),
            attributes: fields.attributes,
        });
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let open = self.lock().remove(&id.into_u64());
        // Outside the lock: the sink and the warning below may log.
        let Some(open) = open.filter(|open| open.recording) else {
            return;
        };
        if !(self.sink)(open.finish()) {
            self.note_dropped();
        }
    }
}

impl Open {
    fn new(name: &str, span_id: SpanId, lineage: Lineage) -> Self {
        Self {
            recording: true,
            trace_id: lineage.trace_id,
            span_id,
            parent_span_id: lineage.parent_span_id,
            trace_state: lineage.trace_state,
            name: name.to_owned(),
            kind: Kind::default(),
            status: Status::default(),
            status_message: None,
            start_wall: now_nanos(),
            start: Instant::now(),
            attributes: Vec::new(),
            events: Vec::new(),
        }
    }

    /// A span that is not exported; children of it follow.
    fn unrecorded() -> Self {
        Self {
            recording: false,
            trace_id: [0; 16],
            span_id: [0; 8],
            parent_span_id: None,
            trace_state: None,
            name: String::new(),
            kind: Kind::default(),
            status: Status::default(),
            status_message: None,
            start_wall: 0,
            start: Instant::now(),
            attributes: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Take what a set of fields says.
    fn apply(&mut self, fields: Fields) {
        if let Some(name) = fields.name {
            self.name = name;
        }
        if let Some(kind) = fields.kind {
            self.kind = kind;
        }
        if let Some(status) = fields.status {
            self.status = status;
        }
        if let Some(message) = fields.status_message {
            self.status_message = Some(message);
        }
        for (key, value) in fields.attributes {
            put(&mut self.attributes, &key, value);
        }
    }

    fn finish(self) -> FinishedSpan {
        // The wall clock marks the start; the monotonic clock measures the span,
        // so a clock step during it cannot make the end precede the start.
        let elapsed = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        FinishedSpan {
            trace_id: self.trace_id,
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            trace_state: self.trace_state,
            name: self.name,
            kind: self.kind,
            start_nanos: self.start_wall,
            end_nanos: self.start_wall.saturating_add(elapsed),
            attributes: self.attributes,
            events: self.events,
            status: self.status,
            status_message: self.status_message,
        }
    }
}

/// `N` random bytes, never all zero, or `None` if the system has none to give.
fn random<const N: usize>() -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    rusty_rand::fill(&mut bytes).ok()?;
    if bytes == [0u8; N] {
        // A valid id is never zero; 2^-64 at worst, so just say the last bit.
        bytes[N - 1] = 1;
    }
    Some(bytes)
}

/// The caller's span, as `TraceContext::span` records it in fields.
struct Remote {
    trace_id: TraceId,
    span_id: SpanId,
    sampled: bool,
    trace_state: Option<String>,
}

/// What a set of `tracing` fields says about a span or event.
#[derive(Default)]
struct Fields {
    event: bool,
    name: Option<String>,
    kind: Option<Kind>,
    status: Option<Status>,
    status_message: Option<String>,
    message: Option<String>,
    attributes: Vec<Attribute>,
}

impl Fields {
    fn for_event() -> Self {
        Self {
            event: true,
            ..Self::default()
        }
    }

    /// The remote parent the fields name, if they name a valid one. Its fields
    /// are taken out of the attributes: they are now the span's own ids.
    fn take_remote(&mut self) -> Option<Remote> {
        let text = |attributes: &[Attribute], key: &str| {
            attributes.iter().find_map(|(k, v)| match v {
                AttributeValue::String(text) if k == key => Some(text.clone()),
                _ => None,
            })
        };
        let trace_id = parse_id::<16>(&text(&self.attributes, "trace_id")?)?;
        let span_id = parse_id::<8>(&text(&self.attributes, "parent_span_id")?)?;
        let sampled = self
            .attributes
            .iter()
            .find_map(|(k, v)| match v {
                AttributeValue::Bool(flag) if k == "sampled" => Some(*flag),
                _ => None,
            })
            .unwrap_or(true);
        let trace_state = text(&self.attributes, "tracestate");
        self.attributes.retain(|(key, _)| {
            !matches!(
                key.as_str(),
                "trace_id" | "parent_span_id" | "sampled" | "tracestate"
            )
        });
        Some(Remote {
            trace_id,
            span_id,
            sampled,
            trace_state,
        })
    }

    fn set(&mut self, key: &str, value: AttributeValue) {
        let text = match &value {
            AttributeValue::String(text) => Some(text.as_str()),
            _ => None,
        };
        match (key, text) {
            ("otel.name", Some(text)) => self.name = Some(clip(text.to_owned())),
            ("otel.kind", Some(text)) => self.kind = Kind::parse(text),
            ("otel.status_code", Some(text)) => self.status = Status::parse(text),
            ("otel.status_message", Some(text)) => {
                self.status_message = Some(clip(text.to_owned()))
            }
            ("message", Some(text)) if self.event => self.message = Some(clip(text.to_owned())),
            _ => put(&mut self.attributes, key, value),
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.set(field.name(), AttributeValue::String(value.to_owned()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.set(field.name(), AttributeValue::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.set(field.name(), AttributeValue::Int(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        let value = i64::try_from(value).map_or_else(
            // OTLP integers are signed; a larger one is kept as text.
            |_| AttributeValue::String(value.to_string()),
            AttributeValue::Int,
        );
        self.set(field.name(), value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.set(field.name(), AttributeValue::Float(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.set(field.name(), AttributeValue::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.set(field.name(), AttributeValue::String(format!("{value:?}")));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::otel::set_attribute;

    const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
    const PARENT: &str = "00f067aa0ba902b7";

    type Collected = Arc<Mutex<Vec<FinishedSpan>>>;

    /// A layer sampling at `ratio` whose sink collects into the returned list
    /// and answers `accept`.
    fn collecting(ratio: f64, accept: bool) -> (Collected, SpanLayer) {
        let collected: Collected = Arc::default();
        let sink = {
            let collected = Arc::clone(&collected);
            Box::new(move |span| {
                collected.lock().expect("lock").push(span);
                accept
            })
        };
        (collected, SpanLayer::new(ratio, sink))
    }

    fn under(layer: SpanLayer, body: impl FnOnce()) {
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, body);
    }

    /// Run `body` under a layer sampling at `ratio`; return what it exported.
    fn run(ratio: f64, body: impl FnOnce()) -> Vec<FinishedSpan> {
        let (collected, layer) = collecting(ratio, true);
        under(layer, body);
        let spans = collected.lock().expect("lock").clone();
        spans
    }

    /// The installed layer, from inside a `with_default`.
    fn with_installed<T>(f: impl FnOnce(&SpanLayer) -> T) -> T {
        let mut f = Some(f);
        let mut out = None;
        tracing::dispatcher::get_default(|dispatch| {
            let layer = dispatch
                .downcast_ref::<SpanLayer>()
                .expect("a SpanLayer is installed");
            out = f.take().map(|f| f(layer));
        });
        out.expect("ran once")
    }

    fn attribute<'a>(span: &'a FinishedSpan, key: &str) -> Option<&'a AttributeValue> {
        span.attributes
            .iter()
            .find_map(|(k, v)| (k == key).then_some(v))
    }

    #[test]
    fn a_root_span_is_exported_with_its_name_and_typed_fields() {
        let spans = run(1.0, || {
            let span = tracing::info_span!(
                "work",
                text = "hello",
                flag = true,
                count = 3_i64,
                big = u64::MAX,
                ratio = 0.5_f64
            );
            drop(span);
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(span.name, "work");
        assert_eq!(span.parent_span_id, None);
        assert_eq!(attribute(span, "text"), Some(&"hello".into()));
        assert_eq!(attribute(span, "flag"), Some(&true.into()));
        assert_eq!(attribute(span, "count"), Some(&AttributeValue::Int(3)));
        // Larger than an OTLP integer: kept as text, not wrapped.
        assert_eq!(attribute(span, "big"), Some(&u64::MAX.to_string().into()));
        assert_eq!(attribute(span, "ratio"), Some(&AttributeValue::Float(0.5)));
        assert!(span.end_nanos >= span.start_nanos);
        assert_ne!(span.trace_id, [0; 16]);
        assert_ne!(span.span_id, [0; 8]);
    }

    #[test]
    fn otel_fields_set_the_name_kind_and_status_and_are_not_attributes() {
        let spans = run(1.0, || {
            let span = tracing::info_span!(
                "mcp.request",
                otel.name = "tools/call",
                otel.kind = "server",
                otel.status_code = "error",
                otel.status_message = "boom",
            );
            drop(span);
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(span.name, "tools/call");
        assert_eq!(span.kind, Kind::Server);
        assert_eq!(span.status, Status::Error);
        assert_eq!(span.status_message.as_deref(), Some("boom"));
        assert!(span.attributes.is_empty(), "{:?}", span.attributes);
    }

    #[test]
    fn a_field_recorded_later_reaches_the_span() {
        let spans = run(1.0, || {
            let span = tracing::info_span!("work", late = tracing::field::Empty);
            span.record("late", "now");
            drop(span);
        });
        assert_eq!(attribute(&spans[0], "late"), Some(&"now".into()));
    }

    #[test]
    fn a_child_shares_its_parents_trace_and_names_it_as_parent() {
        let spans = run(1.0, || {
            let parent = tracing::info_span!("parent");
            let _entered = parent.enter();
            let child = tracing::info_span!("child");
            drop(child);
        });
        let child = spans.iter().find(|s| s.name == "child").expect("child");
        let parent = spans.iter().find(|s| s.name == "parent").expect("parent");
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.parent_span_id, Some(parent.span_id));
        assert_ne!(child.span_id, parent.span_id);
    }

    #[test]
    fn an_explicit_parent_is_honoured_over_the_current_span() {
        let spans = run(1.0, || {
            let a = tracing::info_span!("a");
            let b = tracing::info_span!("b");
            let _entered = b.enter();
            let child = tracing::info_span!(parent: &a, "child");
            drop(child);
        });
        let child = spans.iter().find(|s| s.name == "child").expect("child");
        let a = spans.iter().find(|s| s.name == "a").expect("a");
        assert_eq!(child.parent_span_id, Some(a.span_id));
    }

    #[test]
    fn a_remote_parent_is_adopted_and_its_fields_are_not_attributes() {
        let spans = run(1.0, || {
            let span = tracing::info_span!(
                "mcp.request",
                trace_id = TRACE,
                parent_span_id = PARENT,
                sampled = true,
                tracestate = "vendor=opaque",
                kept = "yes",
            );
            drop(span);
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(crate::otel::span::hex(&span.trace_id), TRACE);
        assert_eq!(
            span.parent_span_id.map(|id| crate::otel::span::hex(&id)),
            Some(PARENT.to_owned())
        );
        assert_eq!(span.trace_state.as_deref(), Some("vendor=opaque"));
        assert_eq!(span.attributes, [("kept".to_owned(), "yes".into())]);
    }

    #[test]
    fn a_remote_parent_wins_over_a_local_one() {
        let spans = run(1.0, || {
            let local = tracing::info_span!("local");
            let _entered = local.enter();
            let span = tracing::info_span!("request", trace_id = TRACE, parent_span_id = PARENT);
            drop(span);
        });
        let request = spans.iter().find(|s| s.name == "request").expect("request");
        assert_eq!(crate::otel::span::hex(&request.trace_id), TRACE);
    }

    #[test]
    fn a_remote_parent_that_was_not_sampled_exports_nothing_below_it() {
        let spans = run(1.0, || {
            let span = tracing::info_span!(
                "request",
                trace_id = TRACE,
                parent_span_id = PARENT,
                sampled = false
            );
            let _entered = span.enter();
            let child = tracing::info_span!("child");
            drop(child);
        });
        assert!(spans.is_empty(), "{spans:?}");
    }

    #[test]
    fn a_malformed_remote_parent_is_ignored_and_the_fields_stay_attributes() {
        let spans = run(1.0, || {
            let span = tracing::info_span!("request", trace_id = "nope", parent_span_id = PARENT);
            drop(span);
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(span.parent_span_id, None, "it should start a trace");
        assert_eq!(attribute(span, "trace_id"), Some(&"nope".into()));
    }

    #[test]
    fn root_spans_follow_the_sample_ratio_and_children_follow_their_root() {
        let none = run(0.0, || {
            let root = tracing::info_span!("root");
            let _entered = root.enter();
            drop(tracing::info_span!("child"));
        });
        assert!(none.is_empty(), "{none:?}");

        let all = run(1.0, || {
            let root = tracing::info_span!("root");
            let _entered = root.enter();
            drop(tracing::info_span!("child"));
        });
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn events_become_span_events_named_by_their_message() {
        let spans = run(1.0, || {
            let span = tracing::info_span!("work");
            let _entered = span.enter();
            tracing::info!(attempt = 2, "retrying");
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        let [event] = &span.events[..] else {
            panic!("{:?}", span.events)
        };
        assert_eq!(event.name, "retrying");
        assert_eq!(
            event.attributes,
            [("attempt".to_owned(), AttributeValue::Int(2))]
        );
        assert!(event.time_nanos >= span.start_nanos);
    }

    #[test]
    fn an_event_outside_any_span_is_ignored() {
        let spans = run(1.0, || tracing::info!("nobody home"));
        assert!(spans.is_empty());
    }

    #[test]
    fn events_and_attributes_are_capped() {
        let spans = run(1.0, || {
            let span = tracing::info_span!("work");
            let _entered = span.enter();
            for _ in 0..(MAX_EVENTS * 2) {
                tracing::info!("again");
            }
            for n in 0..300 {
                set_attribute(&span, &format!("k{n}"), n as i64);
            }
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(span.events.len(), MAX_EVENTS);
        assert_eq!(span.attributes.len(), crate::otel::span::MAX_ATTRIBUTES);
    }

    #[test]
    fn set_attribute_reaches_the_layer_through_the_subscriber() {
        let spans = run(1.0, || {
            let span = tracing::info_span!("work");
            set_attribute(&span, "dynamic.key", "value");
            set_attribute(&span, "number", 7_i64);
        });
        let [span] = &spans[..] else {
            panic!("{spans:?}")
        };
        assert_eq!(attribute(span, "dynamic.key"), Some(&"value".into()));
        assert_eq!(attribute(span, "number"), Some(&AttributeValue::Int(7)));
    }

    #[test]
    fn set_attribute_without_a_layer_or_a_span_does_nothing() {
        set_attribute(&tracing::Span::none(), "k", "v");
        // A real span, but no subscriber at all.
        let span = tracing::info_span!("orphan");
        set_attribute(&span, "k", "v");
    }

    #[test]
    fn a_span_the_sink_refuses_is_counted() {
        let (collected, layer) = collecting(1.0, false);
        under(layer, || {
            drop(tracing::info_span!("a"));
            drop(tracing::info_span!("b"));
            assert_eq!(with_installed(SpanLayer::dropped), 2);
        });
        assert_eq!(collected.lock().expect("lock").len(), 2);
    }

    #[test]
    fn a_closed_span_is_forgotten() {
        let (_, layer) = collecting(1.0, true);
        under(layer, || {
            drop(tracing::info_span!("a"));
            let leaked = with_installed(|layer| layer.lock().len());
            assert_eq!(leaked, 0, "the layer is leaking open spans");
        });
    }

    #[test]
    fn spans_that_are_not_sampled_are_forgotten_too() {
        let (_, layer) = collecting(0.0, true);
        under(layer, || {
            let root = tracing::info_span!("root");
            drop(tracing::info_span!(parent: &root, "child"));
            drop(root);
            let leaked = with_installed(|layer| layer.lock().len());
            assert_eq!(leaked, 0, "unsampled spans are leaking");
        });
    }
}
