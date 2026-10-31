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
        /// `cancel` is set; returns the socket to connect to.
        fn fresh(&mut self, cancel: &AtomicBool) -> Result<PathBuf, String>;
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
        /// Bring up a fresh path and take these clients over on it.
        Fresh(Plan),
        /// Move these clients back onto the UDP path kept in standby.
        Move(Plan),
        Settle,
    }

    /// The clients a fresh path is to take over.
    struct Plan {
        me: Takeover,
        next: u64,
        /// The window to ask the layout of, when the server answers List.
        window: Option<WindowId>,
        resumes: Vec<(BlockId, Resume)>,
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
        /// After the control connection's client was taken over: hanging
        /// the fresh connection up ends that client, so there is nothing
        /// left to continue.
        Broken(String),
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
                        Ask::Fresh(plan) => (rejoin.fresh(&cancelled), plan, Input::Rejoined),
                        Ask::Move(plan) => (rejoin.standby(), plan, Input::Moved),
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
    }

    /// How to continue a block's connection after the path comes back.
    #[derive(Clone, Copy)]
    struct Resume {
        takeover: Takeover,
        leased: bool,
    }

    /// The UDP path given up for SSH, moved back onto once it is heard
    /// again (UDP-TRANSPORT.md).
    struct Standby {
        /// When it last heard the server.
        pulse: Pulse,
        /// A move is under way: the old path's connections end as their
        /// clients are taken over, and that is no loss.
        moving: bool,
        /// Failed moves since the last one that worked.
        tries: u32,
        /// No move before this.
        next: Instant,
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
        // Held for its `Drop`, which stops the forward.
        Ssh(#[expect(dead_code)] rusty_mux_tui::ssh::Forward),
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
        /// The association in use, if it answers a probe; otherwise a new
        /// SSH forward.
        fn fresh(&mut self, cancel: &AtomicBool) -> Result<PathBuf, String> {
            use rusty_mux_tui::ssh::{Forward, Login};

            self.discard();
            if let Route::Udp(relay) = &self.current
                && relay.probe(PROBE)
            {
                return Ok(relay.socket().to_owned());
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
        if let (Some(rejoin), Some(me)) = (rejoin.as_mut(), takeover)
            && features.contains(Features::DATAGRAM)
            && udp_wanted()
            && let Some(back) = upgrade(rejoin.as_mut(), &socket, &mut conn, me, next)?
        {
            (socket, stream, conn) = (back.socket, back.stream, back.conn);
            (takeover, next) = (Some(back.me), back.next);
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
                moving: false,
                tries: 0,
                next: now,
            },
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
    ) -> Result<Option<Back>, String> {
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
                return Ok(None);
            }
        };
        let missed = match take_over(socket, plan(), &NEVER) {
            Ok(back) => {
                rejoin.settle();
                return Ok(Some(back));
            }
            Err(missed) => missed,
        };
        rejoin.discard();
        let why = match missed {
            Missed::Retry(why) | Missed::Broken(why) => why,
            Missed::Dropped => return Err("the server let this client go".into()),
        };
        eprintln!("rusty-mux: staying on SSH: {why}");
        match take_over(ssh.to_owned(), plan(), &NEVER) {
            Ok(back) => Ok(Some(back)),
            Err(Missed::Dropped) => Err("the server let this client go".into()),
            Err(Missed::Retry(why) | Missed::Broken(why)) => Err(why),
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
                    .map_err(Missed::Broken)?;
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
                    Missed::Retry(why) | Missed::Broken(why) => Missed::Broken(why),
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
        resumed: Option<Resume>,
        features: Features,
    }

    /// Connects to `socket` for `block`, taking `resume`'s client over if
    /// given, up to the Welcome, due `by`.
    fn reach(
        socket: &Path,
        block: BlockId,
        resume: Option<Resume>,
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
            takeover: resume.map(|r| r.takeover),
        })
        .map_err(|e| retry(e.to_string()))?;
        let welcomed = welcomed(answer(&mut conn, by, cancel).map_err(retry)?).map_err(retry)?;
        match (resume, welcomed.takeover) {
            (None, _) => {}
            (Some(r), Some(t)) if t.client == r.takeover.client => {}
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
        leased: bool,
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
                    return Ok(Change::Leased);
                }
                Message::Failed { req, reason, .. } if req == self.attach => {
                    return Ok(Change::Refused(reason));
                }
                Message::Failed { req, .. } => {
                    // A refused lease, or input after losing it.
                    if self.asking == Some(req) {
                        self.asking = None;
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
        /// How to take the control connection's client over.
        me: Option<Takeover>,
        lost: Option<Lost>,
        /// The lost path's connections, until their clients are taken over.
        abandoned: Option<Abandoned>,
        /// The blocks' connections to take over once the path is back.
        resumes: HashMap<BlockId, Resume>,
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
                        self.act(action);
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
                Input::ControlEnded(_) if !self.panes.is_empty() || self.standby.moving => {}
                Input::ControlEnded(_) => self.broke(Ended::Disconnected),
                Input::Pane(tag, message) => self.on_pane(tag, message),
                // Only asked for while lost.
                Input::Rejoined(_) if self.lost.is_none() => {}
                Input::Rejoined(Ok(back)) => self.rejoined(back),
                Input::Rejoined(Err(Missed::Retry(why))) => self.retry(why),
                Input::Rejoined(Err(Missed::Dropped)) => self.end(Ended::Dropped),
                Input::Rejoined(Err(Missed::Broken(why))) => {
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
                        // Taken over by the move under way.
                        false if self.standby.moving => {}
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
                true => self.lose(),
                false => self.end(ended),
            }
        }

        /// Asks for a sign of life every [`BEAT`], and counts the path lost
        /// after [`DEAD`] without one, or after [`SILENT`] on a path that
        /// hears the server's keepalives; brings a lost one back when due.
        fn beat(&mut self) {
            let now = Instant::now();
            if let Some(lost) = self.lost.as_mut() {
                if lost.retry.is_some_and(|at| now >= at) {
                    lost.retry = None;
                    self.ask_back();
                }
                return;
            }
            let Some(beat) = self.beat.as_mut() else {
                return;
            };
            let silent = self
                .pulse
                .last()
                .is_some_and(|heard| now.saturating_duration_since(heard) >= SILENT);
            if silent || now.duration_since(beat.heard) >= DEAD {
                self.lose();
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
        fn move_back(&mut self, now: Instant) {
            let standby = &mut self.standby;
            let heard = standby
                .pulse
                .last()
                .is_some_and(|heard| now.saturating_duration_since(heard) < SILENT);
            if standby.moving || now < standby.next || !heard {
                return;
            }
            let (Some(rejoiner), Some(me)) = (&self.rejoiner, self.me) else {
                return;
            };
            let resumes: Option<Vec<_>> = self
                .panes
                .values()
                .map(|pane| {
                    let takeover = pane.takeover?;
                    let leased = pane.leased;
                    Some((pane.block, Resume { takeover, leased }))
                })
                .collect();
            // A block that cannot be taken over would be left on SSH.
            let Some(resumes) = resumes else {
                return;
            };
            standby.moving = true;
            rejoiner.ask(Ask::Move(Plan {
                me,
                next: self.next,
                window: self.window.filter(|_| self.inspect),
                resumes,
            }));
        }

        /// A move back onto UDP is done or failed. Failing before any
        /// client moved leaves the client on SSH, to try again later, at
        /// most [`BACKOFF`] apart; failing after leaves it a lost path.
        fn moved(&mut self, moved: Result<Back, Missed>) {
            self.standby.moving = false;
            match moved {
                Ok(back) => {
                    self.standby.tries = 0;
                    self.rejoined(back);
                }
                // Nothing moved; the SSH path is still the one in use.
                Err(Missed::Retry(_)) => {
                    let standby = &mut self.standby;
                    standby.tries += 1;
                    let wait = Duration::from_secs(1 << standby.tries.min(5)).min(BACKOFF);
                    standby.next = Instant::now() + wait;
                }
                Err(Missed::Broken(_)) => self.lose(),
                Err(Missed::Dropped) => self.end(Ended::Dropped),
            }
        }

        /// Stops listening to every connection, keeping what it takes to
        /// continue each, and asks for the path back.
        fn lose(&mut self) {
            if self.lost.is_some() {
                return;
            }
            let panes: Vec<Pane> = self.panes.drain().map(|(_, pane)| pane).collect();
            self.resumes = panes
                .iter()
                .filter_map(|pane| {
                    let takeover = pane.takeover?;
                    let leased = pane.leased;
                    Some((pane.block, Resume { takeover, leased }))
                })
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
            });
            self.ask_back();
        }

        /// Asks for a fresh path, with every client to take over on it.
        fn ask_back(&mut self) {
            let (Some(rejoiner), Some(me)) = (&self.rejoiner, self.me) else {
                self.end(Ended::Dropped);
                return;
            };
            rejoiner.ask(Ask::Fresh(Plan {
                me,
                next: self.next,
                window: self.window.filter(|_| self.inspect),
                resumes: self.resumes.iter().map(|(b, r)| (*b, *r)).collect(),
            }));
        }

        /// An attempt to bring the path back failed: try again later, at
        /// most [`BACKOFF`] apart. Ctrl-] d still detaches meanwhile.
        fn retry(&mut self, why: String) {
            let Some(lost) = self.lost.as_mut() else {
                return;
            };
            lost.tries += 1;
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

        fn act(&mut self, action: Action) {
            let focus = self.focus;
            match action {
                Action::Input(data) => {
                    if let Some(pane) = self.panes.get_mut(&focus).filter(|p| p.leased) {
                        pane.type_in(data);
                    }
                }
                Action::Detach => self.end(Ended::Detached),
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
                    }
                    false if pane.leased => {
                        let block = *block;
                        pane.send(|req| Message::ReleaseLease { req, block });
                        pane.leased = false;
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
                Err(Missed::Retry(why) | Missed::Broken(why)) => Err(Ended::Failed(why)),
                Err(Missed::Dropped) => Err(Ended::Dropped),
            }
        }

        /// Listens to a block's connection. One that continues a client is
        /// still attached and leased as it was, and the server sends the
        /// screen again; any other attaches.
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
                leased: opened.resumed.is_some_and(|r| r.leased),
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

        /// Records what a [`Rejoin`] is asked, in order.
        #[derive(Default)]
        struct Calls(Vec<&'static str>);

        impl Rejoin for Calls {
            fn fresh(&mut self, _: &AtomicBool) -> Result<PathBuf, String> {
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
            let path = calls.fresh(&AtomicBool::new(false));
            let back = bring_back(&mut calls, path, |_| Ok(()));
            assert!(back.is_ok());
            // `settle` is left to the main loop; the pulse moved already.
            assert_eq!(calls.0, ["fresh", "listen"]);
        }

        #[test]
        fn a_path_not_brought_back_is_discarded_and_not_listened_to() {
            let mut calls = Calls::default();
            let path = calls.fresh(&AtomicBool::new(false));
            let back = bring_back(&mut calls, path, |_| {
                Err::<(), _>(Missed::Dropped)
            });
            assert!(matches!(back, Err(Missed::Dropped)));
            assert_eq!(calls.0, ["fresh", "discard"]);
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
