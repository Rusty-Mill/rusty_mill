//! One client connection: handshake, then requests in, frames out. A
//! connection is any read half and write half: a Unix socket's, or a
//! datagram stream's.

use std::future::poll_fn;
use std::io;
use std::pin::Pin;
use std::rc::Rc;

use rusty_mux_core::ClientId;
use rusty_mux_proto::{Message, decode};
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use rusty_tokio::sync::Notify;

use crate::outbox::{Inbox, outbox};
use crate::state::{Shared, State};

const READ_CHUNK: usize = 64 << 10;

/// Serves one connection; `on_client` learns which client it serves, and
/// the cancel that tells whether it still does.
pub(crate) async fn run<R, W>(
    state: Shared,
    mut rd: R,
    wr: W,
    on_client: impl FnOnce(ClientId, &Rc<Notify>),
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut frames = Frames::default();

    let (features, resume, takeover) = match frames.next(&mut rd).await {
        Ok(Some(Message::Hello {
            features,
            resume,
            takeover,
        })) => (features, resume, takeover),
        Ok(Some(other)) => return eprintln!("rusty-mux-server: expected Hello, got {other:?}"),
        Ok(None) => return,
        Err(err) => return eprintln!("rusty-mux-server: handshake: {err}"),
    };
    let (out, inbox, cancel) = outbox();
    let client = state.borrow_mut().connect(out, features, resume, takeover);
    on_client(client, &cancel);
    // One owning future races both halves and eviction. Dropping the losing
    // futures closes both socket halves and discards the receiver/backlog,
    // including a writer currently blocked inside write_all.
    rusty_tokio::select! {
        _ = cancel.notified() => {},
        result = serve(&state, client, &cancel, &mut rd, &mut frames) => {
            if let Err(err) = result {
                eprintln!("rusty-mux-server: client {client}: {err}");
            }
        },
        _ = write_frames(inbox, wr) => {},
    }
    // A connection that took this client over owns it now; leave it be.
    let orphaned = state.borrow_mut().disconnect(client, &cancel);
    if orphaned {
        rusty_tokio::time::sleep(crate::state::ORPHAN_GRACE).await;
        state.borrow_mut().expire(client, &cancel);
    }
}

async fn serve(
    state: &Shared,
    client: ClientId,
    cancel: &Rc<Notify>,
    rd: &mut (impl AsyncRead + Unpin),
    frames: &mut Frames,
) -> io::Result<()> {
    while let Some(message) = frames.next(rd).await? {
        if !state.borrow().served_by(client, cancel) {
            return Ok(());
        }
        let Some(write) = State::request(state, client, message) else {
            continue;
        };
        write.pty.write_all(&write.data).await?;
        if let Some(key) = write.key {
            let now = std::time::Instant::now();
            state.borrow_mut().written(client, cancel, key, now);
        }
    }
    Ok(())
}

/// Writes queued frames until the server drops the client, then closes the
/// write side so the client sees end of stream.
async fn write_frames(mut inbox: Inbox, mut wr: impl AsyncWrite + Unpin) {
    while let Some(frame) = inbox.next().await {
        if write_all(&mut wr, &frame).await.is_err() {
            return;
        }
    }
    if let Err(err) = poll_fn(|cx| Pin::new(&mut wr).poll_shutdown(cx)).await {
        eprintln!("rusty-mux-server: closing a connection: {err}");
    }
}

// rusty_tokio's extension methods need `Send` halves; a datagram stream's
// are `Rc`-based, so these poll the traits directly.

async fn read(rd: &mut (impl AsyncRead + Unpin), buf: &mut [u8]) -> io::Result<usize> {
    poll_fn(|cx| {
        let mut buf = ReadBuf::new(buf);
        Pin::new(&mut *rd)
            .poll_read(cx, &mut buf)
            .map_ok(|()| buf.filled().len())
    })
    .await
}

async fn write_all(wr: &mut (impl AsyncWrite + Unpin), mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = poll_fn(|cx| Pin::new(&mut *wr).poll_write(cx, bytes)).await?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = &bytes[n..];
    }
    Ok(())
}

/// Reassembles frames from a byte stream. The buffer never holds more than
/// one frame (at most `MAX_FRAME`) plus one read.
struct Frames {
    buf: Vec<u8>,
    /// Bytes of `buf` already decoded; compacted before the next read.
    start: usize,
    chunk: Box<[u8]>,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            chunk: vec![0; READ_CHUNK].into_boxed_slice(),
        }
    }
}

impl Frames {
    /// The next message, or `None` at a clean end of stream.
    async fn next(&mut self, rd: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Message>> {
        loop {
            let decoded = decode(&self.buf[self.start..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.start += used;
                return Ok(Some(message));
            }
            self.buf.drain(..self.start);
            self.start = 0;
            let n = read(rd, &mut self.chunk).await?;
            if n == 0 {
                return match self.buf.is_empty() {
                    true => Ok(None),
                    false => Err(io::ErrorKind::UnexpectedEof.into()),
                };
            }
            self.buf.extend_from_slice(&self.chunk[..n]);
        }
    }
}

// Windows has no anonymous AF_UNIX pair.
#[cfg(all(test, unix))]
mod regression {
    use super::*;
    use crate::Config;
    use rusty_mux_proto::{Features, encode};
    use rusty_tokio::io::UnixStream;
    use rusty_tokio::{Builder, LocalOptions};
    use std::time::Duration;

    #[test]
    fn eviction_releases_blocked_writer_reader_socket_and_backlog() {
        let rt = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap();
        rt.block_on(async {
            let state = State::shared(
                Config {
                    program: "/bin/sh".into(),
                    args: vec![],
                    cwd: "/".into(),
                },
                None,
            );
            let (peer, server) = UnixStream::pair().unwrap();
            peer.write_all(
                &encode(&Message::Hello {
                    features: Features::CHECKPOINT_V6,
                    resume: None,
                    takeover: None,
                })
                .unwrap(),
            )
            .await
            .unwrap();
            let (rd, wr) = server.into_split();
            let task = rusty_tokio::task::spawn_local(run(state.clone(), rd, wr, |_, _| {}));
            rusty_tokio::task::yield_now().await;
            let client = state.borrow().test_client();
            // The peer stops reading. Force the writer into a pending write_all,
            // then overfill its queue while the reader waits for another request.
            state.borrow().test_checkpoint(client, vec![0; 17 << 20]);
            rusty_tokio::task::yield_now().await;
            let queued = state.borrow().test_queue(client);
            assert_eq!(
                queued.get(),
                0,
                "writer must have dequeued the large checkpoint"
            );
            assert!(
                !task.is_finished(),
                "non-reading peer must keep connection pending"
            );
            let weak = Rc::downgrade(&queued);
            for _ in 0..10 {
                state.borrow_mut().test_send(client, vec![0; 1 << 20]);
            }
            drop(queued);
            rusty_tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
            assert!(!state.borrow().has_client(client));
            assert!(
                weak.upgrade().is_none(),
                "receiver and queued backlog retained"
            );
            // Drain bytes already in the kernel, then observe the socket close.
            let mut buf = [0; 8192];
            rusty_tokio::time::timeout(Duration::from_secs(2), async {
                while peer.read(&mut buf).await.unwrap() != 0 {}
            })
            .await
            .unwrap();
        });
    }

    use std::rc::Rc;
}
