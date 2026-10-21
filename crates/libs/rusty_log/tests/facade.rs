//! The facade is the `tracing` API: events, spans and subscribers reached
//! through `rusty_log` behave exactly as through `tracing`.

use std::sync::{Arc, Mutex};

use rusty_log::tracing_subscriber::{self, layer::SubscriberExt, Layer};
use rusty_log::{Instrument, Level};

/// Collects `(level, message)` of every event, and the names of new spans.
#[derive(Clone, Default)]
struct Capture {
    events: Arc<Mutex<Vec<(Level, String)>>>,
    spans: Arc<Mutex<Vec<String>>>,
}

struct Message(String);

impl rusty_log::field::Visit for Message {
    fn record_debug(&mut self, field: &rusty_log::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: rusty_log::Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &rusty_log::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.events
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message.0));
    }

    fn on_new_span(
        &self,
        attrs: &rusty_log::span::Attributes<'_>,
        _: &rusty_log::span::Id,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.spans
            .lock()
            .unwrap()
            .push(attrs.metadata().name().to_owned());
    }
}

fn capturing() -> (Capture, impl rusty_log::Subscriber) {
    let capture = Capture::default();
    (
        capture.clone(),
        tracing_subscriber::registry().with(capture),
    )
}

#[test]
fn events_at_every_level_reach_the_subscriber() {
    let (capture, subscriber) = capturing();
    rusty_log::subscriber::with_default(subscriber, || {
        rusty_log::error!("e");
        rusty_log::warn!("w");
        rusty_log::info!(answer = 42, "i");
        rusty_log::debug!("d");
        rusty_log::trace!("t");
    });
    let seen = capture.events.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            (Level::ERROR, "e".to_owned()),
            (Level::WARN, "w".to_owned()),
            (Level::INFO, "i".to_owned()),
            (Level::DEBUG, "d".to_owned()),
            (Level::TRACE, "t".to_owned()),
        ]
    );
}

#[test]
fn spans_are_created_and_instrument_runs_a_future_inside_one() {
    let (capture, subscriber) = capturing();
    let _guard = rusty_log::subscriber::set_default(subscriber);

    let _plain = rusty_log::info_span!("plain").entered();
    let ran = std::future::ready(7).instrument(rusty_log::info_span!("instrumented"));
    let value = block_on_ready(ran);

    assert_eq!(value, 7);
    assert_eq!(
        *capture.spans.lock().unwrap(),
        vec!["plain".to_owned(), "instrumented".to_owned()]
    );
}

/// Polls a future that is ready on its first poll; enough for `ready()`.
fn block_on_ready<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};

    // `Waker::noop` is 1.85, above this crate's declared rust-version.
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }

    let waker = Waker::from(Arc::new(Noop));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the future was expected to be ready"),
    }
}
