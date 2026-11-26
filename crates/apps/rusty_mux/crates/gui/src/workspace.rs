//! The explorer panel's tree, kept on the source side (gui-design D99, card
//! #1573): the calls on the block's own connection, the shared
//! [`Explorer`] model, and the flat [`Snapshot`] the window draws.
//!
//! `rusty_term` cannot link this crate's client, so it holds view state
//! only; every key it takes arrives here as an [`Intent`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rusty_mux_client::workspace::{self, Call, Explorer, Kind, Tree, Want};
use rusty_mux_proto::schema::Value;
use rusty_term::gui::explorer::{Intent, Row, Snapshot, State};
use rusty_term::gui::source::ListDone;

/// Where the tree's calls go and where its snapshots land.
pub trait Port: Send + Sync {
    /// Sends `method` on the block's connection; `done` gets the reply
    /// text, or why there is none. `false` when it could not be sent.
    fn call(&self, method: &str, args: Vec<(String, Value)>, done: ListDone) -> bool;
    /// Shows `snapshot`.
    fn show(&self, snapshot: Snapshot);
    /// Seconds since the epoch, for ages.
    fn now(&self) -> u64;
}

/// One open tree.
pub struct Workspace {
    port: Arc<dyn Port>,
    inner: Mutex<Inner>,
    /// Cleared by [`Self::close`]: a reply or key for a closed or replaced
    /// tree does nothing, so it cannot show over its replacement.
    live: AtomicBool,
}

struct Inner {
    explorer: Explorer,
    /// This client holds the block's input lease, which every call needs.
    held: bool,
    /// A marks or stat call failed: read them again when the lease is
    /// gained, as it may have been cut off with the connection or lease.
    retry: bool,
}

/// Which call a reply answers.
#[derive(Clone, Copy)]
enum Which {
    List,
    Touched,
    Stat,
}

impl Workspace {
    /// Opens the tree: asks for the workspace root. `status`: the server
    /// answers the marks and stat calls. `held`: this client holds the
    /// block's input lease, which every call needs.
    pub fn open(port: Arc<dyn Port>, status: bool, held: bool) -> Arc<Self> {
        let this = Arc::new(Self {
            port,
            inner: Mutex::new(Inner {
                explorer: Explorer::opened(status),
                held,
                retry: false,
            }),
            live: AtomicBool::new(true),
        });
        if held {
            this.ask(Which::List, "terminal.workspace-list", Vec::new());
        } else {
            // Nothing is on its way until the lease comes.
            this.lock().explorer.set_waiting(false);
        }
        this.show();
        this
    }

    /// Ends this tree: nothing it has in flight shows, and it asks no more.
    pub fn close(&self) {
        self.live.store(false, Ordering::SeqCst);
    }

    fn live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    /// The block's lease changed (also after a reconnect). Gaining it
    /// reads the root when none is loaded and none is on its way; losing
    /// it keeps the rows, and the next read says why it is refused (D99).
    pub fn lease(self: &Arc<Self>, held: bool) {
        if !self.live() {
            return;
        }
        let root = {
            let mut inner = self.lock();
            let was = std::mem::replace(&mut inner.held, held);
            if held && !was && std::mem::take(&mut inner.retry) {
                inner.explorer.retry_status();
            }
            held && (!was || inner.explorer.note().is_some())
                && inner.explorer.tree().is_none()
                && !inner.explorer.waiting()
        };
        if root {
            self.lock().explorer.set_waiting(true);
            self.ask(Which::List, "terminal.workspace-list", Vec::new());
        }
        self.pump();
    }

    /// Carries out what the user asked of the tree.
    pub fn intent(self: &Arc<Self>, intent: Intent) {
        if !self.live() {
            return;
        }
        let fetch = {
            let mut inner = self.lock();
            let ex = &mut inner.explorer;
            let mut fetch = None;
            match intent {
                Intent::Step(by) => ex.tree_mut().into_iter().for_each(|t| t.step(by)),
                Intent::Leave => ex.tree_mut().into_iter().for_each(Tree::leave),
                Intent::Enter if !ex.waiting() => fetch = enter(ex),
                Intent::Enter => {}
                Intent::Select(at) => {
                    if let Some(t) = ex.tree_mut() {
                        select(t, at);
                    }
                }
            }
            fetch
        };
        if let Some(path) = fetch {
            self.list(&path);
        }
        self.pump();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(self: &Arc<Self>, path: &str) {
        let args = vec![("path".to_owned(), Value::Text(path.to_owned()))];
        self.ask(Which::List, "terminal.workspace-list", args);
    }

    /// Sends one call; its reply comes back through [`Self::reply`].
    fn ask(self: &Arc<Self>, which: Which, method: &str, args: Vec<(String, Value)>) {
        if !self.lock().held {
            return self.reply(which, Err("Take over this block first.".into()));
        }
        let me = Arc::clone(self);
        let sent = self
            .port
            .call(method, args, Box::new(move |reply| me.reply(which, reply)));
        if !sent {
            self.reply(which, Err("the block is not open".into()));
        }
    }

    fn reply(self: &Arc<Self>, which: Which, reply: Result<String, String>) {
        if !self.live() {
            return;
        }
        {
            let mut inner = self.lock();
            let ex = &mut inner.explorer;
            match (which, reply) {
                (Which::List, Ok(text)) => ex.load(&text),
                (Which::List, Err(why)) => ex.refuse(&why),
                (Which::Touched, Ok(text)) => ex.load_touched(&text),
                (Which::Touched, Err(_)) => inner.retry = true,
                (Which::Stat, Ok(text)) => ex.load_stat(Some(&text)),
                (Which::Stat, Err(_)) => {
                    ex.load_stat(None);
                    inner.retry = true;
                }
            }
        }
        self.pump();
    }

    /// Makes the calls the tree wants besides listings, then shows it.
    fn pump(self: &Arc<Self>) {
        loop {
            // Marks and stats wait for the lease: asking without it would
            // spend the one ask each gets on a refusal.
            let call = {
                let mut inner = self.lock();
                match inner.held {
                    true => inner.explorer.wants(),
                    false => None,
                }
            };
            let Some(call) = call else { break };
            match call {
                Call::Touched => self.ask(Which::Touched, "terminal.workspace-touched", Vec::new()),
                Call::Stat(path) => {
                    let args = vec![("path".to_owned(), Value::Text(path))];
                    self.ask(Which::Stat, "terminal.workspace-stat", args);
                }
            }
        }
        self.show();
    }

    fn show(&self) {
        if !self.live() {
            return;
        }
        let snapshot = snapshot(&self.lock(), self.port.now());
        self.port.show(snapshot);
    }
}

/// Opens the directory under the cursor: a listing to ask for, if any.
fn enter(ex: &mut Explorer) -> Option<String> {
    match ex.tree_mut().map(Tree::enter) {
        Some(Want::Fetch(path)) => {
            ex.set_waiting(true);
            Some(path)
        }
        _ => None,
    }
}

/// Moves the cursor to row `at` of the flat rows (summaries are skipped
/// by the panel, so it is a selectable row).
fn select(tree: &mut Tree, at: usize) {
    let before = tree.rows().iter().take(at).filter(|r| !r.summary).count();
    let by = isize::try_from(before).unwrap_or(0) - isize::try_from(tree.cursor()).unwrap_or(0);
    tree.step(by);
}

/// What the panel draws for `inner` at time `now`.
fn snapshot(inner: &Inner, now: u64) -> Snapshot {
    let ex = &inner.explorer;
    let Some(tree) = ex.tree() else {
        let state = match (inner.held, ex.note()) {
            (false, _) => State::TakeOver,
            (_, Some(why)) => State::Refused(why.to_owned()),
            _ => State::Loading(".".into()),
        };
        return Snapshot {
            state,
            ..Snapshot::default()
        };
    };
    let rows = tree.rows();
    let cursor = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.summary)
        .nth(tree.cursor())
        .map_or(0, |(i, _)| i);
    let state = match (rows.is_empty(), ex.note(), ex.waiting()) {
        (true, ..) => State::Empty,
        (_, Some(why), _) => State::Refused(why.to_owned()),
        _ => State::Tree,
    };
    Snapshot {
        state,
        rows: rows.iter().map(flat).collect(),
        cursor,
        path: match tree.selected_path() {
            p if p.is_empty() => ".".to_owned(),
            p => p,
        },
        footer: ex.stat_line(now),
    }
}

fn flat(row: &workspace::Row) -> Row {
    Row {
        depth: u16::try_from(row.depth).unwrap_or(u16::MAX),
        name: match row.summary {
            true => row.text.trim_start_matches(['.', ' ']).to_owned(),
            false => workspace::unescape(&row.name),
        },
        dir: row.kind == Kind::Dir,
        open: row.open,
        mark: row.mark,
        summary: row.summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Sent = (String, Vec<(String, Value)>);

    /// A port that records its calls and shows, and answers on demand.
    #[derive(Default)]
    struct Fake {
        sent: Mutex<Vec<Sent>>,
        waiting: Mutex<Vec<ListDone>>,
        shown: Mutex<Vec<Snapshot>>,
    }

    impl Port for Fake {
        fn call(&self, method: &str, args: Vec<(String, Value)>, done: ListDone) -> bool {
            self.sent.lock().unwrap().push((method.into(), args));
            self.waiting.lock().unwrap().push(done);
            true
        }
        fn show(&self, snapshot: Snapshot) {
            self.shown.lock().unwrap().push(snapshot);
        }
        fn now(&self) -> u64 {
            0
        }
    }

    impl Fake {
        fn answer(&self, reply: Result<String, String>) {
            let done = self.waiting.lock().unwrap().remove(0);
            done(reply);
        }
        fn last(&self) -> Snapshot {
            self.shown.lock().unwrap().last().cloned().expect("shown")
        }
        fn methods(&self) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.0.clone())
                .collect()
        }
    }

    const ROOT: &str = ".\nd\tsrc\nf\tREADME.md\n";

    fn open(held: bool, status: bool) -> (Arc<Fake>, Arc<Workspace>) {
        let fake = Arc::new(Fake::default());
        let ws = Workspace::open(fake.clone(), status, held);
        (fake, ws)
    }

    #[test]
    fn opens_with_a_root_listing_and_shows_it() {
        let (fake, _ws) = open(true, false);
        assert_eq!(fake.methods(), ["terminal.workspace-list"]);
        assert!(matches!(fake.last().state, State::Loading(_)));
        fake.answer(Ok(ROOT.into()));
        let s = fake.last();
        assert_eq!(s.state, State::Tree);
        assert_eq!(s.rows.len(), 3, "the workspace root, then its entries");
        assert_eq!((s.rows[0].depth, s.cursor), (0, 0));
        assert!(s.rows[1].dir && s.rows[1].name == "src");
    }

    #[test]
    fn without_the_lease_it_asks_nothing_and_says_take_over() {
        let (fake, ws) = open(false, true);
        assert!(fake.methods().is_empty());
        assert_eq!(fake.last().state, State::TakeOver);
        ws.intent(Intent::Step(1));
        assert!(fake.methods().is_empty());
    }

    #[test]
    fn a_refused_root_is_one_line() {
        let (fake, _ws) = open(true, false);
        fake.answer(Err("no workspace".into()));
        assert_eq!(fake.last().state, State::Refused("no workspace".into()));
    }

    #[test]
    fn enter_on_a_directory_asks_for_its_listing() {
        let (fake, ws) = open(true, false);
        fake.answer(Ok(ROOT.into()));
        ws.intent(Intent::Step(1));
        ws.intent(Intent::Enter);
        let sent = fake.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[1].1,
            vec![("path".to_owned(), Value::Text("src".into()))]
        );
        drop(sent);
        fake.answer(Ok("src\nf\tmain.rs\n".into()));
        let s = fake.last();
        assert_eq!(s.rows.len(), 4);
        assert!(s.rows[1].open);
        assert_eq!(s.rows[2].depth, 2);
    }

    #[test]
    fn cursor_and_path_follow_the_steps() {
        let (fake, ws) = open(true, false);
        fake.answer(Ok(ROOT.into()));
        assert_eq!(fake.last().path, ".");
        ws.intent(Intent::Step(2));
        let s = fake.last();
        assert_eq!((s.cursor, s.path.as_str()), (2, "README.md"));
        ws.intent(Intent::Select(1));
        assert_eq!(fake.last().path, "src");
    }

    #[test]
    fn marks_and_stat_are_asked_only_with_status() {
        let (fake, _ws) = open(true, true);
        fake.answer(Ok(ROOT.into()));
        assert_eq!(
            fake.methods(),
            [
                "terminal.workspace-list",
                "terminal.workspace-touched",
                "terminal.workspace-stat"
            ]
        );
        let (plain, _ws) = open(true, false);
        plain.answer(Ok(ROOT.into()));
        assert_eq!(plain.methods(), ["terminal.workspace-list"]);
    }

    #[test]
    fn a_second_enter_while_waiting_asks_nothing() {
        let (fake, ws) = open(true, false);
        fake.answer(Ok(ROOT.into()));
        ws.intent(Intent::Step(1));
        ws.intent(Intent::Enter);
        ws.intent(Intent::Enter);
        assert_eq!(fake.methods().len(), 2);
    }

    #[test]
    fn a_closed_tree_shows_nothing_more() {
        let (fake, ws) = open(true, false);
        let shown = fake.shown.lock().unwrap().len();
        ws.close();
        fake.answer(Ok(ROOT.into()));
        ws.intent(Intent::Step(1));
        assert_eq!(fake.shown.lock().unwrap().len(), shown, "no late snapshot");
    }

    #[test]
    fn gaining_the_lease_reads_the_root_once() {
        let (fake, ws) = open(false, false);
        assert!(fake.methods().is_empty());
        ws.lease(true);
        ws.lease(true);
        assert_eq!(fake.methods(), ["terminal.workspace-list"]);
        fake.answer(Ok(ROOT.into()));
        assert_eq!(fake.last().state, State::Tree);
    }

    #[test]
    fn losing_the_lease_keeps_the_rows_and_refuses_the_next_read() {
        let (fake, ws) = open(true, false);
        fake.answer(Ok(ROOT.into()));
        ws.lease(false);
        assert_eq!(fake.last().rows.len(), 3, "rows stay");
        ws.intent(Intent::Step(1));
        ws.intent(Intent::Enter);
        assert_eq!(fake.methods().len(), 1, "no call without the lease");
        assert_eq!(
            fake.last().state,
            State::Refused("Take over this block first.".into())
        );
        assert_eq!(fake.last().path, "src", "copy path still works");
    }

    #[test]
    fn a_root_lost_with_the_connection_is_read_again_when_the_lease_returns() {
        let (fake, ws) = open(true, false);
        fake.answer(Err("the connection to the server was lost".into()));
        assert!(matches!(fake.last().state, State::Refused(_)));
        ws.lease(false);
        ws.lease(true);
        assert_eq!(fake.methods().len(), 2);
        fake.answer(Ok(ROOT.into()));
        assert_eq!(fake.last().state, State::Tree);
    }

    const STAT: &str = "f\t4300\t0";

    #[test]
    fn status_waits_for_the_lease_and_a_cut_off_stat_is_asked_again() {
        let (fake, ws) = open(true, true);
        fake.answer(Ok(ROOT.into()));
        fake.answer(Ok("".into())); // touched
        fake.answer(Ok(STAT.into())); // stat of the root
        ws.lease(false);
        ws.intent(Intent::Step(1));
        let sent = fake.methods();
        assert_eq!(sent.len(), 3, "no stat while the lease is absent: {sent:?}");
        assert_eq!(
            fake.last().footer,
            None,
            "an uncached row has no footer yet"
        );
        ws.lease(true);
        assert_eq!(
            fake.methods().last().map(String::as_str),
            Some("terminal.workspace-stat")
        );
        fake.answer(Ok(STAT.into()));
        assert_eq!(fake.last().footer.as_deref(), Some("4.2 KiB"));
    }

    #[test]
    fn a_stat_that_failed_is_asked_again_after_the_lease_returns() {
        let (fake, ws) = open(true, true);
        fake.answer(Ok(ROOT.into()));
        fake.answer(Ok("".into()));
        fake.answer(Err("the connection to the server was lost".into()));
        assert_eq!(fake.last().footer, None);
        ws.lease(false);
        ws.lease(true);
        let methods = fake.methods();
        assert_eq!(
            &methods[3..],
            ["terminal.workspace-touched", "terminal.workspace-stat"]
        );
        fake.answer(Ok("".into()));
        fake.answer(Ok(STAT.into()));
        assert_eq!(fake.last().footer.as_deref(), Some("4.2 KiB"));
    }
}
