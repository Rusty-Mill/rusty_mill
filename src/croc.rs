//! Port of `src/croc` — the client file-transfer engine (phase 2).
//!
//! Wire-compatible with croc v10 peers. The full flow over a relay room:
//!
//! 1. Both sides join the room `hex(sha256(secret[:4] + "croc"))` on the
//!    relay's main port; the recipient sends the raw frame `handshake`
//!    (the optional pake1/ips? local-discovery dance is answered by our
//!    sender for stock recipients, but our recipient does not initiate it).
//! 2. Peer PAKE: recipient (role 0) sends a `pake` message with its curve
//!    choice; sender (role 1) replies with its PAKE bytes and a fresh 8-byte
//!    salt. Both derive `Key = PBKDF2(pake session key, salt)`.
//! 3. Both connect to every advertised transfer port with room `{room}-{j}`.
//!    Recipient sends `externalip`; sender echoes; channel is "secured".
//! 4. Sender sends `fileinfo` (SenderInfo JSON). Recipient answers each file
//!    with `recipientready` (missing-chunk ranges for resume), data flows on
//!    the transfer connections as `encrypt(maybe_compress(u64_le_pos ‖ data))`
//!    in 32 KiB chunks striped round-robin across connections, then
//!    `close-sender`/`close-recipient` advance to the next file and a final
//!    `finished`/`finished` closes the transfer.
//!
//! Not yet ported from Go (see MIGRATION.md): local-network discovery +
//! local relay, reconnect-and-resume on dropped relays, zip-folder mode,
//! throttling, imohash/highway hashing, QR codes, clipboard.

use crate::comm::Comm;
use crate::crypt;
use crate::message::{self, Message};
use crate::models;
use crate::pake::Pake;
use crate::tcp;
use crate::utils;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::Shutdown;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

const CHUNK_SIZE: usize = models::TCP_BUFFER_SIZE / 2; // 32 KiB, matches Go
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

/// Mirrors the parts of Go's `croc.Options` that are ported.
#[derive(Debug, Clone)]
pub struct Options {
    pub is_sender: bool,
    pub shared_secret: String,
    pub relay_address: String,
    pub relay_password: String,
    pub curve: String,
    pub hash_algorithm: String,
    pub no_prompt: bool,
    pub overwrite: bool,
    pub no_compress: bool,
    pub no_multiplexing: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            is_sender: false,
            shared_secret: String::new(),
            relay_address: format!("{}:{}", models::DEFAULT_RELAY, models::DEFAULT_PORT),
            relay_password: models::DEFAULT_PASSPHRASE.to_string(),
            curve: "p256".to_string(),
            hash_algorithm: "xxhash".to_string(),
            no_prompt: false,
            overwrite: false,
            no_compress: false,
            no_multiplexing: false,
        }
    }
}

fn b64ser<S: Serializer>(b: &Vec<u8>, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&BASE64.encode(b))
}

fn b64de<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<u8>, D::Error> {
    let o: Option<String> = Option::deserialize(d)?;
    match o {
        Some(s) => BASE64.decode(s).map_err(serde::de::Error::custom),
        None => Ok(Vec::new()),
    }
}

/// Mirrors Go's `croc.FileInfo` JSON (`n`,`fr`,`fs`,`h`,`s`,`m`,`md`,…).
/// `ModTime` is carried as its RFC3339 string; this port sends the zero time
/// (Go peers then simply skip their optional chtimes on skipped files).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileInfo {
    #[serde(rename = "n", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "fr", default, skip_serializing_if = "String::is_empty")]
    pub folder_remote: String,
    #[serde(rename = "fs", default, skip_serializing_if = "String::is_empty")]
    pub folder_source: String,
    #[serde(
        rename = "h",
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "b64ser",
        deserialize_with = "b64de"
    )]
    pub hash: Vec<u8>,
    #[serde(rename = "s", default, skip_serializing_if = "is_zero_i64")]
    pub size: i64,
    #[serde(rename = "m", default = "zero_time")]
    pub mod_time: String,
    #[serde(rename = "c", default, skip_serializing_if = "is_false")]
    pub is_compressed: bool,
    #[serde(rename = "e", default, skip_serializing_if = "is_false")]
    pub is_encrypted: bool,
    #[serde(rename = "sy", default, skip_serializing_if = "String::is_empty")]
    pub symlink: String,
    #[serde(rename = "md", default, skip_serializing_if = "is_zero_u32")]
    pub mode: u32,
    #[serde(rename = "tf", default, skip_serializing_if = "is_false")]
    pub temp_file: bool,
    #[serde(rename = "ig", default, skip_serializing_if = "is_false")]
    pub is_ignored: bool,
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}
fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}
fn is_false(b: &bool) -> bool {
    !*b
}
fn zero_time() -> String {
    ZERO_TIME.to_string()
}

/// Mirrors Go's `croc.SenderInfo` (field names are Go's, no tags upstream).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SenderInfo {
    #[serde(rename = "FilesToTransfer")]
    pub files_to_transfer: Option<Vec<FileInfo>>,
    #[serde(rename = "EmptyFoldersToTransfer")]
    pub empty_folders_to_transfer: Option<Vec<FileInfo>>,
    #[serde(rename = "TotalNumberFolders")]
    pub total_number_folders: i64,
    #[serde(rename = "MachineID")]
    pub machine_id: String,
    #[serde(rename = "Ask")]
    pub ask: bool,
    #[serde(rename = "SendingText")]
    pub sending_text: bool,
    #[serde(rename = "NoCompress")]
    pub no_compress: bool,
    #[serde(rename = "HashAlgorithm")]
    pub hash_algorithm: String,
    #[serde(rename = "ReconnectVersion")]
    pub reconnect_version: i64,
    #[serde(rename = "NextReconnectRoom")]
    pub next_reconnect_room: String,
}

/// Mirrors Go's `croc.RemoteFileRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RemoteFileRequest {
    #[serde(rename = "CurrentFileChunkRanges")]
    pub current_file_chunk_ranges: Option<Vec<i64>>,
    #[serde(rename = "FilesToTransferCurrentNum")]
    pub files_to_transfer_current_num: usize,
    #[serde(rename = "MachineID")]
    pub machine_id: String,
    #[serde(rename = "ReconnectVersion")]
    pub reconnect_version: i64,
}

/// Mirrors Go's `croc.SimpleMessage` (pre-transfer handshake envelope).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SimpleMessage {
    #[serde(
        rename = "Bytes",
        serialize_with = "b64ser",
        deserialize_with = "b64de"
    )]
    pub bytes: Vec<u8>,
    #[serde(rename = "Kind")]
    pub kind: String,
}

/// Room name on the relay: `hex(sha256(secret[:4] + "croc"))`.
pub fn room_name(shared_secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(&shared_secret.as_bytes()[..4]);
    h.update(b"croc");
    hex_encode(&h.finalize())
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The PAKE password is everything after the pin + dash (Go: `secret[5:]`).
fn pake_secret(shared_secret: &str) -> &[u8] {
    &shared_secret.as_bytes()[5..]
}

fn machine_id() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "rusty-croc".to_string())
}

// ---------------------------------------------------------------------------
// Path collection (sender side) — mirrors croc.GetFilesInfo (non-zip path).
// ---------------------------------------------------------------------------

fn file_info_from(path: &Path, folder_remote: String) -> Result<FileInfo> {
    let meta = std::fs::symlink_metadata(path)?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let abs = std::fs::canonicalize(parent)?;
    let mut fi = FileInfo {
        name: path
            .file_name()
            .ok_or("bad file name")?
            .to_string_lossy()
            .to_string(),
        folder_remote,
        folder_source: abs.to_string_lossy().to_string(),
        size: meta.len() as i64,
        mod_time: ZERO_TIME.to_string(),
        mode: mode_perm(&meta),
        ..Default::default()
    };
    if meta.file_type().is_symlink() {
        fi.symlink = std::fs::read_link(path)?.to_string_lossy().to_string();
    }
    Ok(fi)
}

#[cfg(unix)]
fn mode_perm(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_perm(_meta: &std::fs::Metadata) -> u32 {
    0o644
}

fn walk_dir(
    base: &Path,
    dir: &Path,
    files: &mut Vec<FileInfo>,
    empty_folders: &mut Vec<FileInfo>,
    total_folders: &mut i64,
) -> Result<()> {
    *total_folders += 1;
    let base_parent = base.parent().unwrap_or(Path::new(""));
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() && dir != base {
        let rel = dir.strip_prefix(base_parent).unwrap_or(dir);
        empty_folders.push(FileInfo {
            folder_remote: format!("{}/", rel.to_string_lossy().replace('\\', "/")),
            mod_time: ZERO_TIME.to_string(),
            ..Default::default()
        });
        return Ok(());
    }
    for entry in entries {
        let p = entry.path();
        let meta = std::fs::symlink_metadata(&p)?;
        if meta.is_dir() {
            walk_dir(base, &p, files, empty_folders, total_folders)?;
        } else {
            let rel_dir = p
                .parent()
                .and_then(|d| d.strip_prefix(base_parent).ok())
                .unwrap_or(Path::new(""));
            let folder_remote = format!("{}/", rel_dir.to_string_lossy().replace('\\', "/"));
            files.push(file_info_from(&p, folder_remote)?);
        }
    }
    Ok(())
}

/// Collect files/folders to send. Mirrors `croc.GetFilesInfo` without the
/// zip/gitignore options.
pub fn get_files_info(paths: &[String]) -> Result<(Vec<FileInfo>, Vec<FileInfo>, i64)> {
    let mut files = Vec::new();
    let mut empty_folders = Vec::new();
    let mut total_folders = 0i64;
    for p in paths {
        let path = PathBuf::from(p);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot access '{p}': {e}"))?;
        if meta.is_dir() {
            let abs = std::fs::canonicalize(&path)?;
            walk_dir(&abs, &abs, &mut files, &mut empty_folders, &mut total_folders)?;
        } else {
            files.push(file_info_from(&path, "./".to_string())?);
        }
    }
    Ok((files, empty_folders, total_folders))
}

// ---------------------------------------------------------------------------
// Receive-side path validation — mirrors croc's normalizeReceiveFilePath.
// ---------------------------------------------------------------------------

fn is_local_receive_path(p: &str) -> bool {
    let p = p.replace('\\', "/");
    if p.starts_with('/') || p.contains(':') {
        return false;
    }
    !p.split('/').any(|part| part == "..")
}

fn normalize_receive_folder(folder: &str) -> Result<String> {
    let mut clean = folder.replace('\\', "/");
    while clean.starts_with("./") {
        clean = clean[2..].to_string();
    }
    clean = clean.trim_end_matches('/').to_string();
    if clean.is_empty() {
        clean = ".".to_string();
    }
    if !is_local_receive_path(&clean) {
        return Err(format!("filename must be a local path: '{folder}'").into());
    }
    if clean.contains(".ssh") {
        return Err(format!("invalid path detected: '{folder}'").into());
    }
    Ok(clean)
}

fn normalize_receive_file_path(folder: &str, name: &str) -> Result<(String, PathBuf)> {
    let clean_folder = normalize_receive_folder(folder)?;
    let clean_name = name.replace('\\', "/");
    if clean_name.is_empty()
        || clean_name.contains('/')
        || clean_name == "."
        || clean_name == ".."
    {
        return Err(format!("filename must be a local path: '{name}'").into());
    }
    let dest = Path::new(&clean_folder).join(&clean_name);
    Ok((clean_folder, dest))
}

// ---------------------------------------------------------------------------
// Shared receive state for the data-connection reader threads.
// ---------------------------------------------------------------------------

struct RecvState {
    file: Option<File>,
    path: PathBuf,
    size: i64,
    closed: bool,
    total_sent: i64,
    chunks_expected: usize,
    chunks_transferred: usize,
    no_compress: bool,
    sending_text: bool,
    last_pct: i64,
}

impl RecvState {
    fn new() -> Self {
        RecvState {
            file: None,
            path: PathBuf::new(),
            size: 0,
            closed: true,
            total_sent: 0,
            chunks_expected: 0,
            chunks_transferred: 0,
            no_compress: false,
            sending_text: false,
            last_pct: -1,
        }
    }
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

pub struct Client {
    opts: Options,
    key: Option<Vec<u8>>,
    pake: Option<Pake>,
    control: Comm,
    control_tx: Arc<Mutex<Comm>>,
    relay_host: String,
    relay_ports: Vec<String>,
    room: String,
    external_ip: String,
    external_ip_connected: String,

    step1_channel_secured: bool,
    step2_file_info_transferred: bool,
    step3_recipient_request_file: bool,
    step4_file_transferring: bool,
    success: bool,

    files: Vec<FileInfo>,
    empty_folders: Vec<FileInfo>,
    total_folders: i64,
    files_finished: HashSet<usize>,
    current_num: usize,
    transferred_files: usize,

    // sender side
    data_conns: Vec<Arc<Mutex<Comm>>>,
    sender_threads: Vec<JoinHandle<Result<()>>>,
    chunk_map: HashSet<u64>,

    // recipient side
    recv: Arc<Mutex<RecvState>>,
    data_streams: Vec<std::net::TcpStream>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Client {
    fn connect_relay(opts: &Options) -> Result<(Comm, String, String, String)> {
        let address = &opts.relay_address;
        let (host, port) = match address.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), p.to_string()),
            _ => (address.to_string(), models::DEFAULT_PORT.to_string()),
        };
        let full = format!("{host}:{port}");
        let room = room_name(&opts.shared_secret);
        let (comm, banner, ipaddr) = tcp::connect_to_tcp_server(
            &full,
            &opts.relay_password,
            &room,
            Some(Duration::from_secs(5)),
        )
        .map_err(|e| -> Error { format!("could not connect to {full}: {e}").into() })?;
        Ok((comm, banner, ipaddr, host))
    }

    fn new(opts: Options, control: Comm, banner: &str, ipaddr: String, host: String) -> Result<Self> {
        let mut relay_ports: Vec<String> =
            banner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if relay_ports.is_empty() {
            return Err(format!("relay banner has no transfer ports: '{banner}'").into());
        }
        if opts.no_multiplexing {
            relay_ports.truncate(1);
        }
        let control_tx = Arc::new(Mutex::new(control.try_clone()?));
        let room = room_name(&opts.shared_secret);
        Ok(Client {
            opts,
            key: None,
            pake: None,
            control,
            control_tx,
            relay_host: host,
            relay_ports,
            room,
            external_ip: ipaddr,
            external_ip_connected: String::new(),
            step1_channel_secured: false,
            step2_file_info_transferred: false,
            step3_recipient_request_file: false,
            step4_file_transferring: false,
            success: false,
            files: Vec::new(),
            empty_folders: Vec::new(),
            total_folders: 0,
            files_finished: HashSet::new(),
            current_num: 0,
            transferred_files: 0,
            data_conns: Vec::new(),
            sender_threads: Vec::new(),
            chunk_map: HashSet::new(),
            recv: Arc::new(Mutex::new(RecvState::new())),
            data_streams: Vec::new(),
        })
    }

    fn send_msg(&self, m: &Message) -> Result<()> {
        let payload = message::encode(self.key.as_deref(), m)?;
        self.control_tx
            .lock()
            .unwrap()
            .send(&payload)
            .map_err(|e| -> Error { e.into() })
    }

    fn send_error(&self, text: &str) {
        let _ = self.send_msg(&Message {
            typ: message::TYPE_ERROR.to_string(),
            message: text.to_string(),
            ..Default::default()
        });
    }

    // -----------------------------------------------------------------------
    // Sender entry point — mirrors croc.Client.Send (relay route only).
    // -----------------------------------------------------------------------
    pub fn send(opts: Options, paths: &[String]) -> Result<()> {
        let (files, empty_folders, total_folders) = get_files_info(paths)?;
        let mut total_size = 0i64;
        let mut files = files;
        for fi in files.iter_mut() {
            let full = Path::new(&fi.folder_source).join(&fi.name);
            if fi.symlink.is_empty() {
                fi.hash = utils::hash_file(&full, &opts.hash_algorithm)
                    .map_err(|e| -> Error { format!("hashing {}: {e}", full.display()).into() })?;
            } else {
                fi.hash = Sha256::digest(fi.symlink.as_bytes())[..].to_vec();
            }
            total_size += fi.size;
        }
        let fname = if files.len() == 1 {
            format!("'{}'", files[0].name)
        } else {
            format!("{} files", files.len())
        };
        eprintln!(
            "Sending {} ({})",
            fname,
            utils::byte_count_decimal(total_size)
        );
        eprintln!("Code is: {}", opts.shared_secret);
        eprintln!("\nOn the other computer run:\n\nrusty-croc {}\n(or with stock croc: CROC_SECRET={:?} croc)", opts.shared_secret, opts.shared_secret);

        let (control, banner, ipaddr, host) = Self::connect_relay(&opts)?;
        let mut c = Self::new(opts, control, &banner, ipaddr, host)?;
        c.files = files;
        c.empty_folders = empty_folders;
        c.total_folders = total_folders;

        c.sender_wait_for_handshake()?;
        let result = c.transfer_loop();
        c.shutdown();
        if result.is_err() {
            if let Err(ref e) = result {
                c.send_error(&e.to_string());
            }
        }
        result
    }

    /// Mirrors senderWaitForHandshake: answer optional `pake1`/`ips?` probes
    /// from stock recipients doing local discovery, until `handshake` arrives.
    fn sender_wait_for_handshake(&mut self) -> Result<()> {
        let mut k_b: Option<Vec<u8>> = None;
        loop {
            let raw = self.control.receive()?;
            // If a session key exists, frames may be encrypted. Short frames
            // (`handshake`, `[1]`) fall through crypt's minimum-length check
            // unchanged — the same trick Go relies on.
            let data = match &k_b {
                Some(k) => match crypt::decrypt(&raw, k) {
                    Ok(d) => d,
                    Err(crypt::CryptError::TooShort) => raw.clone(),
                    Err(_) => return Err("handshake decryption failed (wrong code?)".into()),
                },
                None => raw.clone(),
            };
            if data == b"handshake" {
                return Ok(());
            }
            if data == [1] {
                continue;
            }
            if data == b"ips?" {
                // No local-relay support yet: reply "null" (no candidates).
                let enc = match &k_b {
                    Some(k) => crypt::encrypt(b"null", k)?,
                    None => return Err("ips? before pake".into()),
                };
                self.control.send(&enc)?;
                continue;
            }
            if let Ok(sm) = serde_json::from_slice::<SimpleMessage>(&data) {
                if sm.kind == "pake1" {
                    let mut b = Pake::init_curve(
                        pake_secret(&self.opts.shared_secret),
                        1,
                        &self.opts.curve,
                    )?;
                    b.update(&sm.bytes)?;
                    k_b = Some(b.session_key()?);
                    let reply = SimpleMessage {
                        bytes: b.bytes(),
                        kind: "pake2".to_string(),
                    };
                    self.control.send(&serde_json::to_vec(&reply)?)?;
                    continue;
                }
            }
            return Err("gracefully refusing using the public relay".into());
        }
    }

    // -----------------------------------------------------------------------
    // Recipient entry point — mirrors croc.Client.Receive (relay route only).
    // -----------------------------------------------------------------------
    pub fn receive(opts: Options) -> Result<()> {
        eprintln!("connecting...");
        let (control, banner, ipaddr, host) = Self::connect_relay(&opts)?;
        let mut c = Self::new(opts, control, &banner, ipaddr, host)?;
        // No local-discovery: go straight to the handshake.
        c.control.send(b"handshake")?;
        eprintln!("securing channel...");
        // Recipient initiates the peer PAKE (role 0) with its curve choice.
        let pake = Pake::init_curve(pake_secret(&c.opts.shared_secret), 0, &c.opts.curve)?;
        c.send_msg(&Message {
            typ: message::TYPE_PAKE.to_string(),
            bytes: pake.bytes(),
            bytes2: c.opts.curve.as_bytes().to_vec(),
            ..Default::default()
        })?;
        c.pake = Some(pake);
        let result = c.transfer_loop();
        c.shutdown();
        if result.is_err() {
            if let Err(ref e) = result {
                c.send_error(&e.to_string());
            }
        }
        result
    }

    fn shutdown(&mut self) {
        for s in &self.data_streams {
            let _ = s.shutdown(Shutdown::Both);
        }
        while let Some(h) = self.sender_threads.pop() {
            let _ = h.join();
        }
    }

    // -----------------------------------------------------------------------
    // The message loop — mirrors croc.Client.transfer + processMessage.
    // -----------------------------------------------------------------------
    fn transfer_loop(&mut self) -> Result<()> {
        loop {
            let data = self.control.receive().map_err(|e| -> Error {
                if !self.step1_channel_secured {
                    "could not secure channel".into()
                } else {
                    format!("transfer disconnected: {e}").into()
                }
            })?;
            if data == [1] {
                continue; // relay keep-alive
            }
            let m = message::decode(self.key.as_deref(), &data)
                .map_err(|e| -> Error { format!("problem with decoding: {e}").into() })?;
            if m.typ != message::TYPE_PAKE && self.key.is_none() {
                return Err("unencrypted communication rejected".into());
            }
            match m.typ.as_str() {
                message::TYPE_FINISHED => {
                    let _ = self.send_msg(&Message {
                        typ: message::TYPE_FINISHED.to_string(),
                        ..Default::default()
                    });
                    self.success = true;
                    break;
                }
                message::TYPE_PAKE => self.process_pake(&m)?,
                message::TYPE_EXTERNAL_IP => {
                    if self.opts.is_sender {
                        self.send_msg(&Message {
                            typ: message::TYPE_EXTERNAL_IP.to_string(),
                            message: self.external_ip.clone(),
                            ..Default::default()
                        })?;
                    }
                    if self.external_ip_connected.is_empty() {
                        self.external_ip_connected = m.message.clone();
                    }
                    self.step1_channel_secured = true;
                }
                message::TYPE_ERROR => {
                    return Err(format!("peer error: {}", m.message).into());
                }
                message::TYPE_FILEINFO => {
                    if self.process_file_info(&m)? {
                        break;
                    }
                }
                message::TYPE_RECIPIENT_READY => self.process_recipient_ready(&m)?,
                message::TYPE_CLOSE_SENDER => {
                    // Recipient finished the current file; wind down our
                    // data threads and acknowledge.
                    while let Some(h) = self.sender_threads.pop() {
                        let _ = h.join();
                    }
                    self.step4_file_transferring = false;
                    self.step3_recipient_request_file = false;
                    self.send_msg(&Message {
                        typ: message::TYPE_CLOSE_RECIPIENT.to_string(),
                        ..Default::default()
                    })?;
                }
                message::TYPE_CLOSE_RECIPIENT => {
                    self.step4_file_transferring = false;
                    self.step3_recipient_request_file = false;
                }
                other => {
                    log::debug!("ignoring unknown message type: {other}");
                }
            }
            self.update_state()?;
        }
        Ok(())
    }

    /// Mirrors processMessagePake: derive the transfer key, open the data
    /// connections, and (recipient) start the reader threads.
    fn process_pake(&mut self, m: &Message) -> Result<()> {
        let salt;
        if self.opts.is_sender {
            let curve = String::from_utf8_lossy(&m.bytes2).to_string();
            log::debug!("using curve {curve}");
            let mut pake =
                Pake::init_curve(pake_secret(&self.opts.shared_secret), 1, &curve)?;
            pake.update(&m.bytes)
                .map_err(|e| -> Error { format!("pake not successful: {e}").into() })?;
            let mut s = vec![0u8; 8];
            rand::thread_rng().fill_bytes(&mut s);
            salt = s;
            // Reply (unencrypted — the peer has no key yet).
            self.send_msg(&Message {
                typ: message::TYPE_PAKE.to_string(),
                bytes: pake.bytes(),
                bytes2: salt.clone(),
                ..Default::default()
            })?;
            self.pake = Some(pake);
        } else {
            let pake = self.pake.as_mut().ok_or("pake not initialized")?;
            pake.update(&m.bytes)
                .map_err(|e| -> Error { format!("pake not successful: {e}").into() })?;
            salt = m.bytes2.clone();
        }
        let session = self.pake.as_ref().unwrap().session_key()?;
        let (key, _) = crypt::new_key(&session, Some(&salt))?;
        log::debug!("generated transfer key with salt {salt:02x?}");
        self.key = Some(key);

        // Connect to every transfer port with room "{room}-{j}".
        for j in 0..self.relay_ports.len() {
            let server = format!("{}:{}", self.relay_host, self.relay_ports[j]);
            let (conn, _, _) = tcp::connect_to_tcp_server(
                &server,
                &self.opts.relay_password,
                &format!("{}-{}", self.room, j),
                Some(Duration::from_secs(10)),
            )
            .map_err(|e| -> Error {
                format!("could not connect transfer port {server}: {e}").into()
            })?;
            self.data_streams.push(conn.stream().try_clone()?);
            if self.opts.is_sender {
                self.data_conns.push(Arc::new(Mutex::new(conn)));
            } else {
                // Recipient: persistent reader thread per data connection.
                let recv = Arc::clone(&self.recv);
                let key = self.key.clone().unwrap();
                let control_tx = Arc::clone(&self.control_tx);
                std::thread::spawn(move || receive_data_loop(conn, key, recv, control_tx));
            }
        }
        if !self.opts.is_sender {
            self.send_msg(&Message {
                typ: message::TYPE_EXTERNAL_IP.to_string(),
                message: self.external_ip.clone(),
                bytes: m.bytes.clone(),
                ..Default::default()
            })?;
        }
        Ok(())
    }

    /// Recipient: mirrors processMessageFileInfo.
    /// Returns true when the whole transfer is already complete.
    fn process_file_info(&mut self, m: &Message) -> Result<bool> {
        let si: SenderInfo = serde_json::from_slice(&m.bytes)?;
        self.opts.no_compress = si.no_compress;
        self.opts.hash_algorithm = if si.hash_algorithm.is_empty() {
            "xxhash".to_string()
        } else {
            si.hash_algorithm.clone()
        };
        {
            let mut st = self.recv.lock().unwrap();
            st.no_compress = si.no_compress;
            st.sending_text = si.sending_text;
        }
        self.total_folders = si.total_number_folders;
        let files = si.files_to_transfer.unwrap_or_default();
        // Validate every destination before accepting anything.
        for fi in &files {
            normalize_receive_file_path(&fi.folder_remote, &fi.name)?;
        }
        let empty_folders = si.empty_folders_to_transfer.unwrap_or_default();
        for fi in &empty_folders {
            normalize_receive_folder(&fi.folder_remote)?;
        }

        let total_size: i64 = files.iter().map(|f| f.size).sum();
        let fname = if files.len() == 1 {
            format!("'{}'", files[0].name)
        } else {
            format!("{} files", files.len())
        };
        if !self.opts.no_prompt || si.ask {
            eprint!(
                "Accept {} ({})? (Y/n) ",
                fname,
                utils::byte_count_decimal(total_size)
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            let choice = line.trim().to_lowercase();
            if !choice.is_empty() && choice != "y" && choice != "yes" {
                self.send_error("refusing files");
                return Err("refused files".into());
            }
        } else {
            eprintln!(
                "Receiving {} ({})",
                fname,
                utils::byte_count_decimal(total_size)
            );
        }

        for fi in &empty_folders {
            let folder = normalize_receive_folder(&fi.folder_remote)?;
            if !Path::new(&folder).exists() {
                std::fs::create_dir_all(&folder)?;
                eprintln!("{folder}/");
            }
        }

        self.files = files;
        self.empty_folders = empty_folders;
        if self.files.is_empty() {
            self.success = true;
            self.step3_recipient_request_file = true;
            self.step4_file_transferring = true;
            self.send_msg(&Message {
                typ: message::TYPE_FINISHED.to_string(),
                ..Default::default()
            })?;
        }
        self.step2_file_info_transferred = true;
        Ok(false)
    }

    /// Sender: mirrors the TypeRecipientReady branch.
    fn process_recipient_ready(&mut self, m: &Message) -> Result<()> {
        while let Some(h) = self.sender_threads.pop() {
            let _ = h.join();
        }
        let req: RemoteFileRequest = serde_json::from_slice(&m.bytes)?;
        self.current_num = req.files_to_transfer_current_num;
        let ranges = req.current_file_chunk_ranges.unwrap_or_default();
        let chunks = utils::chunk_ranges_to_chunks(&ranges);
        self.chunk_map = chunks.iter().map(|&c| c as u64).collect();
        self.step3_recipient_request_file = true;
        Ok(())
    }

    /// Mirrors updateState: advance whichever step is now unblocked.
    fn update_state(&mut self) -> Result<()> {
        // Sender: channel secured → send the file manifest.
        if self.opts.is_sender
            && self.step1_channel_secured
            && !self.step2_file_info_transferred
        {
            let si = SenderInfo {
                files_to_transfer: if self.files.is_empty() {
                    None
                } else {
                    Some(self.files.clone())
                },
                empty_folders_to_transfer: if self.empty_folders.is_empty() {
                    None
                } else {
                    Some(self.empty_folders.clone())
                },
                total_number_folders: self.total_folders,
                machine_id: machine_id(),
                hash_algorithm: self.opts.hash_algorithm.clone(),
                no_compress: self.opts.no_compress,
                // reconnect_version 0 = "old peer": Go then disables its
                // reconnect logic, which this port doesn't support yet.
                ..Default::default()
            };
            self.send_msg(&Message {
                typ: message::TYPE_FILEINFO.to_string(),
                bytes: serde_json::to_vec(&si)?,
                ..Default::default()
            })?;
            self.step2_file_info_transferred = true;
        }

        // Recipient: manifest received and no active file → pick next file.
        if !self.opts.is_sender
            && self.step2_file_info_transferred
            && !self.step3_recipient_request_file
        {
            self.recipient_next_file()?;
        }

        // Sender: recipient asked for a file → stream it.
        if self.opts.is_sender
            && self.step3_recipient_request_file
            && !self.step4_file_transferring
        {
            self.step4_file_transferring = true;
            self.spawn_send_data()?;
        }
        Ok(())
    }

    /// Recipient: mirrors updateIfRecipientHasFileInfo + recipientGetFileReady.
    fn recipient_next_file(&mut self) -> Result<()> {
        let mut finished = true;
        let mut ranges: Vec<i64> = Vec::new();
        for i in 0..self.files.len() {
            if self.files_finished.contains(&i) || i < self.current_num {
                continue;
            }
            let fi = self.files[i].clone();
            let (folder, dest) = normalize_receive_file_path(&fi.folder_remote, &fi.name)?;

            // Zero-byte files and symlinks are created directly.
            if fi.size == 0 || !fi.symlink.is_empty() {
                if folder != "." {
                    std::fs::create_dir_all(&folder)?;
                }
                if !fi.symlink.is_empty() {
                    make_symlink(&fi.symlink, &dest)?;
                } else {
                    File::create(&dest)?;
                }
                self.files_finished.insert(i);
                self.transferred_files += 1;
                eprintln!("{}", dest.display());
                continue;
            }

            // Same-size local file: hash to decide skip / resume.
            let local_hash = match std::fs::metadata(&dest) {
                Ok(meta) if meta.len() as i64 == fi.size => {
                    utils::hash_file(&dest, &self.opts.hash_algorithm).ok()
                }
                _ => None,
            };
            if let Some(h) = &local_hash {
                if *h == fi.hash {
                    log::debug!("{} already complete", dest.display());
                    self.files_finished.insert(i);
                    continue;
                }
                // Existing file with same size but different content.
                if !self.opts.overwrite {
                    eprint!(
                        "\nOverwrite/resume '{}'? (y/N) (use --overwrite to omit) ",
                        dest.display()
                    );
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line)?;
                    let choice = line.trim().to_lowercase();
                    if choice != "y" && choice != "yes" {
                        eprintln!("Skipping '{}'", dest.display());
                        self.files_finished.insert(i);
                        continue;
                    }
                }
                ranges = utils::missing_chunks(&dest, fi.size, CHUNK_SIZE);
            }

            // This is the next file to receive.
            finished = false;
            self.current_num = i;
            self.transferred_files += 1;
            if folder != "." {
                std::fs::create_dir_all(&folder)?;
            }
            let file = open_receive_file(&dest, &fi)?;
            {
                let mut st = self.recv.lock().unwrap();
                st.file = Some(file);
                st.path = dest.clone();
                st.size = fi.size;
                st.closed = false;
                st.total_sent = 0;
                st.chunks_expected = utils::chunk_ranges_to_chunks(&ranges).len();
                st.chunks_transferred = 0;
                st.last_pct = -1;
            }
            break;
        }

        if finished {
            self.send_msg(&Message {
                typ: message::TYPE_FINISHED.to_string(),
                ..Default::default()
            })?;
            self.success = true;
            self.files_finished.insert(self.current_num);
            return Ok(());
        }

        let req = RemoteFileRequest {
            current_file_chunk_ranges: Some(ranges),
            files_to_transfer_current_num: self.current_num,
            machine_id: machine_id(),
            reconnect_version: 0,
        };
        eprintln!(
            "Receiving {} ({})",
            self.files[self.current_num].name,
            utils::byte_count_decimal(self.files[self.current_num].size)
        );
        self.send_msg(&Message {
            typ: message::TYPE_RECIPIENT_READY.to_string(),
            bytes: serde_json::to_vec(&req)?,
            ..Default::default()
        })?;
        self.step3_recipient_request_file = true;
        Ok(())
    }

    /// Sender: mirrors the sendData goroutines — stripe chunks round-robin
    /// across the data connections, honoring a resume chunk map.
    fn spawn_send_data(&mut self) -> Result<()> {
        let fi = &self.files[self.current_num];
        let path = Path::new(&fi.folder_source).join(&fi.name);
        let size = fi.size;
        let nconns = self.data_conns.len();
        eprintln!(
            "Sending {} ({})",
            fi.name,
            utils::byte_count_decimal(size)
        );

        // Precompute each connection's chunk list (position, length).
        let mut assignments: Vec<Vec<(u64, usize)>> = vec![Vec::new(); nconns];
        let mut pos: u64 = 0;
        let mut idx: usize = 0;
        while (pos as i64) < size {
            let len = std::cmp::min(CHUNK_SIZE as i64, size - pos as i64) as usize;
            let wanted = self.chunk_map.is_empty() || self.chunk_map.contains(&pos);
            if wanted {
                assignments[idx % nconns].push((pos, len));
            }
            pos += len as u64;
            idx += 1;
        }

        let key = self.key.clone().ok_or("no key")?;
        let no_compress = self.opts.no_compress;
        let started = now_secs();
        for (i, chunk_list) in assignments.into_iter().enumerate() {
            let conn = Arc::clone(&self.data_conns[i]);
            let key = key.clone();
            let path = path.clone();
            self.sender_threads.push(std::thread::spawn(move || {
                let mut f = File::open(&path)
                    .map_err(|e| -> Error { format!("open {}: {e}", path.display()).into() })?;
                let mut conn = conn.lock().unwrap();
                let mut buf = vec![0u8; CHUNK_SIZE];
                for (pos, len) in chunk_list {
                    f.seek(SeekFrom::Start(pos))?;
                    f.read_exact(&mut buf[..len])?;
                    let mut payload = Vec::with_capacity(8 + len);
                    payload.extend_from_slice(&pos.to_le_bytes());
                    payload.extend_from_slice(&buf[..len]);
                    let framed = if no_compress {
                        crypt::encrypt(&payload, &key)?
                    } else {
                        crypt::encrypt(&crate::compress::compress(&payload), &key)?
                    };
                    conn.send(&framed)?;
                }
                Ok(())
            }));
        }
        log::debug!(
            "spawned {} sender threads at t={}",
            self.data_conns.len(),
            started
        );
        Ok(())
    }
}

fn make_symlink(target: &str, dest: &Path) -> Result<()> {
    if std::fs::symlink_metadata(dest).is_ok() {
        std::fs::remove_file(dest)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, dest)?;
    #[cfg(not(unix))]
    return Err("symlinks not supported on this platform".into());
    #[cfg(unix)]
    Ok(())
}

fn open_receive_file(dest: &Path, fi: &FileInfo) -> Result<File> {
    // Refuse to write through a symlink, mirroring rejectSymlinkDestination.
    if let Ok(meta) = std::fs::symlink_metadata(dest) {
        if meta.file_type().is_symlink() {
            return Err(format!("refusing to open symlink destination: '{}'", dest.display()).into());
        }
    }
    let file = match std::fs::OpenOptions::new().write(true).open(dest) {
        Ok(f) => {
            let need_truncate = f.metadata().map(|m| m.len() as i64 != fi.size).unwrap_or(true);
            if need_truncate {
                f.set_len(fi.size as u64)?;
            }
            f
        }
        Err(_) => {
            let f = File::create(dest)?;
            f.set_len(fi.size as u64)?;
            set_perm(dest, fi.mode);
            f
        }
    };
    Ok(file)
}

#[cfg(unix)]
fn set_perm(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if mode & 0o777 != 0 {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777));
    }
}

#[cfg(not(unix))]
fn set_perm(_path: &Path, _mode: u32) {}

/// Recipient data-connection reader — mirrors croc.Client.receiveData.
fn receive_data_loop(
    mut conn: Comm,
    key: Vec<u8>,
    recv: Arc<Mutex<RecvState>>,
    control_tx: Arc<Mutex<Comm>>,
) {
    loop {
        let data = match conn.receive() {
            Ok(d) => d,
            Err(_) => return,
        };
        if data == [1] {
            continue;
        }
        let data = match crypt::decrypt(&data, &key) {
            Ok(d) => d,
            Err(e) => {
                log::debug!("data decrypt error: {e}");
                return;
            }
        };
        let mut st = recv.lock().unwrap();
        let data = if st.no_compress {
            data
        } else {
            crate::compress::decompress(&data)
        };
        if data.len() < 8 {
            log::debug!("short data frame");
            return;
        }
        let pos = u64::from_le_bytes(data[..8].try_into().unwrap());
        if st.closed || st.file.is_none() {
            log::debug!("chunk arrived for closed file");
            return;
        }
        if let Err(e) = write_at(st.file.as_mut().unwrap(), &data[8..], pos) {
            log::debug!("write error: {e}");
            return;
        }
        st.total_sent += (data.len() - 8) as i64;
        st.chunks_transferred += 1;
        let pct = if st.size > 0 {
            st.total_sent * 100 / st.size
        } else {
            100
        };
        if pct / 10 > st.last_pct / 10 {
            eprint!("\r{:3}%", pct.min(100));
            st.last_pct = pct;
        }
        let complete = !st.closed
            && (st.chunks_transferred == st.chunks_expected || st.total_sent == st.size);
        if complete {
            st.closed = true;
            let path = st.path.clone();
            let sending_text = st.sending_text;
            st.file = None; // drop → close
            eprintln!("\r100% {}", path.display());
            if sending_text {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                }
                let _ = std::fs::remove_file(&path);
            }
            drop(st);
            let payload =
                match message::encode(Some(&key), &Message {
                    typ: message::TYPE_CLOSE_SENDER.to_string(),
                    ..Default::default()
                }) {
                    Ok(p) => p,
                    Err(_) => return,
                };
            if control_tx.lock().unwrap().send(&payload).is_err() {
                return;
            }
        }
    }
}

#[cfg(unix)]
fn write_at(f: &mut File, data: &[u8], pos: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(data, pos)
}

#[cfg(not(unix))]
fn write_at(f: &mut File, data: &[u8], pos: u64) -> std::io::Result<()> {
    f.seek(SeekFrom::Start(pos))?;
    f.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_name_matches_go_scheme() {
        // hex(sha256("8888" + "croc"))
        let mut h = Sha256::new();
        h.update(b"8888croc");
        let expect: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(room_name("8888-test-interop-run"), expect);
    }

    // Exact JSON produced by Go for the same structs (see MIGRATION.md).
    #[test]
    fn fileinfo_json_matches_go() {
        let fi = FileInfo {
            name: "hello.txt".into(),
            folder_remote: "./".into(),
            folder_source: "/tmp/src".into(),
            hash: vec![1, 2, 3, 4, 5, 6, 7, 8],
            size: 1234,
            mod_time: "2026-07-20T12:34:56.789012345Z".into(),
            mode: 0o644,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&fi).unwrap(),
            r#"{"n":"hello.txt","fr":"./","fs":"/tmp/src","h":"AQIDBAUGBwg=","s":1234,"m":"2026-07-20T12:34:56.789012345Z","md":420}"#
        );
        // And we can parse Go's empty FileInfo.
        let empty: FileInfo = serde_json::from_str(r#"{"m":"0001-01-01T00:00:00Z"}"#).unwrap();
        assert_eq!(empty.size, 0);
        assert!(empty.hash.is_empty());
    }

    #[test]
    fn senderinfo_json_round_trips_with_go() {
        let go = r#"{"FilesToTransfer":[{"n":"hello.txt","fr":"./","fs":"/tmp/src","h":"AQIDBAUGBwg=","s":1234,"m":"2026-07-20T12:34:56.789012345Z","md":420}],"EmptyFoldersToTransfer":null,"TotalNumberFolders":0,"MachineID":"mid","Ask":false,"SendingText":false,"NoCompress":false,"HashAlgorithm":"xxhash","ReconnectVersion":0,"NextReconnectRoom":""}"#;
        let si: SenderInfo = serde_json::from_str(go).unwrap();
        assert_eq!(si.files_to_transfer.as_ref().unwrap().len(), 1);
        assert_eq!(si.hash_algorithm, "xxhash");
        assert!(si.empty_folders_to_transfer.is_none());
        let back = serde_json::to_string(&si).unwrap();
        // must at least parse identically on the Go side field names
        assert!(back.contains("\"FilesToTransfer\""));
        assert!(back.contains("\"HashAlgorithm\":\"xxhash\""));
    }

    #[test]
    fn rfr_json_matches_go() {
        let go = r#"{"CurrentFileChunkRanges":[],"FilesToTransferCurrentNum":0,"MachineID":"mid2","ReconnectVersion":0}"#;
        let r: RemoteFileRequest = serde_json::from_str(go).unwrap();
        assert_eq!(r.current_file_chunk_ranges.as_deref(), Some(&[][..]));
        let null = r#"{"CurrentFileChunkRanges":null,"FilesToTransferCurrentNum":2,"MachineID":"m","ReconnectVersion":1}"#;
        let r: RemoteFileRequest = serde_json::from_str(null).unwrap();
        assert!(r.current_file_chunk_ranges.is_none());
        assert_eq!(r.files_to_transfer_current_num, 2);
    }

    #[test]
    fn simple_message_matches_go() {
        let sm = SimpleMessage {
            bytes: b"xyz".to_vec(),
            kind: "pake1".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&sm).unwrap(),
            r#"{"Bytes":"eHl6","Kind":"pake1"}"#
        );
    }

    #[test]
    fn receive_path_validation() {
        assert!(normalize_receive_file_path("./", "ok.txt").is_ok());
        assert!(normalize_receive_file_path("sub/dir/", "ok.txt").is_ok());
        assert!(normalize_receive_file_path("../evil", "x").is_err());
        assert!(normalize_receive_file_path("/abs", "x").is_err());
        assert!(normalize_receive_file_path("./", "../x").is_err());
        assert!(normalize_receive_file_path(".ssh/", "authorized_keys").is_err());
    }
}
