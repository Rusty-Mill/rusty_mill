//! Helpers shared by the integration tests.

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use fs4::fs_std::FileExt;

/// Ports per band. The largest test binary here asks for a few hundred, so
/// this is sized well above that rather than just above it.
const BAND: u32 = 1024;
/// Number of bands. Three times the test binaries in this workspace, so two
/// suites can run at once and still each get a band apiece.
const BANDS: u32 = 39;
/// Bands start above the range a system typically hands out for itself.
const FLOOR: u32 = 20_000;

/// A port nothing else in this test run is using.
///
/// Two problems, and asking the OS for port 0 solves neither on its own.
///
/// Binding to port 0 and dropping the listener leaves a window in which the
/// same port can be handed out twice, so a set of what has already been issued
/// closes the gap *within* one binary.
///
/// That is not enough, because cargo runs the test binaries concurrently and
/// each has its own set. Two of them asking for port 0 at the same moment can
/// be handed the same port, which is how a passing suite fails one run in
/// twenty with `Address already in use` -- somewhere unrelated to whatever was
/// actually being changed.
///
/// So each process draws from a band of its own, claimed by [`claim_band`].
/// Every candidate is still probed, which covers anything outside this run
/// holding a port.
pub async fn free_port() -> u16 {
    static ISSUED: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

    if let Some(base) = band_base() {
        for offset in 0..BAND {
            let port = (base + offset) as u16;
            if !ISSUED.lock().expect("lock").insert(port) {
                continue;
            }
            // Probing costs a bind and tells us the port is genuinely free,
            // which a band alone cannot: something outside this run may hold
            // it.
            if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                drop(listener);
                return port;
            }
        }
    }

    // No band, or a band that ran dry, falls back to what this used to do
    // rather than failing the run. Losing the guarantee is better than turning
    // a sizing mistake here into a suite that cannot pass at all.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind an ephemeral port");
    let port = listener.local_addr().expect("should have an addr").port();
    drop(listener);
    ISSUED.lock().expect("lock").insert(port);
    port
}

/// The first port of this process's band, claimed once.
///
/// The claim outlives the call because the `LazyLock` holds the locked file
/// for as long as the process runs.
fn band_base() -> Option<u32> {
    static CLAIM: LazyLock<Option<(File, u32)>> = LazyLock::new(|| {
        // A fixed directory rather than a fresh one, because the point is to
        // coordinate with processes that did not come from here: the other
        // test binaries cargo started alongside this one.
        let directory = std::env::temp_dir().join("rusty-agent-gateway-test-ports");
        std::fs::create_dir_all(&directory).ok()?;
        claim_band(&directory)
    });
    CLAIM.as_ref().map(|(_, base)| *base)
}

/// Take exclusive hold of a band, and return it with the lock that holds it.
///
/// An earlier version of this keyed the band on `pid % BANDS` and hoped: cargo
/// starts the binaries together, so their ids are close and usually landed in
/// different bands. Usually is the problem. Two binaries sharing a band shared
/// its ports, which put back the rare `Address already in use` this is meant to
/// prevent -- and it came back looking like a flake in whichever test drew the
/// port second.
///
/// So the band is *claimed* rather than derived. An advisory file lock is the
/// right primitive for it: the OS releases the lock when the process ends
/// however it ends, so a killed test run leaves nothing behind to clean up and
/// no band that later runs believe is taken. The files themselves are empty and
/// reused. `fs4` supplies the lock so this works on Windows too (`LockFileEx`)
/// as well as on Unix (`flock`).
///
/// `None` when every band is held, which is a signal to fall back rather than
/// an error -- see [`free_port`].
#[allow(dead_code, reason = "only tests/ports.rs calls this directly")]
pub fn claim_band(directory: &Path) -> Option<(File, u32)> {
    for index in 0..BANDS {
        let Ok(file) = File::create(directory.join(format!("band-{index}"))) else {
            // Someone else's file, on a machine where the tests run as more
            // than one user. Their band, then.
            continue;
        };
        if file.try_lock_exclusive().unwrap_or(false) {
            return Some((file, FLOOR + index * BAND));
        }
    }
    None
}
