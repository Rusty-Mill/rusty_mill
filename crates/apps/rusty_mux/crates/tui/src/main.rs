//! `rusty-mux [--ssh HOST] <socket> [block]`: show a rusty_mux block's
//! window on this terminal, starting with that block focused. With `--ssh`
//! the socket is the server's absolute path on HOST, reached through one
//! OpenSSH (8.7 or later) forward (Unix only; `RUSTY_MUX_SSH` names another
//! `ssh` program); the client asks for nothing until the forward is up, so
//! a password prompt works. If the path to HOST is lost (a silent
//! heartbeat or a dropped forward), the client logs in again without
//! prompting, with backoff, and takes its clients over, keeping the
//! screen and the input lease; it stops if the server let them go.
//! Without a block id it creates a session, window and block first.
//! Ctrl-] then `d` detaches; the blocks keep running and the focused one's
//! id is printed for reattaching. The other Ctrl-] commands are listed in
//! `rusty_mux_tui::keys`. With `RUSTY_MUX_KEYLOG` set to a new file's path,
//! every keyboard read is written to it as one line of hex, to see what a
//! terminal really sends. That is everything typed, passwords included:
//! the file is created the owner's alone (mode 0600 on Unix, an
//! owner-only DACL on Windows), and an existing file is refused.
//! Over a slow path, typing at a prompt shows underlined ahead of the
//! server's echo (`rusty_mux_tui::predict`), in a split window for now;
//! `RUSTY_MUX_PREDICT=off` turns that off.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (mut ssh, mut udp_host, mut rest) = (None, None, args.as_slice());
    loop {
        match rest {
            [flag, host, tail @ ..] if flag == "--ssh" => {
                (ssh, rest) = (Some(host.as_os_str()), tail)
            }
            [flag, host, tail @ ..] if flag == "--udp-host" => {
                (udp_host, rest) = (Some(host.as_os_str()), tail)
            }
            _ => break,
        }
    }
    if rest.is_empty() || rest.len() > 2 || (udp_host.is_some() && ssh.is_none()) {
        eprintln!("usage: rusty-mux [--ssh HOST [--udp-host HOST]] <socket> [block]");
        return ExitCode::from(2);
    }
    match client::run(ssh, udp_host, rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod client {
    pub fn run(
        _ssh: Option<&std::ffi::OsStr>,
        _udp_host: Option<&std::ffi::OsStr>,
        _args: &[std::ffi::OsString],
    ) -> Result<(), String> {
        Err("the client runs on Linux and Windows only".into())
    }
}

#[cfg(any(target_os = "linux", windows))]
mod client {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::io::{self, Write};
    use std::net::Shutdown;
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver as Inbox, RecvTimeoutError, Sender, channel};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use platform::term::Terminal;
    #[cfg(target_os = "linux")]
    use platform_linux::LinuxTerminal as Term;
    #[cfg(windows)]
    use platform_windows::{WindowsTerminal as Term, WindowsUnixStream as UnixStream};
    use rusty_mux_core::{BlockId, Event, Layout, Rect, SessionId, Side, Size, WindowId};
    use rusty_mux_proto::{Entry, Features, Launch, Message, Takeover};
    use rusty_mux_server::checkpoint::Receiver;
    use rusty_mux_tui::keys::{Action, Keys};
    use rusty_mux_tui::predict::{Gate, Overlay, Predictor};
    use rusty_mux_tui::screen::{self, Frame, Passed, View};
    use rusty_mux_tui::{Conn, NoWin32Input, RESTORE, create_private, modes, pass};
    use rusty_term_core::{AnsiParser, Grid};
    use rusty_uuid::Uuid;

    /// How often the loop wakes with nothing to do, to notice the local
    /// terminal changing size.
    const TICK: Duration = Duration::from_millis(100);

    /// What the control connection asks for; a server without `LAYOUT`
    /// shows only the block asked for.
    const CONTROL: Features = Features::CHECKPOINT_V6
        .union(Features::DATAGRAM)
        .union(Features::INSPECT)
        .union(Features::LAYOUT)
        .union(Features::SYNC)
        .union(Features::TAKEOVER);

    /// What a block's connection asks for.
    const PANE: Features = Features::CHECKPOINT_V6
        .union(Features::TAKEOVER)
        .union(Features::ECHO_ACK)
        .union(Features::SYNC);

    /// What a block's connection needs for predictive echo: acks to check
    /// guesses by and round trips to time the path with.
    const PREDICT: Features = Features::ECHO_ACK.union(Features::SYNC);

    /// How often a client that can reconnect asks the server for a sign of
    /// life, and how long without one counts as a lost path.
    const BEAT: Duration = Duration::from_secs(5);
    const DEAD: Duration = Duration::from_secs(15);

    /// How long a path that can tell (UDP, whose server sends a keepalive
    /// after 3 s of silence) may go without a datagram before it is lost.
    const SILENT: Duration = Duration::from_secs(6);
    /// At most this many typed actions wait for a move onto UDP to end.
    const HELD: usize = 1024;
    /// At most this many bytes typed wait for a block's lease.
    const WAITING: usize = 64 * 1024;

    /// The longest wait between attempts to bring the path back.
    const BACKOFF: Duration = Duration::from_secs(30);

    /// How long a layout change that drops shown blocks waits for their own
    /// close, so their last output shows first.
    const GRACE: Duration = Duration::from_millis(500);

    /// How long the server may take over the answers to bring a path back,
    /// or to open a block's connection from the main loop.
    const HANDSHAKE: Duration = Duration::from_secs(10);

    /// Never set: for waits nothing cancels.
    static NEVER: AtomicBool = AtomicBool::new(false);

    /// A block's connection: the block, and which of the connections the
    /// client opened for it, so a stale one's messages are told apart.
    type Tag = (BlockId, u64);

    /// Something for the main loop.
    enum Input {
        Keys(Vec<u8>),
        /// A message on the control connection opened `n`th.
        Control(u64, Message),
        /// The control connection opened `n`th ended.
        ControlEnded(u64),
        Pane(Tag, Message),
        PaneEnded(Tag),
        /// The path to the server came back, its clients taken over, or
        /// did not.
        Rejoined(Result<Back, Missed>),
        /// The clients moved back onto UDP after a fallback to SSH, or
        /// did not.
        Moved(Result<Back, Missed>),
    }

    /// Brings back a lost path to the server, make before break: a fresh
    /// path comes up beside the old one, which may still hold the server's
    /// connections, and so its clients, open until they are taken over.
    trait Rejoin: Send {
        /// Brings up a fresh path beside the one in use, giving up once
        /// `cancel` is set; returns the socket to connect to. With `keep`,
        /// an SSH forward in use that is still running is tried first.
        fn fresh(&mut self, keep: bool, cancel: &AtomicBool) -> Result<PathBuf, String>;
        /// The clients are taken over on the fresh path: the [`Pulses`] read
        /// from now on are those it will settle to, set before the main
        /// loop hears of it, so the old path's silence cannot count against
        /// the fresh one.
        fn listen(&mut self);
        /// The clients are taken over: the fresh path is the one in use,
        /// and the old one stops.
        fn settle(&mut self);
        /// The fresh path failed: it stops, and the old one stays.
        fn discard(&mut self);
        /// Brings up association `id` under `key` on the server's UDP
        /// `port` as a fresh path, once it answers a probe; returns the
        /// socket to connect to.
        fn datagram(&mut self, port: u16, id: [u8; 8], key: [u8; 32]) -> Result<PathBuf, String>;
        /// Makes the UDP path kept since a fallback to SSH the fresh path,
        /// once it answers a probe; returns the socket to connect to.
        fn standby(&mut self) -> Result<PathBuf, String>;
        /// When the path in use, and the UDP path kept since a fallback,
        /// last heard the server, kept current as paths change.
        fn pulses(&self) -> Pulses;
    }

    /// The [`Pulse`]s of the path in use and of the one kept in standby.
    #[derive(Clone, Default)]
    struct Pulses {
        current: Pulse,
        standby: Pulse,
    }

    /// When the path in use last heard the server, for a path that can
    /// tell; shared between the path, which sets it, and the main loop.
    #[derive(Clone, Default)]
    struct Pulse(Arc<Mutex<Option<Heard>>>);

    type Heard = Arc<dyn Fn() -> Option<Instant> + Send + Sync>;

    impl Pulse {
        #[cfg(unix)]
        fn set(&self, heard: Option<Heard>) {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = heard;
        }

        fn last(&self) -> Option<Instant> {
            let heard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            heard.as_ref().and_then(|heard| heard())
        }
    }

    /// What the main loop asks of a [`Rejoiner`].
    enum Ask {
        /// Bring up a fresh path and take these clients over on it; with
        /// `true`, try the SSH forward in use first.
        Fresh(Plan, bool),
        /// Move these clients onto a UDP path.
        Move(Onto, Plan),
        Settle,
    }

    /// The UDP path a move goes onto.
    enum Onto {
        /// The one kept in standby since a fallback to SSH.
        Standby,
        /// A new association the server offered: its port, id and key.
        Offer(u16, [u8; 8], [u8; 32]),
    }

    /// The clients a fresh path is to take over.
    struct Plan {
        me: Takeover,
        next: u64,
        /// The window to ask the layout of, when the server answers List.
        window: Option<WindowId>,
        resumes: Vec<(BlockId, Takeover)>,
    }

    /// A path brought back: its connections, past their Welcomes, with
    /// every client taken over. Dropping it hangs them up.
    struct Back {
        socket: PathBuf,
        stream: UnixStream,
        conn: Conn<UnixStream>,
        writer: Conn<UnixStream>,
        me: Takeover,
        next: u64,
        /// The window's layout and zoomed block, when asked for.
        layout: Option<(Option<Layout>, Option<BlockId>)>,
        panes: Vec<Opened>,
    }

    /// Why a path was not brought back.
    enum Missed {
        /// Before any client was taken over: worth another try.
        Retry(String),
        /// The server had let a client go.
        Dropped,
        /// After the control connection's client was taken over, with the
        /// takeover it was welcomed with: a request on the fresh connection
        /// may have confirmed that token and retired the one asked with,
        /// so only this one continues the client.
        Broken(String, Takeover),
    }

    /// Runs a [`Rejoin`] on its own thread, so the keys, the heartbeat and
    /// detaching stay live while it logs in and takes the clients over;
    /// each attempt comes back as [`Input::Rejoined`]. Dropping it cancels
    /// the attempt under way, waits for it to give up, then drops the
    /// [`Rejoin`].
    struct Rejoiner {
        ask: Option<Sender<Ask>>,
        cancel: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Rejoiner {
        fn spawn(mut rejoin: Box<dyn Rejoin>, tx: Sender<Input>) -> Self {
            let (ask, asked) = channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let cancelled = cancel.clone();
            let thread = std::thread::spawn(move || {
                while let Ok(ask) = asked.recv() {
                    let (path, plan, tell): (_, _, fn(_) -> _) = match ask {
                        Ask::Fresh(plan, keep) => {
                            (rejoin.fresh(keep, &cancelled), plan, Input::Rejoined)
                        }
                        Ask::Move(onto, plan) => {
                            let path = match onto {
                                Onto::Standby => rejoin.standby(),
                                Onto::Offer(port, id, key) => rejoin.datagram(port, id, key),
                            };
                            (path, plan, Input::Moved)
                        }
                        Ask::Settle => {
                            rejoin.settle();
                            continue;
                        }
                    };
                    let back = bring_back(rejoin.as_mut(), path, |socket| {
                        take_over(socket, plan, &cancelled)
                    });
                    if tx.send(tell(back)).is_err() {
                        return;
                    }
                }
            });
            Self {
                ask: Some(ask),
                cancel,
                thread: Some(thread),
            }
        }

        fn ask(&self, ask: Ask) {
            if let Some(sender) = &self.ask {
                // The thread only stops when this is dropped.
                let _ = sender.send(ask);
            }
        }
    }

    /// Takes the clients over by `take` on `path`, a fresh path or the one
    /// in standby. On success the path is listened to before the attempt
    /// returns, so before the main loop can read the [`Pulses`] again; on
    /// failure it is discarded.
    fn bring_back<T>(
        rejoin: &mut dyn Rejoin,
        path: Result<PathBuf, String>,
        take: impl FnOnce(PathBuf) -> Result<T, Missed>,
    ) -> Result<T, Missed> {
        let back = path.map_err(Missed::Retry).and_then(take);
        match &back {
            Ok(_) => rejoin.listen(),
            Err(_) => rejoin.discard(),
        }
        back
    }

    impl Drop for Rejoiner {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Release);
            drop(self.ask.take());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// The connections of a lost path, kept open until their clients are
    /// taken over: hanging up could reach the server first, and end them.
    /// The control connection's stream stays the app's until then; its
    /// reader is joined once that stream is shut, and each block's
    /// connection is hung up as it drops.
    struct Abandoned {
        reader: Option<JoinHandle<()>>,
        panes: Vec<Pane>,
    }

    impl Drop for Abandoned {
        fn drop(&mut self) {
            self.panes.clear();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    /// What the client knows of the server's answer to its Hello.
    struct Welcomed {
        features: Features,
        /// How to take this connection's client over later, if the server
        /// allows it.
        takeover: Option<Takeover>,
    }

    /// The path to the server is down.
    struct Lost {
        /// Failed attempts to bring it back, and why the last one failed.
        tries: u32,
        why: Option<String>,
        /// When to try again; `None` while an attempt is under way.
        retry: Option<Instant>,
        /// The SSH forward in use is likely sound: a move onto UDP broke
        /// after it took over the control client. Tried once, before a
        /// new login.
        keep: bool,
    }

    /// Getting onto UDP from SSH (UDP-TRANSPORT.md): back onto the path
    /// given up for SSH once it is heard again, or, for a client that never
    /// reached UDP, onto a new association the server offers.
    struct Standby {
        /// When the path given up last heard the server.
        pulse: Pulse,
        /// Whether to ask the server for a new association: the client
        /// stayed on SSH from the start.
        asks: bool,
        moving: Moving,
        /// Failed moves since the last one that worked.
        tries: u32,
        /// No move before this.
        next: Instant,
    }

    /// Where a move onto UDP stands.
    enum Moving {
        Idle,
        /// The server was asked for a new association by this request.
        Asked(u64),
        /// The clients are being taken over on UDP.
        Taking(Taking),
    }

    /// While the clients are taken over, the old path's connections end
    /// one by one: no loss if the move goes through, but a real one if it
    /// fails before any client moved. What is typed meanwhile, keys and
    /// focus alike, waits for the path it will go on, in order.
    #[derive(Default)]
    struct Taking {
        /// An old connection ended, or a request on one failed.
        ended: bool,
        held: Vec<Aimed>,
        /// Until every block's old connection has written what was sent on
        /// it, the clients are not asked to move.
        drain: Option<Drain>,
    }

    /// Before the clients are taken over, each block's old connection is
    /// asked for a `Sync`. The server answers it only once every input sent
    /// before it is written to the block, so the takeover, which ends the
    /// old connection's reads, cannot drop keys still on their way.
    struct Drain {
        onto: Onto,
        /// The `Sync`s not answered yet, by block.
        syncs: Vec<(BlockId, u64)>,
        /// Given up after this: the move waits for a later try.
        by: Instant,
    }

    /// An action held for later, with the block it was typed for: the one
    /// in focus then, or for a focus change, the one it moved to. Taken
    /// later only on that block, never on another the focus has since
    /// gone to, by this client's keys or by another client's layout.
    #[derive(Debug, PartialEq)]
    struct Aimed {
        at: BlockId,
        action: Action,
    }

    /// Whether `held` holds a detach.
    fn detaching(held: &[Aimed]) -> bool {
        held.iter().any(|aimed| aimed.action == Action::Detach)
    }

    /// What became of an action typed while a move may be under way.
    #[derive(Debug, PartialEq)]
    enum Held {
        /// No move: act on it now.
        Now(Action),
        Kept,
        /// [`HELD`] actions are waiting already: dropped.
        Full,
    }

    /// What to do once a move onto UDP is over.
    #[derive(Debug, PartialEq)]
    enum After<T> {
        /// Every client moved: take `T`, the path brought up.
        Install(T),
        /// Nothing moved, and nothing was lost meanwhile: stay on SSH.
        Stay,
        /// Lose the path, taking the clients over with this control
        /// takeover if the move left a newer one.
        Lose(Option<Takeover>),
        /// The server let a client go.
        End,
    }

    impl Standby {
        /// An action typed: held during a move, else given back to act on
        /// now. A detach is held too, past the bound, so the clients the
        /// move takes over are detached on the path they end up on, not
        /// left to the server as orphans; a move is over within its
        /// probe and [`HANDSHAKE`]. Nothing typed after it is kept: it was
        /// meant for whatever the terminal shows once the client stops.
        fn hold(&mut self, aimed: Aimed) -> Held {
            match &mut self.moving {
                Moving::Taking(taking) if detaching(&taking.held) => Held::Full,
                Moving::Taking(taking)
                    if taking.held.len() >= HELD && aimed.action != Action::Detach =>
                {
                    Held::Full
                }
                Moving::Taking(taking) => {
                    taking.held.push(aimed);
                    Held::Kept
                }
                Moving::Idle | Moving::Asked(_) => Held::Now(aimed.action),
            }
        }

        /// Gives up a move still draining, which no client has moved for:
        /// the actions it held, to take on the path in use.
        fn undrain(&mut self) -> Option<Vec<Aimed>> {
            let Moving::Taking(Taking { drain: Some(_), .. }) = &self.moving else {
                return None;
            };
            match std::mem::replace(&mut self.moving, Moving::Idle) {
                Moving::Taking(taking) => Some(taking.held),
                Moving::Idle | Moving::Asked(_) => None,
            }
        }

        /// A block's `Sync` was answered, if `answered`: the move to ask
        /// for once every block's was.
        fn synced(&mut self, answered: Option<(BlockId, u64)>) -> Option<Onto> {
            let Moving::Taking(Taking { drain, .. }) = &mut self.moving else {
                return None;
            };
            let syncs = &mut drain.as_mut()?.syncs;
            syncs.retain(|sync| Some(*sync) != answered);
            match syncs.is_empty() {
                true => drain.take().map(|drain| drain.onto),
                false => None,
            }
        }

        /// Whether a move still draining is past its time at `now`.
        fn late(&self, now: Instant) -> bool {
            matches!(&self.moving, Moving::Taking(Taking { drain: Some(drain), .. }) if now >= drain.by)
        }

        /// The actions held by a move under way.
        fn held(&self) -> &[Aimed] {
            match &self.moving {
                Moving::Taking(taking) => &taking.held,
                Moving::Idle | Moving::Asked(_) => &[],
            }
        }

        /// A connection ended or failed: whether to lose the path now.
        /// During a move it is left to the move: see [`Standby::finish`].
        fn lose(&mut self) -> bool {
            match &mut self.moving {
                Moving::Taking(taking) => {
                    taking.ended = true;
                    false
                }
                // An answer to it would come on the connection given up.
                Moving::Asked(_) => {
                    self.moving = Moving::Idle;
                    true
                }
                Moving::Idle => true,
            }
        }

        /// No move before a while from `now`, longer after each failed one,
        /// at most [`BACKOFF`].
        fn wait(&mut self, now: Instant) {
            self.tries += 1;
            let wait = Duration::from_secs(1 << self.tries.min(5)).min(BACKOFF);
            self.next = now + wait;
        }

        /// A move is over, at `now`: what to do, and the actions held during
        /// it, in order, to take on whatever path the client is left on.
        /// Connections that ended during a move that went through were its
        /// own takeovers; during one that failed before any client moved,
        /// they ended for real.
        fn finish<T>(&mut self, moved: Result<T, Missed>, now: Instant) -> (After<T>, Vec<Aimed>) {
            let taking = match std::mem::replace(&mut self.moving, Moving::Idle) {
                Moving::Taking(taking) => taking,
                Moving::Idle | Moving::Asked(_) => Taking::default(),
            };
            let after = match moved {
                Ok(back) => {
                    // On UDP: a later fallback keeps its path in standby.
                    (self.tries, self.asks) = (0, false);
                    After::Install(back)
                }
                Err(Missed::Retry(_)) => {
                    self.wait(now);
                    match taking.ended {
                        true => After::Lose(None),
                        false => After::Stay,
                    }
                }
                Err(Missed::Broken(_, me)) => After::Lose(Some(me)),
                Err(Missed::Dropped) => After::End,
            };
            (after, taking.held)
        }
    }

    /// When the client last asked for and had a sign of life.
    struct Beat {
        asked: Instant,
        heard: Instant,
    }

    /// Why the client stopped.
    enum Ended {
        Detached,
        /// The window's last block closed, or the window did.
        Closed,
        /// The server went away.
        Disconnected,
        /// The path came back, but the server had let this client go.
        Dropped,
        /// The local terminal stopped taking output.
        Stdout,
        Failed(String),
    }

    #[cfg(unix)]
    fn connect(path: &Path) -> Result<UnixStream, String> {
        UnixStream::connect(path).map_err(|e| format!("connecting: {e}"))
    }

    #[cfg(windows)]
    fn connect(path: &Path) -> Result<UnixStream, String> {
        UnixStream::connect(path).map_err(|e| format!("connecting: {e:?}"))
    }

    fn clone(stream: &UnixStream) -> Result<UnixStream, String> {
        stream.try_clone().map_err(|e| e.to_string())
    }

    /// How long the client waits for an SSH forward to come up, the login
    /// included: a first login may wait on a password, a later one may not.
    #[cfg(unix)]
    const LOGIN: Duration = Duration::from_secs(120);
    #[cfg(unix)]
    const RELOGIN: Duration = Duration::from_secs(10);

    /// Attaches to the server at `args[0]`, through a forward to `ssh`'s
    /// host when given, which stops when the client does. A server that
    /// offers UDP is reached on `udp_host`, or on the host `ssh` resolves.
    pub fn run(
        ssh: Option<&OsStr>,
        udp_host: Option<&OsStr>,
        args: &[OsString],
    ) -> Result<(), String> {
        match ssh {
            Some(host) => remote(host, udp_host, Path::new(&args[0]), args.get(1)),
            None => attach(PathBuf::from(&args[0]), args.get(1), None),
        }
    }

    /// Attaches through a forward to `socket` on `host`, stopped on return.
    /// A lost path is brought back with a fresh forward ([`SshPath`]) that
    /// may not ask for anything, since the client is reading the terminal.
    #[cfg(unix)]
    fn remote(
        host: &OsStr,
        udp_host: Option<&OsStr>,
        socket: &Path,
        block: Option<&OsString>,
    ) -> Result<(), String> {
        use rusty_mux_tui::ssh::{Forward, Login};

        let host = host
            .to_str()
            .ok_or("--ssh: the host is not UTF-8")?
            .to_owned();
        let udp_host = udp_host
            .map(|h| h.to_str().ok_or("--udp-host: the host is not UTF-8"))
            .transpose()?
            .map(str::to_owned);
        let remote = socket.to_owned();
        let program = std::env::var_os("RUSTY_MUX_SSH").unwrap_or_else(|| "ssh".into());
        let mut first = Forward::start(&program, &host, &remote, Login::Interactive)?;
        first.wait_ready(LOGIN, &NEVER)?;
        let local = first.socket().to_owned();
        let path = SshPath {
            program,
            host,
            udp_host,
            remote,
            current: Route::Ssh(first),
            fresh: None,
            standby: None,
            onto_standby: false,
            pulses: Pulses::default(),
        };
        attach(local, block, Some(Box::new(path)))
    }

    /// How long the server has to answer a UDP probe before the client
    /// stays on, or goes back to, SSH.
    #[cfg(unix)]
    const PROBE: Duration = Duration::from_secs(2);

    /// One way to the server: an SSH forward, or a UDP association.
    #[cfg(unix)]
    enum Route {
        /// Dropping it stops the forward.
        Ssh(rusty_mux_tui::ssh::Forward),
        Udp(rusty_mux_tui::udp::Relay),
    }

    /// The routes to the server: the one in use, a fresh one until the
    /// clients are taken over on it, and the association given up for SSH,
    /// kept to move back to once it is heard again.
    #[cfg(unix)]
    struct SshPath {
        program: OsString,
        host: String,
        /// Where to send UDP, if not to the host `ssh` resolves.
        udp_host: Option<String>,
        remote: PathBuf,
        current: Route,
        fresh: Option<Route>,
        standby: Option<rusty_mux_tui::udp::Relay>,
        /// The fresh path is the one in standby.
        onto_standby: bool,
        pulses: Pulses,
    }

    #[cfg(unix)]
    impl Rejoin for SshPath {
        /// The association in use, if it answers a probe; with `keep`, the
        /// SSH forward in use, if still running; otherwise a new SSH
        /// forward.
        fn fresh(&mut self, keep: bool, cancel: &AtomicBool) -> Result<PathBuf, String> {
            use rusty_mux_tui::ssh::{Forward, Login};

            self.discard();
            match &mut self.current {
                Route::Udp(relay) if relay.probe(PROBE) => return Ok(relay.socket().to_owned()),
                Route::Ssh(forward) if keep => {
                    if forward.running() {
                        return Ok(forward.socket().to_owned());
                    }
                }
                _ => {}
            }
            let mut next = Forward::start(&self.program, &self.host, &self.remote, Login::Batch)?;
            next.wait_ready(RELOGIN, cancel)?;
            let local = next.socket().to_owned();
            self.fresh = Some(Route::Ssh(next));
            Ok(local)
        }

        fn listen(&mut self) {
            use rusty_mux_tui::udp::Relay;

            // The paths in use and in standby once settled; see `settle`.
            let (current, standby): (Option<&Relay>, Option<&Relay>) =
                match (self.onto_standby, &self.fresh, &self.current) {
                    (true, _, _) => (self.standby.as_ref(), None),
                    (false, None, _) => return,
                    (false, Some(Route::Udp(relay)), _) => (Some(relay), None),
                    (false, Some(Route::Ssh(_)), Route::Udp(relay)) => (None, Some(relay)),
                    (false, Some(Route::Ssh(_)), Route::Ssh(_)) => (None, self.standby.as_ref()),
                };
            self.pulses.current.set(current.map(heard));
            self.pulses.standby.set(standby.map(heard));
        }

        fn settle(&mut self) {
            self.listen();
            if std::mem::take(&mut self.onto_standby) {
                self.fresh = self.standby.take().map(Route::Udp);
            }
            let Some(fresh) = self.fresh.take() else {
                return;
            };
            // Stops the old forward, or the old association unless it is
            // given up for SSH: that one is kept to move back to.
            match (std::mem::replace(&mut self.current, fresh), &self.current) {
                (Route::Udp(relay), Route::Ssh(_)) => self.standby = Some(relay),
                (_, Route::Udp(_)) => self.standby = None,
                _ => {}
            }
        }

        fn standby(&mut self) -> Result<PathBuf, String> {
            self.discard();
            let relay = self.standby.as_ref().ok_or("no UDP path in standby")?;
            if !relay.probe(PROBE) {
                return Err("the UDP path did not answer".into());
            }
            self.onto_standby = true;
            Ok(relay.socket().to_owned())
        }

        fn pulses(&self) -> Pulses {
            self.pulses.clone()
        }

        fn discard(&mut self) {
            self.fresh = None;
            self.onto_standby = false;
        }

        fn datagram(&mut self, port: u16, id: [u8; 8], key: [u8; 32]) -> Result<PathBuf, String> {
            use std::net::ToSocketAddrs;

            self.fresh = None;
            let host = match &self.udp_host {
                Some(host) => host.clone(),
                None => rusty_mux_tui::ssh::hostname(&self.program, &self.host)?,
            };
            let mut servers: Vec<_> = (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|e| format!("resolving {host}: {e}"))?
                .collect();
            servers.dedup();
            let relay = rusty_mux_tui::udp::reach(&servers, id, &key, PROBE)?;
            let local = relay.socket().to_owned();
            self.fresh = Some(Route::Udp(relay));
            Ok(local)
        }
    }

    /// When `relay` last heard the server, for a [`Pulse`].
    #[cfg(unix)]
    fn heard(relay: &rusty_mux_tui::udp::Relay) -> Heard {
        let heard = relay.heard();
        Arc::new(move || heard.last())
    }

    #[cfg(windows)]
    fn remote(_: &OsStr, _: Option<&OsStr>, _: &Path, _: Option<&OsString>) -> Result<(), String> {
        Err("--ssh is supported on Unix only for now".into())
    }

    /// Shows `block` (or a new one) from the server at `socket` until the
    /// client stops; `rejoin`, if given, brings back a lost path.
    fn attach(
        socket: PathBuf,
        block: Option<&OsString>,
        mut rejoin: Option<Box<dyn Rejoin>>,
    ) -> Result<(), String> {
        let stream = connect(&socket)?;
        let mut conn = Conn::new(clone(&stream)?);
        let mut term = Term::new();
        let classic = Size::new(80, 24).map_err(|e| e.to_string())?;
        let area = window_size(&term, classic);
        conn.send(&Message::Hello {
            features: CONTROL,
            resume: None,
            takeover: None,
        })
        .map_err(|e| e.to_string())?;
        let mut req = Requests {
            conn: &mut conn,
            next: 1,
            by: None,
            cancel: &NEVER,
        };
        let Welcomed { features, takeover } = req.welcome()?;
        let block = match block {
            Some(id) => BlockId::from_uuid(parse_uuid(id)?),
            None => req.create(area)?,
        };
        let (window, layout, zoomed) = match features.contains(Features::INSPECT) {
            true => req.place(block)?,
            false => (None, Layout::Block(block), None),
        };
        let mut next = req.next;
        let (mut socket, mut stream, mut conn, mut takeover) = (socket, stream, conn, takeover);
        // On SSH though UDP was offered and wanted, whether it never
        // answered or the move onto it failed: asked again later.
        let mut stayed = false;
        if let (Some(rejoin), Some(me)) = (rejoin.as_mut(), takeover)
            && features.contains(Features::DATAGRAM)
            && udp_wanted()
        {
            let back = match upgrade(rejoin.as_mut(), &socket, &mut conn, me, next)? {
                Upgraded::Udp(back) => Some(back),
                Upgraded::Ssh(back) => {
                    stayed = true;
                    Some(back)
                }
                Upgraded::Stayed => {
                    stayed = true;
                    None
                }
            };
            if let Some(back) = back {
                (socket, stream, conn) = (back.socket, back.stream, back.conn);
                (takeover, next) = (Some(back.me), back.next);
            }
        }

        let (tx, inbox) = channel();
        let control = Conn::new(clone(&stream)?);
        let reader = {
            let tx = tx.clone();
            std::thread::spawn(move || {
                read(conn, &tx, |m| Input::Control(0, m), Input::ControlEnded(0))
            })
        };
        let keylog = keylog()?;
        let predicting = predicting()?;
        term.enter_raw().map_err(|e| format!("raw mode: {e:?}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let keyboard = keyboard(tx.clone(), stop.clone(), keylog);
        // Without a sign of life to ask for, or a client to take over, a
        // lost path cannot be told or brought back.
        let rejoins = features.contains(Features::SYNC) && takeover.is_some();
        let pulses = rejoin.as_ref().map(|r| r.pulses()).unwrap_or_default();
        // Kept to the end either way: it owns the forward in use.
        let (rejoiner, kept) = match rejoin {
            Some(rejoin) if rejoins => (Some(Rejoiner::spawn(rejoin, tx.clone())), None),
            other => (None, other),
        };
        let now = Instant::now();
        let mut app = App {
            socket,
            beat: rejoiner.as_ref().map(|_| Beat {
                asked: now,
                heard: now,
            }),
            rejoiner,
            pulse: pulses.current,
            standby: Standby {
                pulse: pulses.standby,
                asks: stayed,
                moving: Moving::Idle,
                tries: 0,
                next: now,
            },
            held: Vec::new(),
            kept: None,
            me: takeover,
            lost: None,
            abandoned: None,
            resumes: HashMap::new(),
            tx,
            control_stream: stream,
            control,
            reader: Some(reader),
            joined: 0,
            predicting,
            next,
            inspect: features.contains(Features::INSPECT),
            layouts: features.contains(Features::LAYOUT),
            window,
            layout: Some(layout),
            zoomed,
            focus: block,
            panes: HashMap::new(),
            keys: Keys::default(),
            area,
            shown: Shown::Nothing,
            split: None,
            arriving: None,
            opened: 0,
            mirrored: None,
            passed: None,
            no_win32: NoWin32Input::default(),
            pending: None,
            detaching: None,
            ended: None,
        };
        app.sync();
        let ended = app.serve(&term, &inbox);
        let focus = app.focus;
        let down = app.lost.as_ref().map(|lost| lost.why.clone());
        // Stop every reader before touching the terminal, so nothing a
        // block prints lands after the restore. Shutting a socket ends
        // its read; an error only means the server already closed it.
        stop.store(true, Ordering::Release);
        // Detached, a server drops each block's client at once; one whose
        // connection just ends is kept a while for a reconnect, lease and
        // all. Shutting the socket below still delivers this first.
        if app.lost.is_none() {
            for pane in app.panes.values_mut() {
                pane.send(|req| Message::Detach { req });
            }
        }
        let _ = app.control_stream.shutdown(Shutdown::Both);
        app.panes.values().for_each(Pane::hang_up);
        let reader = app.reader.take();
        // Also stops the forward, once any login under way gives up.
        drop(app);
        drop(kept);
        let _ = (reader.map(JoinHandle::join), keyboard.join());
        restore(&mut term);
        match down {
            Some(Some(why)) => eprintln!("\r\nrusty-mux: the path to the server was down: {why}"),
            Some(None) => eprintln!("\r\nrusty-mux: the path to the server was down"),
            None => {}
        }
        match ended {
            Ended::Closed => println!("\r\nrusty-mux: block {focus} closed"),
            Ended::Detached => println!("\r\nrusty-mux: detached from block {focus}"),
            Ended::Disconnected => {
                println!("\r\nrusty-mux: lost the server; block {focus} may still be running")
            }
            Ended::Dropped => println!(
                "\r\nrusty-mux: the server let this client go while the path was down; \
                 block {focus} may still be running"
            ),
            Ended::Stdout => eprintln!("rusty-mux: the terminal stopped taking output"),
            Ended::Failed(why) => return Err(why),
        }
        Ok(())
    }

    /// Whether to try UDP: unless `RUSTY_MUX_UDP=off`.
    fn udp_wanted() -> bool {
        std::env::var_os("RUSTY_MUX_UDP").is_none_or(|v| v != "off")
    }

    /// Where [`upgrade`] left the control connection.
    enum Upgraded {
        /// Moved onto UDP.
        Udp(Back),
        /// Taken back over SSH after a failed move onto UDP.
        Ssh(Back),
        /// Still on the connection it had: nothing moved.
        Stayed,
    }

    /// Moves the control connection, the only one so far, from the SSH
    /// forward at `ssh` onto a UDP association, once the association
    /// answers a probe; the forward then stops (UDP-TRANSPORT.md). Until
    /// then nothing has moved, and a failure keeps the client on SSH, said
    /// once. If the move itself fails, its Hello may have taken the client
    /// over already, so the client takes it back over SSH. `Err` only when
    /// that fails too.
    fn upgrade(
        rejoin: &mut dyn Rejoin,
        ssh: &Path,
        conn: &mut Conn<UnixStream>,
        me: Takeover,
        next: u64,
    ) -> Result<Upgraded, String> {
        let mut req = Requests {
            conn,
            next,
            by: Some(Instant::now() + HANDSHAKE),
            cancel: &NEVER,
        };
        let offered = req.open_datagram();
        let next = req.next;
        let plan = || Plan {
            me,
            next,
            window: None,
            resumes: Vec::new(),
        };
        let socket = match offered.and_then(|(port, id, key)| rejoin.datagram(port, id, key)) {
            Ok(socket) => socket,
            Err(why) => {
                rejoin.discard();
                eprintln!("rusty-mux: staying on SSH: {why}");
                return Ok(Upgraded::Stayed);
            }
        };
        let missed = match take_over(socket, plan(), &NEVER) {
            Ok(back) => {
                rejoin.settle();
                return Ok(Upgraded::Udp(back));
            }
            Err(missed) => missed,
        };
        rejoin.discard();
        let why = match missed {
            Missed::Retry(why) | Missed::Broken(why, _) => why,
            Missed::Dropped => return Err("the server let this client go".into()),
        };
        eprintln!("rusty-mux: staying on SSH: {why}");
        match take_over(ssh.to_owned(), plan(), &NEVER) {
            Ok(back) => Ok(Upgraded::Ssh(back)),
            Err(Missed::Dropped) => Err("the server let this client go".into()),
            Err(Missed::Retry(why) | Missed::Broken(why, _)) => Err(why),
        }
    }

    /// The local terminal's size, or `fallback` when it has none: a pty
    /// nobody sized reports 0x0.
    fn window_size(term: &Term, fallback: Size) -> Size {
        term.window_size()
            .ok()
            .and_then(|w| Size::new(w.cols, w.rows).ok())
            .unwrap_or(fallback)
    }

    /// Reads messages from `conn` into the main loop until it ends, then
    /// sends `ended`.
    fn read(
        mut conn: Conn<UnixStream>,
        tx: &Sender<Input>,
        wrap: impl Fn(Message) -> Input,
        ended: Input,
    ) {
        while let Ok(message) = conn.recv() {
            if tx.send(wrap(message)).is_err() {
                return;
            }
        }
        let _ = tx.send(ended);
    }

    /// The file `RUSTY_MUX_KEYLOG` names, created for the log; `None` when
    /// it is unset. See [`create_private`].
    /// Whether `RUSTY_MUX_PREDICT` leaves predictive echo on: unset or
    /// `on`, and `off` turns it off.
    fn predicting() -> Result<bool, String> {
        match std::env::var_os("RUSTY_MUX_PREDICT") {
            None => Ok(true),
            Some(v) if v == "on" => Ok(true),
            Some(v) if v == "off" => Ok(false),
            Some(v) => Err(format!("RUSTY_MUX_PREDICT is {v:?}, not on or off")),
        }
    }

    fn keylog() -> Result<Option<std::fs::File>, String> {
        let Some(path) = std::env::var_os("RUSTY_MUX_KEYLOG") else {
            return Ok(None);
        };
        let path = Path::new(&path);
        let file = create_private(path)
            .map_err(|e| format!("RUSTY_MUX_KEYLOG {}: {e}", path.display()))?;
        eprintln!(
            "rusty-mux: logging every key typed, passwords included, to {}",
            path.display()
        );
        Ok(Some(file))
    }

    /// Reads the keyboard into the main loop until `stop`; an empty read
    /// means the keyboard is gone. Each read goes to `keylog` too, as a
    /// line of hex; logging stops at the first failed write, since the
    /// terminal is raw and has nowhere to report it.
    fn keyboard(
        tx: Sender<Input>,
        stop: Arc<AtomicBool>,
        mut keylog: Option<std::fs::File>,
    ) -> JoinHandle<()> {
        std::thread::spawn(move || {
            let term = Term::new();
            let mut buf = [0u8; 4096];
            while !stop.load(Ordering::Acquire) {
                let n = match term.poll_readable(Some(TICK)) {
                    Ok(false) => continue,
                    Ok(true) => term.read_chunk(&mut buf).unwrap_or(0),
                    Err(_) => 0,
                };
                if let Some(file) = keylog.as_mut() {
                    let hex: String = buf[..n].iter().map(|b| format!("{b:02x} ")).collect();
                    if writeln!(file, "{}", hex.trim_end()).is_err() {
                        keylog = None;
                    }
                }
                if tx.send(Input::Keys(buf[..n].to_vec())).is_err() || n == 0 {
                    return;
                }
            }
        })
    }

    /// Undoes the modes a block's program may have set on this terminal,
    /// then leaves raw mode.
    fn restore(term: &mut Term) {
        let mut stdout = std::io::stdout();
        if let Err(err) = stdout.write_all(RESTORE).and_then(|()| stdout.flush()) {
            eprintln!("rusty-mux: resetting the terminal: {err}");
        }
        if let Err(err) = term.leave_raw() {
            eprintln!("rusty-mux: restoring the terminal: {err:?}");
        }
    }

    /// Requests answered before the main loop starts, or on a fresh path,
    /// each answer due `by`, if given, unless `cancel` is set first.
    struct Requests<'a> {
        conn: &'a mut Conn<UnixStream>,
        next: u64,
        by: Option<Instant>,
        cancel: &'a AtomicBool,
    }

    impl Requests<'_> {
        fn recv(&mut self) -> Result<Message, String> {
            match self.by {
                Some(by) => answer(self.conn, by, self.cancel),
                None => self.conn.recv().map_err(|e| e.to_string()),
            }
        }

        fn welcome(&mut self) -> Result<Welcomed, String> {
            let welcomed = welcomed(self.recv()?)?;
            match welcomed.features.contains(Features::CHECKPOINT_V6) {
                true => Ok(welcomed),
                false => Err("the server cannot send checkpoints".into()),
            }
        }

        /// Sends a request and returns its answer, every message of it,
        /// skipping all else; `Err` if it failed.
        fn call(&mut self, make: impl FnOnce(u64) -> Message) -> Result<Vec<Message>, String> {
            let req = self.next;
            self.next += 1;
            self.conn.send(&make(req)).map_err(|e| e.to_string())?;
            let mut answer = Vec::new();
            loop {
                let message = self.recv()?;
                match message {
                    Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
                    Message::Done { req: r, .. } if r == req => {
                        answer.push(message);
                        return Ok(answer);
                    }
                    Message::Entry { req: r, .. } if r == req => answer.push(message),
                    _ => {}
                }
            }
        }

        /// Asks for a UDP association: the server's port, its id and key.
        fn open_datagram(&mut self) -> Result<(u16, [u8; 8], [u8; 32]), String> {
            let req = self.next;
            self.next += 1;
            let ask = Message::OpenDatagram { req };
            self.conn.send(&ask).map_err(|e| e.to_string())?;
            loop {
                match self.recv()? {
                    Message::Datagram {
                        req: r,
                        port,
                        id,
                        key,
                    } if r == req => return Ok((port, id, key.0)),
                    Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
                    _ => {}
                }
            }
        }

        fn create_id(&mut self, make: impl FnOnce(u64) -> Message) -> Result<Uuid, String> {
            match self.call(make)?.last() {
                Some(Message::Done {
                    created: Some(id), ..
                }) => Ok(*id),
                _ => Err("the server did not return an id".into()),
            }
        }

        fn create(&mut self, size: Size) -> Result<BlockId, String> {
            let session = SessionId::from_uuid(self.create_id(|req| Message::CreateSession {
                req,
                name: "main".into(),
            })?);
            let window = WindowId::from_uuid(self.create_id(|req| Message::CreateWindow {
                req,
                session,
                name: "main".into(),
            })?);
            let block = self.create_id(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch: Launch::default(),
            })?;
            Ok(BlockId::from_uuid(block))
        }

        /// The window `block` is in, its layout and its zoomed block.
        fn place(
            &mut self,
            block: BlockId,
        ) -> Result<(Option<WindowId>, Layout, Option<BlockId>), String> {
            let entries = self.call(|req| Message::List { req })?;
            let window = entries
                .iter()
                .find_map(|m| match m {
                    Message::Entry {
                        entry:
                            Entry::Block {
                                block: b, window, ..
                            },
                        ..
                    } if *b == block => Some(*window),
                    _ => None,
                })
                .ok_or_else(|| format!("unknown block {block}"))?;
            let (layout, zoomed) = layout_of(entries, window).unwrap_or_default();
            Ok((Some(window), layout.unwrap_or(Layout::Block(block)), zoomed))
        }
    }

    /// `window`'s layout and zoomed block in a List's `entries`; `None`
    /// when the window is gone.
    fn layout_of(
        entries: Vec<Message>,
        window: WindowId,
    ) -> Option<(Option<Layout>, Option<BlockId>)> {
        entries.into_iter().find_map(|m| match m {
            Message::Entry {
                entry:
                    Entry::Window {
                        window: w,
                        layout,
                        zoomed,
                        ..
                    },
                ..
            } if w == window => Some((layout, zoomed)),
            _ => None,
        })
    }

    /// The next message on `conn`, due `by`. Each read waits at most
    /// [`TICK`] and returns whatever came, so `by` and `cancel` are checked
    /// between reads, however slowly a message trickles in; what came of
    /// it is kept. A connection handed on waits without a deadline again.
    fn answer(
        conn: &mut Conn<UnixStream>,
        by: Instant,
        cancel: &AtomicBool,
    ) -> Result<Message, String> {
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            let left = by.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the server did not answer in time".into());
            }
            let wait = left.min(TICK);
            conn.get_ref()
                .set_read_timeout(Some(wait))
                .map_err(|e| e.to_string())?;
            let got = conn.try_recv();
            conn.get_ref()
                .set_read_timeout(None)
                .map_err(|e| e.to_string())?;
            match got {
                Ok(Some(message)) => return Ok(message),
                Ok(None) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// Takes the clients in `plan` over on the fresh path at `socket`: the
    /// control connection's first, with the layout asked for again, then
    /// each block's. Every answer is due within [`HANDSHAKE`].
    fn take_over(socket: PathBuf, plan: Plan, cancel: &AtomicBool) -> Result<Back, Missed> {
        let by = Instant::now() + HANDSHAKE;
        let retry = |e: String| Missed::Retry(format!("reconnecting: {e}"));
        let stream = connect(&socket).map_err(retry)?;
        let mut conn = Conn::new(clone(&stream).map_err(retry)?);
        let writer = Conn::new(clone(&stream).map_err(retry)?);
        conn.send(&Message::Hello {
            features: CONTROL,
            resume: None,
            takeover: Some(plan.me),
        })
        .map_err(|e| retry(e.to_string()))?;
        let mut req = Requests {
            conn: &mut conn,
            next: plan.next,
            by: Some(by),
            cancel,
        };
        let me = match req.welcome().map_err(retry)?.takeover {
            Some(takeover) if takeover.client == plan.me.client => takeover,
            _ => return Err(Missed::Dropped),
        };
        // Taken over: from here on, a failure ends the client.
        let layout = match plan.window {
            Some(window) => {
                let entries = req
                    .call(|req| Message::List { req })
                    .map_err(|why| Missed::Broken(why, me))?;
                Some(layout_of(entries, window).unwrap_or_default())
            }
            None => None,
        };
        let next = req.next;
        let panes = plan
            .resumes
            .into_iter()
            .map(|(block, resume)| {
                reach(&socket, block, Some(resume), by, cancel).map_err(|missed| match missed {
                    Missed::Retry(why) | Missed::Broken(why, _) => Missed::Broken(why, me),
                    Missed::Dropped => Missed::Dropped,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Back {
            socket,
            stream,
            conn,
            writer,
            me,
            next,
            layout,
            panes,
        })
    }

    /// A block's connection past its Welcome, its reader not started yet.
    struct Opened {
        block: BlockId,
        stream: UnixStream,
        conn: Conn<UnixStream>,
        writer: Conn<UnixStream>,
        takeover: Option<Takeover>,
        /// The client taken over, if this continues one.
        resumed: Option<Takeover>,
        features: Features,
    }

    /// Connects to `socket` for `block`, taking `resume`'s client over if
    /// given, up to the Welcome, due `by`.
    fn reach(
        socket: &Path,
        block: BlockId,
        resume: Option<Takeover>,
        by: Instant,
        cancel: &AtomicBool,
    ) -> Result<Opened, Missed> {
        let retry = Missed::Retry;
        let stream = connect(socket).map_err(retry)?;
        let mut conn = Conn::new(clone(&stream).map_err(retry)?);
        let writer = Conn::new(clone(&stream).map_err(retry)?);
        conn.send(&Message::Hello {
            features: PANE,
            resume: None,
            takeover: resume,
        })
        .map_err(|e| retry(e.to_string()))?;
        let welcomed = welcomed(answer(&mut conn, by, cancel).map_err(retry)?).map_err(retry)?;
        match (resume, welcomed.takeover) {
            (None, _) => {}
            (Some(r), Some(t)) if t.client == r.client => {}
            (Some(_), _) => return Err(Missed::Dropped),
        }
        Ok(Opened {
            block,
            stream,
            conn,
            writer,
            takeover: welcomed.takeover,
            resumed: resume,
            features: welcomed.features,
        })
    }

    /// What a Welcome tells, or why `message` is not one.
    fn welcomed(message: Message) -> Result<Welcomed, String> {
        match message {
            Message::Welcome {
                features,
                client,
                token,
                ..
            } => Ok(Welcomed {
                features,
                takeover: token.map(|token| Takeover { client, token }),
            }),
            other => Err(format!("expected Welcome, got {other:?}")),
        }
    }

    /// One block's connection and its local copy of the block's terminal.
    struct Pane {
        /// Which connection to its block this is.
        generation: u64,
        block: BlockId,
        /// How to take this connection's client over after a lost path.
        takeover: Option<Takeover>,
        /// The block's close came on this connection, after its last output.
        closed: bool,
        stream: UnixStream,
        conn: Conn<UnixStream>,
        next: u64,
        attach: u64,
        /// Assembles the attach checkpoint until Ready.
        receiver: Option<Receiver>,
        term: Option<(Grid, AnsiParser)>,
        /// The pending lease request, if any.
        asking: Option<u64>,
        /// Keys typed while the lease is asked for, sent once it is held.
        waiting: Vec<Vec<u8>>,
        leased: bool,
        /// Continues a client that may still hold the block's lease on the
        /// server: until [`App::lease`] settles it, asked for again in
        /// focus, given back out of it.
        retained: bool,
        /// The size last asked for, until the block reports a resize.
        sizing: Option<Size>,
        reader: Option<JoinHandle<()>>,
        /// Guesses typed keys' echo, when the connection can check them.
        predict: Option<Predictor>,
        gate: Gate,
    }

    impl Pane {
        fn send(&mut self, make: impl FnOnce(u64) -> Message) -> u64 {
            let req = self.next;
            self.next += 1;
            // A failed send ends the reader too, which reports it.
            let _ = self.conn.send(&make(req));
            req
        }

        /// Keys typed into this block: sent if it holds the lease, kept
        /// until it does while asked for, up to [`WAITING`] bytes, else
        /// dropped as the server would refuse them.
        fn typed(&mut self, data: Vec<u8>) {
            if self.leased {
                return self.type_in(data);
            }
            let kept: usize = self.waiting.iter().map(Vec::len).sum();
            match (self.asking.is_some(), kept + data.len() <= WAITING) {
                (true, true) => self.waiting.push(data),
                (true, false) => bell(),
                (false, _) => {}
            }
        }

        /// Sends typed `data` to the block and predicts its echo, timing
        /// the path with a `Sync` when one is due.
        fn type_in(&mut self, data: Vec<u8>) {
            let block = self.block;
            let now = Instant::now();
            if self.predict.is_some() && self.gate.due(now) {
                let req = self.send(|req| Message::Sync { req });
                self.gate.sent(req, now);
            }
            let req = self.next;
            if let (Some(predict), Some((grid, _))) = (self.predict.as_mut(), self.term.as_ref()) {
                predict.key(req, &data, grid);
            }
            self.send(|req| Message::Input { req, block, data });
        }

        /// Drops the predictions: the block's screen, size or lease changed.
        fn unpredict(&mut self) {
            if let Some(predict) = self.predict.as_mut() {
                predict.reset();
            }
        }

        /// Applies a message from the block's connection.
        fn take(&mut self, block: BlockId, message: Message) -> Result<Change, String> {
            match message {
                Message::Checkpoint {
                    block: b,
                    offset,
                    total,
                    data,
                } if b == block => {
                    let receiver = self.receiver.as_mut().ok_or("checkpoint after ready")?;
                    receiver
                        .push(offset, total, &data)
                        .map_err(|e| format!("bad checkpoint: {e}"))?;
                }
                Message::Ready { block: b } if b == block => {
                    let receiver = self.receiver.take().ok_or("ready twice")?;
                    let term = receiver
                        .finish()
                        .map_err(|e| format!("bad checkpoint: {e}"))?;
                    self.term = Some(term);
                    self.unpredict();
                    return Ok(Change::Ready);
                }
                Message::Output { block: b, data } if b == block => {
                    if let Some((grid, parser)) = self.term.as_mut() {
                        match self.predict.as_mut() {
                            Some(predict) => predict.output(parser, grid, &data),
                            None => parser.advance(grid, &data),
                        }
                    }
                    return Ok(Change::Output(data));
                }
                Message::EchoAck { block: b, req } if b == block => {
                    if let (Some(predict), Some((grid, _))) = (self.predict.as_mut(), &self.term) {
                        predict.ack(req, grid);
                    }
                }
                Message::EchoMode { block: b, mode } if b == block => {
                    if let Some(predict) = self.predict.as_mut() {
                        predict.mode(mode);
                    }
                }
                Message::Event {
                    event: Event::BlockResized { block: b, size },
                    ..
                } if b == block => {
                    if let Some((grid, _)) = self.term.as_mut() {
                        grid.resize(size.cols().into(), size.rows().into());
                    }
                    self.sizing = None;
                    self.unpredict();
                }
                Message::Event {
                    event: Event::BlockClosed { block: b },
                    ..
                } if b == block => return Ok(Change::Closed),
                Message::Done { req, .. } if self.gate.awaits(req) => {
                    self.gate.answered(Instant::now());
                }
                Message::Done { req, .. } if self.asking == Some(req) => {
                    self.asking = None;
                    self.leased = true;
                    for data in std::mem::take(&mut self.waiting) {
                        self.type_in(data);
                    }
                    return Ok(Change::Leased);
                }
                Message::Failed { req, reason, .. } if req == self.attach => {
                    return Ok(Change::Refused(reason));
                }
                Message::Failed { req, .. } => {
                    // A refused lease, or input after losing it.
                    if self.asking == Some(req) {
                        self.asking = None;
                        self.waiting.clear();
                    }
                    self.leased = false;
                    self.unpredict();
                }
                _ => {}
            }
            Ok(Change::None)
        }

        fn hang_up(&self) {
            let _ = self.stream.shutdown(Shutdown::Both);
        }

        fn size(&self) -> Option<Size> {
            let (grid, _) = self.term.as_ref()?;
            Size::new(
                u16::try_from(grid.cols).ok()?,
                u16::try_from(grid.rows).ok()?,
            )
            .ok()
        }
    }

    /// What a block's message changed, for the screen.
    enum Change {
        None,
        /// The attach checkpoint is in: the block can be drawn.
        Ready,
        Output(Vec<u8>),
        Leased,
        /// The block closed; told on its own connection after its last
        /// output, so that output is shown first.
        Closed,
        /// The server would not attach the block.
        Refused(String),
    }

    /// What the local terminal shows.
    #[derive(PartialEq)]
    enum Shown {
        Nothing,
        /// One block's raw output.
        Passing(BlockId),
        /// Several blocks, composed; the last frame drawn.
        Composed(Option<Frame>),
    }

    struct App {
        socket: PathBuf,
        /// Brings back a lost path, when the client can.
        rejoiner: Option<Rejoiner>,
        /// Asks for signs of life, when the client can bring a path back.
        beat: Option<Beat>,
        /// When a path that can tell last heard the server.
        pulse: Pulse,
        /// Moving back onto the UDP path given up for SSH.
        standby: Standby,
        /// Actions held during a move that lost the path, taken once it is
        /// back.
        held: Vec<Aimed>,
        /// Keys that were waiting on a block's lease when a move started or
        /// the path was lost, with that block: taken ahead of the held
        /// actions, and only if the block still has the focus then.
        kept: Option<(BlockId, Vec<u8>)>,
        /// How to take the control connection's client over.
        me: Option<Takeover>,
        lost: Option<Lost>,
        /// The lost path's connections, until their clients are taken over.
        abandoned: Option<Abandoned>,
        /// The blocks' connections to take over once the path is back.
        resumes: HashMap<BlockId, Takeover>,
        tx: Sender<Input>,
        control_stream: UnixStream,
        control: Conn<UnixStream>,
        /// Reads the control connection.
        reader: Option<JoinHandle<()>>,
        /// Control connections opened after the first, to tell a stale
        /// one's messages apart.
        joined: u64,
        /// Whether typed keys' echo may be predicted: `RUSTY_MUX_PREDICT`.
        predicting: bool,
        next: u64,
        /// Whether the server answers List.
        inspect: bool,
        /// Whether the server keeps layouts; without, only the first block
        /// shows.
        layouts: bool,
        window: Option<WindowId>,
        layout: Option<Layout>,
        zoomed: Option<BlockId>,
        focus: BlockId,
        panes: HashMap<BlockId, Pane>,
        keys: Keys,
        area: Size,
        shown: Shown,
        /// The pending split, whose new block takes the focus.
        split: Option<u64>,
        /// The split's new block, until the layout shows it.
        arriving: Option<BlockId>,
        /// Block connections opened so far, to tag each.
        opened: u64,
        /// The input modes last mirrored from the focused block while
        /// composed.
        mirrored: Option<Vec<u8>>,
        /// Predictions drawn over the passing block since its last output.
        passed: Option<Passed>,
        /// Filters the passing block's raw output.
        no_win32: NoWin32Input,
        /// A layout change held back while blocks it drops drain.
        pending: Option<Pending>,
        /// Detaching once no keys wait on a lease, or by this at the latest.
        detaching: Option<Instant>,
        ended: Option<Ended>,
    }

    /// A layout change that drops shown blocks. The server tells it on the
    /// control connection, which is not ordered with the blocks' own; so
    /// the old layout stays on screen until each dropped block's connection
    /// has told its close (after its last output) or ended, or [`GRACE`]
    /// runs out.
    struct Pending {
        layout: Option<Layout>,
        zoomed: Option<BlockId>,
        leaving: Vec<BlockId>,
        by: Instant,
    }

    impl App {
        /// The main loop: until something ends it, take keys and server
        /// messages and redraw.
        fn serve(&mut self, term: &Term, inbox: &Inbox<Input>) -> Ended {
            loop {
                match inbox.recv_timeout(TICK) {
                    Ok(input) => self.take(input),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return Ended::Disconnected,
                }
                while let Ok(input) = inbox.try_recv() {
                    self.take(input);
                }
                let area = window_size(term, self.area);
                if area != self.area {
                    self.area = area;
                    self.sync();
                }
                self.draw();
                self.beat();
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| Instant::now() >= p.by)
                {
                    self.settle();
                }
                if let Some(ended) = self.ended.take() {
                    return ended;
                }
            }
        }

        fn take(&mut self, input: Input) {
            match input {
                Input::Keys(bytes) if bytes.is_empty() => self.end(Ended::Detached),
                Input::Keys(bytes) => {
                    for action in self.keys.feed(&bytes) {
                        self.typed(action);
                    }
                }
                // From a control connection given up on.
                Input::Control(..) | Input::ControlEnded(_) if self.lost.is_some() => {}
                Input::Control(joined, message) if joined == self.joined => {
                    if let Some(beat) = self.beat.as_mut() {
                        beat.heard = Instant::now();
                    }
                    self.on_control(message)
                }
                Input::Control(..) => {}
                // The server closes every connection when it goes; a
                // block's own is left to report it, after its last output.
                Input::ControlEnded(joined) if joined != self.joined => {}
                Input::ControlEnded(_) if !self.panes.is_empty() => {}
                Input::ControlEnded(_) => self.broke(Ended::Disconnected),
                Input::Pane(tag, message) => {
                    self.on_pane(tag, message);
                    self.detach_when_sent(Instant::now());
                }
                // Only asked for while lost.
                Input::Rejoined(_) if self.lost.is_none() => {}
                Input::Rejoined(Ok(back)) => self.rejoined(back),
                Input::Rejoined(Err(Missed::Retry(why))) => self.retry(why),
                Input::Rejoined(Err(Missed::Dropped)) => self.end(Ended::Dropped),
                Input::Rejoined(Err(Missed::Broken(why, _))) => {
                    self.end(Ended::Failed(format!("reconnecting: {why}")))
                }
                Input::Moved(moved) => self.moved(moved),
                Input::PaneEnded(tag) if self.current(tag) => {
                    let block = tag.0;
                    let leaving = self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.leaving.contains(&block));
                    let closed = self.panes.get(&block).is_some_and(|p| p.closed);
                    match leaving || closed {
                        true => self.drained(block),
                        // A block this client still shows lost its connection.
                        false => self.broke(Ended::Disconnected),
                    }
                }
                // A connection this client already hung up on.
                Input::PaneEnded(_) => {}
            }
        }

        /// A connection failed: the path is lost if the client can bring
        /// it back, else the client stops with `ended`.
        fn broke(&mut self, ended: Ended) {
            match self.rejoiner.is_some() {
                true => self.lose(false),
                false => self.end(ended),
            }
        }

        /// Asks for a sign of life every [`BEAT`], and counts the path lost
        /// after [`DEAD`] without one, or after [`SILENT`] on a path that
        /// hears the server's keepalives; brings a lost one back when due.
        fn beat(&mut self) {
            let now = Instant::now();
            self.detach_when_sent(now);
            if let Some(lost) = self.lost.as_mut() {
                if lost.retry.is_some_and(|at| now >= at) {
                    lost.retry = None;
                    self.ask_back();
                }
                return;
            }
            if self.standby.late(now) {
                self.give_up_move();
            }
            let Some(beat) = self.beat.as_mut() else {
                return;
            };
            let silent = self
                .pulse
                .last()
                .is_some_and(|heard| now.saturating_duration_since(heard) >= SILENT);
            if silent || now.duration_since(beat.heard) >= DEAD {
                self.lose(false);
            } else if now.duration_since(beat.asked) >= BEAT {
                beat.asked = now;
                self.request(|req| Message::Sync { req });
            } else {
                self.move_back(now);
            }
        }

        /// Moves every client back onto the UDP path given up for SSH,
        /// once that path hears the server again, make before break as a
        /// lost path is brought back; the SSH path stays in use until then.
        /// A client that stayed on SSH from the start asks the server for
        /// a new association instead, and moves once the server offers one.
        fn move_back(&mut self, now: Instant) {
            let standby = &self.standby;
            if !matches!(standby.moving, Moving::Idle) || now < standby.next {
                return;
            }
            if self.detaching.is_some() {
                return;
            }
            let last = standby.pulse.last();
            let heard = last.is_some_and(|heard| now.saturating_duration_since(heard) < SILENT);
            if heard {
                self.move_onto(Onto::Standby);
            } else if standby.asks || last.is_some() {
                // A silent path in standby may be gone from the server, past
                // its `--udp-idle`: a new association is asked for as well.
                self.standby.moving = Moving::Asked(self.next);
                self.request(|req| Message::OpenDatagram { req });
            }
        }

        /// The server answered the request for a new association.
        fn offered(&mut self, offer: Option<Onto>) {
            self.standby.moving = Moving::Idle;
            match offer {
                Some(onto) => self.move_onto(onto),
                None => self.wait_to_move(),
            }
        }

        /// Whether `req` asked for the new association awaited.
        fn asked(&self, req: u64) -> bool {
            matches!(self.standby.moving, Moving::Asked(asked) if asked == req)
        }

        /// Asks for every client to be taken over on `onto`.
        /// What is typed meanwhile is held; first each block's old
        /// connection is drained (see [`Drain`]).
        fn move_onto(&mut self, onto: Onto) {
            // A block that cannot be taken over would be left on SSH; a
            // client detaching is about to stop, and would only take keys
            // still to send off the connections it detaches.
            let stuck = self.panes.values().any(|p| p.takeover.is_none());
            if self.rejoiner.is_none() || stuck || self.detaching.is_some() {
                return;
            }
            self.kept = self.unsent();
            let syncs = self
                .panes
                .values_mut()
                .map(|pane| (pane.block, pane.send(|req| Message::Sync { req })))
                .collect();
            let drain = Drain {
                onto,
                syncs,
                by: Instant::now() + HANDSHAKE,
            };
            self.standby.moving = Moving::Taking(Taking {
                drain: Some(drain),
                ..Taking::default()
            });
            self.synced(None);
        }

        /// A `Sync` asked for by a move was answered, if `answered`: once
        /// every block's was, asks for the clients to move.
        fn synced(&mut self, answered: Option<(BlockId, u64)>) {
            let Some(onto) = self.standby.synced(answered) else {
                return;
            };
            let resumes: Option<Vec<_>> = self
                .panes
                .values()
                .map(|pane| Some((pane.block, pane.takeover?)))
                .collect();
            let (Some(resumes), Some(rejoiner), Some(me)) = (resumes, &self.rejoiner, self.me)
            else {
                return self.give_up_move();
            };
            rejoiner.ask(Ask::Move(
                onto,
                Plan {
                    me,
                    next: self.next,
                    window: self.window.filter(|_| self.inspect),
                    resumes,
                },
            ));
        }

        /// Gives up a move still draining: what it held is taken on the
        /// path in use, and the next try waits.
        fn give_up_move(&mut self) {
            let held = self.standby.undrain().unwrap_or_default();
            self.wait_to_move();
            self.replay(held);
        }

        /// No move for a while, longer after each failed one, at most
        /// [`BACKOFF`].
        fn wait_to_move(&mut self) {
            self.standby.wait(Instant::now());
        }

        /// A move onto UDP is done or failed: see [`Standby::finish`].
        fn moved(&mut self, moved: Result<Back, Missed>) {
            let (after, held) = self.standby.finish(moved, Instant::now());
            match after {
                After::Install(back) => {
                    self.held = held;
                    self.rejoined(back);
                }
                After::Stay => self.replay(held),
                After::Lose(me) => {
                    self.me = me.or(self.me);
                    self.held = held;
                    self.lose(me.is_some());
                }
                After::End => self.end(Ended::Dropped),
            }
        }

        /// Keys typed while a lease was asked for, taken off the path being
        /// left so they go ahead of the actions held for the next one: the
        /// focused block's, with that block, any other block's (typed
        /// before the focus moved on) dropped with a bell.
        fn unsent(&mut self) -> Option<(BlockId, Vec<u8>)> {
            let focus = self.focus;
            let mut keys = None;
            for (block, pane) in &mut self.panes {
                let waiting = std::mem::take(&mut pane.waiting);
                match (waiting.is_empty(), *block == focus) {
                    (true, _) => {}
                    (false, true) => keys = Some((*block, waiting.concat())),
                    (false, false) => bell(),
                }
            }
            keys
        }

        /// Takes the kept keys, then actions held during a move, in order,
        /// up to a detach. The keys go only to the block they were typed
        /// into, if it still has the focus: the layout the path came back
        /// with may have moved it, and they would reach the wrong program.
        /// Each held action is taken only on the block it was aimed at, a
        /// focus change only onto a block still shown.
        fn replay(&mut self, held: Vec<Aimed>) {
            if let Some((block, keys)) = self.kept.take() {
                match (block == self.focus, self.panes.get_mut(&block)) {
                    (true, Some(pane)) => pane.typed(keys),
                    _ => bell(),
                }
            }
            for Aimed { at, action } in held {
                match action {
                    Action::Detach => return self.detach(),
                    Action::Focus(_) if self.visible().iter().any(|(b, _)| *b == at) => {
                        self.focus = at;
                        self.sync();
                    }
                    action if at == self.focus && !matches!(action, Action::Focus(_)) => {
                        self.act(action)
                    }
                    _ => bell(),
                }
            }
        }

        /// Stops listening to every connection, keeping what it takes to
        /// continue each, and asks for the path back, trying the SSH
        /// forward in use first if `keep`.
        fn lose(&mut self, keep: bool) {
            if self.lost.is_some() {
                return;
            }
            // No client moved yet: what the move held waits for the path.
            if let Some(held) = self.standby.undrain() {
                self.held = held;
            }
            if !self.standby.lose() {
                return;
            }
            if let Some(keys) = self.unsent() {
                self.kept = Some(keys);
            }
            let panes: Vec<Pane> = self.panes.drain().map(|(_, pane)| pane).collect();
            self.resumes = panes
                .iter()
                .filter_map(|pane| Some((pane.block, pane.takeover?)))
                .collect();
            self.abandoned = Some(Abandoned {
                reader: self.reader.take(),
                panes,
            });
            // The layout is asked for again once the path is back.
            self.pending = None;
            self.lost = Some(Lost {
                tries: 0,
                why: None,
                retry: None,
                keep,
            });
            self.ask_back();
        }

        /// Asks for a fresh path, with every client to take over on it.
        fn ask_back(&mut self) {
            let (Some(rejoiner), Some(me)) = (&self.rejoiner, self.me) else {
                self.end(Ended::Dropped);
                return;
            };
            let keep = self.lost.as_ref().is_some_and(|lost| lost.keep);
            let plan = Plan {
                me,
                next: self.next,
                window: self.window.filter(|_| self.inspect),
                resumes: self.resumes.iter().map(|(b, r)| (*b, *r)).collect(),
            };
            rejoiner.ask(Ask::Fresh(plan, keep));
        }

        /// An attempt to bring the path back failed: try again later, at
        /// most [`BACKOFF`] apart. Ctrl-] d still detaches meanwhile. A
        /// detach held through a broken move gets one attempt, to detach
        /// the clients it moved; then the client stops, leaving them to
        /// the server's grace.
        fn retry(&mut self, why: String) {
            if detaching(&self.held) {
                return self.end(Ended::Detached);
            }
            let Some(lost) = self.lost.as_mut() else {
                return;
            };
            lost.tries += 1;
            lost.keep = false;
            let wait = Duration::from_secs(1 << lost.tries.min(5)).min(BACKOFF);
            lost.retry = Some(Instant::now() + wait);
            // Told if the client stops before the path is back: the
            // terminal shows a block until then.
            lost.why = Some(why);
        }

        /// The path is back, every client taken over: listens to its
        /// connections, takes the layout it read, and shows the blocks
        /// again, each sending its screen with its takeover. Only then are
        /// the old path's connections, and the old path, let go.
        fn rejoined(&mut self, back: Back) {
            self.me = Some(back.me);
            // Past any id the old path used meanwhile, as on a move.
            self.next = self.next.max(back.next);
            if let Some((layout, zoomed)) = back.layout {
                (self.layout, self.zoomed) = (layout, zoomed);
            }
            self.joined += 1;
            let joined = self.joined;
            let tx = self.tx.clone();
            let conn = back.conn;
            let reader = std::thread::spawn(move || {
                read(
                    conn,
                    &tx,
                    move |m| Input::Control(joined, m),
                    Input::ControlEnded(joined),
                )
            });
            self.reader = Some(reader);
            let old = std::mem::replace(&mut self.control_stream, back.stream);
            self.control = back.writer;
            self.socket = back.socket;
            self.lost = None;
            let now = Instant::now();
            self.beat = Some(Beat {
                asked: now,
                heard: now,
            });
            for opened in back.panes {
                let pane = self.pane(opened);
                self.panes.insert(pane.block, pane);
            }
            self.resumes.clear();
            self.shown = Shown::Nothing;
            self.sync();
            let _ = old.shutdown(Shutdown::Both);
            drop(self.abandoned.take());
            if let Some(rejoiner) = &self.rejoiner {
                rejoiner.ask(Ask::Settle);
            }
            let held = std::mem::take(&mut self.held);
            self.replay(held);
        }

        /// Whether `tag` is the open connection to its block.
        fn current(&self, (block, generation): Tag) -> bool {
            self.panes
                .get(&block)
                .is_some_and(|p| p.generation == generation)
        }

        /// Takes the window's new layout: at once, or once the shown blocks
        /// it drops have drained (see [`Pending`]).
        fn relayout(&mut self, layout: Option<Layout>, zoomed: Option<BlockId>) {
            let staying = layout.as_ref().map(Layout::blocks).unwrap_or_default();
            let mut leaving: Vec<BlockId> = self
                .panes
                .iter()
                .filter(|(b, p)| !staying.contains(b) && !p.closed)
                .map(|(b, _)| *b)
                .collect();
            if let Some(old) = self.pending.take() {
                for block in old.leaving {
                    if !leaving.contains(&block) {
                        leaving.push(block);
                    }
                }
            }
            self.pending = Some(Pending {
                layout,
                zoomed,
                by: Instant::now() + GRACE,
                leaving,
            });
            if self.pending.as_ref().is_some_and(|p| p.leaving.is_empty()) {
                self.settle();
            }
        }

        /// A block's connection told its close, or ended: once drawn, it
        /// is done, and a held layout change may go ahead.
        fn drained(&mut self, block: BlockId) {
            self.draw();
            self.panes.remove(&block);
            let Some(pending) = self.pending.as_mut() else {
                return;
            };
            pending.leaving.retain(|b| *b != block);
            if pending.leaving.is_empty() {
                self.settle();
            }
        }

        /// Applies the held layout change, with what the leaving blocks
        /// last showed drawn first.
        fn settle(&mut self) {
            let Some(pending) = self.pending.take() else {
                return;
            };
            self.draw();
            self.layout = pending.layout;
            self.zoomed = pending.zoomed;
            self.sync();
        }

        /// Detaches once the keys waiting on a lease are sent (or refused),
        /// at most [`HANDSHAKE`] from now: a detach typed after them must
        /// not drop them. Nothing typed meanwhile is taken.
        fn detach(&mut self) {
            self.detaching.get_or_insert(Instant::now() + HANDSHAKE);
            self.detach_when_sent(Instant::now());
        }

        /// Ends a detach under way once no keys wait on a lease, or at `now`
        /// past its time.
        fn detach_when_sent(&mut self, now: Instant) {
            let Some(by) = self.detaching else {
                return;
            };
            // Keys kept off a lost path are still to send once it is back.
            let waiting =
                self.kept.is_some() || self.panes.values().any(|pane| !pane.waiting.is_empty());
            if !waiting || now >= by {
                self.end(Ended::Detached);
            }
        }

        /// Keeps the first reason the client stops.
        fn end(&mut self, ended: Ended) {
            self.ended.get_or_insert(ended);
        }

        fn request(&mut self, make: impl FnOnce(u64) -> Message) -> u64 {
            if self.lost.is_some() {
                bell();
                return 0;
            }
            let req = self.next;
            self.next += 1;
            if self.control.send(&make(req)).is_err() {
                self.broke(Ended::Disconnected);
            }
            req
        }

        /// Takes a typed action, or holds it during a move, and after a
        /// move that lost the path, behind what that move held; either
        /// way aimed at the block it was typed for (see [`Aimed`]).
        fn typed(&mut self, action: Action) {
            // A detach is held or under way already: the client stops
            // either way.
            if detaching(&self.held) || self.detaching.is_some() {
                return bell();
            }
            let Some(aimed) = self.aim(action) else {
                return bell();
            };
            if !self.held.is_empty() && aimed.action != Action::Detach {
                return match self.held.len() < HELD {
                    true => self.held.push(aimed),
                    false => bell(),
                };
            }
            match self.standby.hold(aimed) {
                Held::Now(action) => self.act(action),
                Held::Kept => {}
                Held::Full => bell(),
            }
        }

        /// `action` aimed at the block in focus once what is held is taken;
        /// `None` for a focus change with nowhere to go.
        fn aim(&self, action: Action) -> Option<Aimed> {
            let focus = self
                .held
                .iter()
                .chain(self.standby.held())
                .rev()
                .find(|aimed| matches!(aimed.action, Action::Focus(_)))
                .map_or(self.focus, |aimed| aimed.at);
            let at = match &action {
                Action::Focus(dir) => screen::neighbor(&self.visible(), focus, *dir)?,
                _ => focus,
            };
            Some(Aimed { at, action })
        }

        fn act(&mut self, action: Action) {
            let focus = self.focus;
            match action {
                Action::Input(data) => {
                    if let Some(pane) = self.panes.get_mut(&focus) {
                        pane.typed(data);
                    }
                }
                Action::Detach => self.detach(),
                Action::Focus(dir) => {
                    match screen::neighbor(&self.visible(), focus, dir) {
                        Some(block) => self.focus = block,
                        None => bell(),
                    }
                    self.sync();
                }
                Action::ToggleZoom if !self.layouts => bell(),
                Action::ToggleZoom => match (self.zoomed, self.window) {
                    (None, _) => {
                        self.request(|req| Message::Zoom { req, block: focus });
                    }
                    (Some(_), Some(window)) => {
                        self.request(|req| Message::Unzoom { req, window });
                    }
                    (Some(_), None) => bell(),
                },
                Action::Split { .. } if !self.layouts => bell(),
                Action::Split { beside } => {
                    let side = if beside { Side::Right } else { Side::Down };
                    let req = self.request(|req| Message::Split {
                        req,
                        block: focus,
                        side,
                        launch: Launch::default(),
                    });
                    self.split = Some(req);
                }
                Action::Close => {
                    self.request(|req| Message::CloseBlock { req, block: focus });
                }
                Action::Unknown => bell(),
            }
        }

        fn on_control(&mut self, message: Message) {
            match message {
                Message::Event { event, .. } => self.on_event(event),
                Message::Done {
                    req,
                    created: Some(id),
                } if self.split == Some(req) => {
                    self.split = None;
                    self.arriving = Some(BlockId::from_uuid(id));
                    self.sync();
                }
                Message::Datagram { req, port, id, key } if self.asked(req) => {
                    self.offered(Some(Onto::Offer(port, id, key.0)))
                }
                Message::Failed { req, .. } if self.asked(req) => self.offered(None),
                Message::Failed { .. } => bell(),
                _ => {}
            }
        }

        fn on_event(&mut self, event: Event) {
            match event {
                Event::LayoutChanged {
                    window,
                    layout,
                    zoomed,
                } if Some(window) == self.window => self.relayout(layout, zoomed),
                Event::WindowClosed { window } if Some(window) == self.window => {
                    self.relayout(None, None)
                }
                // Without layouts the only block shown is the first.
                Event::BlockClosed { block }
                    if !self.layouts && self.layout == Some(Layout::Block(block)) =>
                {
                    self.relayout(None, None)
                }
                _ => {}
            }
        }

        fn on_pane(&mut self, tag: Tag, message: Message) {
            if !self.current(tag) {
                return;
            }
            let block = tag.0;
            if let Message::Done { req, .. } = &message {
                self.synced(Some((block, *req)));
            }
            let Some(pane) = self.panes.get_mut(&block) else {
                return;
            };
            match pane.take(block, message) {
                Ok(Change::None) => {}
                Ok(Change::Ready) => {
                    if self.shown == Shown::Passing(block) {
                        let painted = self.panes.get(&block).and_then(|p| p.term.as_ref());
                        if let Some((grid, _)) = painted {
                            write(&pass(grid), &mut self.ended);
                            self.passed = None;
                        }
                    }
                    self.fit();
                }
                Ok(Change::Output(data)) => {
                    if self.shown == Shown::Passing(block) {
                        // Put back what predictions cover first, so the
                        // output lands on the screen it was written for.
                        let erased = self.passed.take().map(|p| p.erase());
                        let bytes = self.no_win32.filter(&data);
                        write(
                            &[erased.unwrap_or_default(), bytes].concat(),
                            &mut self.ended,
                        );
                    }
                }
                // The focus may have moved on while the lease was asked for.
                Ok(Change::Leased) => {
                    self.lease();
                    self.fit();
                }
                Ok(Change::Closed) => {
                    pane.closed = true;
                    self.on_event(Event::BlockClosed { block });
                    let leaving = self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.leaving.contains(&block));
                    if leaving {
                        self.drained(block);
                    }
                }
                // Only the block asked for is worth stopping over; any
                // other is closing, and the layout will drop it.
                Ok(Change::Refused(reason)) if self.panes.len() == 1 => {
                    self.end(Ended::Failed(reason))
                }
                Ok(Change::Refused(_)) => {}
                Err(why) => self.end(Ended::Failed(why)),
            }
        }

        /// The blocks shown and where: the zoomed block over everything,
        /// else every block with room in the layout.
        fn visible(&self) -> Vec<(BlockId, Rect)> {
            let Some(layout) = &self.layout else {
                return Vec::new();
            };
            let whole = Rect {
                x: 0,
                y: 0,
                size: self.area,
            };
            match self.zoomed {
                Some(block) if layout.blocks().contains(&block) => vec![(block, whole)],
                _ => layout.rects(self.area),
            }
        }

        /// Brings connections, leases and the screen in line with the
        /// layout, the zoom, the focus and the terminal's size.
        fn sync(&mut self) {
            // The path is down: nothing to open, lease or show until it is
            // back, when this runs again.
            if self.lost.is_some() {
                return;
            }
            let visible = self.visible();
            let Some(&(first, _)) = visible.first() else {
                self.end(Ended::Closed);
                return;
            };
            let shows = |block: BlockId| visible.iter().any(|(b, _)| *b == block);
            if let Some(block) = self.arriving.filter(|b| shows(*b)) {
                self.focus = block;
                self.arriving = None;
            }
            if !shows(self.focus) {
                self.focus = first;
            }
            let gone: Vec<BlockId> = self.panes.keys().filter(|b| !shows(**b)).copied().collect();
            // Left on purpose, so detached first: a server keeps a client
            // whose connection just ends, lease and all, for a reconnect,
            // and the block's next client could not lease it meanwhile.
            for block in gone {
                if let Some(mut pane) = self.panes.remove(&block) {
                    pane.send(|req| Message::Detach { req });
                    pane.hang_up();
                }
            }
            for (block, _) in &visible {
                if !self.panes.contains_key(block) {
                    match self.open(*block) {
                        Ok(pane) => {
                            self.panes.insert(*block, pane);
                        }
                        Err(ended) => {
                            self.end(ended);
                            return;
                        }
                    }
                }
            }
            self.lease();
            self.fit();
            let shown = match visible.as_slice() {
                [(only, _)] => Shown::Passing(*only),
                _ => Shown::Composed(None),
            };
            if shown != self.shown || matches!(shown, Shown::Composed(_)) {
                self.show(shown);
            }
        }

        /// Holds the focused block's input lease and no other, so other
        /// clients and agents may type into the blocks not in focus.
        fn lease(&mut self) {
            let focus = self.focus;
            for (block, pane) in &mut self.panes {
                match *block == focus {
                    true if !pane.leased && pane.asking.is_none() => {
                        let block = *block;
                        pane.asking = Some(pane.send(|req| Message::AcquireLease { req, block }));
                        pane.retained = false;
                    }
                    // The server refuses a release of a lease that is not
                    // this client's, so another holder keeps it.
                    false if pane.leased || pane.retained => {
                        let block = *block;
                        pane.send(|req| Message::ReleaseLease { req, block });
                        pane.leased = false;
                        pane.retained = false;
                        pane.unpredict();
                    }
                    _ => {}
                }
            }
        }

        /// Resizes the focused block to its place, if it holds the lease
        /// and is a different size.
        fn fit(&mut self) {
            let focus = self.focus;
            let Some((_, rect)) = self.visible().into_iter().find(|(b, _)| *b == focus) else {
                return;
            };
            let Some(pane) = self.panes.get_mut(&focus) else {
                return;
            };
            let size = rect.size;
            if pane.leased && pane.sizing != Some(size) && pane.size().is_some_and(|s| s != size) {
                pane.sizing = Some(size);
                pane.send(|req| Message::Resize {
                    req,
                    block: focus,
                    size,
                });
            }
        }

        /// Connects to the server for `block` and attaches to it; the
        /// Welcome is due within [`HANDSHAKE`].
        fn open(&mut self, block: BlockId) -> Result<Pane, Ended> {
            let by = Instant::now() + HANDSHAKE;
            match reach(&self.socket, block, None, by, &NEVER) {
                Ok(opened) => Ok(self.pane(opened)),
                Err(Missed::Retry(why) | Missed::Broken(why, _)) => Err(Ended::Failed(why)),
                Err(Missed::Dropped) => Err(Ended::Dropped),
            }
        }

        /// Listens to a block's connection. One that continues a client is
        /// still attached as it was, and the server sends the screen again;
        /// any other attaches. Either asks for the lease when in focus.
        fn pane(&mut self, opened: Opened) -> Pane {
            let block = opened.block;
            self.opened += 1;
            let tag = (block, self.opened);
            let tx = self.tx.clone();
            let conn = opened.conn;
            let reader = std::thread::spawn(move || {
                read(
                    conn,
                    &tx,
                    move |message| Input::Pane(tag, message),
                    Input::PaneEnded(tag),
                )
            });
            let mut pane = Pane {
                generation: tag.1,
                block,
                takeover: opened.takeover,
                closed: false,
                conn: opened.writer,
                stream: opened.stream,
                next: 1,
                attach: 0,
                receiver: Some(Receiver::default()),
                term: None,
                asking: None,
                waiting: Vec::new(),
                // Asked for again even on a takeover: silence may have
                // released the lease meanwhile (UDP-TRANSPORT.md).
                leased: false,
                retained: opened.resumed.is_some(),
                sizing: None,
                reader: Some(reader),
                predict: (self.predicting && opened.features.contains(PREDICT))
                    .then(Predictor::default),
                gate: Gate::default(),
            };
            if opened.resumed.is_none() {
                pane.attach = pane.send(|req| Message::Attach { req, block });
            }
            pane
        }

        /// Switches what the terminal shows: one block's own output, or
        /// several composed on the alternate screen.
        fn show(&mut self, shown: Shown) {
            let bytes = match &shown {
                // A block not ready yet is passed on Ready.
                Shown::Passing(block) => {
                    // A new stream: what was held belongs to the last one.
                    self.no_win32 = NoWin32Input::default();
                    match self.panes.get(block).and_then(|p| p.term.as_ref()) {
                        Some((grid, _)) => pass(grid),
                        None => b"\x1b[?1049l".to_vec(),
                    }
                }
                Shown::Composed(_) => match self.shown {
                    Shown::Composed(_) => Vec::new(),
                    // Undo the modes the passed-through program set, then
                    // draw where the shell's screen is kept.
                    _ => [RESTORE, b"\x1b[?1049h"].concat(),
                },
                Shown::Nothing => Vec::new(),
            };
            write(&bytes, &mut self.ended);
            self.mirrored = None;
            self.passed = None;
            self.shown = shown;
        }

        /// What `block` shows: from its connection, or while the path is
        /// down, what it last showed on the lost one.
        fn grid(&self, block: BlockId) -> Option<&Grid> {
            let lost = self.abandoned.iter().flat_map(|a| &a.panes);
            let mut panes = self.panes.get(&block).into_iter().chain(lost);
            let pane = panes.find(|p| p.block == block)?;
            pane.term.as_ref().map(|(grid, _)| grid)
        }

        /// The predictions to draw over `block`: the focused block's, on a
        /// slow path.
        fn overlay(&self, block: BlockId) -> Option<Overlay> {
            let pane = self.panes.get(&block).filter(|_| block == self.focus)?;
            pane.predict
                .as_ref()
                .filter(|_| pane.gate.slow())?
                .overlay()
        }

        /// Brings the predictions drawn over the passing `block` in line
        /// with its overlay. Drawn only where `SGR 0` and a plain cursor
        /// move put the terminal back as the block left it; the last ones
        /// drawn stay until its next output otherwise.
        fn pass_ahead(&mut self, block: BlockId) {
            let Some((grid, parser)) = self.panes.get(&block).and_then(|p| p.term.as_ref()) else {
                return;
            };
            if !screen::passable(grid, parser) {
                return;
            }
            let wanted = self.overlay(block);
            if wanted.as_ref() == self.passed.as_ref().map(Passed::overlay) {
                return;
            }
            let erased = self.passed.take().map(|p| p.erase()).unwrap_or_default();
            let drawn = wanted.map(|o| screen::pass_ahead(grid, &o));
            let (bytes, passed) = drawn.unzip();
            self.passed = passed;
            write(
                &[erased, bytes.unwrap_or_default()].concat(),
                &mut self.ended,
            );
        }

        /// Redraws the composed screen where it changed.
        fn draw(&mut self) {
            if let Shown::Passing(block) = self.shown {
                return self.pass_ahead(block);
            }
            let Shown::Composed(last) = &self.shown else {
                return;
            };
            let visible = self.visible();
            let views: Vec<View<'_>> = visible
                .iter()
                .map(|(block, rect)| View {
                    block: *block,
                    rect: *rect,
                    grid: self.grid(*block),
                    overlay: self.overlay(*block),
                })
                .collect();
            let frame = screen::compose(self.area, &views, self.focus);
            let mut bytes = screen::render(last.as_ref(), &frame);
            // Keys reach the focused block encoded as it asked for.
            let focused = views.iter().find(|v| v.block == self.focus);
            let wanted = focused.and_then(|v| v.grid).map(|g| modes(g, false));
            if wanted.is_some() && wanted != self.mirrored {
                bytes.extend(wanted.iter().flatten());
                self.mirrored = wanted;
            }
            write(&bytes, &mut self.ended);
            self.shown = Shown::Composed(Some(frame));
        }
    }

    impl Drop for Pane {
        fn drop(&mut self) {
            self.hang_up();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    /// Writes to the terminal; a terminal that stops taking output ends
    /// the client.
    fn write(bytes: &[u8], ended: &mut Option<Ended>) {
        if bytes.is_empty() {
            return;
        }
        let mut stdout = std::io::stdout();
        if stdout
            .write_all(bytes)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            ended.get_or_insert(Ended::Stdout);
        }
    }

    fn bell() {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"\x07").and_then(|()| stdout.flush());
    }

    fn parse_uuid(text: &OsString) -> Result<Uuid, String> {
        let text = text.to_str().ok_or("block id is not UTF-8")?;
        text.parse().map_err(|e| format!("block id: {e}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rusty_mux_tui::keys::Dir;

        /// Records what a [`Rejoin`] is asked, in order.
        #[derive(Default)]
        struct Calls(Vec<&'static str>);

        impl Rejoin for Calls {
            fn fresh(&mut self, _: bool, _: &AtomicBool) -> Result<PathBuf, String> {
                self.0.push("fresh");
                Ok(PathBuf::from("fresh.sock"))
            }
            fn listen(&mut self) {
                self.0.push("listen");
            }
            fn settle(&mut self) {
                self.0.push("settle");
            }
            fn discard(&mut self) {
                self.0.push("discard");
            }
            fn datagram(&mut self, _: u16, _: [u8; 8], _: [u8; 32]) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn standby(&mut self) -> Result<PathBuf, String> {
                self.0.push("standby");
                Ok(PathBuf::from("standby.sock"))
            }
            fn pulses(&self) -> Pulses {
                Pulses::default()
            }
        }

        #[test]
        fn a_path_brought_back_is_listened_to_before_the_main_loop_hears() {
            let mut calls = Calls::default();
            let path = calls.fresh(false, &AtomicBool::new(false));
            let back = bring_back(&mut calls, path, |_| Ok(()));
            assert!(back.is_ok());
            // `settle` is left to the main loop; the pulse moved already.
            assert_eq!(calls.0, ["fresh", "listen"]);
        }

        #[test]
        fn a_path_not_brought_back_is_discarded_and_not_listened_to() {
            let mut calls = Calls::default();
            let path = calls.fresh(false, &AtomicBool::new(false));
            let back = bring_back(&mut calls, path, |_| Err::<(), _>(Missed::Dropped));
            assert!(matches!(back, Err(Missed::Dropped)));
            assert_eq!(calls.0, ["fresh", "discard"]);
        }

        fn idle() -> Standby {
            Standby {
                pulse: Pulse::default(),
                asks: false,
                moving: Moving::Idle,
                tries: 0,
                next: Instant::now(),
            }
        }

        fn taking() -> Standby {
            Standby {
                moving: Moving::Taking(Taking::default()),
                ..idle()
            }
        }

        #[cfg(unix)]
        fn block(n: u8) -> BlockId {
            BlockId::from_uuid(Uuid::from_bytes([n; 16]))
        }

        fn takeover(n: u8) -> Takeover {
            Takeover {
                client: rusty_mux_core::ClientId::from_uuid(Uuid::from_bytes([n; 16])),
                token: Uuid::from_bytes([n + 100; 16]),
            }
        }

        fn input(data: &[u8]) -> Action {
            Action::Input(data.to_vec())
        }

        fn aimed(action: Action) -> Aimed {
            Aimed {
                at: BlockId::from_uuid(Uuid::nil()),
                action,
            }
        }

        fn actions(held: &[Aimed]) -> Vec<&Action> {
            held.iter().map(|aimed| &aimed.action).collect()
        }

        #[test]
        fn actions_typed_during_a_move_are_held_in_order_up_to_a_detach() {
            let mut standby = taking();
            assert_eq!(standby.hold(aimed(input(b"ab"))), Held::Kept);
            assert_eq!(standby.hold(aimed(Action::Focus(Dir::Right))), Held::Kept);
            assert_eq!(standby.hold(aimed(Action::Detach)), Held::Kept);
            assert_eq!(
                standby.hold(aimed(input(b"c"))),
                Held::Full,
                "nothing after it"
            );
            let (after, held) = standby.finish(Ok(()), Instant::now());
            assert_eq!(after, After::Install(()));
            assert_eq!(
                actions(&held),
                [&input(b"ab"), &Action::Focus(Dir::Right), &Action::Detach]
            );
            // Once over, actions go straight through, and none come back.
            assert_eq!(standby.hold(aimed(input(b"e"))), Held::Now(input(b"e")));
            assert_eq!(standby.finish(Ok(()), Instant::now()).1, []);
        }

        #[test]
        fn actions_held_during_a_move_are_bounded() {
            let mut standby = taking();
            for _ in 0..HELD {
                assert_eq!(standby.hold(aimed(input(b"k"))), Held::Kept);
            }
            assert_eq!(standby.hold(aimed(input(b"k"))), Held::Full);
            assert_eq!(
                standby.hold(aimed(Action::Detach)),
                Held::Kept,
                "never dropped"
            );
            assert_eq!(standby.finish(Ok(()), Instant::now()).1.len(), HELD + 1);
        }

        #[test]
        fn actions_held_during_a_failed_move_are_given_back_to_take_on_ssh() {
            let mut standby = taking();
            assert_eq!(standby.hold(aimed(input(b"x"))), Held::Kept);
            let (after, held) =
                standby.finish::<()>(Err(Missed::Retry("no".into())), Instant::now());
            assert_eq!(after, After::Stay);
            assert_eq!(actions(&held), [&input(b"x")]);
        }

        #[test]
        fn a_loss_during_a_move_is_left_to_it_and_ignored_if_it_goes_through() {
            let mut standby = taking();
            // A heartbeat on the SSH control the move took over fails.
            assert!(!standby.lose());
            assert!(!standby.lose());
            assert_eq!(standby.finish(Ok(()), Instant::now()).0, After::Install(()));
            assert!(standby.lose(), "after the move, a loss is a loss");
        }

        #[test]
        fn a_loss_during_a_move_that_failed_before_any_client_moved_is_real() {
            let mut standby = taking();
            assert!(!standby.lose());
            let now = Instant::now();
            let (after, _) = standby.finish::<()>(Err(Missed::Retry("no".into())), now);
            assert_eq!(after, After::Lose(None));
            assert_eq!(
                (standby.tries, standby.next),
                (1, now + Duration::from_secs(2))
            );
            assert!(matches!(standby.moving, Moving::Idle));
        }

        #[test]
        fn a_move_broken_after_the_control_moved_loses_the_path_with_its_new_takeover() {
            let mut standby = taking();
            let missed = Missed::Broken("list".into(), takeover(7));
            let (after, _) = standby.finish::<()>(Err(missed), Instant::now());
            assert_eq!(after, After::Lose(Some(takeover(7))));
        }

        #[test]
        fn a_loss_while_an_offer_is_awaited_drops_the_offer() {
            let mut standby = Standby {
                moving: Moving::Asked(4),
                ..idle()
            };
            assert!(standby.lose());
            assert!(matches!(standby.moving, Moving::Idle));
        }

        /// A server at a fresh socket, its connections handed to `script`
        /// in turn.
        #[cfg(unix)]
        fn scripted(
            script: impl FnOnce(&std::os::unix::net::UnixListener) + Send + 'static,
        ) -> PathBuf {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusty-mux-tui-unit-{}-{n}.sock",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
            std::thread::spawn(move || script(&listener));
            path
        }

        /// Accepts a connection and answers its Hello with a Welcome
        /// giving `client` the token `taken`.
        #[cfg(unix)]
        fn welcome(
            listener: &std::os::unix::net::UnixListener,
            taken: Takeover,
        ) -> Conn<UnixStream> {
            let (stream, _) = listener.accept().expect("accept");
            let mut conn = Conn::new(stream);
            assert!(matches!(conn.recv().expect("hello"), Message::Hello { .. }));
            conn.send(&Message::Welcome {
                features: CONTROL.union(PANE),
                client: taken.client,
                cursor: None,
                token: Some(taken.token),
            })
            .expect("welcome");
            conn
        }

        #[cfg(unix)]
        fn plan(me: Takeover, window: Option<WindowId>, resumes: Vec<(BlockId, Takeover)>) -> Plan {
            Plan {
                me,
                next: 1,
                window,
                resumes,
            }
        }

        #[cfg(unix)]
        #[test]
        fn a_list_whose_answer_is_lost_breaks_the_move_with_the_new_takeover() {
            let (old, new) = (
                takeover(1),
                Takeover {
                    token: Uuid::from_bytes([9; 16]),
                    ..takeover(1)
                },
            );
            let socket = scripted(move |listener| {
                let mut conn = welcome(listener, new);
                // The List reaches the server, confirming `new`; no answer.
                assert!(matches!(conn.recv().expect("list"), Message::List { .. }));
            });
            let window = WindowId::from_uuid(Uuid::from_bytes([3; 16]));
            let missed = take_over(socket, plan(old, Some(window), Vec::new()), &NEVER).err();
            assert!(
                matches!(missed, Some(Missed::Broken(_, me)) if me == new),
                "the new takeover is kept"
            );
        }

        #[cfg(unix)]
        #[test]
        fn a_pane_failing_after_another_moved_breaks_the_move_with_the_new_takeover() {
            let (old, new) = (
                takeover(1),
                Takeover {
                    token: Uuid::from_bytes([9; 16]),
                    ..takeover(1)
                },
            );
            let (first, second) = (takeover(2), takeover(3));
            let socket = scripted(move |listener| {
                let _control = welcome(listener, new);
                let _moved = welcome(listener, first);
                // The second pane's connection ends before its Welcome.
                drop(listener.accept().expect("accept"));
                std::thread::sleep(Duration::from_millis(200));
            });
            let resumes = vec![(block(2), first), (block(3), second)];
            let missed = take_over(socket, plan(old, None, resumes), &NEVER).err();
            assert!(
                matches!(missed, Some(Missed::Broken(_, me)) if me == new),
                "the new takeover is kept"
            );
        }

        /// A connection's two ends: the client's, and the server's to
        /// read what the client sent.
        #[cfg(unix)]
        fn ends() -> (UnixStream, UnixStream) {
            UnixStream::pair().expect("pair")
        }

        /// A block's connection past its Welcome, continuing `who`.
        #[cfg(unix)]
        fn opened(block: BlockId, who: Takeover) -> (Opened, UnixStream) {
            let (near, far) = ends();
            let opened = Opened {
                block,
                conn: Conn::new(near.try_clone().expect("clone")),
                writer: Conn::new(near.try_clone().expect("clone")),
                stream: near,
                takeover: Some(who),
                resumed: Some(who),
                features: PANE,
            };
            (opened, far)
        }

        /// What the client sent on a connection, until it goes quiet.
        #[cfg(unix)]
        fn sent(far: &UnixStream) -> Vec<Message> {
            far.set_read_timeout(Some(Duration::from_millis(200)))
                .expect("timeout");
            let mut conn = Conn::new(far.try_clone().expect("clone"));
            std::iter::from_fn(|| conn.recv().ok()).collect()
        }

        /// The bytes typed among `messages`, in order.
        #[cfg(unix)]
        fn typed_in(messages: &[Message]) -> Vec<Vec<u8>> {
            messages
                .iter()
                .filter_map(|m| match m {
                    Message::Input { data, .. } => Some(data.clone()),
                    _ => None,
                })
                .collect()
        }

        /// The path back, with blocks A and B taken over.
        #[cfg(unix)]
        fn back(me: Takeover) -> (Back, UnixStream, UnixStream, UnixStream) {
            let (stream, control) = ends();
            let (a, a_far) = opened(block(1), takeover(11));
            let (b, b_far) = opened(block(2), takeover(12));
            let back = Back {
                socket: PathBuf::from("udp.sock"),
                conn: Conn::new(stream.try_clone().expect("clone")),
                writer: Conn::new(stream.try_clone().expect("clone")),
                stream,
                me,
                next: 50,
                layout: None,
                panes: vec![a, b],
            };
            (back, control, a_far, b_far)
        }

        /// A client showing blocks A and B side by side over SSH, A focused
        /// and leased, with what the server's ends of A and B receive.
        #[cfg(unix)]
        fn app(rejoin: Option<Box<dyn Rejoin>>) -> (App, Inbox<Input>, [UnixStream; 3]) {
            let (tx, inbox) = channel();
            let (stream, control) = ends();
            let (a, b) = (block(1), block(2));
            let mut app = App {
                socket: PathBuf::from("ssh.sock"),
                rejoiner: rejoin.map(|r| Rejoiner::spawn(r, tx.clone())),
                beat: None,
                pulse: Pulse::default(),
                standby: idle(),
                held: Vec::new(),
                kept: None,
                me: Some(takeover(1)),
                lost: None,
                abandoned: None,
                resumes: HashMap::new(),
                tx,
                control: Conn::new(stream.try_clone().expect("clone")),
                control_stream: stream,
                reader: None,
                joined: 0,
                predicting: false,
                next: 1,
                inspect: false,
                layouts: true,
                window: None,
                layout: Some(Layout::Split {
                    axis: rusty_mux_core::Axis::Columns,
                    share: rusty_mux_core::Share::HALF,
                    first: Box::new(Layout::Block(a)),
                    second: Box::new(Layout::Block(b)),
                }),
                zoomed: None,
                focus: a,
                panes: HashMap::new(),
                keys: Keys::default(),
                area: Size::new(80, 24).expect("size"),
                shown: Shown::Nothing,
                split: None,
                arriving: None,
                opened: 0,
                mirrored: None,
                passed: None,
                no_win32: NoWin32Input::default(),
                pending: None,
                detaching: None,
                ended: None,
            };
            let (a_open, a_far) = opened(a, takeover(11));
            let (b_open, b_far) = opened(b, takeover(12));
            for opened in [a_open, b_open] {
                let pane = app.pane(opened);
                app.panes.insert(pane.block, pane);
            }
            app.panes.get_mut(&a).expect("A").leased = true;
            (app, inbox, [control, a_far, b_far])
        }

        /// Answers a block's lease request on its fresh connection, as the
        /// server would, and hands the answer to the client.
        #[cfg(unix)]
        fn grant(app: &mut App, inbox: &Inbox<Input>, far: &UnixStream, asked: &[Message]) {
            let req = asked
                .iter()
                .find_map(|m| match m {
                    Message::AcquireLease { req, .. } => Some(*req),
                    _ => None,
                })
                .expect("the lease asked for");
            let mut server = Conn::new(far.try_clone().expect("clone"));
            server
                .send(&Message::Done { req, created: None })
                .expect("done");
            loop {
                if let input @ Input::Pane(..) =
                    inbox.recv_timeout(Duration::from_secs(5)).expect("answer")
                {
                    return app.take(input);
                }
            }
        }

        /// Keys for A, a move of the focus to B, then keys for B, all typed
        /// while every client moves onto UDP: once it has, each block asks
        /// for its lease again; A gets its keys once it holds it, then gives
        /// it up, and B gets its keys once its lease is held, each exactly
        /// once; nothing went to the old connections.
        #[cfg(unix)]
        #[test]
        fn typing_and_focus_during_a_move_reach_their_blocks_once_it_is_done() {
            let (mut app, inbox, [_control, a_old, b_old]) = app(None);
            sent(&a_old); // A's lease and the like, before the move.
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(input(b"for-a"));
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"for-b"));
            assert_eq!(app.focus, block(1), "the focus moves once the move is done");

            let (back, _control, a_new, b_new) = back(takeover(1));
            app.take(Input::Moved(Ok(back)));
            assert!(typed_in(&sent(&a_old)).is_empty() && typed_in(&sent(&b_old)).is_empty());
            let asked = sent(&a_new);
            assert!(typed_in(&asked).is_empty(), "not before A holds the lease");
            grant(&mut app, &inbox, &a_new, &asked);
            let at_a = sent(&a_new);
            assert_eq!(typed_in(&at_a), [b"for-a".to_vec()]);
            assert!(
                at_a.iter()
                    .any(|m| matches!(m, Message::ReleaseLease { .. }))
            );
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty(), "not before B holds the lease");
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"for-b".to_vec()]);
            assert_eq!(app.focus, block(2));
        }

        /// The lease asked for on `asked`'s connection.
        #[cfg(unix)]
        fn lease_asked(asked: &[Message]) -> Option<u64> {
            asked.iter().find_map(|m| match m {
                Message::AcquireLease { req, .. } => Some(*req),
                _ => None,
            })
        }

        /// A block taken over on a path back asks for its lease again,
        /// as silence may have released it on the server meanwhile; keys
        /// typed until it holds it are kept, not refused by the server.
        #[cfg(unix)]
        #[test]
        fn a_block_taken_over_asks_for_its_lease_again() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.lose(false);
            let (back, _control, a_new, b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(back)));
            app.typed(input(b"typed"));
            let asked = sent(&a_new);
            assert!(
                lease_asked(&asked).is_some(),
                "the focused block asks again"
            );
            assert!(typed_in(&asked).is_empty(), "not before it holds the lease");
            assert!(
                lease_asked(&sent(&b_new)).is_none(),
                "not a block out of focus"
            );
            grant(&mut app, &inbox, &a_new, &asked);
            assert_eq!(typed_in(&sent(&a_new)), [b"typed".to_vec()]);
        }

        /// Whether `messages` give a lease back.
        #[cfg(unix)]
        fn released(messages: &[Message]) -> bool {
            messages
                .iter()
                .any(|m| matches!(m, Message::ReleaseLease { .. }))
        }

        /// A's lease, kept by the server through a short loss, is given
        /// back once the focus moved to B while the path was down; B's keys
        /// wait for B's own lease.
        #[cfg(unix)]
        #[test]
        fn a_lease_left_out_of_focus_through_a_loss_is_given_back() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.lose(false);
            app.typed(Action::Focus(Dir::Right));
            let (back, _control, a_new, b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(back)));
            assert_eq!(app.focus, block(2));
            app.typed(input(b"for-b"));
            let at_a = sent(&a_new);
            assert!(released(&at_a), "A's lease is given back");
            assert!(lease_asked(&at_a).is_none() && typed_in(&at_a).is_empty());
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty(), "not before B holds the lease");
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"for-b".to_vec()]);
        }

        /// Giving back a lease the client no longer holds (released by
        /// silence, or taken by another client) is refused by the server,
        /// which leaves any other holder be; the client asks once only.
        #[cfg(unix)]
        #[test]
        fn a_refused_give_back_is_not_repeated() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.lose(false);
            app.typed(Action::Focus(Dir::Right));
            let (back, _control, a_new, _b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(back)));
            let req = sent(&a_new)
                .iter()
                .find_map(|m| match m {
                    Message::ReleaseLease { req, .. } => Some(*req),
                    _ => None,
                })
                .expect("A's lease given back");
            let mut server = Conn::new(a_new.try_clone().expect("clone"));
            server
                .send(&Message::Failed {
                    req,
                    reason: "no lease".into(),
                    code: None,
                })
                .expect("failed");
            loop {
                if let input @ Input::Pane(..) =
                    inbox.recv_timeout(Duration::from_secs(5)).expect("answer")
                {
                    break app.take(input);
                }
            }
            app.typed(Action::Focus(Dir::Right));
            assert!(!released(&sent(&a_new)), "given back once");
            assert!(app.ended.is_none());
        }

        /// A lease taken by another client while the path was away is
        /// refused on the takeover: the keys kept for it are dropped, never
        /// sent for the server to refuse.
        #[cfg(unix)]
        #[test]
        fn a_lease_taken_meanwhile_drops_the_keys_kept_for_it() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.lose(false);
            let (back, _control, a_new, _b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(back)));
            app.typed(input(b"typed"));
            let req = lease_asked(&sent(&a_new)).expect("the lease asked for");
            let mut server = Conn::new(a_new.try_clone().expect("clone"));
            server
                .send(&Message::Failed {
                    req,
                    reason: "lease held".into(),
                    code: None,
                })
                .expect("failed");
            loop {
                if let input @ Input::Pane(..) =
                    inbox.recv_timeout(Duration::from_secs(5)).expect("answer")
                {
                    break app.take(input);
                }
            }
            app.typed(input(b"more"));
            assert!(typed_in(&sent(&a_new)).is_empty());
            assert!(app.ended.is_none());
        }

        /// A [`Rejoin`] that never brings a path back by itself.
        #[cfg(unix)]
        struct Never;

        #[cfg(unix)]
        impl Rejoin for Never {
            fn fresh(&mut self, _: bool, _: &AtomicBool) -> Result<PathBuf, String> {
                Err("not in this test".into())
            }
            fn listen(&mut self) {}
            fn settle(&mut self) {}
            fn discard(&mut self) {}
            fn datagram(&mut self, _: u16, _: [u8; 8], _: [u8; 32]) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn standby(&mut self) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn pulses(&self) -> Pulses {
                Pulses::default()
            }
        }

        /// Keys typed during a move that broke after the control client
        /// moved are kept through the lost path and sent once it is back,
        /// which takes the clients over with the control's new takeover;
        /// a focus change typed while it is down comes after them.
        #[cfg(unix)]
        #[test]
        fn keys_typed_during_a_broken_move_reach_their_block_once_the_path_is_back() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(input(b"kept"));
            let new = Takeover {
                token: Uuid::from_bytes([42; 16]),
                ..takeover(1)
            };
            app.take(Input::Moved(Err(Missed::Broken("pane".into(), new))));
            assert!(app.lost.is_some());
            assert_eq!(app.me, Some(new), "recovery asks with the new takeover");
            // Typed while the path is down: after the held keys, not before.
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"after"));
            assert_eq!(app.focus, block(1));

            let (back, _control, a_new, b_new) = back(new);
            app.take(Input::Rejoined(Ok(back)));
            assert!(typed_in(&sent(&a_old)).is_empty());
            let asked = sent(&a_new);
            assert!(typed_in(&asked).is_empty(), "not before A holds the lease");
            grant(&mut app, &inbox, &a_new, &asked);
            assert_eq!(typed_in(&sent(&a_new)), [b"kept".to_vec()]);
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty(), "not before B holds the lease");
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(
                typed_in(&sent(&b_new)),
                [b"after".to_vec()],
                "B never gets A's keys"
            );
            assert!(typed_in(&sent(&a_new)).is_empty(), "A's keys went once");
            assert!(app.held.is_empty());
        }

        /// Keys typed into B while its lease is asked for, when a move
        /// starts: not sent on B's old connection, even once the lease is
        /// granted there, but on its new one once it holds the lease.
        #[cfg(unix)]
        #[test]
        fn keys_waiting_on_a_lease_when_a_move_starts_reach_the_block() {
            let (mut app, inbox, [_control, a_old, b_old]) = app(Some(Box::new(Calls::default())));
            sent(&a_old);
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"early"));
            let asked = sent(&b_old);
            assert!(typed_in(&asked).is_empty(), "not before B holds the lease");
            app.move_onto(Onto::Standby);
            assert!(matches!(app.standby.moving, Moving::Taking(_)));
            // The lease comes on the old connection while the move is on.
            grant(&mut app, &inbox, &b_old, &asked);
            assert!(typed_in(&sent(&b_old)).is_empty());

            let (back, _control, _a_new, b_new) = back(takeover(1));
            app.take(Input::Moved(Ok(back)));
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty(), "not before B holds the lease");
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"early".to_vec()]);
        }

        /// Keys typed into B while its lease is asked for, when the path is
        /// lost: sent once the path is back and B holds the lease.
        #[cfg(unix)]
        #[test]
        fn keys_waiting_on_a_lease_when_the_path_is_lost_reach_the_block() {
            let (mut app, inbox, [_control, _a_old, b_old]) = app(Some(Box::new(Never)));
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"early"));
            sent(&b_old);
            app.lose(false);
            let (back, _control, _a_new, b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(back)));
            let at_b = sent(&b_new);
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"early".to_vec()]);
        }

        /// Keys waiting on B's lease when the path is lost, with B gone
        /// from the focus by the time it is back (another client zoomed A):
        /// dropped, never sent to A.
        #[cfg(unix)]
        #[test]
        fn keys_waiting_on_a_lease_never_reach_another_block() {
            let (mut app, _inbox, [_control, _a_old, b_old]) = app(Some(Box::new(Never)));
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"for-b"));
            sent(&b_old);
            app.lose(false);
            let (mut path, _control, a_new, b_new) = back(takeover(1));
            path.layout = Some((Some(Layout::Block(block(1))), None));
            app.take(Input::Rejoined(Ok(path)));
            assert_eq!(app.focus, block(1));
            assert!(typed_in(&sent(&a_new)).is_empty(), "not A's keys");
            assert!(typed_in(&sent(&b_new)).is_empty());
            assert!(app.kept.is_none());
        }

        /// Keys for A typed during a move, with A gone from the layout the
        /// move came back with (another client zoomed B): dropped, never
        /// sent to B, even once B holds its lease.
        #[cfg(unix)]
        #[test]
        fn keys_held_during_a_move_never_reach_another_block() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(None);
            sent(&a_old);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(input(b"for-a"));
            let (mut path, _control, a_new, b_new) = back(takeover(1));
            path.layout = Some((Some(Layout::Block(block(2))), None));
            app.take(Input::Moved(Ok(path)));
            assert_eq!(app.focus, block(2));
            assert!(typed_in(&sent(&a_new)).is_empty());
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty());
            grant(&mut app, &inbox, &b_new, &at_b);
            assert!(typed_in(&sent(&b_new)).is_empty(), "not A's keys");
        }

        /// The same, when the move broke and the path came back by recovery.
        #[cfg(unix)]
        #[test]
        fn keys_held_through_a_broken_move_never_reach_another_block() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(Some(Box::new(Never)));
            sent(&a_old);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(input(b"for-a"));
            app.take(Input::Moved(Err(Missed::Broken(
                "pane".into(),
                takeover(1),
            ))));
            let (mut path, _control, a_new, b_new) = back(takeover(1));
            path.layout = Some((Some(Layout::Block(block(2))), None));
            app.take(Input::Rejoined(Ok(path)));
            assert_eq!(app.focus, block(2));
            assert!(typed_in(&sent(&a_new)).is_empty());
            let at_b = sent(&b_new);
            grant(&mut app, &inbox, &b_new, &at_b);
            assert!(typed_in(&sent(&b_new)).is_empty(), "not A's keys");
            assert!(app.held.is_empty());
        }

        /// The `Sync` among `messages`.
        #[cfg(unix)]
        fn sync_in(messages: &[Message]) -> u64 {
            messages
                .iter()
                .find_map(|m| match m {
                    Message::Sync { req } => Some(*req),
                    _ => None,
                })
                .expect("a Sync")
        }

        /// Answers `req` on a block's connection, as the server would once
        /// what came before it is written, and hands it to the client.
        #[cfg(unix)]
        fn answer(app: &mut App, inbox: &Inbox<Input>, far: &UnixStream, req: u64) {
            let mut server = Conn::new(far.try_clone().expect("clone"));
            server
                .send(&Message::Done { req, created: None })
                .expect("done");
            loop {
                if let input @ Input::Pane(..) =
                    inbox.recv_timeout(Duration::from_secs(5)).expect("answer")
                {
                    return app.take(input);
                }
            }
        }

        /// Keys sent on SSH just before a move are written before any
        /// client is taken over: each block's old connection is asked for a
        /// `Sync` after them, and the move is asked for only once every one
        /// is answered; keys typed meanwhile are held.
        #[cfg(unix)]
        #[test]
        fn a_move_waits_for_keys_already_sent_to_be_written() {
            let (mut app, inbox, [_control, a_old, b_old]) = app(Some(Box::new(Calls::default())));
            sent(&a_old);
            app.typed(input(b"sent"));
            app.move_onto(Onto::Standby);
            app.typed(input(b"held"));
            let at_a = sent(&a_old);
            assert_eq!(typed_in(&at_a), [b"sent".to_vec()]);
            assert!(
                matches!(at_a.last(), Some(Message::Sync { .. })),
                "after the keys"
            );
            let at_b = sent(&b_old);

            answer(&mut app, &inbox, &a_old, sync_in(&at_a));
            assert!(
                app.standby.late(Instant::now() + HANDSHAKE),
                "still draining"
            );
            answer(&mut app, &inbox, &b_old, sync_in(&at_b));
            assert!(
                !app.standby.late(Instant::now() + HANDSHAKE),
                "the move is asked for"
            );
            assert!(matches!(app.standby.moving, Moving::Taking(_)));
            assert_eq!(actions(app.standby.held()), [&input(b"held")]);
        }

        /// A move whose blocks never answer is given up once the handshake
        /// time is past: what it held goes on the path in use, and the next
        /// try waits.
        #[cfg(unix)]
        #[test]
        fn a_move_that_cannot_drain_is_given_up_keeping_what_was_typed() {
            let (mut app, _inbox, [_control, a_old, _b_old]) =
                app(Some(Box::new(Calls::default())));
            sent(&a_old);
            app.move_onto(Onto::Standby);
            app.typed(input(b"held"));
            assert!(app.standby.late(Instant::now() + HANDSHAKE));
            app.give_up_move();
            assert!(matches!(app.standby.moving, Moving::Idle));
            assert_eq!(app.standby.tries, 1);
            assert_eq!(typed_in(&sent(&a_old)), [b"held".to_vec()]);
        }

        /// A path lost while a move drains loses it for real, keeping what
        /// the move held for the path brought back.
        #[cfg(unix)]
        #[test]
        fn a_loss_while_a_move_drains_keeps_what_it_held() {
            let (mut app, _inbox, _ends) = app(Some(Box::new(Never)));
            app.move_onto(Onto::Standby);
            app.typed(input(b"held"));
            app.lose(false);
            assert!(app.lost.is_some());
            assert!(matches!(app.standby.moving, Moving::Idle));
            assert_eq!(actions(&app.held), [&input(b"held")]);
        }

        /// Focus to B, keys for B and Ctrl-] d, all held during a move:
        /// once it is done, the detach waits for B's lease so the keys are
        /// sent first, exactly once; nothing typed after it is sent.
        #[cfg(unix)]
        #[test]
        fn a_held_detach_waits_for_keys_waiting_on_a_lease() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(None);
            sent(&a_old);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"before"));
            app.typed(Action::Detach);
            let (path, _control, _a_new, b_new) = back(takeover(1));
            app.take(Input::Moved(Ok(path)));
            assert!(app.ended.is_none(), "not before B's keys are sent");
            app.typed(input(b"after"));
            let at_b = sent(&b_new);
            assert!(typed_in(&at_b).is_empty());
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"before".to_vec()]);
            assert!(matches!(app.ended, Some(Ended::Detached)));
        }

        /// A detach waiting on a lease that is refused, or never answered,
        /// still ends: the keys are dropped, as the server would refuse them.
        #[cfg(unix)]
        #[test]
        fn a_detach_waiting_on_a_lease_ends_on_refusal_or_in_time() {
            let (mut app, inbox, [_control, _a_old, b_old]) = app(None);
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"before"));
            app.typed(Action::Detach);
            assert!(app.ended.is_none());
            let req = sent(&b_old)
                .iter()
                .find_map(|m| match m {
                    Message::AcquireLease { req, .. } => Some(*req),
                    _ => None,
                })
                .expect("B's lease asked for");
            let mut server = Conn::new(b_old.try_clone().expect("clone"));
            server
                .send(&Message::Failed {
                    req,
                    reason: "held elsewhere".into(),
                    code: None,
                })
                .expect("failed");
            loop {
                if let input @ Input::Pane(..) =
                    inbox.recv_timeout(Duration::from_secs(5)).expect("answer")
                {
                    app.take(input);
                    break;
                }
            }
            assert!(typed_in(&sent(&b_old)).is_empty());
            assert!(matches!(app.ended, Some(Ended::Detached)));

            let (mut late, _inbox, _ends) = super::tests::app(None);
            late.typed(Action::Focus(Dir::Right));
            late.typed(input(b"before"));
            late.typed(Action::Detach);
            late.detach_when_sent(Instant::now());
            assert!(late.ended.is_none());
            late.detach_when_sent(Instant::now() + HANDSHAKE);
            assert!(matches!(late.ended, Some(Ended::Detached)));
        }

        /// A detach waiting on B's lease starts no move, even on an offer
        /// already asked for: B's keys stay where they are, to be sent.
        #[cfg(unix)]
        #[test]
        fn a_detach_waiting_on_a_lease_starts_no_move() {
            let (mut app, inbox, [_control, _a_old, b_old]) = app(Some(Box::new(Calls::default())));
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"before"));
            app.typed(Action::Detach);
            app.offered(Some(Onto::Standby));
            assert!(matches!(app.standby.moving, Moving::Idle));
            app.detach_when_sent(Instant::now());
            assert!(app.ended.is_none(), "B's keys are not sent yet");
            let asked = sent(&b_old);
            assert!(!asked.iter().any(|m| matches!(m, Message::Sync { .. })));
            grant(&mut app, &inbox, &b_old, &asked);
            assert_eq!(typed_in(&sent(&b_old)), [b"before".to_vec()]);
            assert!(matches!(app.ended, Some(Ended::Detached)));
        }

        /// A detach waiting on B's lease when the path is lost waits, within
        /// its time, for the path back: B's keys are sent first.
        #[cfg(unix)]
        #[test]
        fn a_detach_waiting_on_a_lease_through_a_loss_sends_the_keys_first() {
            let (mut app, inbox, [_control, _a_old, b_old]) = app(Some(Box::new(Never)));
            app.typed(Action::Focus(Dir::Right));
            app.typed(input(b"before"));
            app.typed(Action::Detach);
            sent(&b_old);
            app.lose(false);
            app.detach_when_sent(Instant::now());
            assert!(app.ended.is_none(), "the keys are kept, not sent");
            let (path, _control, _a_new, b_new) = back(takeover(1));
            app.take(Input::Rejoined(Ok(path)));
            let at_b = sent(&b_new);
            grant(&mut app, &inbox, &b_new, &at_b);
            assert_eq!(typed_in(&sent(&b_new)), [b"before".to_vec()]);
            assert!(matches!(app.ended, Some(Ended::Detached)));
        }

        /// A second Ctrl-] d while a detach held through a broken move
        /// waits on its one attempt is ignored: the path still comes back
        /// to detach the moved clients there.
        #[cfg(unix)]
        #[test]
        fn a_second_detach_waits_for_the_one_held_through_a_broken_move() {
            let asked = Arc::new(Mutex::new(Vec::new()));
            let (mut app, inbox, _ends) = app(Some(Box::new(Keeps(asked.clone()))));
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(Action::Detach);
            app.take(Input::Moved(Err(Missed::Broken(
                "pane".into(),
                takeover(1),
            ))));
            inbox.recv_timeout(Duration::from_secs(5)).expect("a try");
            app.typed(Action::Detach);
            app.typed(input(b"x"));
            assert!(app.ended.is_none(), "still waiting on the attempt");
            assert_eq!(actions(&app.held), [&Action::Detach]);
            assert!(
                inbox.recv_timeout(Duration::from_millis(300)).is_err(),
                "no second attempt"
            );
            assert_eq!(asked.lock().expect("lock").len(), 1);
        }

        /// A [`Rejoin`] that records whether each fresh path was to try the
        /// forward in use, and brings none back.
        #[cfg(unix)]
        struct Keeps(Arc<Mutex<Vec<bool>>>);

        #[cfg(unix)]
        impl Rejoin for Keeps {
            fn fresh(&mut self, keep: bool, _: &AtomicBool) -> Result<PathBuf, String> {
                self.0.lock().expect("lock").push(keep);
                Err("not in this test".into())
            }
            fn listen(&mut self) {}
            fn settle(&mut self) {}
            fn discard(&mut self) {}
            fn datagram(&mut self, _: u16, _: [u8; 8], _: [u8; 32]) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn standby(&mut self) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn pulses(&self) -> Pulses {
                Pulses::default()
            }
        }

        /// A move that broke after the control client moved leaves the SSH
        /// forward sound: it is tried first, and a new login only after
        /// that fails. A plain loss goes straight to a new login.
        #[cfg(unix)]
        #[test]
        fn a_broken_move_tries_the_forward_in_use_once_before_a_new_login() {
            let asked = Arc::new(Mutex::new(Vec::new()));
            let (mut app, inbox, _ends) = app(Some(Box::new(Keeps(asked.clone()))));
            app.standby.moving = Moving::Taking(Taking::default());
            app.take(Input::Moved(Err(Missed::Broken(
                "pane".into(),
                takeover(1),
            ))));
            let failed = inbox
                .recv_timeout(Duration::from_secs(5))
                .expect("first try");
            app.take(failed);
            app.lost.as_mut().expect("lost").retry = Some(Instant::now());
            app.beat();
            inbox
                .recv_timeout(Duration::from_secs(5))
                .expect("second try");
            assert_eq!(*asked.lock().expect("lock"), [true, false]);

            let plain = Arc::new(Mutex::new(Vec::new()));
            let (mut other, inbox, _ends) = super::tests::app(Some(Box::new(Keeps(plain.clone()))));
            other.lose(false);
            inbox.recv_timeout(Duration::from_secs(5)).expect("a try");
            assert_eq!(*plain.lock().expect("lock"), [false]);
        }

        /// Ctrl-] d during a move waits for it, so the clients it took over
        /// are detached on their new connections rather than orphaned; with
        /// the path lost after a broken move, the client stops at once.
        #[cfg(unix)]
        #[test]
        fn a_detach_during_a_move_is_taken_on_the_path_the_clients_end_up_on() {
            let (mut app, _inbox, _ends) = app(None);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(Action::Detach);
            assert!(app.ended.is_none(), "held until the move is over");
            let (back, _control, _a_new, _b_new) = back(takeover(1));
            app.take(Input::Moved(Ok(back)));
            assert!(matches!(app.ended, Some(Ended::Detached)));
            assert_eq!(
                app.joined, 1,
                "on the new path, whose panes are detached on exit"
            );

            // Broken after the control moved: the path is asked back on the
            // forward in use, to detach there; brought back, it detaches.
            let asked = Arc::new(Mutex::new(Vec::new()));
            let (mut lost, inbox, _ends) = super::tests::app(Some(Box::new(Keeps(asked.clone()))));
            lost.standby.moving = Moving::Taking(Taking::default());
            lost.typed(Action::Detach);
            lost.take(Input::Moved(Err(Missed::Broken(
                "pane".into(),
                takeover(1),
            ))));
            assert!(lost.ended.is_none() && lost.lost.is_some());
            inbox.recv_timeout(Duration::from_secs(5)).expect("a try");
            assert_eq!(*asked.lock().expect("lock"), [true]);
            let (path, _control, _a_new, _b_new) = super::tests::back(takeover(1));
            lost.take(Input::Rejoined(Ok(path)));
            assert!(matches!(lost.ended, Some(Ended::Detached)));
            assert_eq!(lost.joined, 1, "on the path brought back");

            // If that one attempt fails, the client stops anyway.
            let (mut gone, _inbox, _ends) = super::tests::app(Some(Box::new(Never)));
            gone.standby.moving = Moving::Taking(Taking::default());
            gone.typed(Action::Detach);
            gone.take(Input::Moved(Err(Missed::Broken(
                "pane".into(),
                takeover(1),
            ))));
            gone.take(Input::Rejoined(Err(Missed::Retry("down".into()))));
            assert!(matches!(gone.ended, Some(Ended::Detached)));
        }

        /// Keys typed after Ctrl-] d during a move are meant for the local
        /// shell the client returns to: never sent, whether the move goes
        /// through or not.
        #[cfg(unix)]
        #[test]
        fn keys_typed_after_a_detach_during_a_move_are_not_sent() {
            let (mut app, inbox, [_control, a_old, _b_old]) = app(None);
            sent(&a_old);
            app.standby.moving = Moving::Taking(Taking::default());
            app.typed(input(b"before"));
            app.typed(Action::Detach);
            app.typed(input(b"after"));
            let (back, _control, a_new, _b_new) = back(takeover(1));
            app.take(Input::Moved(Ok(back)));
            let asked = sent(&a_new);
            assert!(app.ended.is_none(), "the detach waits for the lease");
            grant(&mut app, &inbox, &a_new, &asked);
            assert_eq!(typed_in(&sent(&a_new)), [b"before".to_vec()]);
            assert!(matches!(app.ended, Some(Ended::Detached)));

            let (mut stay, _inbox, [_control, a_old, _b_old]) = super::tests::app(None);
            sent(&a_old);
            stay.standby.moving = Moving::Taking(Taking::default());
            stay.typed(Action::Detach);
            stay.typed(input(b"after"));
            stay.take(Input::Moved(Err(Missed::Retry("no".into()))));
            assert!(typed_in(&sent(&a_old)).is_empty());
            assert!(matches!(stay.ended, Some(Ended::Detached)));
        }

        /// A [`Rejoin`] whose UDP path is the socket given.
        #[cfg(unix)]
        struct Udp(PathBuf);

        #[cfg(unix)]
        impl Rejoin for Udp {
            fn fresh(&mut self, _: bool, _: &AtomicBool) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn listen(&mut self) {}
            fn settle(&mut self) {}
            fn discard(&mut self) {}
            fn datagram(&mut self, _: u16, _: [u8; 8], _: [u8; 32]) -> Result<PathBuf, String> {
                Ok(self.0.clone())
            }
            fn standby(&mut self) -> Result<PathBuf, String> {
                unreachable!("not asked here")
            }
            fn pulses(&self) -> Pulses {
                Pulses::default()
            }
        }

        #[cfg(unix)]
        #[test]
        fn a_failed_move_onto_udp_at_start_says_the_client_is_back_on_ssh() {
            let me = takeover(1);
            // The probe answered, but the handoff over UDP fails.
            let udp = scripted(|listener| drop(listener.accept().expect("accept")));
            let ssh = scripted(move |listener| {
                let _back = welcome(listener, me);
                std::thread::sleep(Duration::from_millis(200));
            });
            let (near, far) = UnixStream::pair().expect("pair");
            std::thread::spawn(move || {
                let mut server = Conn::new(far);
                let Message::OpenDatagram { req } = server.recv().expect("ask") else {
                    panic!("expected OpenDatagram");
                };
                let key = rusty_mux_proto::DatagramKey([5; 32]);
                server
                    .send(&Message::Datagram {
                        req,
                        port: 1,
                        id: [6; 8],
                        key,
                    })
                    .expect("offer");
            });
            let mut conn = Conn::new(near);
            let upgraded = upgrade(&mut Udp(udp), &ssh, &mut conn, me, 1);
            assert!(
                matches!(upgraded, Ok(Upgraded::Ssh(_))),
                "asked again later"
            );
        }

        #[test]
        fn a_move_onto_standby_is_listened_to_before_the_main_loop_hears() {
            let mut calls = Calls::default();
            let path = calls.standby();
            let back = bring_back(&mut calls, path, |_| Ok(()));
            assert!(back.is_ok());
            assert_eq!(calls.0, ["standby", "listen"]);
        }
    }
}
