//! Helpers shared by the integration tests.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

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
/// So each process draws from its own band, keyed by process id. Cargo spawns
/// the binaries together and their ids are close, so they land in different
/// bands; a band is wide enough that no run exhausts one. Every candidate is
/// still probed, which covers anything outside this run holding a port.
pub async fn free_port() -> u16 {
    /// Ports per band. The largest test binary here asks for a few hundred,
    /// so this is sized well above that rather than just above it.
    const BAND: u32 = 1024;
    /// Number of bands. More than the test binaries in this workspace, so the
    /// near-consecutive ids cargo hands them land in different ones.
    const BANDS: u32 = 39;
    /// Bands start above the range a system typically hands out for itself.
    const FLOOR: u32 = 20_000;

    static BASE: LazyLock<u32> = LazyLock::new(|| FLOOR + (std::process::id() % BANDS) * BAND);
    static ISSUED: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

    for offset in 0..BAND {
        let port = (*BASE + offset) as u16;
        if !ISSUED.lock().expect("lock").insert(port) {
            continue;
        }
        // Probing costs a bind and tells us the port is genuinely free, which
        // a band alone cannot: something outside this run may hold it.
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            drop(listener);
            return port;
        }
    }

    // A band that runs dry falls back to what this used to do rather than
    // failing the run. Losing the guarantee is better than turning a sizing
    // mistake here into a suite that cannot pass at all.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind an ephemeral port");
    let port = listener.local_addr().expect("should have an addr").port();
    drop(listener);
    ISSUED.lock().expect("lock").insert(port);
    port
}
