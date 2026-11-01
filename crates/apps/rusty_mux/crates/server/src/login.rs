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
//! Its stdin, stdout and stderr are `/dev/null`: no descriptor the front
//! door opened reaches it. Under systemd it outlives the front door only if
//! the unit does not kill its whole cgroup (`KillMode=process`), since a
//! new session does not leave the cgroup; no unit ships yet.
//!
//! The map is checked when read: a regular file owned by root or by the
//! front door's own user, writable by no one else, at most [`MAX_MAP`]
//! bytes. A bad map stops the front door from starting; a reload (SIGHUP)
//! that finds one keeps the old map. A good reload closes only the relays
//! whose principal now maps to a different account, or to none.
//!
//! Nothing that can block waits on the front door's one thread: the map
//! is read on a thread of its own, account lookups (which may ask LDAP)
//! run on the blocking pool, and each new server is reaped by a thread of
//! its own from the moment it starts. A connection is admitted against the
//! map in force once its lookups are done, so a reload during them cannot
//! let it through on the old map.
//!
//! The front door's socket ([`bind`]) is open to every user (mode 0666),
//! given that mode as it is made, in a directory only root or this user
//! can change.
//!
//! Account checks are only these: a rule admits the principal, the account
//! exists, and root comes only through a rule naming its principal. No
//! passwords are ever asked for, and PAM is not consulted yet. A front door
//! that is not root can only serve its own account.

pub(crate) mod forward;
pub(crate) mod map;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
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
/// The largest map read, in bytes.
pub const MAX_MAP: u64 = 1 << 20;

/// Where the front door finds its map and puts each account's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// The account map file (see [`map`]).
    pub map: PathBuf,
    /// The directory holding one directory per account.
    pub runtime: PathBuf,
    /// The server binary to start for each account.
    pub server: PathBuf,
    /// How many connections it holds at once.
    pub limits: Limits,
}

/// How many connections the front door holds at once, counting those still
/// being admitted or waiting for their server: at most `total`, and at
/// most `per_peer` from one uid, so no one user can take every slot.
/// Those over a limit are closed at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub total: usize,
    pub per_peer: usize,
}

impl Default for Limits {
    /// 64 per uid, 1024 in all: two descriptors each, well inside the
    /// usual 4096 a service may open.
    fn default() -> Self {
        Self {
            total: 1024,
            per_peer: 64,
        }
    }
}

/// How long accepting waits after running out of descriptors or memory.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

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
    let map = load_map(login.map.clone()).await?;
    runtime_dir(&login.runtime)?;
    let front = Rc::new(RefCell::new(Front {
        login,
        map,
        relays: HashMap::new(),
        next: 0,
        starting: HashMap::new(),
    }));
    // One reload at a time; accepting goes on meanwhile.
    let reload = async {
        while let Some(answer) = reloads.recv().await {
            // Nobody waiting for the answer (SIGHUP) is fine.
            let _ = answer.send(reload(&front).await);
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

/// A connection being relayed, and, once admitted, what let it in.
struct Relay {
    /// The peer's uid, for [`Limits::per_peer`].
    uid: u32,
    task: AbortHandle,
    /// `None` while its lookups run: [`admit`] checks it against the map
    /// in force when they end, so a reload meanwhile has nothing to close.
    admitted: Option<Admitted>,
}

/// What admitted a relay, for a reload to check against the new map.
struct Admitted {
    uid: u32,
    user: String,
    account: String,
    account_uid: u32,
}

impl Admitted {
    fn still_by(&self, map: &AccountMap) -> bool {
        map.route(self.uid, &self.user).is_some_and(|route: Route| {
            route.account == self.account && admits(&route, self.uid, self.account_uid)
        })
    }
}

/// Accepts until the listener fails for good. Running out of descriptors
/// or memory only pauses it: one user's connections must not end the
/// front door for everyone.
async fn accept(listener: UnixListener, front: Shared) -> io::Result<()> {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                pause_after(e).await?;
                continue;
            }
        };
        let uid = match stream.peer_cred() {
            Ok(cred) => cred.uid(),
            Err(e) => {
                eprintln!("rusty-mux-server: login: reading a client's credentials: {e}");
                continue;
            }
        };
        let id = {
            let mut f = front.borrow_mut();
            if let Err(why) = room_for(&f, uid) {
                eprintln!("rusty-mux-server: login: refusing uid {uid}: {why}");
                continue;
            }
            f.next = f.next.wrapping_add(1);
            f.next
        };
        // The task first runs after the insert: nothing yields in between.
        let task = rusty_tokio::task::spawn_local(relay(front.clone(), id, uid, stream));
        front.borrow_mut().relays.insert(
            id,
            Relay {
                uid,
                task: task.abort_handle(),
                admitted: None,
            },
        );
    }
}

/// Whether the front door may hold one more connection from `uid`.
fn room_for(front: &Front, uid: u32) -> Result<(), &'static str> {
    let limits = front.login.limits;
    if front.relays.len() >= limits.total {
        return Err("the front door is full");
    }
    let from_peer = front.relays.values().filter(|r| r.uid == uid).count();
    if from_peer >= limits.per_peer {
        return Err("too many connections from this uid");
    }
    Ok(())
}

/// Waits a little after an accept error that passes (out of descriptors
/// or memory), retries at once after one a client caused, and returns any
/// other.
async fn pause_after(err: io::Error) -> io::Result<()> {
    use rusty_libc::Errno;
    /// No buffer space (`accept(2)`); rusty_libc does not name it.
    const ENOBUFS: i32 = 105;
    const PASSING: [i32; 4] = [Errno::EMFILE.0, Errno::ENFILE.0, Errno::ENOMEM.0, ENOBUFS];
    let kind = err.kind();
    if matches!(
        kind,
        io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted
    ) {
        return Ok(());
    }
    if !err
        .raw_os_error()
        .is_some_and(|code| PASSING.contains(&code))
    {
        return Err(err);
    }
    eprintln!("rusty-mux-server: login: accepting: {err}; pausing");
    rusty_tokio::time::sleep(ACCEPT_BACKOFF).await;
    Ok(())
}

/// Maps the peer `uid` of relay `id` to an account, or refuses it with the
/// reason. The lookups run off this thread; the decision is made, and
/// recorded on the relay, against the map in force after them, with no
/// await in between.
async fn admit(front: &Shared, id: u64, uid: u32) -> Result<(Account, Principal), String> {
    let user = blocking(move || platform_linux::user_name(uid))
        .await?
        .unwrap_or_default();
    let refuse = |why: &str| format!("refusing uid {uid} ({user}): {why}");
    let name = front
        .borrow()
        .map
        .route(uid, &user)
        .ok_or_else(|| refuse("no rule"))?
        .account;
    let looked_up = name.clone();
    let account = blocking(move || platform_linux::account(&looked_up))
        .await?
        .ok_or_else(|| refuse(&format!("no account {name}")))?;
    let admitted = Admitted {
        uid,
        user: user.clone(),
        account: account.name.clone(),
        account_uid: account.uid,
    };
    let mut f = front.borrow_mut();
    if !admitted.still_by(&f.map) {
        return Err(refuse(
            "no rule now, root only through a rule that names the principal, \
             and `=` only to its own uid",
        ));
    }
    let me = rusty_libc::process::geteuid();
    if me != 0 && account.uid != me {
        return Err(refuse("the front door is not root"));
    }
    let principal = Principal::unix(uid, Some(user.clone()).filter(|u| !u.is_empty()))
        .map_err(|e| refuse(&e.to_string()))?;
    let relay = f.relays.get_mut(&id).ok_or_else(|| refuse("closed"))?;
    relay.admitted = Some(admitted);
    Ok((account, principal))
}

/// Runs `lookup` (one that may block, as NSS may) on the blocking pool.
async fn blocking<T: Send + 'static>(
    lookup: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    rusty_tokio::spawn_blocking(lookup)
        .await
        .map_err(|e| format!("an account lookup failed: {e}"))
}

/// Admits the connection from `uid`, then relays `stream` to its account's
/// server until either side ends; forgets it either way.
async fn relay(front: Shared, id: u64, uid: u32, mut stream: UnixStream) {
    let _forget = Forget(front.clone(), id);
    let (account, principal) = match admit(&front, id, uid).await {
        Ok(admitted) => admitted,
        Err(why) => {
            eprintln!("rusty-mux-server: login: {why}");
            return;
        }
    };
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
    let groups = if account.uid == rusty_libc::process::geteuid() {
        None
    } else {
        let (name, gid) = (account.name.clone(), account.gid);
        let groups = blocking(move || platform_linux::groups(&name, gid))
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other(format!("groups of {}", account.name)))?;
        Some(groups)
    };
    let started = watch(command(&server, &socket, account, groups))?;
    served_by(ready(&socket, &started).await?, account)
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

/// Checks `dir`, then creates it if needed, and refuses one that anyone but
/// root or this user could change or swap: its entries are where the front
/// door looks for servers, and it creates, gives away and connects by path.
/// `dir` must be absolute with no symbolic link in it (each later use would
/// follow the link again), this user's and writable by no one else; each
/// directory above it root's or this user's, writable by others only when
/// sticky (as `/tmp`), where no one else can rename what is in it.
fn runtime_dir(dir: &Path) -> io::Result<()> {
    let me = rusty_libc::process::geteuid();
    // Before anything is created, then again after: a part missing at the
    // first look may have been made by another user in between.
    check_parts(dir, me, true)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)?;
    check_parts(dir, me, false)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if meta.uid() != me {
        return Err(unsafe_dir(dir, "is not this user's".into()));
    }
    Ok(())
}

/// Binds the front door's socket at `socket`, open to every user (0666)
/// as it is made: under a umask that gives it that mode, so no chmod
/// afterwards follows a path someone could have swapped for a link. Its
/// directory must pass the same checks as the runtime directory's parts
/// (absolute, no link, root's or this user's, writable by no one else),
/// so no other user can put anything in its place. A socket left by an
/// earlier front door, which nothing answers, is replaced. Call it before
/// starting any thread: the umask is the process's.
pub fn bind(socket: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    let dir = socket
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| unsafe_dir(socket, "has no directory".into()))?;
    check_parts(dir, rusty_libc::process::geteuid(), false)?;
    match bind_open(socket) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse && stale(socket) => {
            std::fs::remove_file(socket)?;
            bind_open(socket)
        }
        bound => bound,
    }
}

fn bind_open(socket: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    let old = rusty_libc::umask::umask(0o111);
    let bound = std::os::unix::net::UnixListener::bind(socket);
    rusty_libc::umask::umask(old);
    bound
}

/// Whether `socket` is a socket nothing answers on.
fn stale(socket: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let is_socket = std::fs::symlink_metadata(socket).is_ok_and(|m| m.file_type().is_socket());
    is_socket
        && std::os::unix::net::UnixStream::connect(socket)
            .is_err_and(|e| e.kind() == io::ErrorKind::ConnectionRefused)
}

/// Checks each part of `dir` as [`runtime_dir`] requires; a part that does
/// not exist passes only when `allow_missing`.
fn check_parts(dir: &Path, me: u32, allow_missing: bool) -> io::Result<()> {
    if !dir.is_absolute() {
        return Err(unsafe_dir(dir, "is not an absolute path".into()));
    }
    for part in dir.ancestors() {
        let meta = match std::fs::symlink_metadata(part) {
            Ok(meta) => meta,
            Err(e) if allow_missing && e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let shown = part.display();
        if !meta.is_dir() {
            return Err(unsafe_dir(
                dir,
                format!("goes through {shown}, not a directory"),
            ));
        }
        let owner_ok = meta.uid() == 0 || meta.uid() == me;
        let sticky = meta.mode() & 0o1000 != 0;
        if !owner_ok || (meta.mode() & 0o022 != 0 && (part == dir || !sticky)) {
            return Err(unsafe_dir(
                dir,
                format!("goes through {shown}, which another user could change"),
            ));
        }
    }
    Ok(())
}

fn unsafe_dir(dir: &Path, why: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{}: {why}", dir.display()),
    )
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

/// The command that starts `server` for `account` on `socket`, in a
/// session of its own, as the account (with its `groups`) when the front
/// door is root.
fn command(server: &Path, socket: &Path, account: &Account, groups: Option<Vec<u32>>) -> Command {
    let me = rusty_libc::process::geteuid();
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
        // Not the front door's: a descriptor root opened (its journal or
        // terminal) must not reach a process the account can take fds from.
        .stderr(Stdio::null());
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
    command
}

fn errno(e: rusty_libc::Errno) -> io::Error {
    io::Error::from_raw_os_error(e.0)
}

/// A server just started: its exit, from the thread that reaps it, and a
/// handle that signals it and no process that later reuses its pid.
struct Started {
    exited: mpsc::Receiver<io::Result<ExitStatus>>,
    pidfd: OwnedFd,
}

/// Runs `command` with a thread to reap it whenever it ends, so a relay
/// closed while it starts leaves no zombie behind. The thread comes first:
/// if it cannot be made, nothing is started. A child with no pidfd (Linux
/// before 5.3, or a policy that denies it) could not be stopped safely
/// later, so it is killed at once, while unreaped, and the start fails.
fn watch(mut command: Command) -> io::Result<Started> {
    let (give, take) = mpsc::channel::<Child>();
    let (tx, exited) = mpsc::channel();
    std::thread::Builder::new()
        .name("rusty-mux-reap".into())
        .spawn(move || {
            // No child comes if the start failed. Nobody waiting for the
            // exit any more is fine; the reaping is the point.
            if let Ok(mut child) = take.recv() {
                let _ = tx.send(child.wait());
            }
        })?;
    let mut child = command.spawn()?;
    let pidfd = pidfd_of(&child);
    if pidfd.is_err() {
        // Unreaped, so the pid is still this child's.
        let _ = child.kill();
    }
    // The reaper lives until it gets this, so the send cannot fail.
    give.send(child)
        .map_err(|_| io::Error::other("the reaper stopped"))?;
    Ok(Started {
        exited,
        pidfd: pidfd?,
    })
}

fn pidfd_of(child: &Child) -> io::Result<OwnedFd> {
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    let fd = rusty_libc::process::pidfd_open(pid, 0).map_err(errno)?;
    // SAFETY: `pidfd_open` just returned this fd, owned by no one else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Waits for the server in `started` to listen on `socket`. Fails if it
/// exits first or takes longer than [`READY`], when it is killed.
async fn ready(socket: &Path, started: &Started) -> io::Result<UnixStream> {
    let deadline = Instant::now() + READY;
    loop {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Ok(stream);
        }
        match started.exited.try_recv() {
            Ok(status) => {
                return Err(io::Error::other(format!(
                    "the server exited first: {}",
                    status?
                )));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other("the server's reaper stopped"));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            // Not ours to leave half-started. Through the pidfd: never a
            // process that took the pid after this one was reaped.
            let sigkill = rusty_libc::signal::SIGKILL;
            let _ = rusty_libc::process::pidfd_send_signal(started.pidfd.as_raw_fd(), sigkill);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the server did not start listening",
            ));
        }
        rusty_tokio::time::sleep(READY_POLL).await;
    }
}

/// Reads the map at `path` on a thread of its own: a map on a hung file
/// system stalls only reloads, never the relays.
async fn load_map(path: PathBuf) -> io::Result<AccountMap> {
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("rusty-mux-map".into())
        .spawn(move || {
            let _ = tx.send(read_map(&path));
        })?;
    rx.await
        .map_err(|_| io::Error::other("the map reader stopped"))?
}

/// Reads the map at `path`, refusing a file anyone but root or this user
/// could have written, anything but a regular file (opened without
/// blocking, so a FIFO is refused, not waited on), and one over
/// [`MAX_MAP`] bytes.
fn read_map(path: &Path) -> io::Result<AccountMap> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rusty_libc::fd::O_NONBLOCK)
        .open(path)?;
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
    let mut text = String::new();
    file.take(MAX_MAP + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_MAP {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: the map is over {MAX_MAP} bytes", path.display()),
        ));
    }
    AccountMap::parse(&text).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// Reads the map again; on success, closes every relay its principal no
/// longer maps to the same account. On failure keeps the old map.
/// Relays still in their lookups are left to [`admit`], which checks the
/// map in force when they end.
async fn reload(front: &Shared) -> io::Result<()> {
    let path = front.borrow().login.map.clone();
    let map = load_map(path).await.inspect_err(|err| {
        eprintln!("rusty-mux-server: login: keeping the old map: {err}");
    })?;
    // From here to the swap, no await: no relay is admitted in between.
    let revoked: Vec<AbortHandle> = {
        let mut f = front.borrow_mut();
        let revoked = f
            .relays
            .values()
            .filter(|relay| relay.admitted.as_ref().is_some_and(|a| !a.still_by(&map)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-parts-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("scratch");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        dir
    }

    /// The second look, after creating, allows nothing missing: a part
    /// that was missing at the first look is checked like the rest.
    #[test]
    fn a_missing_part_passes_only_the_first_look() {
        let dir = scratch("missing");
        let me = rusty_libc::process::geteuid();
        let run = dir.join("mid").join("run");
        assert!(check_parts(&run, me, true).is_ok());
        assert!(check_parts(&run, me, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A part made in between that others can write fails the second look.
    #[test]
    fn a_part_others_can_write_fails_the_second_look() {
        let dir = scratch("between");
        let me = rusty_libc::process::geteuid();
        let mid = dir.join("mid");
        let run = mid.join("run");
        assert!(check_parts(&run, me, true).is_ok());
        // Someone makes `mid` before the front door does.
        std::fs::create_dir(&mid).expect("mid");
        std::fs::set_permissions(&mid, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        std::fs::DirBuilder::new()
            .mode(0o755)
            .create(&run)
            .expect("run");
        assert!(check_parts(&run, me, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
