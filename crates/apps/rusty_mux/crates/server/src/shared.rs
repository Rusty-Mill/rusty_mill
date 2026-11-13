//! Shared mode (C26, D232): one server, run by one account, that also
//! serves the other users named in an allowlist.
//!
//! `rusty-mux-server --shared ALLOW <socket>` binds its socket open to
//! every local user, like the login front door, and the allowlist decides:
//! the server's own user is always served, and so is any peer whose uid the
//! list names. Each connection carries the principal the kernel verified
//! for it (`uid:N` and the user's name), never what the client says. The
//! blocks run as the server's account, so a name on the list is account-level
//! power, as an entry in a login map is.
//!
//! The list has one entry per line, `uid:N` or `user:NAME`; `#` starts a
//! comment and blank lines are skipped. There is no wildcard, so root is
//! served only when an entry names it (`uid:0` or `user:root`). A file
//! with no entries lets in the server's own user alone. A repeated user,
//! an unknown account or any other line refuses the whole file. The file
//! has the same checks as the login map: a regular file of root's or the
//! server user's that no one else can write, at most [`MAX_MAP`] bytes.
//!
//! Names are looked up when the file is read, on a thread of its own, so
//! admitting a peer costs no lookup that could block the server. SIGHUP
//! reads the file again; a bad file keeps the old list, and a good one
//! closes only the connections of principals it no longer names.
//!
//! What a guest may do once in is the rest of the C33 and C77 rules: it
//! sees every session, may disconnect and rename only what C33 allows, and
//! may take an open block's input lease only if it is the block's creator's
//! principal or was granted it (`Mux::share_leases`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use rusty_mux_core::Principal;
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::{Notify, mpsc::UnboundedReceiver};

use crate::login::{Reload, read_checked};
use crate::{Config, Peer};

/// Who a shared server serves besides its own user: uid to user name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Allow {
    users: HashMap<u32, Option<String>>,
}

impl Allow {
    /// The list `text` says, looking each name up with `account`, which
    /// maps a user name to its uid, and `name` for a uid's user name.
    pub(crate) fn parse(
        text: &str,
        account: impl Fn(&str) -> Option<u32>,
        name: impl Fn(u32) -> Option<String>,
    ) -> Result<Self, String> {
        let mut users = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let at = |why: String| format!("line {}: {why}", n + 1);
            let uid = match line.split_once(':') {
                Some(("uid", id)) => id.parse().map_err(|_| at(format!("{id:?} is not a uid")))?,
                Some(("user", user)) => {
                    account(user).ok_or_else(|| at(format!("no account {user}")))?
                }
                _ => {
                    return Err(at(format!(
                        "{line:?} is not `uid:N` or `user:NAME` (there is no wildcard)"
                    )));
                }
            };
            if users.insert(uid, name(uid)).is_some() {
                return Err(at(format!("uid {uid} is listed already")));
            }
        }
        Ok(Self { users })
    }

    /// The principal of the listed `uid`, or `None` if it is not listed.
    pub(crate) fn principal(&self, uid: u32) -> Option<Principal> {
        let user = self.users.get(&uid)?;
        Principal::unix(uid, user.clone()).ok()
    }

    /// Whether `principal` is still listed.
    pub(crate) fn names(&self, principal: &Principal) -> bool {
        self.users
            .keys()
            .any(|uid| format!("uid:{uid}") == principal.id())
    }
}

/// Whether the peer `uid` is served: the server's own user (`owner`), or a
/// listed one as the principal the list gives it.
pub(crate) fn admit(owner: u32, uid: u32, allow: &Allow) -> Peer {
    if uid == owner {
        return Peer::Allowed;
    }
    match allow.principal(uid) {
        Some(principal) => Peer::Guest(principal),
        None => Peer::Refused(format!("uid {uid}")),
    }
}

/// A guest's connection: its principal, and the notice that closes it.
type Connection = (Principal, Weak<Notify>);

/// The connections of guests, so a reload can close the ones it revokes.
#[derive(Clone, Default)]
pub(crate) struct Guests(Rc<RefCell<Vec<Connection>>>);

impl Guests {
    /// Remembers a connection of `principal`; `cancel` closes it.
    pub(crate) fn add(&self, principal: Principal, cancel: &Rc<Notify>) {
        let mut all = self.0.borrow_mut();
        all.retain(|(_, c)| c.strong_count() > 0);
        all.push((principal, Rc::downgrade(cancel)));
    }

    /// Closes every connection whose principal `allow` no longer names.
    pub(crate) fn revoke_unlisted(&self, allow: &Allow) {
        let mut all = self.0.borrow_mut();
        all.retain(|(principal, cancel)| {
            let Some(cancel) = cancel.upgrade() else {
                return false;
            };
            let listed = allow.names(principal);
            if !listed {
                cancel.notify_one();
            }
            listed
        });
    }
}

/// How a shared server is set up.
pub struct Sharing {
    /// The allowlist file.
    pub allow: PathBuf,
}

/// Reads the list at `path` on a thread of its own: a file on a hung file
/// system, or a directory service that is slow, stalls only reloads.
async fn load(path: PathBuf) -> io::Result<Allow> {
    let (tx, rx) = rusty_tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("rusty-mux-allow".into())
        .spawn(move || {
            let _ = tx.send(read(&path));
        })?;
    rx.await
        .map_err(|_| io::Error::other("the allowlist reader stopped"))?
}

fn read(path: &std::path::Path) -> io::Result<Allow> {
    let text = read_checked(path, "allowlist")?;
    Allow::parse(
        &text,
        |user| platform_linux::account(user).map(|a| a.uid),
        platform_linux::user_name,
    )
    .map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// [`crate::serve_until`] for a shared server: serves its own user and the
/// users `sharing.allow` lists, until `stop` resolves. A bad list stops it
/// before it serves anyone. Each [`Reload`] on `reloads` reads the list
/// again; a bad one is refused, keeps the old list, and closes nothing.
pub async fn serve_shared(
    listener: UnixListener,
    config: Config,
    sharing: Sharing,
    mut reloads: UnboundedReceiver<Reload>,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    let allow = Rc::new(RefCell::new(load(sharing.allow.clone()).await?));
    let guests = Guests::default();
    let owner = rusty_libc::process::geteuid();
    let admitting = allow.clone();
    let allowed = move |stream: &rusty_tokio::io::UnixStream| {
        let uid = stream.peer_cred()?.uid();
        Ok(admit(owner, uid, &admitting.borrow()))
    };
    let reload = async {
        while let Some(answer) = reloads.recv().await {
            let result = load(sharing.allow.clone()).await.inspect_err(|err| {
                eprintln!("rusty-mux-server: shared: keeping the old allowlist: {err}");
            });
            if let Ok(fresh) = result.as_ref() {
                // No await between the swap and the closing: nobody is
                // admitted on the old list in between.
                guests.revoke_unlisted(fresh);
                *allow.borrow_mut() = fresh.clone();
            }
            let _ = answer.send(result.map(|_| ()));
        }
        std::future::pending::<()>().await;
    };
    let stop = async {
        rusty_tokio::select! {
            () = stop => {},
            () = reload => {},
        }
    };
    crate::serve_for(
        listener,
        config,
        None,
        allowed,
        None,
        Some(guests.clone()),
        stop,
    )
    .await
}

#[cfg(test)]
mod tests;
