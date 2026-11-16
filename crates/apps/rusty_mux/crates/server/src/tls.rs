//! The TLS listener of a shared server (#506, D235-D237).
//!
//! `--tls ADDR:PORT --tls-cert PEM --tls-key PEM --tokens FILE` serves TCP
//! peers over TLS with the certificate the user supplied. A peer proves who
//! it is by sending its token, 32 raw bytes, as the first thing after the
//! handshake; the token file ([`crate::token`]) names it `token:NAME`, and
//! the shared-mode allowlist must name that principal too. Anything else is
//! closed after [`FAIL_DELAY`], and before it says a word of the protocol.
//! A guest like any other: no descriptors (C75), no UDP.
//!
//! The certificate and key files are checked as the allowlist is (a regular
//! file of root's or this user's that no one else can write); the key is also
//! unreadable by others.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use rusty_mux_core::{Principal, Transport};
use rusty_tls::TlsAcceptor;
use rusty_tokio::io::{AsyncReadExt, TcpListener};

use crate::login::read_checked;
use crate::shared::{Allow, Guests};
use crate::{conn, pem, state};
use rusty_mux_script::token::{self, Tokens};

/// Handshakes and token reads in progress at once; more are closed.
const PENDING: usize = 64;
/// How long a peer has for its handshake and its token.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a refused peer is held before it is closed, so guessing is slow.
const FAIL_DELAY: Duration = Duration::from_secs(1);
/// How long the listener waits after a failed accept.
const ACCEPT_RETRY: Duration = Duration::from_millis(250);

/// The TLS side of a shared server.
pub struct Tls {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    pub(crate) tokens_file: PathBuf,
    pub(crate) tokens: Rc<RefCell<Tokens>>,
    pub(crate) allow: Rc<RefCell<Allow>>,
}

impl Tls {
    /// A TLS listener on `listener`, serving the certificate chain in
    /// `cert` and the key in `key`, for the tokens in `tokens_file`. A bad
    /// file stops the server before it serves anyone.
    pub fn new(
        listener: TcpListener,
        cert: &Path,
        key: &Path,
        tokens_file: PathBuf,
    ) -> io::Result<Self> {
        let bad = |why: String| io::Error::new(io::ErrorKind::InvalidData, why);
        let (chain, key) = pem::identity(
            &read_checked(cert, "certificate")?,
            &read_checked(key, "key")?,
        )
        .map_err(bad)?;
        let acceptor =
            TlsAcceptor::new(chain, key).map_err(|e| bad(format!("certificate: {e}")))?;
        let tokens = read_tokens(&tokens_file)?;
        Ok(Self {
            listener,
            acceptor,
            tokens_file,
            tokens: Rc::new(RefCell::new(tokens)),
            allow: Rc::default(),
        })
    }
}

/// The tokens in `path`, checked as the allowlist is.
pub(crate) fn read_tokens(path: &Path) -> io::Result<Tokens> {
    Tokens::parse(&read_checked(path, "token file")?).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// Reads the token the peer sends first; returns its name and the bytes, if
/// they are a token's.
async fn authenticate<S>(
    stream: &mut S,
    tokens: &RefCell<Tokens>,
) -> Option<(String, [u8; token::LEN])>
where
    S: rusty_tokio::io::AsyncRead + Unpin + Send,
{
    let mut presented = [0u8; token::LEN];
    stream.read_exact(&mut presented).await.ok()?;
    let name = tokens.borrow().verify(&presented)?.to_string();
    Some((name, presented))
}

/// One peer: the handshake, the token, then the protocol as a guest.
async fn serve_peer(
    acceptor: TlsAcceptor,
    stream: rusty_tokio::io::TcpStream,
    tokens: Rc<RefCell<Tokens>>,
    allow: Rc<RefCell<Allow>>,
    state: state::Shared,
    guests: Guests,
    pending: Rc<Cell<usize>>,
) {
    // The stream comes back with its verdict, so a refusal can hold it.
    let seen = rusty_tokio::time::timeout(PATIENCE, async {
        let mut tls = acceptor.accept_async(stream).ok()?;
        tls.complete_handshake().await.ok()?;
        let admitted = async {
            let (name, presented) = authenticate(&mut tls, &tokens).await?;
            let principal = Principal::new(format!("token:{name}"), "").ok()?;
            let listed = allow.borrow().names(&principal);
            listed.then_some((principal, presented))
        }
        .await;
        Some((tls, admitted))
    })
    .await
    .ok()
    .flatten();
    let (tls, principal, presented) = match seen {
        Some((tls, Some((principal, presented)))) => (tls, principal, presented),
        refused => {
            // Held, not answered: a refusal costs the guesser a second.
            if refused.is_some() {
                rusty_tokio::time::sleep(FAIL_DELAY).await;
            }
            return pending.set(pending.get() - 1);
        }
    };
    pending.set(pending.get() - 1);
    let (rd, wr) = rusty_tokio::io::split(tls);
    let who = principal.clone();
    conn::run(
        state,
        rd,
        wr,
        principal,
        Transport::Socket,
        move |_, cancel| {
            // The peer may have waited at `Hello` through a reload: it is
            // served only if its token is still the one it presented.
            let name = who.id().trim_start_matches("token:");
            if tokens.borrow().verify(&presented) != Some(name) {
                return cancel.notify_one();
            }
            guests.add(who.clone(), cancel)
        },
    )
    .await;
}

/// Serves TLS peers until the server stops.
pub(crate) async fn accept_tls(tls: Tls, state: state::Shared, guests: Guests) -> io::Result<()> {
    let pending = Rc::new(Cell::new(0usize));
    let mut dropped = 0u64;
    loop {
        let (stream, peer) = match tls.listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                eprintln!("rusty-mux-server: tls: accept: {err}");
                rusty_tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        if pending.get() >= PENDING {
            if dropped.is_multiple_of(100) {
                eprintln!(
                    "rusty-mux-server: tls: too many peers at the door, closing {peer} ({dropped} closed before)"
                );
            }
            dropped += 1;
            continue;
        }
        pending.set(pending.get() + 1);
        rusty_tokio::task::spawn_local(serve_peer(
            tls.acceptor.clone(),
            stream,
            tls.tokens.clone(),
            tls.allow.clone(),
            state.clone(),
            guests.clone(),
            pending.clone(),
        ));
    }
}
