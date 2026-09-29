//! The application behind the socket: authentication, then the caller's data
//! (ADR-0002, step 3). No sockets here, so every path is tested without a
//! network and [`crate::server`] only moves bytes.
//!
//! Two ways to run, chosen by [`Backend::is_multi_user`]:
//!
//! | | Single user | Multi user |
//! |---|---|---|
//! | Started by | `RUSTY_TICK_TOKEN` | `<data-dir>/users.json` exists |
//! | Token | the shared one | `<user key>.<secret>` per user |
//! | Data | `<data-dir>` itself | `<data-dir>/users/<key>/`, at most 32 open |
//!
//! Both take a [`DirLock`] on every directory they open, so two processes
//! cannot serve one store.

use crate::api::{Api, Request, Response};
use crate::pool::{ClockFactory, PoolError, ServicePool};
use crate::service::{Clock, Service, ServiceError};
use crate::users::{RegistryFile, UsersError, DEFAULT_RELOAD_INTERVAL};
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use std::num::NonZeroUsize;
use std::path::Path;

/// The registry that turns the server multi-user, inside the data directory.
pub const USERS_FILE: &str = "users.json";
/// Where multi-user mode keeps each user's directory, inside the data
/// directory.
pub const USERS_DIR: &str = "users";
/// The lock file in each opened directory.
pub const LOCK_FILE: &str = "store.lock";

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("{0}")]
    Token(String),
    #[error(transparent)]
    Lock(#[from] DirLockError),
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error(transparent)]
    Users(#[from] UsersError),
}

enum Data {
    /// The whole data directory, and its lock. Fields drop in order, so the
    /// service closes before the lock is released.
    Single {
        service: Service,
        _lock: DirLock,
    },
    Multi(ServicePool),
}

pub struct Backend {
    api: Api,
    data: Data,
}

impl Backend {
    /// Whether `data_dir` is set up for several users: its `users.json` exists.
    pub fn is_multi_user(data_dir: &Path) -> bool {
        data_dir.join(USERS_FILE).exists()
    }

    /// One shared `token`, one store in `data_dir`.
    ///
    /// # Errors
    ///
    /// [`BackendError::Token`] for a token that is too short;
    /// [`BackendError::Lock`] if another process is serving `data_dir`;
    /// [`BackendError::Service`] if it cannot be opened.
    pub fn single(data_dir: &Path, token: String, clock: Clock) -> Result<Self, BackendError> {
        let api = Api::new(token).map_err(BackendError::Token)?;
        let lock = DirLock::acquire(data_dir, LOCK_FILE)?;
        let service = Service::open(data_dir, clock)?;
        Ok(Self {
            api,
            data: Data::Single {
                service,
                _lock: lock,
            },
        })
    }

    /// Users from `data_dir/users.json`, each with a store under
    /// `data_dir/users/`, at most `capacity` open at once.
    ///
    /// # Errors
    ///
    /// [`BackendError::Users`] if `users.json` is missing or does not parse.
    pub fn multi(
        data_dir: &Path,
        capacity: NonZeroUsize,
        clock: ClockFactory,
    ) -> Result<Self, BackendError> {
        let users = RegistryFile::open(&data_dir.join(USERS_FILE), DEFAULT_RELOAD_INTERVAL)?;
        Ok(Self {
            api: Api::multi_user(users),
            data: Data::Multi(ServicePool::new(&data_dir.join(USERS_DIR), capacity, clock)),
        })
    }

    /// How many users' stores are open now: one in single-user mode.
    pub fn open_stores(&self) -> usize {
        match &self.data {
            Data::Single { .. } => 1,
            Data::Multi(pool) => pool.open_count(),
        }
    }

    /// Answer one request. `/health` needs no user and opens no store; every
    /// other request is refused with a bare 401 unless its token is good, and
    /// then runs against that user's data alone.
    pub fn handle(&mut self, request: &Request<'_>) -> Response {
        if let Some(response) = Api::public(request) {
            return response;
        }
        let user = match self.api.authenticate(request) {
            Ok(user) => user,
            Err(denied) => {
                if let Some(claimed) = denied.claimed {
                    eprintln!(
                        "rusty_tick: refused a token for user {:?}",
                        claimed.as_str()
                    );
                }
                return Response::unauthorized();
            }
        };
        match &mut self.data {
            Data::Single { service, .. } => Api::serve(service, request),
            Data::Multi(pool) => match pool.with(&user, |service| Api::serve(service, request)) {
                Ok(response) => response,
                // Another process holds this user's directory: try again.
                Err(PoolError::Lock(DirLockError::Held(_))) => Response::unavailable(),
                Err(error) => {
                    eprintln!("rusty_tick: opening {:?}: {error}", user.as_str());
                    Response::internal_error()
                }
            },
        }
    }
}
