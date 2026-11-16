//! The hub's DERP relay: the server half of `ts-derp`. A node upgrades
//! `GET /derp`, proves its node key with a NaCl box, and then its
//! `SendPacket` frames reach the destination's `RecvPacket`. Only node keys
//! the hub has registered are admitted, so the relay never carries traffic
//! for a stranger. Each sender is rate limited; a full queue drops.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::task::JoinHandle;
use ts_derp::frame::{self, FrameType, MAGIC, MAX_FRAME_SIZE};
use ts_key::NodePrivate;
use ts_types::NodePublic;

use crate::limits::{Bucket, Limits};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
/// Bound on the ClientInfo JSON we are willing to open.
const MAX_INFO_LEN: u32 = (KEY_LEN + NONCE_LEN + 4096) as u32;
/// Frames queued per connected node before the relay drops for it.
const QUEUE: usize = 256;
/// `PeerGone` reason: the destination is not connected here.
const PEER_NOT_HERE: u8 = 0x01;

type Out = (FrameType, Vec<u8>);

/// A registered session. Dropping the entry drops `_cancel`, which ends the
/// session's reader: a reconnect therefore closes the connection it replaces.
struct Entry {
    id: u64,
    tx: mpsc::Sender<Out>,
    _cancel: oneshot::Sender<()>,
}

/// Holds a session's slot in the relay and its writer task; both go when the
/// session ends, however it ends (error, close, cancelled future, panic).
struct Attached<'a> {
    relay: &'a Relay,
    client: NodePublic,
    id: u64,
    writer: Option<JoinHandle<()>>,
}

impl Drop for Attached<'_> {
    fn drop(&mut self) {
        self.relay.detach(&self.client, self.id);
        if let Some(writer) = &self.writer {
            writer.abort();
        }
    }
}

/// Why a DERP session ended before it started.
#[derive(Debug, thiserror::Error)]
pub enum DerpRefusal {
    #[error("frame: {0}")]
    Frame(#[from] frame::FrameError),
    #[error("bad ClientInfo")]
    BadClientInfo,
    #[error("node key is not registered")]
    Unregistered,
    #[error("too many DERP sessions")]
    Full,
    #[error("handshake timed out")]
    Timeout,
}

/// Connected nodes and the relay's own identity.
pub struct Relay {
    key: NodePrivate,
    /// Node key to its live session.
    clients: Mutex<HashMap<NodePublic, Entry>>,
    next_id: Mutex<u64>,
}

impl Relay {
    pub fn new() -> Self {
        Self {
            key: NodePrivate::generate(),
            clients: Mutex::default(),
            next_id: Mutex::new(0),
        }
    }

    /// The relay's node public key, as sent in `ServerKey`.
    pub fn public_key(&self) -> NodePublic {
        self.key.public()
    }

    /// Runs one session over an upgraded stream: handshake, then relay until
    /// either side closes. `admit` decides whether a node key may connect.
    pub async fn session<R, W>(
        &self,
        mut read: R,
        mut write: W,
        admit: impl FnOnce(&NodePublic) -> bool,
        limits: &Limits,
    ) -> Result<(), DerpRefusal>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let handshake = self.handshake(&mut read, &mut write, admit);
        let (client, salsa) = rusty_tokio::time::timeout(limits.handshake_timeout, handshake)
            .await
            .map_err(|_| DerpRefusal::Timeout)??;

        let (tx, rx) = mpsc::channel::<Out>(QUEUE);
        let (cancel, cancelled) = oneshot::channel::<()>();
        // Claim the slot before answering, so a refused session gets no
        // ServerInfo. `attached` frees it on every exit path.
        let mut attached = self
            .attach(client, tx.clone(), cancel, limits.max_derp_sessions)
            .ok_or(DerpRefusal::Full)?;

        let reply_nonce = SalsaBox::generate_nonce(&mut OsRng);
        let sealed = salsa
            .encrypt(&reply_nonce, &br#"{"version":2}"#[..])
            .map_err(|_| DerpRefusal::BadClientInfo)?;
        let mut server_info = reply_nonce.to_vec();
        server_info.extend_from_slice(&sealed);
        frame::write_frame(&mut write, FrameType::ServerInfo, &server_info).await?;

        attached.writer = Some(rusty_tokio::spawn(write_loop(write, rx)));
        rusty_tokio::select! {
            _ = self.read_loop(&mut read, client, &tx, limits) => {}
            _ = cancelled => rusty_log::debug!("derp: session replaced by a reconnect"),
        }
        Ok(())
    }

    /// `ServerKey`, then the client's boxed `ClientInfo`; returns who the
    /// client is and the box that talks to it.
    async fn handshake<R, W>(
        &self,
        read: &mut R,
        write: &mut W,
        admit: impl FnOnce(&NodePublic) -> bool,
    ) -> Result<(NodePublic, SalsaBox), DerpRefusal>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let mut greeting = MAGIC.to_vec();
        greeting.extend_from_slice(&self.public_key().0);
        frame::write_frame(write, FrameType::ServerKey, &greeting).await?;

        let (ft, info) = frame::read_frame(read, MAX_INFO_LEN).await?;
        if ft != FrameType::ClientInfo || info.len() < KEY_LEN + NONCE_LEN {
            return Err(DerpRefusal::BadClientInfo);
        }
        let mut client_bytes = [0u8; KEY_LEN];
        client_bytes.copy_from_slice(&info[..KEY_LEN]);
        let client = NodePublic(client_bytes);
        let salsa = SalsaBox::new(
            &PublicKey::from(client_bytes),
            &SecretKey::from(self.key.to_bytes()),
        );
        let (nonce, boxed) = info[KEY_LEN..].split_at(NONCE_LEN);
        salsa
            .decrypt(nonce.into(), boxed)
            .map_err(|_| DerpRefusal::BadClientInfo)?;
        if !admit(&client) {
            return Err(DerpRefusal::Unregistered);
        }
        Ok((client, salsa))
    }

    /// Registers the session; a node that reconnects replaces (and so
    /// closes) its old session. `None` when `max` other nodes are connected.
    fn attach(
        &self,
        client: NodePublic,
        tx: mpsc::Sender<Out>,
        cancel: oneshot::Sender<()>,
        max: usize,
    ) -> Option<Attached<'_>> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if !clients.contains_key(&client) && clients.len() >= max {
            return None;
        }
        let mut next = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
        *next += 1;
        let id = *next;
        clients.insert(
            client,
            Entry {
                id,
                tx,
                _cancel: cancel,
            },
        );
        Some(Attached {
            relay: self,
            client,
            id,
            writer: None,
        })
    }

    /// Removes the session unless a newer one has replaced it.
    fn detach(&self, client: &NodePublic, id: u64) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if clients.get(client).is_some_and(|e| e.id == id) {
            clients.remove(client);
        }
    }

    /// Closes the node's live session, if it has one (a revoked node must
    /// not keep relaying).
    pub fn kick(&self, client: &NodePublic) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.remove(client);
    }

    fn queue_of(&self, key: &NodePublic) -> Option<mpsc::Sender<Out>> {
        let clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.get(key).map(|e| e.tx.clone())
    }

    async fn read_loop<R: AsyncRead + Unpin + Send>(
        &self,
        read: &mut R,
        from: NodePublic,
        own: &mpsc::Sender<Out>,
        limits: &Limits,
    ) {
        let mut bucket = Bucket::new(limits.derp_bytes_per_sec, limits.derp_burst, Instant::now());
        loop {
            let (ft, payload) = match frame::read_frame(read, MAX_FRAME_SIZE).await {
                Ok(f) => f,
                Err(e) => {
                    rusty_log::debug!("derp: reader ended: {e}");
                    return;
                }
            };
            match ft {
                FrameType::SendPacket if payload.len() >= KEY_LEN => {
                    if bucket.allow(payload.len(), Instant::now()) {
                        self.forward(&from, &payload, own);
                    } else {
                        rusty_log::debug!("derp: sender over its rate, packet dropped");
                    }
                }
                FrameType::Ping if payload.len() == 8 => {
                    // A full queue drops the pong like any packet.
                    let _ = own.try_send((FrameType::Pong, payload));
                }
                _ => {}
            }
        }
    }

    /// Relays one `SendPacket` payload (`dst key ++ data`); tells the sender
    /// when the destination is not connected. A full queue drops the packet:
    /// the tunnel above is lossy by design.
    fn forward(&self, from: &NodePublic, payload: &[u8], own: &mpsc::Sender<Out>) {
        let (dst, data) = payload.split_at(KEY_LEN);
        let mut dst_key = [0u8; KEY_LEN];
        dst_key.copy_from_slice(dst);
        let dst_key = NodePublic(dst_key);
        let Some(queue) = self.queue_of(&dst_key) else {
            let mut gone = dst_key.0.to_vec();
            gone.push(PEER_NOT_HERE);
            let _ = own.try_send((FrameType::PeerGone, gone));
            return;
        };
        let mut recv = from.0.to_vec();
        recv.extend_from_slice(data);
        if queue.try_send((FrameType::RecvPacket, recv)).is_err() {
            rusty_log::debug!("derp: destination queue full or closed, packet dropped");
        }
    }
}

async fn write_loop<W: AsyncWrite + Unpin + Send>(mut write: W, mut rx: mpsc::Receiver<Out>) {
    while let Some((ft, payload)) = rx.recv().await {
        if let Err(e) = frame::write_frame(&mut write, ft, &payload).await {
            rusty_log::debug!("derp: writer ended: {e}");
            return;
        }
    }
}
