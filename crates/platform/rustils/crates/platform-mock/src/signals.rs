//! Scripted `SignalSource`: tests raise events by hand; consumers see
//! the same single-slot, coalescing, take-at-safe-points contract as the
//! native sources.

use std::sync::atomic::{AtomicU32, Ordering};

use platform::error::Result;
use platform::events::{SignalEvent, SignalSource};

const NONE: u32 = 0;

/// A mock deferred-signal source; [`MockSignalSource::raise`] plays the
/// role of the OS handler (one atomic store — bursts coalesce).
#[derive(Debug, Default)]
pub struct MockSignalSource {
    slot: AtomicU32,
}

fn code_of(event: SignalEvent) -> u32 {
    match event {
        SignalEvent::Interrupt => 1,
        SignalEvent::Terminate => 2,
        SignalEvent::Hangup => 3,
    }
}

fn event_of(code: u32) -> Option<SignalEvent> {
    match code {
        1 => Some(SignalEvent::Interrupt),
        2 => Some(SignalEvent::Terminate),
        3 => Some(SignalEvent::Hangup),
        _ => None,
    }
}

impl MockSignalSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deliver `event` the way the OS handler would: one store into the
    /// single slot.
    pub fn raise(&self, event: SignalEvent) {
        self.slot.store(code_of(event), Ordering::SeqCst);
    }
}

impl SignalSource for MockSignalSource {
    fn install(&self, _events: &[SignalEvent]) -> Result<()> {
        Ok(())
    }

    fn take(&self) -> Option<SignalEvent> {
        event_of(self.slot.swap(NONE, Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_consumes_and_bursts_coalesce() {
        let s = MockSignalSource::new();
        s.install(&[SignalEvent::Interrupt]).expect("install");
        assert_eq!(s.take(), None);
        s.raise(SignalEvent::Interrupt);
        s.raise(SignalEvent::Terminate); // burst: latest wins
        assert_eq!(s.take(), Some(SignalEvent::Terminate));
        assert_eq!(s.take(), None, "take consumes");
    }
}
