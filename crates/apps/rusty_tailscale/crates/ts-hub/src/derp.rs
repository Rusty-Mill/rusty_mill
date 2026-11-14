//! The hub's DERP relay: the server half of `ts-derp`. A node upgrades
//! `GET /derp`, proves its node key with a NaCl box, and then its
//! `SendPacket` frames reach the destination's `RecvPacket`. Only node keys
//! the hub has registered are admitted, so the relay never carries traffic
//! for a stranger.

use std::collections::HashMap;
use std::sync::Mutex;

use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use rusty_tokio::io::AsyncRead;
use rusty_tokio::net::tcp::OwnedWriteHalf;
use rusty_tokio::sync::mpsc;
use ts_derp::frame::{self, FrameType, MAGIC, MAX_FRAME_SIZE};
use ts_key::NodePrivate;
use ts_types::NodePublic;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
/// Bound on the ClientInfo JSON we are willing to open.
const MAX_INFO_LEN: u32 = (KEY_LEN + NONCE_LEN + 4096) as u32;
/// Frames queued per connected node before the relay drops for it.
const QUEUE: usize = 256;
/// `PeerGone` reason: the destination is not connected here.
const PEER_NOT_HERE: u8 = 0x01;

type Out = (FrameType, Vec<u8>);

/// Why a DERP session ended before it started.
#[derive(Debug, thiserror::Error)]
pub enum DerpRefusal {
    #[error("frame: {0}")]
    Frame(#[from] frame::FrameError),
    #[error("bad ClientInfo")]
    BadClientInfo,
    #[error("node key is not registered")]
    Unregistered,
}

/// Connected nodes and the relay's own identity.
pub struct Relay {
    key: NodePrivate,
    /// Node key to its session id and outbound queue.
    clients: Mutex<HashMap<NodePublic, (u64, mpsc::Sender<Out>)>>,
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
    pub async fn session<R>(
        &self,
        mut read: R,
        mut write: OwnedWriteHalf,
        admit: impl FnOnce(&NodePublic) -> bool,
    ) -> Result<(), DerpRefusal>
    where
        R: AsyncRead + Unpin + Send,
    {
        let mut greeting = MAGIC.to_vec();
        greeting.extend_from_slice(&self.public_key().0);
        frame::write_frame(&mut write, FrameType::ServerKey, &greeting).await?;

        let (ft, info) = frame::read_frame(&mut read, MAX_INFO_LEN).await?;
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

        let reply_nonce = SalsaBox::generate_nonce(&mut OsRng);
        let sealed = salsa
            .encrypt(&reply_nonce, &br#"{"version":2}"#[..])
            .map_err(|_| DerpRefusal::BadClientInfo)?;
        let mut server_info = reply_nonce.to_vec();
        server_info.extend_from_slice(&sealed);
        frame::write_frame(&mut write, FrameType::ServerInfo, &server_info).await?;

        let (tx, rx) = mpsc::channel::<Out>(QUEUE);
        let id = self.attach(client, tx.clone());
        let writer = rusty_tokio::spawn(write_loop(write, rx));
        self.read_loop(&mut read, client, &tx).await;
        self.detach(&client, id);
        writer.abort();
        Ok(())
    }

    fn attach(&self, client: NodePublic, tx: mpsc::Sender<Out>) -> u64 {
        let mut next = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
        *next += 1;
        let id = *next;
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(client, (id, tx));
        id
    }

    /// Removes the session unless a newer one has replaced it.
    fn detach(&self, client: &NodePublic, id: u64) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if clients.get(client).is_some_and(|(cur, _)| *cur == id) {
            clients.remove(client);
        }
    }

    fn queue_of(&self, key: &NodePublic) -> Option<mpsc::Sender<Out>> {
        let clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.get(key).map(|(_, tx)| tx.clone())
    }

    async fn read_loop<R: AsyncRead + Unpin + Send>(
        &self,
        read: &mut R,
        from: NodePublic,
        own: &mpsc::Sender<Out>,
    ) {
        while let Ok((ft, payload)) = frame::read_frame(read, MAX_FRAME_SIZE).await {
            match ft {
                FrameType::SendPacket if payload.len() >= KEY_LEN => {
                    self.forward(&from, &payload, own);
                }
                FrameType::Ping if payload.len() == 8 => {
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
        let _ = queue.try_send((FrameType::RecvPacket, recv));
    }
}

async fn write_loop(mut write: OwnedWriteHalf, mut rx: mpsc::Receiver<Out>) {
    while let Some((ft, payload)) = rx.recv().await {
        if frame::write_frame(&mut write, ft, &payload).await.is_err() {
            return;
        }
    }
}
