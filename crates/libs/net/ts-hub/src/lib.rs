//! First-party ts2021 coordination server: the server half of what
//! `ts-control` speaks. Plain-HTTP `GET /key` and `POST /ts2021` (upgrade to
//! Noise IK), then HTTP/2 over the Noise channel serving `/machine/register`
//! and the `/machine/map` long-poll.
//!
//! Scope: registration by pre-auth key and a netmap carrying each node's
//! disco key and endpoints. The relay, STUN, ACLs, persistence and a TLS
//! story are separate slices (docs/projects/mux/MESH.md).

mod store;
mod zstd;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusty_h2::driver::{self, Body, Request, Respond};
use rusty_h2::hpack::HeaderField;
use rusty_h2::server::builder::Builder as H2Builder;
use rusty_http::async_tokio::{AsyncTransport, Replay};
use rusty_http::head::ResponseHead;
use rusty_http::{HeaderMap, Method, StatusCode, Version};
use rusty_tokio::net::{TcpListener, TcpStream};
use rusty_tokio::sync::watch;
use ts_control::controlbase::{self, INITIATION_LEN};
use ts_key::MachinePrivate;
use ts_types::tailcfg::{
    FilterRule, Hostinfo, MapResponse, NetPortRange, PortRange, RegisterResponse,
};
use ts_types::{DiscoPublic, MachinePublic, NodePublic};

pub use store::{MemoryStore, NodeRec, NodeStore, RegisterError};

const MAX_HEAD_LEN: usize = 16 * 1024;
const MAX_BODY_LEN: usize = 1 << 20;
/// A heartbeat frame this often keeps idle map streams from being reaped.
const KEEPALIVE: Duration = Duration::from_secs(60);

/// A coordination server: the Noise identity plus the shared node table.
pub struct Hub<S: NodeStore = MemoryStore> {
    key: MachinePrivate,
    store: Mutex<S>,
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
    /// A hub with a fresh Noise identity over `store`.
    pub fn with_store(store: S) -> Arc<Self> {
        let (changed, changes) = watch::channel(0);
        Arc::new(Self {
            key: MachinePrivate::generate(),
            store: Mutex::new(store),
            changed,
            changes,
        })
    }

    /// Admits `login` for nodes that register with `key`.
    pub fn add_preauth(&self, key: &str, login: &str) {
        self.locked(|s| s.add_preauth(key, login));
    }

    /// The hub's Noise public key, as served on `GET /key`.
    pub fn public_key(&self) -> MachinePublic {
        self.key.public()
    }

    fn netmap(&self, machine: &MachinePublic, omit_peers: bool) -> Option<MapResponse> {
        let mut map = self.locked(|s| s.netmap(machine, omit_peers))?;
        map.packet_filter = Some(allow_all());
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

    /// Serves connections from `listener` until it fails. A connection that
    /// ends in an error is logged and does not stop the hub.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, peer) = listener.accept().await?;
            let hub = self.clone();
            rusty_tokio::spawn(async move {
                if let Err(e) = hub.handle(stream).await {
                    rusty_log::warn!("hub: connection from {peer} ended: {e}");
                }
            });
        }
    }

    async fn handle(self: Arc<Self>, stream: TcpStream) -> Result<(), HubError> {
        let mut transport = AsyncTransport::new(stream);
        let head = transport.read_request_head(MAX_HEAD_LEN).await?;
        let path = head.target.split('?').next().unwrap_or("");
        match (&head.method, path) {
            (Method::Get, "/key") => self.serve_key(transport).await,
            (Method::Post, "/ts2021") => {
                let init = decode_initiation(head.headers.get("X-Tailscale-Handshake"))?;
                self.upgrade(transport, &init).await
            }
            _ => respond_plain(transport, 404).await,
        }
    }

    async fn serve_key(&self, mut transport: AsyncTransport<TcpStream>) -> Result<(), HubError> {
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

    async fn upgrade(
        self: Arc<Self>,
        mut transport: AsyncTransport<TcpStream>,
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
        let (conn, machine) =
            controlbase::accept_deferred(Replay::new(leftover, stream), &self.key, init).await?;
        let machine = MachinePublic(machine);

        let (mut acceptor, connection) = driver::server(conn, H2Builder::new());
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
        let body = read_body(&mut request).await?;
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
        let outcome =
            self.locked(|s| s.register(machine, req.node_key, &req.hostinfo.hostname, &auth_key));
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
fn allow_all() -> Vec<FilterRule> {
    vec![FilterRule {
        src_ips: vec!["*".into()],
        dst_ports: vec![NetPortRange {
            ip: "*".into(),
            ports: PortRange {
                first: 0,
                last: u16::MAX,
            },
        }],
        ip_proto: Vec::new(),
    }]
}

fn decode_initiation(value: Option<&str>) -> Result<[u8; INITIATION_LEN], HubError> {
    let value = value.ok_or_else(|| HubError::Bad("no X-Tailscale-Handshake".into()))?;
    rusty_base64::decode_standard(value)
        .ok()
        .and_then(|b| <[u8; INITIATION_LEN]>::try_from(b).ok())
        .ok_or_else(|| HubError::Bad("bad X-Tailscale-Handshake".into()))
}

async fn write_all(t: &mut AsyncTransport<TcpStream>, data: &[u8]) -> Result<(), HubError> {
    use rusty_tokio::io::AsyncWriteExt;
    t.get_mut().write_all(data).await?;
    t.get_mut().flush().await?;
    Ok(())
}

async fn respond_plain(
    mut transport: AsyncTransport<TcpStream>,
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
