//! Replication to a standby (MESH.md stage 4). The active hub answers
//! `GET /replicate` with its whole state, and a standby keeps the latest
//! copy on disk. Promotion is a restart of the standby's hub on that copy.
//!
//! The state is the node table plus the hub's Noise key: nodes pin that key,
//! so a promoted standby with another one would be refused by every node. A
//! change reaches the standby within a second or so, so a revoke made just
//! before the active dies can be lost; revoke again after promoting if the
//! active went down right after one.
//!
//! The link carries every pre-auth key, so it needs a shared secret and is
//! refused over plain HTTP except on loopback (checked by the launcher).

use std::path::Path;
use std::time::Duration;

use rusty_http::async_tokio::AsyncTransport;
use rusty_http::head::RequestHead;
use rusty_http::{HeaderMap, Method, Version};
use serde::{Deserialize, Serialize};
use ts_dial::Trust;
use ts_key::{MachinePrivate, NodeState};

use crate::file_store::{check_table, save_table};
use crate::{Hub, HubError, Io, MAX_HEAD_LEN, NodeStore, respond_body};

/// Shortest shared secret the hub accepts (`openssl rand -hex 32` is 64).
pub const MIN_SECRET: usize = 16;
/// The active hub answers at least this often, change or not, so a standby
/// notices a hub that restarted with its change counter reset.
const WAIT: Duration = Duration::from_secs(25);
/// The standby gives up on one request after this long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
/// Pause between requests, and the longest backoff after failures.
const PAUSE: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_SNAPSHOT: usize = 64 << 20;
const TABLE_FILE: &str = "nodes.json";

/// What the active hub sends.
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// The active hub's change counter when this was taken.
    pub version: u64,
    /// The hub's Noise key, in `NodeState` form.
    pub machine_key: String,
    /// The node table, as the hub saves it.
    pub table: serde_json::Value,
}

/// Why replication failed.
#[derive(Debug, thiserror::Error)]
pub enum ReplicaError {
    #[error("bad URL {0:?}")]
    BadUrl(String),
    #[error("the replication secret is too short (at least {MIN_SECRET} characters)")]
    WeakSecret,
    #[error("connect: {0}")]
    Dial(#[from] ts_dial::DialError),
    #[error("HTTP: {0}")]
    Http(String),
    #[error("the active hub answered {0}")]
    Status(u16),
    #[error("timed out")]
    Timeout,
    #[error("bad snapshot: {0}")]
    Snapshot(String),
    #[error("saving the copy: {0}")]
    Save(String),
}

impl<S: NodeStore> Hub<S> {
    /// Lets a standby that knows `secret` copy this hub's state.
    pub fn enable_replication(&self, secret: &str) -> Result<(), ReplicaError> {
        if secret.len() < MIN_SECRET {
            return Err(ReplicaError::WeakSecret);
        }
        *self
            .replica_secret
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(secret.to_string());
        Ok(())
    }

    /// `GET /replicate[?after=N]`: waits (up to [`WAIT`]) while the standby
    /// already has version `N`, then sends the state. Answers 404 when
    /// replication is off, so the endpoint does not show itself.
    pub(crate) async fn serve_replicate<T: Io>(
        &self,
        transport: AsyncTransport<T>,
        target: &str,
        authorization: Option<&str>,
    ) -> Result<(), HubError> {
        let secret = self
            .replica_secret
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(secret) = secret else {
            return crate::respond_plain(transport, 404).await;
        };
        let given = authorization
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !rusty_crypto_key::constant_time_eq(given.as_bytes(), secret.as_bytes()) {
            return crate::respond_plain(transport, 401).await;
        }
        let after = target
            .split_once("after=")
            .and_then(|(_, v)| v.split('&').next()?.parse::<u64>().ok());
        let mut rx = self.changes.clone();
        let version = *rx.borrow_and_update();
        if after == Some(version) {
            // Nothing new: wait for a change or the heartbeat.
            rusty_tokio::time::timeout(WAIT, rx.changed()).await.ok();
        }
        let snapshot = self.snapshot_bytes(*rx.borrow())?;
        respond_body(transport, 200, &snapshot).await
    }

    fn snapshot_bytes(&self, version: u64) -> Result<Vec<u8>, HubError> {
        let table = self
            .locked(|s| s.export())
            .map_err(|e| HubError::Bad(e.to_string()))?;
        Ok(serde_json::to_vec(&Snapshot {
            version,
            machine_key: self.key.to_state_string(),
            table: serde_json::from_slice(&table)?,
        })?)
    }
}

/// One request to the active hub. `after` is the version already held.
pub async fn fetch(
    url: &str,
    secret: &str,
    after: Option<u64>,
    trust: &Trust,
) -> Result<Snapshot, ReplicaError> {
    let target = match after {
        Some(v) => format!("{}?after={v}", crate::replica::PATH),
        None => crate::replica::PATH.to_string(),
    };
    let body = get(url, &target, Some(secret), trust).await?;
    serde_json::from_slice(&body).map_err(|e| ReplicaError::Snapshot(e.to_string()))
}

/// Whether the hub at `url` still answers `GET /key`: a standby must not be
/// promoted while it does.
pub async fn answers(url: &str, trust: &Trust) -> bool {
    get(url, "/key", None, trust).await.is_ok()
}

pub(crate) const PATH: &str = "/replicate";

async fn get(
    url: &str,
    target: &str,
    secret: Option<&str>,
    trust: &Trust,
) -> Result<Vec<u8>, ReplicaError> {
    let (tls, dial, authority) =
        ts_dial::split(url).ok_or_else(|| ReplicaError::BadUrl(url.to_string()))?;
    let work = async {
        let tls = tls.then(|| (ts_dial::server_name(&dial), trust));
        let stream = ts_dial::connect(&dial, tls).await?;
        let mut transport = AsyncTransport::new(stream);
        let http = |e: &dyn std::fmt::Display| ReplicaError::Http(e.to_string());
        let mut headers = HeaderMap::new();
        headers.insert("Host", &authority).map_err(|e| http(&e))?;
        headers
            .insert("Connection", "close")
            .map_err(|e| http(&e))?;
        if let Some(secret) = secret {
            headers
                .insert("Authorization", &format!("Bearer {secret}"))
                .map_err(|e| http(&e))?;
        }
        transport
            .write_request_head(&RequestHead {
                method: Method::Get,
                target: target.to_string(),
                version: Version::Http11,
                headers,
            })
            .await
            .map_err(|e| http(&e))?;
        let head = transport
            .read_response_head(MAX_HEAD_LEN)
            .await
            .map_err(|e| http(&e))?;
        if head.status.as_u16() != 200 {
            return Err(ReplicaError::Status(head.status.as_u16()));
        }
        let framing = rusty_http::body::response_framing(&head.headers, &Method::Get, head.status)
            .map_err(|e| http(&e))?;
        let body = transport.read_body(framing).await.map_err(|e| http(&e))?;
        if body.len() > MAX_SNAPSHOT {
            return Err(ReplicaError::Snapshot("too large".into()));
        }
        Ok(body)
    };
    rusty_tokio::time::timeout(REQUEST_TIMEOUT, work)
        .await
        .map_err(|_| ReplicaError::Timeout)?
}

/// Stores `snapshot` in `state_dir`: the Noise key in the hub identity file
/// and the table in `nodes.json`, so `ts-daemon hub --state-dir` opens the
/// same hub. Both are checked before anything is written.
pub fn apply(state_dir: &Path, snapshot: &Snapshot) -> Result<(), ReplicaError> {
    let machine: MachinePrivate = snapshot
        .machine_key
        .parse()
        .map_err(|e| ReplicaError::Snapshot(format!("machine_key: {e}")))?;
    let table = serde_json::to_vec_pretty(&snapshot.table)
        .map_err(|e| ReplicaError::Snapshot(e.to_string()))?;
    check_table(&table).map_err(|e| ReplicaError::Snapshot(format!("table: {e}")))?;
    let save = |e: &dyn std::fmt::Display| ReplicaError::Save(e.to_string());
    std::fs::create_dir_all(state_dir).map_err(|e| save(&e))?;
    let mut state = NodeState::load_or_generate(state_dir).map_err(|e| save(&e))?;
    state.machine = machine;
    state.save(state_dir).map_err(|e| save(&e))?;
    save_table(&state_dir.join(TABLE_FILE), &table).map_err(|e| save(&e))
}

/// Follows the active hub forever, keeping `state_dir` current. Returns only
/// for a reason retrying cannot fix. Logs counts, never keys.
pub async fn follow(
    url: &str,
    secret: &str,
    state_dir: &Path,
    trust: &Trust,
) -> Result<std::convert::Infallible, ReplicaError> {
    let mut after = None;
    let mut backoff = PAUSE;
    loop {
        match fetch(url, secret, after, trust).await {
            Ok(snapshot) => {
                if after != Some(snapshot.version) {
                    apply(state_dir, &snapshot)?;
                    rusty_log::info!("standby: copied version {}", snapshot.version);
                }
                after = Some(snapshot.version);
                backoff = PAUSE;
            }
            // A wrong secret will not get better by asking again.
            Err(ReplicaError::Status(401 | 404)) => {
                return Err(ReplicaError::Status(401));
            }
            Err(e) => {
                rusty_log::warn!("standby: {e}; retrying in {}s", backoff.as_secs());
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
        rusty_tokio::time::sleep(backoff).await;
    }
}
