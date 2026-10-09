//! Storage port and the in-memory adapter used for stage 1.

use crate::codec::decode_artifact;
use crate::command::{AgentAction, Code, Command, Rejection, Response};
use crate::engine::handle;
use crate::event::Event;
use crate::ids::*;
use crate::record::ArtifactPayload;
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
    /// Re-read a task from the medium so another writer's appends are visible.
    /// No-op for a store that is the only writer.
    fn refresh(&mut self, _task: &TaskId) -> Result<(), StoreError> {
        Ok(())
    }
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

    /// The same driver keyed by `master`: every token and run secret it
    /// derives or checks uses it. Survives `reload`.
    pub fn with_master(mut self, master: [u8; 32]) -> Driver<S> {
        self.state.master = master;
        self
    }

    fn check_boundary(&self, cmd: &Command) -> Option<Rejection> {
        let (blob, payload) = match cmd {
            Command::Agent {
                action: AgentAction::PutArtifact { blob, payload },
                ..
            } => (blob, payload),
            Command::Runner { blob, payload, .. } => (blob, payload),
            Command::Open(o) => (&o.brief, &ArtifactPayload::Brief),
            _ => return None,
        };
        let bytes = match self.store.blob_get(&blob.sha) {
            Ok(b) => b,
            Err(_) => {
                return Some(Rejection::new(
                    Code::BlobMissing,
                    format!("{:?} is not in the store", blob.sha),
                ))
            }
        };
        if bytes.len() as u64 != blob.len {
            return Some(Rejection::new(
                Code::PayloadMismatch,
                "blob length disagrees with the stored bytes",
            ));
        }
        match decode_artifact(payload.kind(), &bytes) {
            Ok(decoded) if decoded == *payload => None,
            Ok(_) => Some(Rejection::new(
                Code::PayloadMismatch,
                "payload is not what the stored bytes decode to",
            )),
            Err(e) => Some(Rejection::new(
                Code::PayloadMismatch,
                format!("stored bytes do not decode: {e}"),
            )),
        }
    }

    /// Refresh the store from its medium, then rebuild state from the log.
    pub fn sync(&mut self) -> Result<(), StoreError> {
        let task = self.state.task.clone();
        self.store.refresh(&task)?;
        self.reload()
    }

    /// Rebuild state from the log.
    pub fn reload(&mut self) -> Result<(), StoreError> {
        let events = self.store.events(&self.state.task, 0)?;
        self.state = TaskState::fold_with(self.state.task.clone(), self.state.master, &events);
        Ok(())
    }

    /// Dispatch one command. On a revision conflict, reload and recompute once
    /// with the same command; the command's own `op` and `expected_rev` are untouched.
    ///
    /// Before the core sees an artifact command, the driver checks the
    /// boundary: the blob is already in the store (A3, blobs before references)
    /// and the claimed payload is what those bytes decode to (A1). A failure is
    /// a rejection returned without touching the log.
    pub fn dispatch(&mut self, cmd: &Command, now: Time) -> Result<Response, StoreError> {
        if let Some(rej) = self.check_boundary(cmd) {
            return Ok(Response::Rejected(rej));
        }
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
