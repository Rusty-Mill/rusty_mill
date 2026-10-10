//! Newline-delimited JSON over a pair of byte streams: a child process's
//! stdin and stdout, or any other pair (the tests use in-process pipes).
//!
//! A reader thread parses lines and queues the messages, so a wait can time
//! out. A line that is not a valid message is skipped: stdout of some servers
//! carries stray text, and one bad line should not end the conversation.

use crate::transport::{Recv, Transport};
use rusty_mcp_proto::{Message, Wire};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Longest line accepted; a longer one is dropped.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// A transport over any reader and writer.
pub struct StdioTransport {
    writer: Option<Box<dyn Write + Send>>,
    inbox: Receiver<Message>,
    child: Option<Child>,
}

fn read_lines(reader: impl Read, tx: &std::sync::mpsc::Sender<Message>) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line);
        match n {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.len() > MAX_LINE_BYTES {
            // Too long: drain to the end of the line and move on.
            while line.last() != Some(&b'\n') {
                line.clear();
                match reader
                    .by_ref()
                    .take(MAX_LINE_BYTES as u64)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
            }
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Ok(message) = Message::from_json(text) {
            if tx.send(message).is_err() {
                return;
            }
        }
    }
}

impl StdioTransport {
    /// Talk over `reader` (what the server writes) and `writer` (what it
    /// reads).
    pub fn new(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        let (tx, inbox) = channel();
        std::thread::spawn(move || read_lines(reader, &tx));
        Self {
            writer: Some(Box::new(writer)),
            inbox,
            child: None,
        }
    }

    /// Start `command` as a server and talk to its stdin and stdout. Its
    /// stderr is inherited.
    ///
    /// # Errors
    /// The process could not be started.
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin on the child"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout on the child"))?;
        let mut transport = Self::new(stdout, stdin);
        transport.child = Some(child);
        Ok(transport)
    }
}

impl Transport for StdioTransport {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
        let mut line = message.to_json();
        line.push('\n');
        writer.write_all(line.as_bytes())?;
        writer.flush()
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Recv> {
        match self.inbox.recv_timeout(timeout) {
            Ok(message) => Ok(Recv::Message(message)),
            Err(RecvTimeoutError::Timeout) => Ok(Recv::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(Recv::Closed),
        }
    }
}

impl Drop for StdioTransport {
    /// Close the child's stdin, give it a moment to exit, then end it.
    fn drop(&mut self) {
        self.writer = None;
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}
