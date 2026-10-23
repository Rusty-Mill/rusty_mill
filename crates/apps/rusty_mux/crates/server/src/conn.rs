//! One client connection: handshake, then requests in, frames out.

use std::io;

use rusty_mux_core::ClientId;
use rusty_mux_proto::{Message, decode};
use rusty_tokio::io::{
    AsyncReadExt, AsyncWriteExt, OwnedUnixReadHalf, OwnedUnixWriteHalf, UnixStream,
};

use crate::outbox::{Inbox, outbox};
use crate::state::{Shared, State};

const READ_CHUNK: usize = 64 << 10;

pub(crate) async fn run(state: Shared, stream: UnixStream) {
    let (mut rd, wr) = stream.into_split();
    let mut frames = Frames::default();

    let features = match frames.next(&mut rd).await {
        Ok(Some(Message::Hello { features })) => features,
        Ok(Some(other)) => return eprintln!("rusty-mux-server: expected Hello, got {other:?}"),
        Ok(None) => return,
        Err(err) => return eprintln!("rusty-mux-server: handshake: {err}"),
    };
    let (out, inbox) = outbox();
    let client = state.borrow_mut().connect(out, features);
    rusty_tokio::task::spawn_local(write_frames(inbox, wr));

    if let Err(err) = serve(&state, client, &mut rd, &mut frames).await {
        eprintln!("rusty-mux-server: client {client}: {err}");
    }
    state.borrow_mut().disconnect(client);
}

async fn serve(
    state: &Shared,
    client: ClientId,
    rd: &mut OwnedUnixReadHalf,
    frames: &mut Frames,
) -> io::Result<()> {
    while let Some(message) = frames.next(rd).await? {
        if !state.borrow().has_client(client) {
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
