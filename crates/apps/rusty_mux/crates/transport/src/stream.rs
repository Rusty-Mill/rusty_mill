//! One stream's ordered bytes: what is sent and owed, and what arrived.

use std::collections::{BTreeMap, VecDeque};

use crate::chunk::{MAX_RANGES, Ranges};
use crate::pace::MSS;
use crate::range::RangeSet;

/// The most bytes a stream holds on either side: unread bytes received, or
/// bytes written and not yet acknowledged.
pub const WINDOW: u64 = 1 << 20;

/// What the peer has acknowledged by bytes beyond a hole before the hole
/// counts as lost: three datagrams' worth, as TCP's three duplicate acks.
const REORDER: u64 = 3 * MSS;

/// The sending half: bytes from the first unacknowledged one onward.
#[derive(Debug)]
pub(crate) struct Send {
    /// Bytes from `una` to `una + buf.len()`.
    buf: VecDeque<u8>,
    /// Everything below has been acknowledged.
    una: u64,
    /// Everything below has been sent at least once.
    sent: u64,
    /// Acknowledged above `una`.
    acked: RangeSet,
    /// Sent, not acknowledged, and to be sent again.
    lost: RangeSet,
    /// The peer takes bytes below this.
    limit: u64,
    /// No new loss is declared until `una` reaches this.
    recovery: u64,
}

/// What an acknowledgement changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Acked {
    /// Bytes newly acknowledged.
    pub(crate) delivered: u64,
    /// Bytes newly declared lost.
    pub(crate) lost: u64,
}

impl Default for Send {
    fn default() -> Self {
        Self {
            buf: VecDeque::new(),
            una: 0,
            sent: 0,
            acked: RangeSet::default(),
            lost: RangeSet::default(),
            limit: WINDOW,
            recovery: 0,
        }
    }
}

impl Send {
    /// Takes as much of `bytes` as fits in [`WINDOW`]; returns how much.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> usize {
        let room = (WINDOW as usize).saturating_sub(self.buf.len());
        let n = room.min(bytes.len());
        self.buf.extend(&bytes[..n]);
        n
    }

    /// Bytes written and not yet acknowledged.
    pub(crate) fn queued(&self) -> usize {
        self.buf.len()
    }

    fn end(&self) -> u64 {
        self.una + self.buf.len() as u64
    }

    /// Bytes sent and neither acknowledged nor declared lost.
    pub(crate) fn in_flight(&self) -> u64 {
        self.sent - self.una - self.acked.len() - self.lost.len()
    }

    /// Bytes sent and not acknowledged.
    fn unacked(&self) -> u64 {
        self.sent - self.una - self.acked.len()
    }

    /// Whether new bytes wait only on the peer's window.
    pub(crate) fn blocked(&self) -> bool {
        self.lost.is_empty() && self.sent < self.end() && self.sent >= self.limit
    }

    /// The next bytes to send, at most `max` of them: lost ones first.
    pub(crate) fn next(&mut self, max: usize) -> Option<(u64, Vec<u8>)> {
        let (offset, len) = match self.lost.first() {
            Some((start, end)) => (start, (end - start).min(max as u64)),
            None if self.sent < self.end().min(self.limit) => {
                let end = self.end().min(self.limit);
                (self.sent, (end - self.sent).min(max as u64))
            }
            None => return None,
        };
        if len == 0 {
            return None;
        }
        self.lost.remove(offset, offset + len);
        self.sent = self.sent.max(offset + len);
        let at = (offset - self.una) as usize;
        let bytes = self.buf.range(at..at + len as usize).copied().collect();
        Some((offset, bytes))
    }

    /// Applies the peer's ack. A cumulative offset past what was sent is a
    /// lying or confused peer; it is ignored.
    pub(crate) fn ack(&mut self, cumulative: u64, limit: u64, ranges: &Ranges) -> Acked {
        if cumulative > self.sent {
            return Acked::default();
        }
        let before = self.unacked();
        self.limit = self.limit.max(limit);
        if cumulative > self.una {
            self.buf.drain(..(cumulative - self.una) as usize);
            self.una = cumulative;
            self.acked.remove(0, cumulative);
            self.lost.remove(0, cumulative);
        }
        for &(start, end) in ranges {
            let (start, end) = (start.max(self.una), end.min(self.sent));
            if start < end {
                self.acked.insert(start, end);
                self.lost.remove(start, end);
            }
        }
        let delivered = before - self.unacked();
        Acked {
            delivered,
            lost: self.detect_loss(),
        }
    }

    /// Declares lost the holes below acknowledged bytes once [`REORDER`]
    /// bytes beyond the first hole have been acknowledged.
    fn detect_loss(&mut self) -> u64 {
        if self.una < self.recovery {
            return 0;
        }
        let Some((_, top)) = self.acked.last() else {
            return 0;
        };
        if self.acked.len() < REORDER {
            return 0;
        }
        self.mark_lost(top)
    }

    /// Every byte sent and not acknowledged is lost: the retransmission
    /// timer fired.
    pub(crate) fn timeout(&mut self) -> u64 {
        self.mark_lost(self.sent)
    }

    fn mark_lost(&mut self, below: u64) -> u64 {
        let before = self.lost.len();
        let mut at = self.una;
        for (start, end) in self.acked.iter().chain([(below, below)]) {
            if start > at && at < below {
                self.lost.insert(at, start.min(below));
            }
            at = at.max(end);
        }
        self.recovery = self.sent;
        self.lost.len() - before
    }
}

/// The receiving half: bytes not yet read, and those that came early.
#[derive(Debug, Default)]
pub(crate) struct Recv {
    /// Bytes from `read` to `cumulative`, in order.
    ready: VecDeque<u8>,
    /// The application has read everything below.
    read: u64,
    /// Everything below has arrived.
    cumulative: u64,
    /// Segments beyond `cumulative`, by offset.
    early: BTreeMap<u64, Vec<u8>>,
    /// Bytes held in `early`.
    early_len: u64,
}

impl Recv {
    /// The peer may send bytes below this.
    pub(crate) fn limit(&self) -> u64 {
        self.read + WINDOW
    }

    pub(crate) fn cumulative(&self) -> u64 {
        self.cumulative
    }

    /// Takes in a segment. Bytes at or past the limit are dropped, unacked,
    /// so memory stays within [`WINDOW`].
    pub(crate) fn data(&mut self, offset: u64, bytes: &[u8]) {
        let end = offset + bytes.len() as u64;
        if end > self.limit() || end <= self.cumulative {
            return;
        }
        if offset > self.cumulative {
            self.keep_early(offset, bytes);
            return;
        }
        self.ready
            .extend(&bytes[(self.cumulative - offset) as usize..]);
        self.cumulative = end;
        self.drain_early();
    }

    fn keep_early(&mut self, offset: u64, bytes: &[u8]) {
        if self.early.contains_key(&offset) {
            return;
        }
        if self.early_len + bytes.len() as u64 > WINDOW {
            return;
        }
        self.early_len += bytes.len() as u64;
        self.early.insert(offset, bytes.to_vec());
    }

    fn drain_early(&mut self) {
        while let Some(entry) = self.early.first_entry() {
            let offset = *entry.key();
            if offset > self.cumulative {
                return;
            }
            let bytes = entry.remove();
            self.early_len -= bytes.len() as u64;
            let end = offset + bytes.len() as u64;
            if end > self.cumulative {
                self.ready
                    .extend(&bytes[(self.cumulative - offset) as usize..]);
                self.cumulative = end;
            }
        }
    }

    /// Moves up to `buf.len()` bytes in order into `buf`.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.ready.len());
        for (slot, byte) in buf.iter_mut().zip(self.ready.drain(..n)) {
            *slot = byte;
        }
        self.read += n as u64;
        n
    }

    /// Bytes ready to read.
    pub(crate) fn readable(&self) -> usize {
        self.ready.len()
    }

    /// The lowest few ranges that came early.
    pub(crate) fn ranges(&self) -> Ranges {
        let mut ranges: Ranges = Vec::new();
        for (&start, bytes) in &self.early {
            let end = start + bytes.len() as u64;
            let full = ranges.len() == MAX_RANGES;
            match ranges.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ if full => break,
                _ => ranges.push((start, end)),
            }
        }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(send: &mut Send, bytes: &[u8], segment: usize) -> Vec<(u64, Vec<u8>)> {
        send.write(bytes);
        std::iter::from_fn(|| send.next(segment)).collect()
    }

    #[test]
    fn bytes_go_out_in_segments_and_leave_when_acknowledged() {
        let mut send = Send::default();
        let segments = sent(&mut send, b"abcdefg", 3);
        assert_eq!(
            segments,
            vec![
                (0, b"abc".to_vec()),
                (3, b"def".to_vec()),
                (6, b"g".to_vec())
            ]
        );
        assert_eq!(send.in_flight(), 7);
        let acked = send.ack(3, WINDOW, &vec![]);
        assert_eq!(
            acked,
            Acked {
                delivered: 3,
                lost: 0
            }
        );
        assert_eq!((send.queued(), send.in_flight()), (4, 4));
        assert_eq!(
            send.ack(99, WINDOW, &vec![]),
            Acked::default(),
            "past what was sent"
        );
    }

    #[test]
    fn a_hole_is_resent_once_enough_beyond_it_is_acknowledged() {
        let mut send = Send::default();
        sent(&mut send, &vec![b'x'; 6000], 1000);
        // 1000..2000 went missing; 2000..3000 beyond it is not enough.
        assert_eq!(
            send.ack(1000, WINDOW, &vec![(2000, 3000)]),
            Acked {
                delivered: 2000,
                lost: 0
            }
        );
        let acked = send.ack(1000, WINDOW, &vec![(2000, 6000)]);
        assert_eq!(
            acked,
            Acked {
                delivered: 3000,
                lost: 1000
            }
        );
        assert_eq!(send.in_flight(), 0);
        assert_eq!(send.next(1000), Some((1000, vec![b'x'; 1000])));
        assert_eq!(send.next(1000), None);
        // A repeat of the same ack does not declare it lost again.
        assert_eq!(send.ack(1000, WINDOW, &vec![(2000, 6000)]).lost, 0);
        assert_eq!(send.ack(6000, WINDOW, &vec![]).delivered, 1000);
        assert_eq!(send.queued(), 0);
    }

    #[test]
    fn a_timeout_resends_everything_unacknowledged_and_nothing_acknowledged() {
        let mut send = Send::default();
        sent(&mut send, b"abcdefghij", 2);
        send.ack(2, WINDOW, &vec![(4, 6)]);
        assert_eq!(send.timeout(), 6);
        let again: Vec<_> = std::iter::from_fn(|| send.next(10)).collect();
        assert_eq!(again, vec![(2, b"cd".to_vec()), (6, b"ghij".to_vec())]);
    }

    #[test]
    fn the_peer_window_holds_new_bytes_back() {
        let mut send = Send {
            limit: 4,
            ..Send::default()
        };
        let segments = sent(&mut send, b"abcdef", 10);
        assert_eq!(segments, vec![(0, b"abcd".to_vec())]);
        send.ack(4, 4, &vec![]);
        assert!(send.blocked());
        assert_eq!(send.next(10), None);
        send.ack(4, 8, &vec![]);
        assert_eq!(send.next(10), Some((4, b"ef".to_vec())));
    }

    #[test]
    fn a_writer_stops_at_the_window() {
        let mut send = Send::default();
        assert_eq!(send.write(&vec![0; WINDOW as usize + 10]), WINDOW as usize);
        assert_eq!(send.write(b"x"), 0);
    }

    #[test]
    fn early_bytes_wait_for_the_hole_then_read_in_order() {
        let mut recv = Recv::default();
        recv.data(3, b"def");
        recv.data(8, b"ij");
        recv.data(3, b"def");
        assert_eq!((recv.cumulative(), recv.readable()), (0, 0));
        assert_eq!(recv.ranges(), vec![(3, 6), (8, 10)]);
        recv.data(0, b"abcd");
        assert_eq!(recv.cumulative(), 6);
        recv.data(5, b"fgh");
        assert_eq!(recv.cumulative(), 10);
        let mut buf = [0; 16];
        let n = recv.read(&mut buf);
        assert_eq!(&buf[..n], b"abcdefghij");
        assert!(recv.ranges().is_empty());
        assert_eq!(recv.limit(), 10 + WINDOW);
    }

    #[test]
    fn a_reader_that_stops_closes_the_window_without_losing_bytes() {
        let mut recv = Recv::default();
        let chunk = vec![1; 1000];
        let mut offset = 0;
        while offset + 1000 <= WINDOW {
            recv.data(offset, &chunk);
            offset += 1000;
        }
        recv.data(offset, &chunk);
        assert_eq!(
            recv.cumulative(),
            offset,
            "past the limit: dropped, unacked"
        );
        assert_eq!(recv.limit(), WINDOW);
        let mut buf = vec![0; 5000];
        assert_eq!(recv.read(&mut buf), 5000);
        assert_eq!(recv.limit(), WINDOW + 5000);
        recv.data(offset, &chunk);
        assert_eq!(recv.cumulative(), offset + 1000);
    }

    #[test]
    fn at_most_four_early_ranges_are_reported() {
        let mut recv = Recv::default();
        for i in 1..8 {
            recv.data(i * 10, b"x");
        }
        assert_eq!(recv.ranges(), vec![(10, 11), (20, 21), (30, 31), (40, 41)]);
    }
}
