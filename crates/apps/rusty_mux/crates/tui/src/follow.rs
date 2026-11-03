//! Keeping the attach client's focus in step with the server's, which
//! every client of a window shares.
//!
//! The client moves its own focus at once, so keys never wait on a round
//! trip, and tells the server. The server announces each focus change,
//! this client's own included, right after answering the request that made
//! it. While a request of this client's is unanswered, an announcement may
//! be stale (an echo of an earlier move) so it is only noted. Once the last
//! request is answered:
//!
//! - done, the server's focus is the block asked for, which the client has
//!   already: what was noted is older, and dropped. The request's own
//!   announcement follows, and is taken like any other.
//! - refused, the server's focus is the last one noted, which is taken.
//!
//! Events and answers come in order on one connection, so nothing is
//! missed. Requests on a connection given up are never answered: the
//! client starts afresh with each new one.

use rusty_mux_core::BlockId;

/// Focus requests in flight and the latest announcement meanwhile.
#[derive(Debug, Default)]
pub struct Follow {
    asked: Vec<u64>,
    /// The last focus announced while requests were in flight.
    noted: Option<Option<BlockId>>,
}

impl Follow {
    /// This client asked the server to focus a block, with request `req`.
    pub fn asked(&mut self, req: u64) {
        self.asked.push(req);
    }

    /// Whether `req` is one of this client's focus requests in flight.
    pub fn owns(&self, req: u64) -> bool {
        self.asked.contains(&req)
    }

    /// The server announced its focus: `Some` with the focus to take now,
    /// `None` while requests are in flight.
    pub fn told(&mut self, focus: Option<BlockId>) -> Option<Option<BlockId>> {
        if self.asked.is_empty() {
            return Some(focus);
        }
        self.noted = Some(focus);
        None
    }

    /// Request `req` was done. Nothing to take: the last one done leaves
    /// the focus where this client already put it.
    pub fn done(&mut self, req: u64) {
        if self.settled(req) {
            self.noted = None;
        }
    }

    /// Request `req` was refused: once none is left in flight, the focus
    /// announced meanwhile, if any.
    pub fn refused(&mut self, req: u64) -> Option<Option<BlockId>> {
        match self.settled(req) {
            true => self.noted.take(),
            false => None,
        }
    }

    /// Drops `req`; whether none is left in flight.
    fn settled(&mut self, req: u64) -> bool {
        self.asked.retain(|r| *r != req);
        self.asked.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use rusty_uuid::Uuid;

    use super::*;

    fn block(n: u128) -> BlockId {
        BlockId::from_uuid(Uuid::from_bytes(n.to_be_bytes()))
    }

    #[test]
    fn with_nothing_asked_an_announcement_is_taken_at_once() {
        let mut follow = Follow::default();
        assert_eq!(follow.told(Some(block(1))), Some(Some(block(1))));
        assert_eq!(follow.told(None), Some(None));
    }

    /// The server's order: each answer, then its announcement.
    #[test]
    fn echoes_of_moves_in_flight_never_pull_back() {
        let mut follow = Follow::default();
        // Moved to 2 then 3 quickly; the echo of 2 must not pull back.
        follow.asked(10);
        follow.asked(11);
        follow.done(10);
        assert_eq!(follow.told(Some(block(2))), None);
        follow.done(11);
        assert!(!follow.owns(11));
        assert_eq!(follow.told(Some(block(3))), Some(Some(block(3))));
    }

    #[test]
    fn another_clients_later_move_wins() {
        let mut follow = Follow::default();
        follow.asked(10);
        follow.done(10);
        assert_eq!(follow.told(Some(block(2))), Some(Some(block(2)))); // Our echo.
        assert_eq!(follow.told(Some(block(5))), Some(Some(block(5)))); // Someone else's.
    }

    #[test]
    fn a_move_answered_before_another_clients_is_overtaken_by_it() {
        let mut follow = Follow::default();
        follow.asked(10);
        follow.asked(11);
        follow.done(10);
        assert_eq!(follow.told(Some(block(2))), None); // Our first echo.
        assert_eq!(follow.told(Some(block(5))), None); // Someone else's.
        follow.done(11); // Ours again, after theirs: ours stands.
        assert_eq!(follow.told(Some(block(3))), Some(Some(block(3))));
    }

    #[test]
    fn a_refused_last_request_takes_what_was_announced_meanwhile() {
        let mut follow = Follow::default();
        follow.asked(10);
        follow.asked(11);
        follow.done(10);
        assert_eq!(follow.told(Some(block(2))), None);
        assert_eq!(follow.refused(11), Some(Some(block(2))));
    }

    #[test]
    fn a_refused_request_with_no_announcement_changes_nothing() {
        let mut follow = Follow::default();
        follow.asked(10);
        assert!(follow.owns(10));
        assert_eq!(follow.refused(10), None);
        assert_eq!(follow.refused(99), None);
    }
}
