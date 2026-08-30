//! `SignalSource` impl over the console-control deferral core. No
//! OS-level code here.

use platform::error::Result;
use platform::events::{SignalEvent, SignalSource};

use crate::ffi::win32_surface as w;
use crate::sys::csignals;

/// The Windows deferred-signal source (D6, divergence 003): console
/// control events mapped onto the portable identities — Ctrl-C →
/// `Interrupt`, Ctrl-Break → `Terminate` (Windows has no SIGTERM
/// analog), console close → `Hangup`. Delivery requires a console;
/// detached/service processes receive none of these.
#[derive(Debug, Default)]
pub struct WindowsSignalSource;

fn event_of(ctrl: u32) -> Option<SignalEvent> {
    match ctrl {
        c if c == w::CTRL_C_EVENT => Some(SignalEvent::Interrupt),
        c if c == w::CTRL_BREAK_EVENT => Some(SignalEvent::Terminate),
        c if c == w::CTRL_CLOSE_EVENT => Some(SignalEvent::Hangup),
        _ => None,
    }
}

impl SignalSource for WindowsSignalSource {
    /// One handler covers all three events; `events` is accepted for
    /// signature parity (installing for any subset installs the handler,
    /// and `take` filters nothing — the mapped identities are exactly
    /// the three portable ones).
    fn install(&self, _events: &[SignalEvent]) -> Result<()> {
        csignals::install()
    }

    fn take(&self) -> Option<SignalEvent> {
        csignals::take().and_then(event_of)
    }
}
