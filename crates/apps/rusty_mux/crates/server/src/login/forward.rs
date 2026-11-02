//! The preface the login front door writes on each connection it relays to
//! an account's server: the principal it verified, before any protocol
//! byte. An account server reads it only from the peer it was told runs
//! the front door (`--forwarded-by`), so no client can claim a principal.
//!
//! Layout: the 8-byte magic, then the principal's `id` and `user`, each a
//! big-endian `u16` length and that many UTF-8 bytes, each at most
//! [`MAX_NAME`] bytes. Reads are exact, so the first protocol byte after
//! it stays unread.

use std::io;

use rusty_mux_core::{MAX_NAME, Principal};
use rusty_tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAGIC: &[u8; 8] = b"RMXFWD1\0";

/// The preface for `principal`.
pub(crate) fn encode(principal: &Principal) -> io::Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    for field in [principal.id(), principal.user()] {
        let len = u16::try_from(field.len())
            .ok()
            .filter(|&len| usize::from(len) <= MAX_NAME)
            .ok_or_else(|| bad("a principal field is too long"))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    Ok(out)
}

/// Writes the preface for `principal` to `wr`.
pub(crate) async fn write(
    wr: &mut (impl AsyncWrite + Unpin + Send),
    principal: &Principal,
) -> io::Result<()> {
    wr.write_all(&encode(principal)?).await?;
    wr.flush().await
}

/// Reads one preface from `rd`, leaving every byte after it unread.
pub(crate) async fn read(rd: &mut (impl AsyncRead + Unpin + Send)) -> io::Result<Principal> {
    let mut magic = [0u8; 8];
    rd.read_exact(&mut magic).await?;
    if &magic != MAGIC {
        return Err(bad("not a forwarded connection"));
    }
    let id = field(rd).await?;
    let user = field(rd).await?;
    Principal::new(id, user).map_err(|e| bad(&e.to_string()))
}

async fn field(rd: &mut (impl AsyncRead + Unpin + Send)) -> io::Result<String> {
    let mut len = [0u8; 2];
    rd.read_exact(&mut len).await?;
    let len = usize::from(u16::from_be_bytes(len));
    if len > MAX_NAME {
        return Err(bad("a principal field is too long"));
    }
    let mut bytes = vec![0u8; len];
    rd.read_exact(&mut bytes).await?;
    String::from_utf8(bytes).map_err(|_| bad("a principal field is not UTF-8"))
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(bytes: &[u8]) -> io::Result<(Principal, Vec<u8>)> {
        let runtime = rusty_tokio::Builder::new_current_thread()
            .build_local(&rusty_tokio::LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async {
            let mut rd = bytes;
            let principal = read(&mut rd).await?;
            Ok((principal, rd.to_vec()))
        })
    }

    #[test]
    fn a_preface_round_trips_and_leaves_the_rest_unread() {
        let alice = Principal::new("uid:1000", "alice").unwrap();
        let mut bytes = encode(&alice).unwrap();
        bytes.extend_from_slice(b"hello");
        let (got, rest) = read_all(&bytes).unwrap();
        assert_eq!(got, alice);
        assert_eq!(rest, b"hello");
    }

    #[test]
    fn anything_else_is_refused() {
        let alice = Principal::new("uid:1000", "alice").unwrap();
        let good = encode(&alice).unwrap();
        // Cut anywhere.
        for cut in 0..good.len() {
            assert!(read_all(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut wrong = good.clone();
        wrong[0] = b'X';
        assert!(read_all(&wrong).is_err());
        // An empty id is no principal.
        let mut empty = MAGIC.to_vec();
        empty.extend_from_slice(&[0, 0, 0, 0]);
        assert!(read_all(&empty).is_err());
        // Over-long and non-UTF-8 fields.
        let mut long = MAGIC.to_vec();
        long.extend_from_slice(&u16::MAX.to_be_bytes());
        assert!(read_all(&long).is_err());
        let mut latin = MAGIC.to_vec();
        latin.extend_from_slice(&[0, 1, 0xff, 0, 0]);
        assert!(read_all(&latin).is_err());
    }
}
