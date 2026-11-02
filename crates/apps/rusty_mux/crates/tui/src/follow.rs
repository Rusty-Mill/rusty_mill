//! Keeping the attach client's focus in step with the server's, which
//! every client of a window shares.
//!
//! The client moves its own focus at once, so keys never wait on a round
//! trip, and tells the server. The server announces each focus change,
//! this client's own included. While a request of this client's is
//! unanswered, an announcement may be stale (an echo of an earlier move)
//! so it is only noted; once every request is answered, the last one
//! noted is where the server's focus is, and the client takes it. Events
//! and answers come in order on one connection, so nothing is missed.

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

    /// Request `req` was answered, done or failed: once none is left in
    /// flight, the focus announced meanwhile, if any.
    pub fn answered(&mut self, req: u64) -> Option<Option<BlockId>> {
        self.asked.retain(|r| *r != req);
        match self.asked.is_empty() {
            true => self.noted.take(),
            false => None,
        }
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

    #[test]
    fn echoes_of_moves_in_flight_wait_for_the_last_answer() {
        let mut follow = Follow::default();
        // Moved to 2 then 3 quickly; the echo of 2 must not pull back.
        follow.asked(10);
        follow.asked(11);
        assert_eq!(follow.told(Some(block(2))), None);
        assert_eq!(follow.answered(10), None);
        assert_eq!(follow.told(Some(block(3))), None);
        assert_eq!(follow.answered(11), Some(Some(block(3))));
        assert!(!follow.owns(11));
    }

    #[test]
    fn another_clients_later_move_wins_once_answered() {
        let mut follow = Follow::default();
        follow.asked(10);
        assert_eq!(follow.told(Some(block(2))), None); // Our echo.
        assert_eq!(follow.told(Some(block(5))), None); // Someone else's.
        assert_eq!(follow.answered(10), Some(Some(block(5))));
    }

    #[test]
    fn a_refused_request_with_no_announcement_changes_nothing() {
        let mut follow = Follow::default();
        follow.asked(10);
        assert!(follow.owns(10));
        assert_eq!(follow.answered(10), None);
        assert_eq!(follow.answered(99), None);
    }
}
