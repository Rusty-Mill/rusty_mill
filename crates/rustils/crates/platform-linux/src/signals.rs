//! `SignalSource` impl over the sys deferral core. No `unsafe` here.

use platform::error::Result;
use platform::events::{SignalEvent, SignalSource};

use crate::ffi::libc_surface as c;
use crate::sys::signals;

/// The Linux deferred-signal source (D6): `SIGINT`/`SIGTERM`/`SIGHUP`
/// recorded by a one-store handler, consumed at safe points.
#[derive(Debug, Default)]
pub struct LinuxSignalSource;

fn signum_of(event: SignalEvent) -> c::c_int {
    match event {
        SignalEvent::Interrupt => c::SIGINT,
        SignalEvent::Terminate => c::SIGTERM,
        SignalEvent::Hangup => c::SIGHUP,
    }
}

fn event_of(signum: c::c_int) -> Option<SignalEvent> {
    match signum {
        s if s == c::SIGINT => Some(SignalEvent::Interrupt),
        s if s == c::SIGTERM => Some(SignalEvent::Terminate),
        s if s == c::SIGHUP => Some(SignalEvent::Hangup),
        _ => None,
    }
}

impl SignalSource for LinuxSignalSource {
    fn install(&self, events: &[SignalEvent]) -> Result<()> {
        for &event in events {
            signals::install(signum_of(event))?;
        }
        Ok(())
    }

    fn take(&self) -> Option<SignalEvent> {
        signals::take().and_then(event_of)
    }
}
