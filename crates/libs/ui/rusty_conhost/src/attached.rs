//! The processes attached to a console, newest first, as
//! `GetConsoleProcessList` orders them: one entry per process, placed by
//! its first connection. Fed by [`crate::console::Effect::Joined`] and
//! `Left`; read by the terminal side for its foreground process (C31).

/// Attached pids, newest first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Clients(Vec<u32>);

impl Clients {
    /// A connection from `pid`. A process that already has one keeps its
    /// place: only its first connection orders it.
    pub(crate) fn join(&mut self, pid: u32) {
        if !self.0.contains(&pid) {
            self.0.insert(0, pid);
        }
    }

    /// `pid`'s last connection closed.
    pub(crate) fn leave(&mut self, pid: u32) {
        self.0.retain(|&p| p != pid);
    }

    /// Nothing is attached any more: the console has closed.
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    /// The attached pids, newest first.
    pub(crate) fn newest_first(&self) -> &[u32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_process_comes_first_and_keeps_its_first_place() {
        let mut a = Clients::default();
        a.join(10);
        a.join(20);
        a.join(30);
        a.join(10);
        assert_eq!(
            a.newest_first(),
            [30, 20, 10],
            "a second connection moves nothing"
        );
        a.leave(30);
        assert_eq!(
            a.newest_first(),
            [20, 10],
            "the job ended: its parent is newest"
        );
        a.leave(99);
        assert_eq!(a.newest_first(), [20, 10], "an unknown pid changes nothing");
        a.clear();
        assert!(a.newest_first().is_empty());
    }
}
