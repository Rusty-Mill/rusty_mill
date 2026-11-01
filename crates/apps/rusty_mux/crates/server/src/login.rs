//! Login mode (C26, Rex delta D9): one front door for every local user.
//!
//! `rusty-mux-server --login MAP <socket>` runs as root and lets any local
//! user connect. For each connection it takes the peer's uid (the
//! principal; over `rusty-mux --ssh` that is the SSH user), maps it to an
//! account through the [`map`] file, and relays the connection's bytes to
//! that account's own server, starting it first if it is not running. The
//! relay begins with a [`forward`] preface naming the principal, so the
//! account server's clients carry who connected, not just the account.
//!
//! Each account's server runs as the account, from its passwd entry (uid,
//! gid, supplementary groups, `HOME`, `SHELL`), in a session of its own,
//! with its socket at `RUNTIME/ACCOUNT/server.sock` in a directory only the
//! account can open. It outlives the front door: stopping or restarting the
//! front door ends the relays, and clients reconnect to the same sessions.
//! It trusts a preface only from the front door's uid (`--forwarded-by`).
//!
//! The map is checked when read: a regular file owned by root or by the
//! front door's own user, writable by no one else. A bad map stops the
//! front door from starting; a reload (SIGHUP) that finds one keeps the old
//! map. A good reload closes only the relays whose principal now maps to a
//! different account, or to none.
//!
//! Account checks are only these: a rule admits the principal, the account
//! exists, and root comes only through a rule naming its principal. No
//! passwords are ever asked for, and PAM is not consulted yet. A front door
//! that is not root can only serve its own account.

pub(crate) mod forward;
pub(crate) mod map;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use platform_linux::Account;
use rusty_mux_core::Principal;
use rusty_tokio::io::{UnixListener, UnixStream};
use rusty_tokio::sync::Mutex;
use rusty_tokio::sync::mpsc::UnboundedReceiver;
use rusty_tokio::sync::oneshot;
use rusty_tokio::task::AbortHandle;

use map::{AccountMap, Route, admits};

/// How long an account's new server has to start listening.
const READY: Duration = Duration::from_secs(10);
/// How often the front door looks for a new server's socket meanwhile.
const READY_POLL: Duration = Duration::from_millis(20);
/// `PATH` for an account's server and the programs in its blocks.
const PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Where the front door finds its map and puts each account's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// The account map file (see [`map`]).
    pub map: PathBuf,
    /// The directory holding one directory per account.
    pub runtime: PathBuf,
    /// The server binary to start for each account.
    pub server: PathBuf,
}

/// A request to read the map again, answered with whether the new map
/// took (the old one stays when it did not).
pub type Reload = oneshot::Sender<io::Result<()>>;

/// Serves the front door on `listener` until accepting fails, reading the
/// map again for each [`Reload`] on `reloads`. A bad map, or one it cannot
/// read, stops it before it serves anyone.
pub async fn serve_login(
    listener: UnixListener,
    login: Login,
    mut reloads: UnboundedReceiver<Reload>,
) -> io::Result<()> {
    let map = read_map(&login.map)?;
    runtime_dir(&login.runtime)?;
    let front = Rc::new(RefCell::new(Front {
        login,
        map,
        relays: HashMap::new(),
        next: 0,
        starting: HashMap::new(),
    }));
    let reload = async {
        while let Some(answer) = reloads.recv().await {
            // Nobody waiting for the answer (SIGHUP) is fine.
            let _ = answer.send(reload(&front));
        }
        std::future::pending::<()>().await;
    };
    rusty_tokio::select! {
        result = accept(listener, front.clone()) => result,
        () = reload => Ok(()),
    }
}

type Shared = Rc<RefCell<Front>>;

struct Front {
    login: Login,
    map: AccountMap,
    relays: HashMap<u64, Relay>,
    next: u64,
    /// One lock per account, so two first connections start one server.
    starting: HashMap<String, Rc<Mutex<()>>>,
}

/// A connection being relayed, and what let it in.
struct Relay {
    uid: u32,
    user: String,
    account: String,
    account_uid: u32,
    task: AbortHandle,
}

async fn accept(listener: UnixListener, front: Shared) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        if let Err(why) = admit(&front, stream) {
            eprintln!("rusty-mux-server: login: {why}");
        }
    }
}

/// Maps the connection's peer to an account and starts relaying it there,
/// or refuses it (dropping the stream) with the reason.
fn admit(front: &Shared, stream: UnixStream) -> Result<(), String> {
    let uid = stream
        .peer_cred()
        .map_err(|e| format!("reading a client's credentials: {e}"))?
        .uid();
    let user = platform_linux::user_name(uid).unwrap_or_default();
    let refuse = |why: &str| format!("refusing uid {uid} ({user}): {why}");
    let route = front
        .borrow()
        .map
        .route(uid, &user)
        .ok_or_else(|| refuse("no rule"))?;
    let account = platform_linux::account(&route.account)
        .ok_or_else(|| refuse(&format!("no account {}", route.account)))?;
    if !admits(&route, account.uid) {
        return Err(refuse("root only through a rule that names the principal"));
    }
    let me = rusty_libc::process::geteuid();
    if me != 0 && account.uid != me {
        return Err(refuse("the front door is not root"));
    }
    let principal = Principal::unix(uid, Some(user.clone()).filter(|u| !u.is_empty()))
        .map_err(|e| refuse(&e.to_string()))?;
    let id = {
        let mut f = front.borrow_mut();
        f.next = f.next.wrapping_add(1);
        f.next
    };
    let task = rusty_tokio::task::spawn_local(relay(
        front.clone(),
        id,
        stream,
        principal,
        account.clone(),
    ));
    front.borrow_mut().relays.insert(
        id,
        Relay {
            uid,
            user,
            account: account.name,
            account_uid: account.uid,
            task: task.abort_handle(),
        },
    );
    Ok(())
}

/// Relays `stream` to its account's server until either side ends, then
/// forgets it.
async fn relay(
    front: Shared,
    id: u64,
    mut stream: UnixStream,
    principal: Principal,
    account: Account,
) {
    let _forget = Forget(front.clone(), id);
    let mut upstream = match server_for(&front, &account).await {
        Ok(upstream) => upstream,
        Err(err) => {
            eprintln!(
                "rusty-mux-server: login: server for {}: {err}",
                account.name
            );
            return;
        }
    };
    if let Err(err) = forward::write(&mut upstream, &principal).await {
        eprintln!(
            "rusty-mux-server: login: forwarding to {}: {err}",
            account.name
        );
        return;
    }
    // Either side ending, or failing, ends the relay; neither is news.
    let _ = rusty_tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
}

/// Removes relay `.1` when its task ends, aborted or not.
struct Forget(Shared, u64);

impl Drop for Forget {
    fn drop(&mut self) {
        if let Ok(mut front) = self.0.try_borrow_mut() {
            front.relays.remove(&self.1);
        }
    }
}

/// A connection to `account`'s server, started first if none answers.
async fn server_for(front: &Shared, account: &Account) -> io::Result<UnixStream> {
    let (dir, server) = {
        let f = front.borrow();
        (f.login.runtime.join(&account.name), f.login.server.clone())
    };
    let socket = dir.join("server.sock");
    let lock = front
        .borrow_mut()
        .starting
        .entry(account.name.clone())
        .or_default()
        .clone();
    let _one_at_a_time = lock.lock().await;
    match UnixStream::connect(&socket).await {
        Ok(stream) => return served_by(stream, account),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => return Err(e),
    }
    account_dir(&dir, account)?;
    match std::fs::remove_file(&socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let child = start(&server, &socket, account)?;
    served_by(ready(&socket, child).await?, account)
}

/// `stream`, if the account itself serves it. The account owns its
/// directory, so it could put a link to someone else's server where its
/// own belongs; root's connect would follow it.
fn served_by(stream: UnixStream, account: &Account) -> io::Result<UnixStream> {
    let uid = stream.peer_cred()?.uid();
    if uid != account.uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}'s socket is served by uid {uid}", account.name),
        ));
    }
    Ok(stream)
}

/// Creates `dir` if needed, and refuses one that anyone but this user
/// could change: its entries are where the front door looks for servers.
fn runtime_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    let me = rusty_libc::process::geteuid();
    if !meta.is_dir() || meta.uid() != me || meta.mode() & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{}: the runtime directory must be this user's, writable by no one else",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Makes `dir` the account's own: created 0700 and given to the account,
/// or, if it exists, a real directory the account owns that no one else
/// can open.
fn account_dir(dir: &Path, account: &Account) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {
            if rusty_libc::process::geteuid() != account.uid {
                std::os::unix::fs::chown(dir, Some(account.uid), Some(account.gid))?;
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(dir)?;
            if !meta.is_dir() || meta.uid() != account.uid || meta.mode() & 0o077 != 0 {
                return Err(io::Error::other(format!(
                    "{} is not a directory of {}'s own",
                    dir.display(),
                    account.name
                )));
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Starts `server` for `account` on `socket`, in a session of its own, as
/// the account when the front door is root.
fn start(server: &Path, socket: &Path, account: &Account) -> io::Result<std::process::Child> {
    let me = rusty_libc::process::geteuid();
    let groups = if account.uid == me {
        None
    } else {
        let groups = platform_linux::groups(&account.name, account.gid)
            .ok_or_else(|| io::Error::other(format!("groups of {}", account.name)))?;
        Some(groups)
    };
    let (uid, gid) = (account.uid, account.gid);
    let mut command = Command::new(server);
    command
        .arg("--forwarded-by")
        .arg(me.to_string())
        .arg(socket)
        .env_clear()
        .env("HOME", &account.home)
        .env("USER", &account.name)
        .env("LOGNAME", &account.name)
        .env("SHELL", &account.shell)
        .env("PATH", PATH)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    // SAFETY: between fork and exec the closure makes raw system calls
    // only (`rusty_libc` wraps each in a syscall, no allocation, no lock):
    // a new session, then groups, gid and uid, in the order that leaves
    // no privilege behind. `groups` was built before the fork.
    unsafe {
        command.pre_exec(move || {
            rusty_libc::process::setsid().map_err(errno)?;
            if let Some(groups) = &groups {
                rusty_libc::process::setgroups(groups).map_err(errno)?;
                rusty_libc::process::setgid(gid).map_err(errno)?;
                rusty_libc::process::setuid(uid).map_err(errno)?;
            }
            Ok(())
        });
    }
    command.spawn()
}

fn errno(e: rusty_libc::Errno) -> io::Error {
    io::Error::from_raw_os_error(e.0)
}

/// Waits for `child` to listen on `socket`, then leaves it running, reaped
/// by a thread of its own when it ends. Fails if it exits first or takes
/// longer than [`READY`].
async fn ready(socket: &Path, mut child: std::process::Child) -> io::Result<UnixStream> {
    let deadline = Instant::now() + READY;
    loop {
        if let Ok(stream) = UnixStream::connect(socket).await {
            std::thread::spawn(move || child.wait());
            return Ok(stream);
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "the server exited first: {status}"
            )));
        }
        if Instant::now() >= deadline {
            // Not ours to leave half-started.
            let _ = child.kill();
            std::thread::spawn(move || child.wait());
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the server did not start listening",
            ));
        }
        rusty_tokio::time::sleep(READY_POLL).await;
    }
}

/// Reads the map at `path`, refusing a file anyone but root or this user
/// could have written.
fn read_map(path: &Path) -> io::Result<AccountMap> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let me = rusty_libc::process::geteuid();
    let mine = meta.uid() == 0 || meta.uid() == me;
    if !meta.is_file() || !mine || meta.permissions().mode() & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{}: the map must be a regular file of root's or this user's that no one else can write",
                path.display()
            ),
        ));
    }
    let text = io::read_to_string(file)?;
    AccountMap::parse(&text).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// Reads the map again; on success, closes every relay its principal no
/// longer maps to the same account. On failure keeps the old map.
fn reload(front: &Shared) -> io::Result<()> {
    let path = front.borrow().login.map.clone();
    let map = read_map(&path).inspect_err(|err| {
        eprintln!("rusty-mux-server: login: keeping the old map: {err}");
    })?;
    let revoked: Vec<AbortHandle> = {
        let mut f = front.borrow_mut();
        let revoked = f
            .relays
            .values()
            .filter(|relay| {
                !map.route(relay.uid, &relay.user)
                    .is_some_and(|route: Route| {
                        route.account == relay.account && admits(&route, relay.account_uid)
                    })
            })
            .map(|relay| relay.task.clone())
            .collect();
        f.map = map;
        revoked
    };
    // Aborting may drop a relay, which forgets itself: no borrow held here.
    for task in revoked {
        task.abort();
    }
    Ok(())
}
