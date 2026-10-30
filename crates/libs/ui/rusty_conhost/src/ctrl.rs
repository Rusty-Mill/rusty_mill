//! Ctrl events (design §5.5, §7.2, §7.4): which clients get one, and the
//! bounded, coalescing queue the ctrl worker drains, so delivery never
//! runs on or blocks the engine thread.
//!
//! Recipients follow microsoft/terminal's conhost (commit 09beddc6,
//! `src/host/input.cpp`, `src/server/ProcessList.cpp`,
//! `ApiDispatchersInternal.cpp`; see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use std::collections::VecDeque;

/// Entries the queue holds; a new one past it is dropped.
pub(crate) const MAX_QUEUED: usize = 256;

/// The events this release sends; CTRL_CLOSE comes with PR 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Ctrl {
    C,
    Break,
}

impl Ctrl {
    /// `GenerateConsoleCtrlEvent`'s event number: CTRL_C_EVENT is 0,
    /// CTRL_BREAK_EVENT 1.
    pub(crate) fn from_event(n: u32) -> Option<Ctrl> {
        match n {
            0 => Some(Ctrl::C),
            1 => Some(Ctrl::Break),
            _ => None,
        }
    }
}

/// What a push did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pushed {
    Queued,
    /// An equal entry was already waiting.
    Coalesced,
    /// The queue was full.
    Dropped,
}

/// Pending `(pid, event)` deliveries in order, without duplicates.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    entries: VecDeque<(u32, Ctrl)>,
}

impl Queue {
    pub(crate) fn push(&mut self, pid: u32, event: Ctrl) -> Pushed {
        if self.entries.contains(&(pid, event)) {
            return Pushed::Coalesced;
        }
        if self.entries.len() >= MAX_QUEUED {
            return Pushed::Dropped;
        }
        self.entries.push_back((pid, event));
        Pushed::Queued
    }

    pub(crate) fn pop(&mut self) -> Option<(u32, Ctrl)> {
        self.entries.pop_front()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The pids an event goes to: every client for group 0, else the clients
/// whose CONNECT `ProcessGroupId` is `group`. Each pid once, in order.
pub(crate) fn recipients(clients: impl Iterator<Item = (u32, u32)>, group: u32) -> Vec<u32> {
    let mut pids = Vec::new();
    for (pid, g) in clients {
        if (group == 0 || g == group) && !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_coalesce_and_the_queue_is_bounded() {
        let mut q = Queue::default();
        assert_eq!(q.push(7, Ctrl::C), Pushed::Queued);
        assert_eq!(q.push(7, Ctrl::C), Pushed::Coalesced);
        assert_eq!(q.push(7, Ctrl::Break), Pushed::Queued);
        for pid in 0..10_000 {
            q.push(pid + 100, Ctrl::C);
        }
        assert_eq!(q.len(), MAX_QUEUED);
        assert_eq!(q.pop(), Some((7, Ctrl::C)));
        assert_eq!(q.pop(), Some((7, Ctrl::Break)));
    }

    #[test]
    fn recipients_are_the_group_or_everyone() {
        let clients = [(10, 10), (11, 10), (12, 12), (10, 10)];
        assert_eq!(recipients(clients.into_iter(), 0), [10, 11, 12]);
        assert_eq!(recipients(clients.into_iter(), 10), [10, 11]);
        assert!(recipients(clients.into_iter(), 99).is_empty());
    }

    #[test]
    fn only_c_and_break_are_events_here() {
        assert_eq!(Ctrl::from_event(0), Some(Ctrl::C));
        assert_eq!(Ctrl::from_event(1), Some(Ctrl::Break));
        assert_eq!(Ctrl::from_event(2), None);
    }
}
