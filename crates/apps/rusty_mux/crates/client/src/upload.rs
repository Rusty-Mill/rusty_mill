//! Sends a file to a block's server, a chunk at a time, with
//! `terminal.upload` (C82). The connection has attached to the block and
//! holds its lease, and negotiated [`Features::UPLOAD`].
//!
//! [`Features::UPLOAD`]: rusty_mux_proto::Features::UPLOAD

use std::io::{self, Read, Write};

use rusty_mux_core::BlockId;
use rusty_mux_proto::Message;
use rusty_mux_proto::schema::Value;

use crate::Conn;

/// Bytes sent per call: the server's chunk limit.
pub const CHUNK: usize = 1 << 20;

/// Reads up to `size` bytes: fewer only at the end of `file`.
fn fill(file: &mut impl Read, size: usize) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(size);
    file.take(size as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Sends `file` as `name` and returns its absolute path on the server.
/// Request numbers start at `first_req`; messages other than the reply
/// (events, say) are skipped.
///
/// An error after the last chunk was sent (a lost reply, say) does not say
/// the file failed: it may already be published in the server's folder. Look
/// there before sending again; a resend from offset 0 is kept beside the
/// first copy as `name-1`, and a resend at a later offset reports an unknown
/// outcome.
pub fn send<S: Read + Write>(
    conn: &mut Conn<S>,
    block: BlockId,
    name: &str,
    file: &mut impl Read,
    first_req: u64,
) -> io::Result<String> {
    let mut req = first_req;
    send_with(name, file, CHUNK, |args| {
        let method = "terminal.upload".into();
        conn.send(&Message::BlockCall {
            req,
            block,
            method,
            args,
        })?;
        let text = reply(conn, req);
        req += 1;
        text
    })
}

/// [`send`] for a caller that owns the connection's reads: `call` takes
/// each chunk's `terminal.upload` arguments, makes the call and returns its
/// reply text. Chunks are `chunk` bytes (1 to [`CHUNK`]): a caller that
/// shares its connection with typing sends smaller ones, so a chunk's write
/// holds the connection for less time. The same outcome rules as [`send`]
/// apply.
pub fn send_with(
    name: &str,
    file: &mut impl Read,
    chunk: usize,
    mut call: impl FnMut(Vec<(String, Value)>) -> io::Result<String>,
) -> io::Result<String> {
    let chunk = chunk.clamp(1, CHUNK);
    let mut data = fill(file, chunk)?;
    let mut offset = 0u32;
    loop {
        // One chunk ahead, to know which is the last.
        let next = fill(file, chunk)?;
        let done = next.is_empty();
        let len = u32::try_from(data.len()).map_err(io::Error::other)?;
        let text = call(vec![
            ("name".to_owned(), Value::Text(name.to_owned())),
            ("offset".to_owned(), Value::Lines(offset)),
            ("data".to_owned(), Value::Bytes(std::mem::take(&mut data))),
            ("done".to_owned(), Value::Flag(done)),
        ])?;
        if done {
            return Ok(text);
        }
        offset = offset.checked_add(len).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a file is at most 2 GiB")
        })?;
        data = next;
    }
}

/// The text answering `req`, or its refusal as an error.
fn reply<S: Read + Write>(conn: &mut Conn<S>, req: u64) -> io::Result<String> {
    loop {
        match conn.recv()? {
            Message::Text { req: r, text } if r == req => return Ok(text),
            Message::Failed { req: r, reason, .. } if r == req => {
                return Err(io::Error::other(reason));
            }
            _ => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::net::UnixStream;

    use rusty_mux_proto::{decode, encode};

    use super::*;

    /// Answers each call with the bytes so far, and the last with a path;
    /// returns what it was sent.
    fn serve(mut stream: UnixStream) -> Vec<(u32, usize, bool)> {
        let mut seen = Vec::new();
        let mut buf = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
            while let Some((message, used)) = decode(&buf).unwrap() {
                buf.drain(..used);
                let Message::BlockCall { req, args, .. } = message else {
                    panic!("{message:?}")
                };
                let get = |n: &str| args.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
                let (Some(Value::Lines(at)), Some(Value::Bytes(data))) =
                    (get("offset"), get("data"))
                else {
                    panic!("{args:?}")
                };
                let done = get("done") == Some(Value::Flag(true));
                seen.push((at, data.len(), done));
                let text = match done {
                    true => "/state/f".to_owned(),
                    false => (at as usize + data.len()).to_string(),
                };
                stream
                    .write_all(&encode(&Message::Text { req, text }).unwrap())
                    .unwrap();
                if done {
                    return seen;
                }
            }
        }
    }

    #[test]
    fn a_file_goes_in_chunks_and_the_last_is_marked() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || serve(theirs));
        let body = vec![7u8; CHUNK * 2 + 10];
        let path = send(
            &mut Conn::new(ours),
            BlockId::from_uuid(rusty_uuid::Uuid::nil()),
            "f",
            &mut body.as_slice(),
            1,
        )
        .unwrap();
        assert_eq!(path, "/state/f");
        let c = CHUNK as u32;
        assert_eq!(
            server.join().unwrap(),
            [(0, CHUNK, false), (c, CHUNK, false), (2 * c, 10, true)]
        );
    }

    #[test]
    fn an_empty_file_is_one_committing_call() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || serve(theirs));
        send(
            &mut Conn::new(ours),
            BlockId::from_uuid(rusty_uuid::Uuid::nil()),
            "e",
            &mut io::empty(),
            1,
        )
        .unwrap();
        assert_eq!(server.join().unwrap(), [(0, 0, true)]);
    }

    #[test]
    fn a_smaller_chunk_size_splits_the_file_and_is_kept_in_range() {
        let mut seen = Vec::new();
        let text = send_with("f", &mut [1u8; 25].as_slice(), 10, |args| {
            let Some((_, Value::Bytes(data))) = args.iter().find(|(k, _)| k == "data") else {
                panic!("{args:?}")
            };
            seen.push(data.len());
            Ok("/p".into())
        })
        .unwrap();
        assert_eq!((text.as_str(), seen), ("/p", vec![10, 10, 5]));
        // Out of range sizes are clamped, never zero (which would loop).
        let mut calls = 0;
        send_with("f", &mut [1u8; 3].as_slice(), 0, |_| {
            calls += 1;
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(calls, 3);
    }
}
