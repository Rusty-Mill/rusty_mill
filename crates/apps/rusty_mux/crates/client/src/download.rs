//! Reads a file of a block's server with `terminal.download` (C83). The
//! connection has attached to the block and holds its lease, and negotiated
//! [`Features::DOWNLOAD`].
//!
//! [`Features::DOWNLOAD`]: rusty_mux_proto::Features::DOWNLOAD

use std::io::{self, Read, Write};

use rusty_mux_core::BlockId;
use rusty_mux_proto::Message;
use rusty_mux_proto::schema::Value;

use crate::Conn;

/// Writes the file at `path` on the server (relative to the block's working
/// directory) into `out` and returns its size. Request numbers start at
/// `first_req`; messages other than the reply (events, say) are skipped.
pub fn fetch<S: Read + Write>(
    conn: &mut Conn<S>,
    block: BlockId,
    path: &str,
    out: &mut impl Write,
    first_req: u64,
) -> io::Result<u64> {
    let mut req = first_req;
    fetch_with(path, out, |args| {
        conn.send(&Message::BlockCall {
            req,
            block,
            method: "terminal.download".into(),
            args,
        })?;
        let text = reply(conn, req);
        req += 1;
        text
    })
}

/// [`fetch`] for a caller that owns the connection's reads: `call` takes
/// each chunk's `terminal.download` arguments, makes the call and returns its
/// reply text.
pub fn fetch_with(
    path: &str,
    out: &mut impl Write,
    call: impl FnMut(Vec<(String, Value)>) -> io::Result<String>,
) -> io::Result<u64> {
    fetch_progress(path, out, |_, _| {}, call)
}

/// [`fetch_with`] that calls `progress` with the bytes written and the
/// file's size after each chunk.
pub fn fetch_progress(
    path: &str,
    out: &mut impl Write,
    mut progress: impl FnMut(u64, u64),
    mut call: impl FnMut(Vec<(String, Value)>) -> io::Result<String>,
) -> io::Result<u64> {
    let mut offset = 0u64;
    let mut first = None;
    loop {
        let at = u32::try_from(offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a file is at most 2 GiB"))?;
        let args = vec![
            ("path".to_owned(), Value::Text(path.to_owned())),
            ("offset".to_owned(), Value::Lines(at)),
        ];
        let (size, data) = split(&call(args)?)?;
        // The first size is the file's; a different one later means it
        // changed under the read, and what is in `out` is not that file.
        if *first.get_or_insert(size) != size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the file changed size while it was read",
            ));
        }
        // Nothing past the declared size is written.
        if offset + data.len() as u64 > size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the reply is longer than the file's declared size",
            ));
        }
        out.write_all(&data)?;
        offset += data.len() as u64;
        progress(offset, size);
        if offset >= size {
            return Ok(size);
        }
        if data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the file got shorter while it was read",
            ));
        }
    }
}

/// A reply's size line and its decoded bytes.
fn split(text: &str) -> io::Result<(u64, Vec<u8>)> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let (size, body) = text.split_once('\n').ok_or_else(|| bad("no size line"))?;
    let size = size.parse().map_err(|_| bad("the size is not a number"))?;
    let data = rusty_base64::decode_standard(body).map_err(|_| bad("the data is not base64"))?;
    Ok((size, data))
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

    /// Serves `file` in chunks of 4 bytes.
    fn serve(mut stream: UnixStream, file: &'static [u8]) {
        let mut buf = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Some((Message::BlockCall { req, args, .. }, used)) = decode(&buf).unwrap() {
                buf.drain(..used);
                let Some((_, Value::Lines(at))) = args.iter().find(|(k, _)| k == "offset") else {
                    panic!("{args:?}")
                };
                let from = (*at as usize).min(file.len());
                let to = (from + 4).min(file.len());
                let text = format!(
                    "{}\n{}",
                    file.len(),
                    rusty_base64::encode_standard(&file[from..to])
                );
                stream
                    .write_all(&encode(&Message::Text { req, text }).unwrap())
                    .unwrap();
            }
        }
    }

    fn fetch_from(file: &'static [u8]) -> Vec<u8> {
        let (ours, theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(theirs, file));
        let mut out = Vec::new();
        let size = fetch(
            &mut Conn::new(ours),
            BlockId::from_uuid(rusty_uuid::Uuid::nil()),
            "f",
            &mut out,
            1,
        )
        .unwrap();
        assert_eq!(size, file.len() as u64);
        out
    }

    #[test]
    fn a_file_comes_in_pieces_and_is_put_together() {
        assert_eq!(fetch_from(b"0123456789"), b"0123456789");
    }

    #[test]
    fn progress_counts_the_bytes_written_up_to_the_size() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(theirs, b"0123456789"));
        let mut conn = Conn::new(ours);
        let block = BlockId::from_uuid(rusty_uuid::Uuid::nil());
        let (mut req, mut seen, mut out) = (1, Vec::new(), Vec::new());
        fetch_progress(
            "f",
            &mut out,
            |got, total| seen.push((got, total)),
            |args| {
                let method = "terminal.download".into();
                conn.send(&Message::BlockCall {
                    req,
                    block,
                    method,
                    args,
                })?;
                let text = reply(&mut conn, req);
                req += 1;
                text
            },
        )
        .unwrap();
        assert_eq!(seen, [(4, 10), (8, 10), (10, 10)]);
    }

    #[test]
    fn a_file_that_changes_size_mid_read_is_refused() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut sizes = [10usize, 6].into_iter();
            loop {
                let mut chunk = [0u8; 4096];
                let n = theirs.read(&mut chunk).unwrap();
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                while let Some((Message::BlockCall { req, .. }, used)) = decode(&buf).unwrap() {
                    buf.drain(..used);
                    let size = sizes.next().unwrap_or(6);
                    let text = format!("{size}\n{}", rusty_base64::encode_standard(b"abcd"));
                    theirs
                        .write_all(&encode(&Message::Text { req, text }).unwrap())
                        .unwrap();
                }
            }
        });
        let err = fetch(
            &mut Conn::new(ours),
            BlockId::from_uuid(rusty_uuid::Uuid::nil()),
            "f",
            &mut Vec::new(),
            1,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_reply_past_the_declared_size_is_refused_unwritten() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            let n = theirs.read(&mut chunk).unwrap();
            let Some((Message::BlockCall { req, .. }, _)) = decode(&chunk[..n]).unwrap() else {
                return;
            };
            let text = format!("3\n{}", rusty_base64::encode_standard(b"abcd"));
            theirs
                .write_all(&encode(&Message::Text { req, text }).unwrap())
                .unwrap();
        });
        let mut out = Vec::new();
        let err = fetch(
            &mut Conn::new(ours),
            BlockId::from_uuid(rusty_uuid::Uuid::nil()),
            "f",
            &mut out,
            1,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(out.is_empty());
    }

    #[test]
    fn an_empty_file_is_one_call() {
        assert_eq!(fetch_from(b""), b"");
    }
}
