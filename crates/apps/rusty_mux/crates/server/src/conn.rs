//! One client connection: handshake, then requests in, frames out.

use std::io;
use std::rc::Rc;

use rusty_mux_core::ClientId;
use rusty_mux_proto::{Message, decode};
use rusty_tokio::io::{
    AsyncReadExt, AsyncWriteExt, OwnedUnixReadHalf, OwnedUnixWriteHalf, UnixStream,
};
use rusty_tokio::sync::Notify;

use crate::outbox::{Inbox, outbox};
use crate::state::{Shared, State};

const READ_CHUNK: usize = 64 << 10;

pub(crate) async fn run(state: Shared, stream: UnixStream) {
    let (mut rd, wr) = stream.into_split();
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
    state.borrow_mut().disconnect(client, &cancel);
}

async fn serve(
    state: &Shared,
    client: ClientId,
    cancel: &Rc<Notify>,
    rd: &mut OwnedUnixReadHalf,
    frames: &mut Frames,
) -> io::Result<()> {
    while let Some(message) = frames.next(rd).await? {
        if !state.borrow().served_by(client, cancel) {
            return Ok(());
        }
        if let Some((pty, data)) = State::request(state, client, message) {
            pty.write_all(&data).await?;
        }
    }
    Ok(())
}

/// Writes queued frames until the server drops the client, then closes the
/// write side so the client sees end of stream.
async fn write_frames(mut inbox: Inbox, mut wr: OwnedUnixWriteHalf) {
    while let Some(frame) = inbox.next().await {
        if wr.write_all(&frame).await.is_err() {
            return;
        }
    }
    if let Err(err) = wr.shutdown().await {
        eprintln!("rusty-mux-server: closing a connection: {err}");
    }
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
    async fn next(&mut self, rd: &mut OwnedUnixReadHalf) -> io::Result<Option<Message>> {
        loop {
            let decoded = decode(&self.buf[self.start..])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.start += used;
                return Ok(Some(message));
            }
            self.buf.drain(..self.start);
            self.start = 0;
            let n = rd.read(&mut self.chunk).await?;
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
            let task = rusty_tokio::task::spawn_local(run(state.clone(), server));
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
