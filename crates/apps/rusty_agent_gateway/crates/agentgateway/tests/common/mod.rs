//! Helpers shared by the integration tests.

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use fs4::fs_std::FileExt;

/// Render a filesystem path for splicing into a *double-quoted* YAML scalar.
///
/// Inside double quotes the YAML scanner reads a backslash as the start of an
/// escape, so a Windows path goes in as escape sequences rather than as a path:
/// `C:\Users\...` is rejected outright ("did not find expected hexadecimal
/// number", because `\U` wants eight hex digits), and a path that happens to
/// escape into something legal would be read as the wrong path, which is worse.
/// Doubling each backslash says "a literal backslash". A no-op on a Unix path.
#[allow(
    dead_code,
    reason = "only the suites that splice a path into their config call this"
)]
pub fn yaml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

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

/// The `rusty_tokio` runtime the gateway under test and its mock upstreams
/// run on.
///
/// The suites drive the gateway with tokio clients (`reqwest`, `rmcp`), while
/// the gateway's own timers and sockets, and the `rusty_axum` mock upstreams,
/// are `rusty_tokio`'s and need a runtime of their own. So the gateway is
/// built and served here, and a test reaches it over sockets as any client
/// would.
fn gateway_runtime() -> &'static rusty_tokio::Runtime {
    static RUNTIME: std::sync::OnceLock<rusty_tokio::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        rusty_tokio::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("the gateway runtime should start")
    })
}

/// [`agentgateway::Gateway::build`], on the gateway's runtime.
#[allow(
    dead_code,
    reason = "every suite builds a gateway, but not every one through each helper"
)]
pub async fn build(
    config: &agentgateway_config::Config,
    instruments: Option<std::sync::Arc<rusty_mcp_middleware::otel::metrics::Instruments>>,
) -> anyhow::Result<agentgateway::Gateway> {
    let config = config.clone();
    gateway_runtime()
        .spawn(async move { agentgateway::Gateway::build(&config, instruments).await })
        .await
        .expect("building the gateway should not panic")
}

/// [`agentgateway::serve::run_with_shutdown`], on the gateway's runtime,
/// stopped when the test's `shutdown` is cancelled.
#[allow(dead_code, reason = "not every suite serves without limits")]
pub async fn serve(
    gateway: agentgateway::Gateway,
    addrs: Vec<std::net::SocketAddr>,
    shutdown: tokio_util::sync::CancellationToken,
) -> anyhow::Result<impl std::future::Future<Output = ()> + Send> {
    serve_with_limits(gateway, addrs, shutdown, rusty_axum::Admission::new()).await
}

/// [`agentgateway::serve::run_with_shutdown_and_limits`], as [`serve`].
#[allow(dead_code, reason = "only the limits suite sets limits")]
pub async fn serve_with_limits(
    gateway: agentgateway::Gateway,
    addrs: Vec<std::net::SocketAddr>,
    shutdown: tokio_util::sync::CancellationToken,
    limits: rusty_axum::Admission,
) -> anyhow::Result<impl std::future::Future<Output = ()> + Send> {
    let stop = rusty_tokio::sync::CancellationToken::new();
    let serving = {
        let stop = stop.clone();
        gateway_runtime()
            .spawn(async move {
                agentgateway::serve::run_with_shutdown_and_limits(gateway, addrs, stop, limits)
                    .await
            })
            .await
            .expect("serving should not panic")?
    };
    // The test's token is tokio's; waiting on it needs no tokio runtime.
    gateway_runtime().spawn(async move {
        shutdown.cancelled().await;
        stop.cancel();
    });
    Ok(serving)
}

/// A mock upstream: `router` served on `rusty_axum`, on the gateway's runtime.
///
/// It keeps serving until [`Upstream::stop`] or the end of the test binary:
/// dropping the handle does not stop it, as a spawned `axum::serve` did not.
#[allow(dead_code, reason = "only the suites with an HTTP upstream serve one")]
pub struct Upstream {
    server: Option<rusty_axum::Server>,
}

#[allow(dead_code, reason = "only the suites with an HTTP upstream serve one")]
impl Upstream {
    /// The address the upstream listens on.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.server
            .as_ref()
            .map(rusty_axum::Server::local_addr)
            .expect("a running upstream has an address")
    }

    /// Stops accepting and waits for the requests in flight to be answered,
    /// as axum's graceful shutdown did.
    pub async fn stop(mut self) {
        if let Some(server) = self.server.take() {
            gateway_runtime()
                .spawn(server.shutdown())
                .await
                .expect("stopping an upstream should not panic");
        }
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        // A dropped `rusty_axum::Server` shuts down; an unstopped upstream
        // keeps serving instead.
        std::mem::forget(self.server.take());
    }
}

/// Serves `router` on `addr` (`127.0.0.1:0` for any free port).
#[allow(dead_code, reason = "only the suites with an HTTP upstream serve one")]
pub async fn upstream(addr: impl Into<String>, router: rusty_axum::Router) -> Upstream {
    let addr = addr.into();
    let server = gateway_runtime()
        .spawn(async move {
            let listener = rusty_tokio::io::TcpListener::bind(addr.as_str())
                .await
                .expect("upstream should bind");
            rusty_axum::serve(listener, None, rusty_axum::ServeOptions::default(), router)
                .expect("upstream should serve")
        })
        .await
        .expect("starting an upstream should not panic");
    Upstream {
        server: Some(server),
    }
}

/// `headers` as `(name, value)` pairs, names lowercased as `axum` handed
/// them over.
#[allow(dead_code, reason = "only the suites that record headers call this")]
pub fn header_pairs(headers: &rusty_axum::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
        .collect()
}

/// A scripted unary gRPC server (the `ExtMcp` processor mocks): HTTP/2
/// without TLS on `rusty_hyper`, on the gateway's runtime. Each call's
/// message goes to `reply` with the call's path, and the message `reply`
/// returns is sent back with `grpc-status: 0` as a trailer. The framing is
/// written out here, so the mock shares no gRPC code with the gateway's
/// `rusty_tonic` client. Cancelling the token stops it.
#[allow(dead_code, reason = "only the guardrail suites run a processor")]
pub async fn grpc_upstream<F>(
    reply: F,
) -> (std::net::SocketAddr, tokio_util::sync::CancellationToken)
where
    F: Fn(&str, &[u8]) -> Vec<u8> + Clone + Send + Sync + 'static,
{
    let shutdown = tokio_util::sync::CancellationToken::new();
    let stopping = shutdown.clone();
    let addr = gateway_runtime()
        .spawn(async move {
            let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("processor should bind");
            let server = rusty_hyper::serve(
                listener,
                None,
                rusty_hyper::ServeOptions::default(),
                move |request, respond| grpc_answer(reply.clone(), request, respond),
            )
            .expect("processor should serve");
            let addr = server.local_addr();
            rusty_tokio::spawn(async move {
                stopping.cancelled().await;
                server.shutdown().await;
            });
            addr
        })
        .await
        .expect("starting a processor should not panic");
    (addr, shutdown)
}

/// One call to a [`grpc_upstream`].
async fn grpc_answer<F>(reply: F, mut request: rusty_hyper::Request, respond: rusty_hyper::Respond)
where
    F: Fn(&str, &[u8]) -> Vec<u8>,
{
    let field = |name: &str, value: &str| rusty_hyper::HeaderField::new(name, value);
    let path = request
        .headers
        .iter()
        .find(|h| h.name == b":path")
        .map(|h| String::from_utf8_lossy(&h.value).into_owned())
        .unwrap_or_default();
    let mut body = Vec::new();
    while let Some(Ok(piece)) = request.body.next().await {
        if let rusty_hyper::Body::Data(data) = piece {
            body.extend(data);
        }
    }
    // gRPC frames a message as a 1-byte compression flag and a 4-byte
    // big-endian length, then the protobuf.
    let payload = reply(&path, body.get(5..).unwrap_or_default());
    let mut framed = Vec::with_capacity(payload.len() + 5);
    framed.push(0);
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&payload);

    let head = vec![
        field(":status", "200"),
        field("content-type", "application/grpc"),
    ];
    let Ok(mut send) = respond.send_response(head, false).await else {
        return;
    };
    // The status is a trailer, after the message.
    if send.send_data(framed, false).await.is_ok() {
        let _ = send.send_trailers(vec![field("grpc-status", "0")]).await;
    }
}
