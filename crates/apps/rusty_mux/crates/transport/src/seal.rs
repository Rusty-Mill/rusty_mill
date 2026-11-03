//! Sealing and opening datagrams: `id | seq | ciphertext | tag`.

use rusty_aead::{ChaCha20Poly1305, KEY_LEN, NONCE_LEN, TAG_LEN};

/// Bytes naming an association, sent in the clear.
pub const ID_LEN: usize = 8;
/// Bytes before the ciphertext: the id and the sequence number.
pub(crate) const HEADER_LEN: usize = ID_LEN + 8;
/// Bytes a datagram adds to its plaintext.
pub(crate) const OVERHEAD: usize = HEADER_LEN + TAG_LEN;
/// The top bit of a sequence number: set on what the server sends.
const FROM_SERVER: u64 = 1 << 63;
/// Sequence numbers below the highest seen that are still accepted.
const WINDOW: u64 = 1024;

/// Which end of the association this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Opens streams and sends with the direction bit clear.
    Client,
    /// Accepts streams and sends with the direction bit set.
    Server,
}

impl Side {
    fn outgoing(self) -> u64 {
        match self {
            Side::Client => 0,
            Side::Server => FROM_SERVER,
        }
    }

    fn incoming(self) -> u64 {
        FROM_SERVER - self.outgoing()
    }
}

/// One end's key, id and sequence state.
pub(crate) struct Sealer {
    cipher: ChaCha20Poly1305,
    id: [u8; ID_LEN],
    side: Side,
    next: u64,
    replay: Replay,
}

impl Sealer {
    pub(crate) fn new(side: Side, id: [u8; ID_LEN], key: &[u8; KEY_LEN]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(key),
            id,
            side,
            next: 0,
            replay: Replay::default(),
        }
    }

    /// Seals `plain` under the next sequence number, or `None` once the
    /// sequence space is spent (2^63 datagrams).
    pub(crate) fn seal(&mut self, plain: &[u8]) -> Option<Vec<u8>> {
        if self.next & FROM_SERVER != 0 {
            return None;
        }
        let seq = self.next | self.side.outgoing();
        self.next += 1;
        let mut out = Vec::with_capacity(plain.len() + OVERHEAD);
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(plain);
        let (header, body) = out.split_at_mut(HEADER_LEN);
        let tag = self.cipher.seal_in_place(&nonce(seq), header, body).ok()?;
        out.extend_from_slice(&tag);
        Some(out)
    }

    /// Checks direction, id, tag and replay window, in that order, and only
    /// then records the sequence number. Returns it with the plaintext.
    pub(crate) fn open(&mut self, datagram: &[u8]) -> Option<(u64, Vec<u8>)> {
        if datagram.len() < OVERHEAD {
            return None;
        }
        let seq = u64::from_be_bytes(datagram[ID_LEN..HEADER_LEN].try_into().ok()?);
        if seq & FROM_SERVER != self.side.incoming() || datagram[..ID_LEN] != self.id {
            return None;
        }
        let count = seq & !FROM_SERVER;
        if !self.replay.fresh(count) {
            return None;
        }
        let (header, rest) = datagram.split_at(HEADER_LEN);
        let (body, tag) = rest.split_at(rest.len() - TAG_LEN);
        let mut plain = body.to_vec();
        let nonce = nonce(seq);
        self.cipher
            .open_in_place(&nonce, header, &mut plain, tag)
            .ok()?;
        self.replay.record(count);
        Some((count, plain))
    }
}

/// Four zero bytes, then the sequence number with its direction bit.
fn nonce(seq: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0; NONCE_LEN];
    nonce[4..].copy_from_slice(&seq.to_be_bytes());
    nonce
}

/// The sequence numbers seen within [`WINDOW`] of the highest.
#[derive(Default)]
struct Replay {
    /// One more than the highest seen; 0 before any.
    top: u64,
    /// Bit `i` is set when `top - 1 - i` has been seen.
    seen: [u64; (WINDOW / 64) as usize],
}

impl Replay {
    fn fresh(&self, seq: u64) -> bool {
        if seq >= self.top {
            return true;
        }
        let back = self.top - 1 - seq;
        back < WINDOW && !self.bit(back)
    }

    fn record(&mut self, seq: u64) {
        if seq >= self.top {
            self.shift(seq + 1 - self.top);
            self.top = seq + 1;
        }
        let back = self.top - 1 - seq;
        self.seen[(back / 64) as usize] |= 1 << (back % 64);
    }

    fn bit(&self, back: u64) -> bool {
        self.seen[(back / 64) as usize] & (1 << (back % 64)) != 0
    }

    /// Ages every bit by `by` places, dropping those past the window.
    fn shift(&mut self, by: u64) {
        if by >= WINDOW {
            self.seen = Default::default();
            return;
        }
        let (words, bits) = ((by / 64) as usize, by % 64);
        let old = self.seen;
        for (i, word) in self.seen.iter_mut().enumerate() {
            let lo = i.checked_sub(words).map_or(0, |j| old[j] << bits);
            let carry = match (i.checked_sub(words + 1), bits) {
                (Some(j), 1..) => old[j] >> (64 - bits),
                _ => 0,
            };
            *word = lo | carry;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];
    const ID: [u8; ID_LEN] = [1; ID_LEN];

    fn pair() -> (Sealer, Sealer) {
        (
            Sealer::new(Side::Client, ID, &KEY),
            Sealer::new(Side::Server, ID, &KEY),
        )
    }

    #[test]
    fn a_sealed_datagram_opens_once_on_the_other_side() {
        let (mut client, mut server) = pair();
        let datagram = client.seal(b"hello").unwrap();
        assert_eq!(datagram.len(), 5 + OVERHEAD);
        assert_eq!(server.open(&datagram), Some((0, b"hello".to_vec())));
        assert_eq!(server.open(&datagram), None, "replayed");
    }

    #[test]
    fn a_datagram_reflected_to_its_sender_is_refused() {
        let (mut client, mut server) = pair();
        let from_client = client.seal(b"up").unwrap();
        let from_server = server.seal(b"down").unwrap();
        assert_eq!(client.open(&from_client), None);
        assert_eq!(server.open(&from_server), None);
        // Refusing it left the window alone: the real one still opens.
        assert!(server.open(&from_client).is_some());
        assert!(client.open(&from_server).is_some());
    }

    #[test]
    fn a_wrong_id_tag_or_key_is_refused_without_touching_the_window() {
        let (mut client, mut server) = pair();
        let good = client.seal(b"x").unwrap();
        let mut forged = good.clone();
        *forged.last_mut().unwrap() ^= 1;
        let mut renamed = good.clone();
        renamed[0] ^= 1;
        let mut stranger = Sealer::new(Side::Client, ID, &[8; KEY_LEN]);
        for bad in [forged, renamed, stranger.seal(b"x").unwrap(), vec![0; 10]] {
            assert_eq!(server.open(&bad), None);
        }
        assert!(server.open(&good).is_some());
    }

    #[test]
    fn late_datagrams_open_within_the_window_and_not_beyond() {
        let (mut client, mut server) = pair();
        let all: Vec<_> = (0..WINDOW + 2).map(|_| client.seal(b"").unwrap()).collect();
        assert!(server.open(&all[WINDOW as usize]).is_some());
        assert!(server.open(&all[1]).is_some(), "1023 back");
        assert!(server.open(&all[0]).is_none(), "1024 back");
        assert!(server.open(&all[WINDOW as usize + 1]).is_some());
        assert!(server.open(&all[1]).is_none(), "now 1024 back");
        assert!(server.open(&all[2]).is_some(), "1023 back, unseen");
    }

    #[test]
    fn the_replay_window_shifts_across_words() {
        let mut replay = Replay::default();
        for seq in [0, 63, 64, 130] {
            replay.record(seq);
        }
        for seq in [0, 63, 64, 130] {
            assert!(!replay.fresh(seq), "{seq}");
        }
        for seq in [1, 62, 65, 129] {
            assert!(replay.fresh(seq), "{seq}");
        }
        replay.record(130 + WINDOW);
        assert!(!replay.fresh(130), "aged out");
        assert!(replay.fresh(131));
    }
}
