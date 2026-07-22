//! The high-level control client: establishes the Noise channel, runs an
//! HTTP/2 client connection over it (like Go's `x/net/http2` on the noise
//! conn), and exposes `register` and the netmap long-poll.
//!
//! Mirrors Go `control/controlclient` at the level Phase 2 needs.

use bytes::Bytes;
use h2::client::SendRequest;
use http::{Method, Request};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use ts_key::MachinePrivate;
use ts_types::tailcfg::{
    CURRENT_CAPABILITY_VERSION, MapRequest, MapResponse, RegisterRequest, RegisterResponse,
};

use crate::controlbase::Conn;
use crate::controlhttp::{self, ControlHttpError, ControlUrl};

/// The 5-byte magic prefixing an optional server "early payload".
const EARLY_PAYLOAD_MAGIC: &[u8; 5] = b"\xff\xff\xffTS";
const EARLY_HEADER_LEN: usize = 9; // magic(5) + BE length(4)

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    ControlHttp(#[from] ControlHttpError),
    #[error("HTTP/2 error: {0}")]
    H2(#[from] h2::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control server returned HTTP {0} for {1}")]
    HttpStatus(u16, String),
    #[error("registration rejected: {0}")]
    RegisterRejected(String),
    #[error("registration requires interactive login at {0}")]
    NeedsLogin(String),
    #[error("map response frame exceeds sanity limit ({0} bytes)")]
    FrameTooLarge(u32),
    #[error("map stream ended unexpectedly")]
    StreamEnded,
}

/// A control client bound to one tailnet identity.
pub struct ControlClient {
    url: ControlUrl,
    machine_key: MachinePrivate,
    control_key: [u8; 32],
    hostinfo: ts_types::tailcfg::Hostinfo,
}

impl ControlClient {
    /// Connects to `control_url` (`http://host:port`), fetching the server's
    /// Noise key. Does not register yet.
    pub async fn connect(
        control_url: &str,
        machine_key: MachinePrivate,
        hostinfo: ts_types::tailcfg::Hostinfo,
    ) -> Result<Self, ClientError> {
        let url = ControlUrl::parse(control_url)?;
        let control_key = controlhttp::fetch_control_key(&url, CURRENT_CAPABILITY_VERSION).await?;
        Ok(Self {
            url,
            machine_key,
            control_key: control_key.0,
            hostinfo,
        })
    }

    /// The server's Noise static public key, for logging/inspection.
    pub fn control_key(&self) -> ts_types::MachinePublic {
        ts_types::MachinePublic(self.control_key)
    }

    /// Opens a fresh Noise channel and starts an HTTP/2 session over it.
    async fn open_h2(&self) -> Result<H2Session, ClientError> {
        let conn = controlhttp::dial(
            &self.url,
            &self.machine_key,
            &self.control_key,
            CURRENT_CAPABILITY_VERSION,
        )
        .await?;
        let conn = skip_early_payload(conn).await?;

        let (send, connection) = h2::client::handshake(conn).await?;
        // Drive the HTTP/2 connection in the background; it ends when the
        // session is dropped or the server closes.
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(H2Session {
            send,
            _driver: DriverGuard(driver),
        })
    }

    /// Registers the node key with the control server using a preauth key.
    /// Returns the register response (`machine_authorized`, `auth_url`, …).
    pub async fn register(
        &self,
        node_key: ts_types::NodePublic,
        auth_key: &str,
    ) -> Result<RegisterResponse, ClientError> {
        let req = RegisterRequest {
            version: CURRENT_CAPABILITY_VERSION,
            node_key,
            old_node_key: None,
            auth: Some(ts_types::tailcfg::RegisterResponseAuth {
                auth_key: auth_key.to_string(),
            }),
            expiry: ts_types::Rfc3339("0001-01-01T00:00:00Z".into()),
            followup: String::new(),
            hostinfo: self.hostinfo.clone(),
            ephemeral: false,
        };
        let body = serde_json::to_vec(&req)?;

        let mut session = self.open_h2().await?;
        let resp_bytes = session
            .request(self.machine_url("/machine/register"), body)
            .await?;
        let resp: RegisterResponse = serde_json::from_slice(&resp_bytes)?;

        if !resp.error.is_empty() {
            return Err(ClientError::RegisterRejected(resp.error));
        }
        if !resp.auth_url.is_empty() && !resp.machine_authorized {
            return Err(ClientError::NeedsLogin(resp.auth_url));
        }
        Ok(resp)
    }

    /// Reports our disco key and endpoints to the control server via a
    /// non-streaming "lite" map request (`Stream=false`, `OmitPeers=true`) —
    /// the update Tailscale sends when its endpoints change. Headscale
    /// persists the disco key + endpoints from this and propagates them to
    /// peers (it does not persist them from the streaming poll). The response
    /// netmap is discarded.
    pub async fn update_endpoints(
        &self,
        node_key: ts_types::NodePublic,
        disco_key: ts_types::DiscoPublic,
        endpoints: Vec<std::net::SocketAddr>,
    ) -> Result<(), ClientError> {
        let req = MapRequest {
            version: CURRENT_CAPABILITY_VERSION,
            compress: String::new(),
            keep_alive: false,
            node_key,
            disco_key,
            stream: false,
            hostinfo: self.hostinfo.clone(),
            endpoints,
            omit_peers: true,
            read_only: false,
        };
        let body = serde_json::to_vec(&req)?;
        let mut session = self.open_h2().await?;
        // The lite response (a single netmap frame) is not needed.
        let _ = session
            .request(self.machine_url("/machine/map"), body)
            .await?;
        Ok(())
    }

    /// Opens the streaming netmap long-poll. Each yielded [`MapResponse`] is
    /// one frame from the server (including keep-alive heartbeats). The
    /// stream lives until the connection drops or `handler` returns
    /// `ControlFlow::Break`.
    pub async fn poll_netmap<F>(
        &self,
        node_key: ts_types::NodePublic,
        disco_key: ts_types::DiscoPublic,
        endpoints: Vec<std::net::SocketAddr>,
        mut handler: F,
    ) -> Result<(), ClientError>
    where
        F: FnMut(MapResponse) -> std::ops::ControlFlow<()>,
    {
        // Reporting our endpoints (and disco key) is what makes the control
        // server advertise us as reachable for direct paths and propagate our
        // disco key to peers. Without endpoints, Headscale sends a zero disco
        // key to peers and NAT traversal can't start.
        let req = MapRequest {
            version: CURRENT_CAPABILITY_VERSION,
            compress: String::new(),
            keep_alive: true,
            node_key,
            disco_key,
            stream: true,
            hostinfo: self.hostinfo.clone(),
            endpoints,
            omit_peers: false,
            read_only: false,
        };
        let body = serde_json::to_vec(&req)?;

        let mut session = self.open_h2().await?;
        let mut frames = session
            .request_stream(self.machine_url("/machine/map"), body)
            .await?;

        while let Some(frame) = frames.next_frame().await? {
            let resp: MapResponse = serde_json::from_slice(&frame)?;
            if let std::ops::ControlFlow::Break(()) = handler(resp) {
                break;
            }
        }
        Ok(())
    }

    fn machine_url(&self, path: &str) -> http::Uri {
        format!("http://{}{}", self.url.authority, path)
            .parse()
            .expect("machine URL is well-formed")
    }
}

/// Consumes the optional server early payload from the Noise plaintext
/// stream before HTTP/2 begins. Headscale sends none; a real Tailscale
/// control server may. Returns an IO stream positioned at the first HTTP/2
/// byte.
///
/// Generic over the Noise conn's own transport `T` (rather than the
/// concrete `TcpStream`) because `controlhttp::dial`'s upgrade handoff may
/// hand back a `Conn` wrapping `rusty_http::tokio_native::Replay<TcpStream>`
/// instead, when Noise bytes arrived bundled with the upgrade response.
async fn skip_early_payload<T: AsyncRead + AsyncWrite + Unpin>(
    mut conn: Conn<T>,
) -> Result<crate::prefixed::Prefixed<Conn<T>>, ClientError> {
    let mut header = [0u8; EARLY_HEADER_LEN];
    conn.read_exact(&mut header).await?;
    if &header[..5] == EARLY_PAYLOAD_MAGIC {
        let len = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
        if len > 10 << 20 {
            return Err(ClientError::FrameTooLarge(len));
        }
        let mut payload = vec![0u8; len as usize];
        conn.read_exact(&mut payload).await?;
        // We don't use the early payload (node-key challenge) for preauth
        // registration; discard it and start h2 with no prefix.
        Ok(crate::prefixed::Prefixed::new(Vec::new(), conn))
    } else {
        // Not an early payload: these 9 bytes are the start of the HTTP/2
        // stream and must be replayed.
        Ok(crate::prefixed::Prefixed::new(header.to_vec(), conn))
    }
}

/// Keeps the HTTP/2 driver task alive for the session's lifetime and aborts
/// it on drop.
struct DriverGuard(tokio::task::JoinHandle<()>);
impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One HTTP/2 session over a Noise channel.
struct H2Session {
    send: SendRequest<Bytes>,
    _driver: DriverGuard,
}

impl H2Session {
    /// Sends a POST and reads the full response body (for unary requests
    /// like register).
    async fn request(&mut self, uri: http::Uri, body: Vec<u8>) -> Result<Vec<u8>, ClientError> {
        let mut stream = self.request_stream(uri, body).await?;
        let mut out = Vec::new();
        while let Some(chunk) = stream.next_chunk().await? {
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// Sends a POST and returns a reader over the response body, for the
    /// streaming map long-poll.
    async fn request_stream(
        &mut self,
        uri: http::Uri,
        body: Vec<u8>,
    ) -> Result<BodyReader, ClientError> {
        let path = uri.path().to_string();
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(())
            .expect("request builds");

        // SendRequest::ready consumes and returns self when the connection
        // has capacity for a new stream.
        let mut send = self.send.clone().ready().await?;
        let (response, mut stream) = send.send_request(request, false)?;
        stream.send_data(Bytes::from(body), true)?;

        let response = response.await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ClientError::HttpStatus(status.as_u16(), path));
        }
        Ok(BodyReader {
            body: response.into_body(),
            buf: Vec::new(),
        })
    }
}

/// Reads an HTTP/2 response body, either chunk-by-chunk or as
/// length-prefixed netmap frames.
struct BodyReader {
    body: h2::RecvStream,
    buf: Vec<u8>,
}

impl BodyReader {
    /// Next raw data chunk, releasing flow-control capacity as it goes.
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, ClientError> {
        match self.body.data().await {
            Some(chunk) => {
                let chunk = chunk?;
                let _ = self.body.flow_control().release_capacity(chunk.len());
                Ok(Some(chunk))
            }
            None => Ok(None),
        }
    }

    /// Next netmap frame: a 4-byte little-endian length followed by that
    /// many JSON bytes (`Compress:""`). Returns `None` at clean end of
    /// stream.
    async fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ClientError> {
        loop {
            // Do we already have a full frame buffered?
            if self.buf.len() >= 4 {
                let len = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
                if len > 16 << 20 {
                    return Err(ClientError::FrameTooLarge(len));
                }
                let total = 4 + len as usize;
                if self.buf.len() >= total {
                    let frame = self.buf[4..total].to_vec();
                    self.buf.drain(..total);
                    return Ok(Some(frame));
                }
            }
            // Need more bytes.
            match self.next_chunk().await? {
                Some(chunk) => self.buf.extend_from_slice(&chunk),
                None => {
                    if self.buf.is_empty() {
                        return Ok(None);
                    }
                    return Err(ClientError::StreamEnded);
                }
            }
        }
    }
}
