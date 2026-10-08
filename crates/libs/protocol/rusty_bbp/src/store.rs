//! Storage port and the in-memory adapter used for stage 1.

use crate::command::{Command, Response};
use crate::engine::handle;
use crate::event::Event;
use crate::ids::*;
use crate::state::TaskState;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    /// Another writer appended first. The driver reloads and recomputes (A3).
    Conflict {
        expected: Rev,
        actual: Rev,
    },
    UnknownTask,
    UnknownBlob,
    /// The backing medium failed or is corrupt.
    Backend(String),
}

/// Single-writer, transactional, append-only log plus content-addressed blobs.
pub trait Store {
    fn append(
        &mut self,
        task: &TaskId,
        expected_rev: Rev,
        events: &[Event],
    ) -> Result<(), StoreError>;
    fn events(&self, task: &TaskId, after: usize) -> Result<Vec<Event>, StoreError>;
    /// The store computes the digest; callers never supply it.
    fn blob_put(&mut self, bytes: &[u8]) -> BlobRef;
    fn blob_get(&self, sha: &Sha256) -> Result<Vec<u8>, StoreError>;
}

#[derive(Default)]
pub struct MemStore {
    logs: HashMap<TaskId, Vec<Event>>,
    blobs: HashMap<Sha256, Vec<u8>>,
}

impl MemStore {
    pub fn new() -> MemStore {
        MemStore::default()
    }
}

impl Store for MemStore {
    fn append(
        &mut self,
        task: &TaskId,
        expected_rev: Rev,
        events: &[Event],
    ) -> Result<(), StoreError> {
        let log = self.logs.entry(task.clone()).or_default();
        let actual = Rev(log.len() as u64);
        if actual != expected_rev {
            return Err(StoreError::Conflict {
                expected: expected_rev,
                actual,
            });
        }
        log.extend_from_slice(events);
        Ok(())
    }
    fn events(&self, task: &TaskId, after: usize) -> Result<Vec<Event>, StoreError> {
        let log = self.logs.get(task).ok_or(StoreError::UnknownTask)?;
        Ok(log.get(after..).unwrap_or(&[]).to_vec())
    }
    fn blob_put(&mut self, bytes: &[u8]) -> BlobRef {
        let sha = Sha256::of(bytes);
        self.blobs.entry(sha).or_insert_with(|| bytes.to_vec());
        BlobRef {
            sha,
            len: bytes.len() as u64,
        }
    }
    fn blob_get(&self, sha: &Sha256) -> Result<Vec<u8>, StoreError> {
        self.blobs.get(sha).cloned().ok_or(StoreError::UnknownBlob)
    }
}

/// The moderator as a function: handle, append, reload on conflict.
pub struct Driver<S: Store> {
    pub store: S,
    pub state: TaskState,
}

impl<S: Store> Driver<S> {
    pub fn new(store: S, task: TaskId) -> Driver<S> {
        Driver {
            state: TaskState::new(task),
            store,
        }
    }

    /// Rebuild state from the log.
    pub fn reload(&mut self) -> Result<(), StoreError> {
        let events = self.store.events(&self.state.task, 0)?;
        self.state = TaskState::fold(self.state.task.clone(), &events);
        Ok(())
    }

    /// Dispatch one command. On a revision conflict, reload and recompute once
    /// with the same command; the command's own `op` and `expected_rev` are untouched.
    pub fn dispatch(&mut self, cmd: &Command, now: Time) -> Result<Response, StoreError> {
        for _ in 0..2 {
            let out = handle(&self.state, cmd, now);
            match self
                .store
                .append(&self.state.task, self.state.rev, &out.events)
            {
                Ok(()) => {
                    for e in &out.events {
                        self.state.apply(e);
                    }
                    return Ok(out.response);
                }
                Err(StoreError::Conflict { .. }) => self.reload()?,
                Err(e) => return Err(e),
            }
        }
        Err(StoreError::Conflict {
            expected: self.state.rev,
            actual: self.state.rev,
        })
    }
}
