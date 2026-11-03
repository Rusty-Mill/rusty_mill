//! The numbered event stream and the recent events a reconnecting client
//! resumes from.

use std::collections::VecDeque;

use rusty_mux_core::Event;
use rusty_mux_proto::Cursor;
use rusty_uuid::Uuid;

/// Numbers this server run's events from 1 and keeps the newest `cap`.
pub(crate) struct History {
    epoch: Uuid,
    seq: u64,
    recent: VecDeque<(u64, Event)>,
    cap: usize,
}

impl History {
    pub(crate) fn new(epoch: Uuid, cap: usize) -> Self {
        Self {
            epoch,
            seq: 0,
            recent: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// The position after the newest event.
    pub(crate) fn cursor(&self) -> Cursor {
        Cursor {
            epoch: self.epoch,
            seq: self.seq,
        }
    }

    /// Numbers `event`, keeps it, and returns its number.
    pub(crate) fn push(&mut self, event: Event) -> u64 {
        self.seq += 1;
        if self.cap == 0 {
            return self.seq;
        }
        if self.recent.len() == self.cap {
            self.recent.pop_front();
        }
        self.recent.push_back((self.seq, event));
        self.seq
    }

    /// Every event after `from`, oldest first, or `None` when `from` is from
    /// another run, ahead of this one, or older than the events kept.
    pub(crate) fn since(&self, from: Cursor) -> Option<impl Iterator<Item = &(u64, Event)>> {
        let missing = self.seq.checked_sub(from.seq)?;
        if from.epoch != self.epoch || missing > self.recent.len() as u64 {
            return None;
        }
        let skip = self.recent.len() - missing as usize;
        Some(self.recent.iter().skip(skip))
    }
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::ClientId;

    use super::*;

    fn event(n: u8) -> Event {
        Event::ClientConnected {
            client: ClientId::from_uuid(Uuid::from_bytes([n; 16])),
            principal: None,
        }
    }

    fn history(cap: usize, events: u8) -> History {
        let mut history = History::new(Uuid::from_bytes([1; 16]), cap);
        for n in 1..=events {
            assert_eq!(history.push(event(n)), u64::from(n));
        }
        history
    }

    fn at(history: &History, seq: u64) -> Cursor {
        Cursor {
            seq,
            ..history.cursor()
        }
    }

    fn seqs(history: &History, from: Cursor) -> Option<Vec<u64>> {
        history.since(from).map(|it| it.map(|(s, _)| *s).collect())
    }

    #[test]
    fn numbers_from_one() {
        let history = history(4, 0);
        assert_eq!(history.cursor().seq, 0);
        assert_eq!(seqs(&history, history.cursor()), Some(vec![]));
    }

    #[test]
    fn replays_exactly_the_events_after_the_cursor() {
        let history = history(4, 6);
        assert_eq!(seqs(&history, at(&history, 6)), Some(vec![]));
        assert_eq!(seqs(&history, at(&history, 4)), Some(vec![5, 6]));
        assert_eq!(seqs(&history, at(&history, 2)), Some(vec![3, 4, 5, 6]));
        let (seq, kept) = history.since(at(&history, 5)).unwrap().next().unwrap();
        assert_eq!((*seq, kept), (6, &event(6)));
    }

    #[test]
    fn refuses_what_it_cannot_replay_whole() {
        let history = history(4, 6);
        assert_eq!(seqs(&history, at(&history, 1)), None, "event 2 is gone");
        assert_eq!(seqs(&history, at(&history, 7)), None, "ahead of the run");
        let other_run = Cursor {
            epoch: Uuid::from_bytes([2; 16]),
            seq: 6,
        };
        assert_eq!(seqs(&history, other_run), None);
    }

    #[test]
    fn keeps_nothing_with_no_capacity() {
        let history = history(0, 3);
        assert_eq!(history.cursor().seq, 3);
        assert_eq!(seqs(&history, at(&history, 3)), Some(vec![]));
        assert_eq!(seqs(&history, at(&history, 2)), None);
    }
}
