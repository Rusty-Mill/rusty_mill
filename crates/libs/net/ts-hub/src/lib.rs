//! First-party ts2021 coordination server: the server half of what
//! `ts-control` speaks. Plain-HTTP `GET /key` and `POST /ts2021` (upgrade to
//! Noise IK), then HTTP/2 over the Noise channel serving `/machine/register`
//! and the `/machine/map` long-poll.
//!
//! Scope: registration by pre-auth key, a netmap carrying each node's disco
//! key and endpoints, the DERP relay (`GET /derp`) that admits only
//! registered node keys, and a STUN responder ([`stun::serve`]). Every
//! connection, stream and relayed byte count is bounded by [`Limits`].
//! [`Hub::serve_tls`] puts the same endpoints behind TLS from an
//! operator-supplied certificate; ACME is a separate slice
//! (docs/projects/mux/MESH.md).

mod derp;
mod file_store;
mod limits;
mod policy;
mod store;
pub mod stun;
mod tls;
mod zstd;

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusty_h2::driver::{self, Body, Request, Respond};
use rusty_h2::hpack::HeaderField;
use rusty_h2::server::builder::Builder as H2Builder;
use rusty_http::async_tokio::{AsyncTransport, Replay};
use rusty_http::head::ResponseHead;
use rusty_http::{HeaderMap, Method, StatusCode, Version};
use rusty_tls::TlsAcceptor;
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_tokio::net::{TcpListener, TcpStream};
use rusty_tokio::sync::watch;
use ts_control::controlbase::{self, INITIATION_LEN};
use ts_key::MachinePrivate;
use ts_types::tailcfg::{Hostinfo, MapResponse, RegisterResponse};
use ts_types::{DiscoPublic, MachinePublic, NodePublic};

pub use derp::DerpRefusal;
pub use file_store::{FileStore, StoreError};
pub use limits::Limits;
pub use policy::{Grant, Policy, PolicyError, Selector, principal_of};
pub use store::{KeyPolicy, MemoryStore, NodeRec, NodeStore, RegisterError, Revocation};
pub use tls::{TlsConfigError, tls_acceptor};

use limits::{Counter, Keyed};

/// What a connection runs over: a socket, or TLS on one.
trait Io: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Io for T {}

const MAX_HEAD_LEN: usize = 16 * 1024;
const MAX_BODY_LEN: usize = 1 << 20;
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A heartbeat frame this often keeps idle map streams from being reaped.
const KEEPALIVE: Duration = Duration::from_secs(60);

/// A coordination server: the Noise identity plus the shared node table.
pub struct Hub<S: NodeStore = MemoryStore> {
    key: MachinePrivate,
    store: Mutex<S>,
    limits: Limits,
    /// Who may reach whom; closed until set.
    policy: Mutex<Arc<Policy>>,
    relay: derp::Relay,
    /// The `DERPMap` every netmap carries, once the hub knows its address.
    derp_map: Mutex<Option<serde_json::Value>>,
    conns: Counter,
    streams: Counter,
    machine_streams: Keyed<MachinePublic>,
    /// Bumped whenever the node table changes; map streams wait on it.
    changed: watch::Sender<u64>,
    /// Kept so streams can clone a fresh receiver.
    changes: watch::Receiver<u64>,
}

impl Hub {
    /// A hub with a fresh Noise identity and an empty in-memory node table.
    pub fn new() -> Arc<Self> {
        Self::with_store(MemoryStore::default())
    }
}

impl<S: NodeStore> Hub<S> {
    /// A hub with a fresh Noise identity over `store`, default limits.
    pub fn with_store(store: S) -> Arc<Self> {
        Self::with_limits(store, Limits::default())
    }

    /// A hub with a fresh Noise identity over `store`.
    pub fn with_limits(store: S, limits: Limits) -> Arc<Self> {
        Self::with_identity(store, limits, MachinePrivate::generate())
    }

    /// A hub with the given Noise identity, so a restarted hub keeps the key
    /// its nodes already trust (`ts_key::NodeState` can save one).
    pub fn with_identity(store: S, limits: Limits, key: MachinePrivate) -> Arc<Self> {
        let (changed, changes) = watch::channel(0);
        Arc::new(Self {
            key,
            store: Mutex::new(store),
            limits,
            policy: Mutex::new(Arc::new(Policy::default())),
            relay: derp::Relay::new(),
            derp_map: Mutex::new(None),
            conns: Counter::default(),
            streams: Counter::default(),
            machine_streams: Keyed::default(),
            changed,
            changes,
        })
    }

    /// Admits `login` for nodes that register with `key`, under `policy`.
    pub fn add_preauth(
        &self,
        key: &str,
        login: &str,
        policy: KeyPolicy,
    ) -> Result<(), RegisterError> {
        self.locked(|s| s.add_preauth(key, login, policy, unix_now()))
    }

    /// Replaces the rules; every netmap stream is woken to receive its new
    /// peer list and packet filter. A hub with no policy lets nothing through.
    pub fn set_policy(&self, policy: Policy) {
        *self.policy.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(policy);
        self.bump();
    }

    /// Forgets a pre-auth key (see [`NodeStore::remove_preauth`]).
    pub fn remove_preauth(&self, key: &str) -> Result<bool, RegisterError> {
        self.locked(|s| s.remove_preauth(key))
    }

    /// Removes a node and bans its machine key: it drops out of every
    /// netmap, its map streams end, its relay session is closed, and it can
    /// never register again (a reusable pre-auth key does not bring it back).
    /// `false` if the machine was not registered (it is banned all the same,
    /// so a revoke can be issued ahead of a registration). If saving the table fails
    /// the revoke still takes effect, and the error says it is not durable.
    pub fn revoke(&self, machine: &MachinePublic) -> Result<bool, RegisterError> {
        let revoked = self.locked(|s| s.revoke(machine));
        if let Some(node_key) = &revoked.node_key {
            self.relay.kick(node_key);
            self.bump();
        }
        revoked.saved?;
        Ok(revoked.node_key.is_some())
    }

    /// Advertises this hub's relay and STUN responder to nodes as a single
    /// DERP region at `host`. Stock clients speak TLS to `derp_port`, which
    /// is a separate slice; the embedded node speaks plain `GET /derp` on
    /// the hub's own port.
    pub fn advertise(&self, host: &str, derp_port: u16, stun_port: u16) {
        let map = serde_json::json!({"Regions": {"1": {
            "RegionID": 1, "RegionCode": "hub", "RegionName": "hub",
            "Nodes": [{
                "Name": "1a", "RegionID": 1, "HostName": host,
                "DERPPort": derp_port, "STUNPort": stun_port,
            }],
        }}});
        *self.derp_map.lock().unwrap_or_else(|e| e.into_inner()) = Some(map);
    }

    /// The relay's node key, as sent in the DERP `ServerKey` frame.
    pub fn derp_public_key(&self) -> NodePublic {
        self.relay.public_key()
    }

    /// The hub's Noise public key, as served on `GET /key`.
    pub fn public_key(&self) -> MachinePublic {
        self.key.public()
    }

    fn netmap(&self, machine: &MachinePublic, omit_peers: bool) -> Option<MapResponse> {
        // The policy needs every peer to work out sources, so ask for the
        // full map and drop the peers afterwards when the node did not want them.
        let mut map = self.locked(|s| s.netmap(machine, false, unix_now()))?;
        self.policy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .apply(&mut map);
        if omit_peers {
            map.peers = None;
        }
        map.derp_map = self
            .derp_map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Some(map)
    }

    /// A netmap and a change cursor that is current *before* the netmap is
    /// read, so a change landing after the read is never marked seen: the
    /// stream either has it in this frame or wakes for it.
    fn snapshot(
        &self,
        machine: &MachinePublic,
        omit_peers: bool,
    ) -> Option<(MapResponse, watch::Receiver<u64>)> {
        let mut rx = self.changes.clone();
        rx.borrow_and_update();
        Some((self.netmap(machine, omit_peers)?, rx))
    }

    fn locked<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    fn bump(&self) {
        self.changed.send_modify(|v| *v += 1);
    }

    /// Serves plain-HTTP connections from `listener` until it fails. A
    /// connection that ends in an error is logged and does not stop the hub.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        self.accept_loop(listener, |stream| async move { Ok(stream) })
            .await
    }

    /// Like [`serve`](Self::serve), but every connection first completes a TLS
    /// handshake with `acceptor` (see [`tls_acceptor`]): the same endpoints,
    /// no plaintext. This is the port stock clients reach, so `/key`, the
    /// Noise upgrade and DERP are all covered. A connection holds its slot
    /// from accept, so a stalled handshake counts against the cap and ends at
    /// [`Limits::handshake_timeout`].
    pub async fn serve_tls(
        self: Arc<Self>,
        listener: TcpListener,
        acceptor: TlsAcceptor,
    ) -> std::io::Result<()> {
        let timeout = self.limits.handshake_timeout;
        self.accept_loop(listener, move |stream| {
            let acceptor = acceptor.clone();
            async move {
                within(timeout, "TLS handshake", async {
                    let mut tls = acceptor.accept_async(stream)?;
                    tls.complete_handshake().await?;
                    Ok(tls)
                })
                .await
            }
        })
        .await
    }

    /// Accepts connections, takes a slot for each, then lets `wrap` turn the
    /// socket into the stream the protocol runs over (itself, or TLS on it).
    async fn accept_loop<T, Fut>(
        self: Arc<Self>,
        listener: TcpListener,
        wrap: impl Fn(TcpStream) -> Fut,
    ) -> std::io::Result<()>
    where
        T: Io,
        Fut: Future<Output = Result<T, HubError>> + Send + 'static,
    {
        loop {
            let (stream, peer) = listener.accept().await?;
            let Some(slot) = self.conns.acquire(self.limits.max_connections) else {
                rusty_log::warn!("hub: at the connection limit, closing {peer}");
                continue;
            };
            let hub = self.clone();
            let wrapped = wrap(stream);
            rusty_tokio::spawn(async move {
                let _slot = slot;
                let result = match wrapped.await {
                    Ok(stream) => hub.handle(stream).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    rusty_log::warn!("hub: connection from {peer} ended: {e}");
                }
            });
        }
    }

    async fn handle<T: Io>(self: Arc<Self>, stream: T) -> Result<(), HubError> {
        let mut transport = AsyncTransport::new(stream);
        let head = within(self.limits.head_timeout, "request head", async {
            Ok(transport.read_request_head(MAX_HEAD_LEN).await?)
        })
        .await?;
        let path = head.target.split('?').next().unwrap_or("");
        match (&head.method, path) {
            (Method::Get, "/key") => self.serve_key(transport).await,
            (Method::Get, "/derp") => {
                let fast = head.headers.get("Derp-Fast-Start") == Some("1");
                self.serve_derp(transport, fast).await
            }
            (Method::Post, "/ts2021") => {
                let init = decode_initiation(head.headers.get("X-Tailscale-Handshake"))?;
                self.upgrade(transport, &init).await
            }
            _ => respond_plain(transport, 404).await,
        }
    }

    async fn serve_key<T: Io>(&self, mut transport: AsyncTransport<T>) -> Result<(), HubError> {
        let key = self.public_key();
        let body = serde_json::to_vec(&serde_json::json!({
            "legacyPublicKey": key,
            "publicKey": key,
        }))?;
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json")?;
        headers.insert("Content-Length", &body.len().to_string())?;
        headers.insert("Connection", "close")?;
        transport
            .write_response_head(&ResponseHead {
                status: StatusCode::OK,
                reason: "OK".into(),
                version: Version::Http11,
                headers,
            })
            .await?;
        write_all(&mut transport, &body).await
    }

    /// With `Derp-Fast-Start: 1` the client expects no HTTP response at all:
    /// the stream goes straight to DERP frames.
    async fn serve_derp<T: Io>(
        &self,
        mut transport: AsyncTransport<T>,
        fast_start: bool,
    ) -> Result<(), HubError> {
        if !fast_start {
            write_upgrade(&mut transport).await?;
        }
        let (stream, leftover) = transport.into_parts();
        let (read, write) = rusty_tokio::io::split(stream);
        self.relay
            .session(
                Replay::new(leftover, read),
                write,
                |k| self.locked(|s| s.has_node_key(k, unix_now())),
                &self.limits,
            )
            .await?;
        Ok(())
    }

    async fn upgrade<T: Io>(
        self: Arc<Self>,
        mut transport: AsyncTransport<T>,
        init: &[u8; INITIATION_LEN],
    ) -> Result<(), HubError> {
        let mut headers = HeaderMap::new();
        headers.insert("Upgrade", "tailscale-control-protocol")?;
        headers.insert("Connection", "upgrade")?;
        transport
            .write_response_head(&ResponseHead {
                status: StatusCode::SWITCHING_PROTOCOLS,
                reason: "Switching Protocols".into(),
                version: Version::Http11,
                headers,
            })
            .await?;
        let (stream, leftover) = transport.into_parts();
        let (conn, machine) = within(self.limits.handshake_timeout, "Noise handshake", async {
            Ok(
                controlbase::accept_deferred(Replay::new(leftover, stream), &self.key, init)
                    .await?,
            )
        })
        .await?;
        let machine = MachinePublic(machine);

        let mut h2 = H2Builder::new();
        h2.max_concurrent_streams(self.limits.max_streams_per_connection);
        let (mut acceptor, connection) = driver::server(conn, h2);
        rusty_tokio::spawn(async move {
            if let Err(e) = connection.await {
                rusty_log::debug!("hub: noise connection ended: {e}");
            }
        });
        while let Some(next) = acceptor.accept().await {
            let (request, respond) = next?;
            let hub = self.clone();
            rusty_tokio::spawn(async move {
                if let Err(e) = hub.route(machine, request, respond).await {
                    rusty_log::debug!("hub: request ended: {e}");
                }
            });
        }
        Ok(())
    }

    async fn route(
        self: Arc<Self>,
        machine: MachinePublic,
        mut request: Request,
        respond: Respond,
    ) -> Result<(), HubError> {
        let path = header(&request.headers, ":path");
        let body = within(
            self.limits.head_timeout,
            "request body",
            read_body(&mut request),
        )
        .await?;
        match path.as_str() {
            "/machine/register" => self.register(machine, &body, respond).await,
            "/machine/map" => self.map(machine, &body, respond).await,
            _ => send_status(respond, 404).await,
        }
    }

    async fn register(
        &self,
        machine: MachinePublic,
        body: &[u8],
        respond: Respond,
    ) -> Result<(), HubError> {
        let req: RegisterReq = serde_json::from_slice(body)?;
        let auth_key = req.auth.map(|a| a.auth_key).unwrap_or_default();
        let outcome = self.locked(|s| {
            s.register(
                machine,
                req.node_key,
                &req.hostinfo.hostname,
                &auth_key,
                unix_now(),
            )
        });
        let resp = match outcome {
            Ok(()) => {
                self.bump();
                RegisterResponse {
                    machine_authorized: true,
                    ..Default::default()
                }
            }
            Err(e) => RegisterResponse {
                error: e.to_string(),
                ..Default::default()
            },
        };
        send_json(respond, &serde_json::to_vec(&resp)?).await
    }

    async fn map(
        &self,
        machine: MachinePublic,
        body: &[u8],
        respond: Respond,
    ) -> Result<(), HubError> {
        let req: MapReq = serde_json::from_slice(body)?;
        let changed = self.locked(|s| match req.disco_key {
            Some(disco) if !disco.0.iter().all(|b| *b == 0) => {
                s.update(&machine, disco, req.endpoints.clone())
            }
            _ => false,
        });
        if changed {
            self.bump();
        }
        let zstd = req.compress == "zstd";
        let Some((first, mut rx)) = self.snapshot(&machine, req.omit_peers) else {
            return send_status(respond, 404).await;
        };
        if !req.stream {
            return send_frame_body(respond, &first, zstd).await;
        }
        let Some(_slot) = self.streams.acquire(self.limits.max_map_streams) else {
            return send_status(respond, 503).await;
        };
        let Some(_own) = self
            .machine_streams
            .acquire(&machine, self.limits.max_map_streams_per_machine)
        else {
            return send_status(respond, 503).await;
        };
        let mut stream = respond
            .send_response(ok_json_headers(), false)
            .await
            .map_err(HubError::H2)?;
        stream.send_data(frame(&first, zstd)?, false).await?;
        loop {
            let frame_bytes = match rusty_tokio::time::timeout(KEEPALIVE, rx.changed()).await {
                Ok(Ok(())) => {
                    let Some(map) = self.netmap(&machine, req.omit_peers) else {
                        return Ok(());
                    };
                    frame(&map, zstd)?
                }
                Ok(Err(_)) => return Ok(()),
                Err(_) if self.netmap(&machine, true).is_none() => return Ok(()),
                Err(_) => keepalive_frame(zstd)?,
            };
            // A send error means the node went away.
            if stream.send_data(frame_bytes, false).await.is_err() {
                return Ok(());
            }
        }
    }
}

/// What a node's `RegisterRequest` carries that the hub reads.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegisterReq {
    node_key: NodePublic,
    #[serde(default)]
    auth: Option<Auth>,
    #[serde(default)]
    hostinfo: Hostinfo,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Auth {
    #[serde(default)]
    auth_key: String,
}

/// What a node's `MapRequest` carries that the hub reads.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MapReq {
    #[serde(default)]
    disco_key: Option<DiscoPublic>,
    #[serde(default, deserialize_with = "null_to_default")]
    endpoints: Vec<std::net::SocketAddr>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    omit_peers: bool,
    /// `"zstd"` when the node expects each map frame zstd-compressed.
    #[serde(default)]
    compress: String,
}

fn null_to_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(<Option<T> as serde::Deserialize>::deserialize(d)?.unwrap_or_default())
}

/// Everything that can end a connection.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP: {0}")]
    Http(#[from] rusty_http::TransportError),
    #[error("handshake: {0}")]
    Handshake(#[from] controlbase::ConnectError),
    #[error("HTTP/2: {0}")]
    H2(rusty_h2::driver::Error),
    #[error("bad request: {0}")]
    Bad(String),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("DERP: {0}")]
    Derp(#[from] derp::DerpRefusal),
    #[error("TLS: {0}")]
    Tls(#[from] rusty_tls::Error),
    #[error("timed out reading the {0}")]
    Timeout(&'static str),
}

impl From<rusty_h2::driver::Error> for HubError {
    fn from(e: rusty_h2::driver::Error) -> Self {
        Self::H2(e)
    }
}

impl From<rusty_http::Error> for HubError {
    fn from(e: rusty_http::Error) -> Self {
        Self::Bad(e.to_string())
    }
}

/// The only policy until ACLs land: every node may reach every port of every
/// node. Without a filter a stock client drops all inbound traffic.
/// Runs `work` with a deadline; a slow peer ends its own connection.
async fn within<T>(
    limit: Duration,
    what: &'static str,
    work: impl std::future::Future<Output = Result<T, HubError>>,
) -> Result<T, HubError> {
    rusty_tokio::time::timeout(limit, work)
        .await
        .map_err(|_| HubError::Timeout(what))?
}

/// The `101 Switching Protocols` answer to a DERP upgrade.
async fn write_upgrade<T: Io>(transport: &mut AsyncTransport<T>) -> Result<(), HubError> {
    let mut headers = HeaderMap::new();
    headers.insert("Upgrade", "DERP")?;
    headers.insert("Connection", "Upgrade")?;
    transport
        .write_response_head(&ResponseHead {
            status: StatusCode::SWITCHING_PROTOCOLS,
            reason: "Switching Protocols".into(),
            version: Version::Http11,
            headers,
        })
        .await?;
    Ok(())
}

fn decode_initiation(value: Option<&str>) -> Result<[u8; INITIATION_LEN], HubError> {
    let value = value.ok_or_else(|| HubError::Bad("no X-Tailscale-Handshake".into()))?;
    rusty_base64::decode_standard(value)
        .ok()
        .and_then(|b| <[u8; INITIATION_LEN]>::try_from(b).ok())
        .ok_or_else(|| HubError::Bad("bad X-Tailscale-Handshake".into()))
}

async fn write_all<T: Io>(t: &mut AsyncTransport<T>, data: &[u8]) -> Result<(), HubError> {
    use rusty_tokio::io::AsyncWriteExt;
    t.get_mut().write_all(data).await?;
    t.get_mut().flush().await?;
    Ok(())
}

async fn respond_plain<T: Io>(
    mut transport: AsyncTransport<T>,
    status: u16,
) -> Result<(), HubError> {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Length", "0")?;
    headers.insert("Connection", "close")?;
    transport
        .write_response_head(&ResponseHead {
            status: StatusCode::from_u16(status),
            reason: String::new(),
            version: Version::Http11,
            headers,
        })
        .await?;
    write_all(&mut transport, &[]).await
}

fn header(fields: &[HeaderField], name: &str) -> String {
    fields
        .iter()
        .find(|h| h.name == name.as_bytes())
        .map(|h| String::from_utf8_lossy(&h.value).into_owned())
        .unwrap_or_default()
}

async fn read_body(request: &mut Request) -> Result<Vec<u8>, HubError> {
    let mut out = Vec::new();
    while let Some(piece) = request.body.next().await {
        if let Body::Data(chunk) = piece? {
            if out.len() + chunk.len() > MAX_BODY_LEN {
                return Err(HubError::Bad("request body too large".into()));
            }
            out.extend_from_slice(&chunk);
        }
    }
    Ok(out)
}

fn ok_json_headers() -> Vec<HeaderField> {
    vec![
        HeaderField::new(":status", "200"),
        HeaderField::new("content-type", "application/json"),
    ]
}

async fn send_status(respond: Respond, status: u16) -> Result<(), HubError> {
    respond
        .send_response(vec![HeaderField::new(":status", status.to_string())], true)
        .await?;
    Ok(())
}

async fn send_json(respond: Respond, body: &[u8]) -> Result<(), HubError> {
    let mut stream = respond.send_response(ok_json_headers(), false).await?;
    stream.send_data(body.to_vec(), true).await?;
    Ok(())
}

/// One length-prefixed (4 bytes, little endian) JSON `MapResponse` frame.
fn frame(map: &ts_types::tailcfg::MapResponse, compress: bool) -> Result<Vec<u8>, HubError> {
    let mut json = serde_json::to_vec(map)?;
    if compress {
        json = zstd::frame(&json);
    }
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

fn keepalive_frame(compress: bool) -> Result<Vec<u8>, HubError> {
    frame(
        &ts_types::tailcfg::MapResponse {
            keep_alive: true,
            ..Default::default()
        },
        compress,
    )
}

async fn send_frame_body(
    respond: Respond,
    map: &ts_types::tailcfg::MapResponse,
    compress: bool,
) -> Result<(), HubError> {
    let mut stream = respond.send_response(ok_json_headers(), false).await?;
    stream.send_data(frame(map, compress)?, true).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
