//! Echo acks: which keys written to a block's PTY have had their grace
//! covered by a drain. Plain data; the block's reader supplies the clock.

use std::collections::VecDeque;
use std::time::Instant;

use rusty_mux_core::ClientId;
use rusty_mux_proto::ECHO_GRACE;

/// One block's keys whose grace no drain has covered yet, oldest first.
/// Keys are recorded as their writes return, so their deadlines only grow.
#[derive(Debug, Default)]
pub(crate) struct Waiting(VecDeque<Key>);

#[derive(Debug)]
struct Key {
    client: ClientId,
    req: u64,
    /// The write's return plus [`ECHO_GRACE`].
    due: Instant,
}

impl Waiting {
    /// Records `client`'s input `req`, whose write returned at `at`.
    pub(crate) fn written(&mut self, client: ClientId, req: u64, at: Instant) {
        self.0.push_back(Key {
            client,
            req,
            due: at + ECHO_GRACE,
        });
    }

    /// When the oldest key's grace ends.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.0.front().map(|key| key.due)
    }

    /// A read at `at` found the PTY empty, so every byte that reached it
    /// before then has been read. Removes the keys whose grace ended by
    /// then and returns, per client, the newest of them, in the order the
    /// clients' first keys were written.
    pub(crate) fn drained(&mut self, at: Instant) -> Vec<(ClientId, u64)> {
        let mut acks: Vec<(ClientId, u64)> = Vec::new();
        while self.0.front().is_some_and(|key| key.due <= at) {
            let Some(key) = self.0.pop_front() else {
                break;
            };
            match acks.iter_mut().find(|(client, _)| *client == key.client) {
                Some((_, req)) => *req = key.req,
                None => acks.push((key.client, key.req)),
            }
        }
        acks
    }

    /// Drops `client`'s keys: its connection ended or was taken over, and a
    /// new connection numbers its requests afresh.
    pub(crate) fn forget(&mut self, client: ClientId) {
        self.0.retain(|key| key.client != client);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusty_uuid::Uuid;

    use super::*;

    fn client() -> ClientId {
        ClientId::from_uuid(Uuid::new_v4())
    }

    #[test]
    fn a_drain_before_the_grace_ends_acks_nothing() {
        let (mut waiting, a, t) = (Waiting::default(), client(), Instant::now());
        waiting.written(a, 1, t);
        assert_eq!(waiting.due(), Some(t + ECHO_GRACE));
        assert!(
            waiting
                .drained(t + ECHO_GRACE - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(waiting.drained(t + ECHO_GRACE), vec![(a, 1)]);
        assert_eq!(waiting.due(), None);
    }

    #[test]
    fn a_drain_acks_the_newest_covered_key_of_each_client_and_keeps_the_rest() {
        let (mut waiting, a, b, t) = (Waiting::default(), client(), client(), Instant::now());
        let ms = Duration::from_millis;
        waiting.written(a, 1, t);
        waiting.written(b, 7, t + ms(1));
        waiting.written(a, 2, t + ms(2));
        waiting.written(a, 3, t + ms(30));
        assert_eq!(
            waiting.drained(t + ECHO_GRACE + ms(2)),
            vec![(a, 2), (b, 7)]
        );
        assert_eq!(waiting.due(), Some(t + ms(30) + ECHO_GRACE));
        assert_eq!(waiting.drained(t + ECHO_GRACE + ms(30)), vec![(a, 3)]);
    }

    #[test]
    fn a_forgotten_client_is_never_acked() {
        let (mut waiting, a, b, t) = (Waiting::default(), client(), client(), Instant::now());
        waiting.written(a, 1, t);
        waiting.written(b, 1, t);
        waiting.forget(a);
        assert_eq!(waiting.drained(t + ECHO_GRACE), vec![(b, 1)]);
    }
}
