use std::collections::VecDeque;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection as RawConnection;

use crate::error::{Error, Result};

struct State {
    idle: VecDeque<RawConnection>,
    /// Connections currently owned by the pool, idle or checked out —
    /// capped at `max_size`. Not decremented when a connection is returned
    /// to `idle`, only when one is dropped for good (never, today — closed
    /// connections aren't recycled, just returned).
    total: u32,
}

struct Shared {
    path: PathBuf,
    max_size: u32,
    acquire_timeout: Duration,
    state: Mutex<State>,
    available: Condvar,
}

/// A pool of SQLite connections, all opened against the same on-disk file.
///
/// Requires the `pool` feature. SQLite allows multiple connections to the
/// same database (WAL mode makes concurrent readers cheap), so a pool is
/// useful for multi-threaded applications that would otherwise serialize on
/// a single [`crate::Connection`]. Cloning a `Pool` is cheap and shares the
/// same underlying connections (it's a handle, not a copy).
#[derive(Clone)]
pub struct Pool(Arc<Shared>);

/// A connection checked out from a [`Pool`]. Derefs to
/// [`rusqlite::Connection`]; returned to the pool automatically on drop.
pub struct PooledConnection {
    pool: Arc<Shared>,
    conn: Option<RawConnection>,
}

impl fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PooledConnection").finish_non_exhaustive()
    }
}

impl Pool {
    /// Checks out a connection, opening a new one if the pool hasn't yet
    /// reached `max_size`, otherwise waiting for one to be returned.
    /// Errors with [`Error::PoolTimeout`] if none becomes available within
    /// the pool's acquire timeout (30s).
    pub fn get(&self) -> Result<PooledConnection> {
        let deadline = Instant::now() + self.0.acquire_timeout;
        let mut state = self.0.state.lock().unwrap();
        loop {
            if let Some(conn) = state.idle.pop_front() {
                return Ok(PooledConnection {
                    pool: Arc::clone(&self.0),
                    conn: Some(conn),
                });
            }

            if state.total < self.0.max_size {
                state.total += 1;
                drop(state);
                return match open_pooled(&self.0.path) {
                    Ok(conn) => Ok(PooledConnection {
                        pool: Arc::clone(&self.0),
                        conn: Some(conn),
                    }),
                    Err(e) => {
                        // Opening failed — give back the slot we reserved.
                        self.0.state.lock().unwrap().total -= 1;
                        self.0.available.notify_one();
                        Err(e)
                    }
                };
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(Error::PoolTimeout);
            }
            let (guard, timeout) = self
                .0
                .available
                .wait_timeout(state, deadline - now)
                .unwrap();
            state = guard;
            if timeout.timed_out() && state.idle.is_empty() {
                return Err(Error::PoolTimeout);
            }
        }
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.state.lock().unwrap().idle.push_back(conn);
            self.pool.available.notify_one();
        }
    }
}

impl Deref for PooledConnection {
    type Target = RawConnection;

    fn deref(&self) -> &RawConnection {
        self.conn.as_ref().expect("connection taken only on drop")
    }
}

impl DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut RawConnection {
        self.conn.as_mut().expect("connection taken only on drop")
    }
}

fn open_pooled(path: &Path) -> Result<RawConnection> {
    let conn = RawConnection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

/// Builds a [`Pool`] of up to `max_size` connections against the database
/// at `path`, each with WAL journaling and foreign key enforcement enabled.
/// Callers waiting for a connection once the pool is exhausted time out
/// after 30s; use [`build_pool_with_timeout`] to change that.
pub fn build_pool(path: impl AsRef<Path>, max_size: u32) -> Result<Pool> {
    build_pool_with_timeout(path, max_size, Duration::from_secs(30))
}

/// Like [`build_pool`], with a configurable acquire timeout instead of the
/// 30s default.
pub fn build_pool_with_timeout(
    path: impl AsRef<Path>,
    max_size: u32,
    acquire_timeout: Duration,
) -> Result<Pool> {
    Ok(Pool(Arc::new(Shared {
        path: path.as_ref().to_path_buf(),
        max_size,
        acquire_timeout,
        state: Mutex::new(State {
            idle: VecDeque::new(),
            total: 0,
        }),
        available: Condvar::new(),
    })))
}
