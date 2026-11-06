//! Where a pane's terminal comes from: the window's session-source port
//! (handoff pack D101).
//!
//! The window draws a pane from a [`PaneIo`]: a [`Feed`] of what to show
//! and a writer for input and size requests. A local shell is one adapter
//! ([`PaneIo::local`]); a rusty_mux block, attached from `rusty_mux_gui`,
//! is another, so the window keeps one draw path for both.

use std::io;
use std::sync::mpsc::{Receiver, SyncSender};

use crate::backend::{BackendHandle, read_batch};
use crate::core::{AnsiParser, Grid};

/// Most PTY output parsed under one grid lock: about 1 ms of parsing, so
/// a flood still leaves the UI thread the lock between batches.
pub(crate) const READ_BATCH: usize = 64 * 1024;

/// Batches read ahead of the parser: at most 1 MiB queued. The kernel's PTY
/// buffer holds only a few KiB, so without a separate read thread the child
/// stalls on every batch the parser works through.
pub(crate) const READ_AHEAD: usize = 16;

/// One step of what a pane shows.
pub enum Feed {
    /// Terminal output to parse into the pane's grid.
    Bytes(Vec<u8>),
    /// A whole terminal to show instead of the pane's own, such as a mux
    /// block's screen on attach.
    Screen(Box<Grid>, Box<AnsiParser>),
    /// The source resized the pane: a mux block's size is the server's.
    Resized { cols: u16, rows: u16 },
}

/// A pane's two ends. Dropping it ends the pane: its feed stops and its
/// writer hangs up (a local shell's PTY, a mux block's connection).
pub struct PaneIo {
    /// Takes the pane's input and size requests.
    pub writer: Box<dyn BackendHandle>,
    /// What to show, blocking until the next step; ends with the pane.
    pub feed: Box<dyn Iterator<Item = Feed> + Send>,
    /// Whether the window answers the program's queries (DA, DSR). A mux
    /// server's own terminal already does, so its blocks say `false`.
    pub answers_queries: bool,
}

impl PaneIo {
    /// A local PTY: its output read ahead in batches on a thread of its
    /// own, so the child keeps writing while the window parses, and its
    /// queries answered here. `name` names that thread.
    pub fn local(handle: Box<dyn BackendHandle>, name: String) -> io::Result<Self> {
        let mut reader = handle.try_clone()?;
        let (batches, ready) = std::sync::mpsc::sync_channel(READ_AHEAD);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || read_ahead(reader.as_mut(), &batches))?;
        Ok(Self {
            writer: handle,
            feed: Box::new(Batches(ready)),
            answers_queries: true,
        })
    }
}

/// The batches [`read_ahead`] queued, as a feed.
struct Batches(Receiver<Vec<u8>>);

impl Iterator for Batches {
    type Item = Feed;

    fn next(&mut self) -> Option<Feed> {
        self.0.recv().ok().map(Feed::Bytes)
    }
}

/// Read the PTY in batches (see [`read_batch`]) and queue each for the
/// parser, until EOF, a read error, or the parser hanging up. The queue is
/// bounded, so a parser that falls behind still pushes back on the child.
pub(crate) fn read_ahead(reader: &mut dyn BackendHandle, batches: &SyncSender<Vec<u8>>) {
    let mut buf = vec![0u8; READ_BATCH];
    loop {
        match read_batch(reader, &mut buf) {
            Ok(0) | Err(_) => return, // EOF (child exited) or a dead PTY
            Ok(n) => {
                if batches.send(buf[..n].to_vec()).is_err() {
                    return; // parser gone
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Feed, PaneIo, READ_AHEAD, read_ahead};
    use crate::backend::BackendHandle;
    use crate::backend::mock::{Chunks, apart};
    use std::io::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The batches `read_ahead` queues for `parts` with nobody parsing.
    fn read_all(parts: Vec<Result<&[u8], Error>>) -> Vec<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(READ_AHEAD);
        read_ahead(&mut apart(parts), &tx);
        drop(tx);
        rx.into_iter().collect()
    }

    #[test]
    fn reading_runs_ahead_of_the_parser() {
        let got = read_all(vec![Ok(b"a"), Ok(b"b"), Ok(b"c")]);
        assert_eq!(got, [b"a", b"b", b"c"], "all queued before any parse");
    }

    #[test]
    fn a_read_error_ends_reading() {
        let got = read_all(vec![Ok(b"a"), Err(Error::other("gone")), Ok(b"b")]);
        assert_eq!(got, [b"a"]);
    }

    /// A PTY handle whose clones share one stream of reads.
    #[derive(Clone)]
    struct Shared(Arc<std::sync::Mutex<Chunks>>);

    impl BackendHandle for Shared {
        fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            self.0.lock().expect("unpoisoned").read_into(buf)
        }
        fn write(&mut self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }
        fn try_clone(&self) -> Result<Box<dyn BackendHandle>, Error> {
            Ok(Box::new(self.clone()))
        }
        fn set_winsize(&mut self, _cols: u16, _rows: u16) -> Result<(), Error> {
            Ok(())
        }
        #[cfg(unix)]
        fn pty_fd(&self) -> std::os::unix::io::RawFd {
            -1
        }
    }

    #[test]
    fn a_local_pane_feeds_its_output_and_answers_queries() {
        let pty = Shared(Arc::new(std::sync::Mutex::new(apart(vec![
            Ok(b"a"),
            Ok(b"b"),
        ]))));
        let io = PaneIo::local(Box::new(pty), "t".into()).expect("a local pane");
        assert!(io.answers_queries);
        let fed: Vec<Vec<u8>> = io
            .feed
            .map(|f| match f {
                Feed::Bytes(b) => b,
                _ => panic!("a PTY feeds only bytes"),
            })
            .collect();
        assert_eq!(fed.concat(), b"ab", "then ends at EOF");
    }

    /// A PTY handle that counts its reads.
    struct Counted {
        inner: Chunks,
        reads: Arc<AtomicUsize>,
    }

    impl BackendHandle for Counted {
        fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.inner.read_into(buf)
        }
        fn output_ready(&self) -> bool {
            self.inner.output_ready()
        }
        fn write(&mut self, data: &[u8]) -> Result<(), Error> {
            self.inner.write(data)
        }
        fn try_clone(&self) -> Result<Box<dyn BackendHandle>, Error> {
            self.inner.try_clone()
        }
        fn set_winsize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
            self.inner.set_winsize(cols, rows)
        }
        #[cfg(unix)]
        fn pty_fd(&self) -> std::os::unix::io::RawFd {
            self.inner.pty_fd()
        }
    }

    /// Ten one-byte batches, and the counter of how many were read.
    fn ten_parts() -> (Counted, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        let parts = (0..10).map(|_| Ok(&b"x"[..])).collect();
        let handle = Counted {
            inner: apart(parts),
            reads: Arc::clone(&reads),
        };
        (handle, reads)
    }

    #[test]
    fn a_gone_parser_ends_reading() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        drop(rx);
        let (mut pty, reads) = ten_parts();
        read_ahead(&mut pty, &tx);
        assert_eq!(
            reads.load(Ordering::Relaxed),
            1,
            "the first failed send stops reading"
        );
    }

    #[test]
    fn a_parser_gone_while_the_queue_is_full_ends_reading() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (mut pty, reads) = ten_parts();
        let counter = Arc::clone(&reads);
        let (done_tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            read_ahead(&mut pty, &tx);
            let _ = done_tx.send(());
        });
        // Wait until the reader is blocked on the full queue, then hang up.
        while counter.load(Ordering::Relaxed) < 2 {
            std::thread::yield_now();
        }
        drop(rx);
        done.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the blocked send is released and reading stops");
        assert!(reads.load(Ordering::Relaxed) < 10);
    }
}
