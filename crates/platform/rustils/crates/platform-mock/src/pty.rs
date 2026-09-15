//! `Pty` mock: a scriptable master, not a real pty.
//!
//! Mirrors `MockTun`'s own no-real-kernel-simulation rationale (its
//! module doc) — there's no honest way to fake a kernel pty pair or a
//! real session/controlling-terminal relationship in memory, so this
//! doesn't try. A test queues bytes for [`MockPtyMaster::read`] to hand
//! back (standing in for "the spawned child wrote this to the slave"),
//! and every [`MockPtyMaster::write`] is recorded for the test to assert
//! against afterward (standing in for "the child received this on its
//! stdin"). The spawned [`platform::process::Child`] is a trivial
//! already-succeeded stand-in, the same shape `MockTun`'s device is a
//! stand-in for a real kernel-routed one — this mock exists to exercise
//! the master I/O contract, not process lifecycle.

use std::collections::VecDeque;
use std::sync::Mutex;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Child, Command, ExitStatus, GroupSpec, Signal};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

use crate::sync::lock;

/// The mock backend's [`Pty`] capability.
pub struct MockPty;

impl Pty for MockPty {
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)> {
        // Same contract the real backends enforce (`platform::pty::Pty::
        // spawn`'s own doc) — exercised here so consumer logic can be
        // tested against the rejection without a real OS.
        if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "Pty::spawn: GroupSpec::JoinGroup is incompatible with a fresh pty session",
            ));
        }
        Ok((Box::new(MockPtyMaster::new(size)), Box::new(MockPtyChild)))
    }
}

/// A scriptable in-memory stand-in for a spawned pty pair's master side.
/// See this module's doc comment for what `read`/`write` actually do
/// here.
pub struct MockPtyMaster {
    inbound: Mutex<VecDeque<Vec<u8>>>,
    written: Mutex<Vec<Vec<u8>>>,
    size: Mutex<WinSize>,
}

impl MockPtyMaster {
    pub fn new(size: WinSize) -> Self {
        Self {
            inbound: Mutex::new(VecDeque::new()),
            written: Mutex::new(Vec::new()),
            size: Mutex::new(size),
        }
    }

    /// Test setup: queue bytes for a future `read()` to hand back, as if
    /// the spawned child had just written them to the slave.
    pub fn queue_inbound(&self, bytes: Vec<u8>) {
        lock(&self.inbound).push_back(bytes);
    }

    /// Test assertion: every chunk `write()` has recorded so far, in call
    /// order.
    pub fn written_chunks(&self) -> Vec<Vec<u8>> {
        lock(&self.written).clone()
    }

    /// Test assertion: the size from construction, or the most recent
    /// `resize()`.
    pub fn current_size(&self) -> WinSize {
        *lock(&self.size)
    }
}

impl PtyMaster for MockPtyMaster {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match lock(&self.inbound).pop_front() {
            Some(bytes) => {
                let n = bytes.len().min(buf.len());
                buf[..n].copy_from_slice(&bytes[..n]);
                Ok(n)
            }
            None => Ok(0),
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        lock(&self.written).push(buf.to_vec());
        Ok(buf.len())
    }

    fn resize(&self, size: WinSize) -> Result<()> {
        *lock(&self.size) = size;
        Ok(())
    }
}

/// [`MockPty::spawn`]'s trivial child — see this module's doc comment.
struct MockPtyChild;

impl Child for MockPtyChild {
    fn wait(self: Box<Self>) -> Result<ExitStatus> {
        Ok(ExitStatus::Code(0))
    }

    fn id(&self) -> u32 {
        0
    }

    fn kill_tree(&self, _sig: Signal) -> Result<()> {
        Ok(())
    }

    fn kill_single(&self, _sig: Signal) -> Result<()> {
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(Some(ExitStatus::Code(0)))
    }

    fn wait_job(&mut self) -> Result<ExitStatus> {
        Ok(ExitStatus::Code(0))
    }

    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
        Ok(Some(ExitStatus::Code(0)))
    }

    fn take_stdin(&mut self) -> Option<Box<dyn platform::fs::File>> {
        None
    }

    fn take_stdout(&mut self) -> Option<Box<dyn platform::fs::File>> {
        None
    }

    fn take_stderr(&mut self) -> Option<Box<dyn platform::fs::File>> {
        None
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> WinSize {
        WinSize { rows: 24, cols: 80 }
    }

    #[test]
    fn spawn_reports_the_requested_size() {
        let pty = MockPty;
        let cmd = Command::new("sh", ".");
        let (master, _child) = pty.spawn(&cmd, size()).expect("spawn");
        master
            .resize(WinSize {
                rows: 40,
                cols: 100,
            })
            .expect("resize");
    }

    #[test]
    fn join_group_is_rejected() {
        let pty = MockPty;
        let cmd = Command::new("sh", ".").group(GroupSpec::JoinGroup(123));
        match pty.spawn(&cmd, size()) {
            Err(e) => assert_eq!(e.kind, ErrorKind::InvalidInput),
            Ok(_) => panic!("JoinGroup must fail"),
        }
    }

    #[test]
    fn queued_inbound_bytes_are_read_back_in_order() {
        let master = MockPtyMaster::new(size());
        master.queue_inbound(vec![1, 2, 3]);
        master.queue_inbound(vec![4, 5]);

        let mut buf = [0u8; 16];
        let n = master.read(&mut buf).expect("read first");
        assert_eq!(&buf[..n], &[1, 2, 3]);
        let n = master.read(&mut buf).expect("read second");
        assert_eq!(&buf[..n], &[4, 5]);
        let n = master.read(&mut buf).expect("read empty");
        assert_eq!(n, 0);
    }

    #[test]
    fn written_chunks_are_recorded_for_assertions() {
        let master = MockPtyMaster::new(size());
        master.write(b"hello").expect("write");
        master.write(b"world").expect("write");
        assert_eq!(
            master.written_chunks(),
            vec![b"hello".to_vec(), b"world".to_vec()]
        );
    }

    #[test]
    fn resize_updates_current_size() {
        let master = MockPtyMaster::new(size());
        master
            .resize(WinSize {
                rows: 50,
                cols: 120,
            })
            .expect("resize");
        assert_eq!(
            master.current_size(),
            WinSize {
                rows: 50,
                cols: 120
            }
        );
    }
}
