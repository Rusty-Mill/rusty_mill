//! `rusty-mux <socket> [block]`: show a rusty_mux block's window on this
//! terminal, starting with that block focused. Without a block id it
//! creates a session, window and block first. Ctrl-] then `d` detaches;
//! the blocks keep running and the focused one's id is printed for
//! reattaching. The other Ctrl-] commands are listed in
//! `rusty_mux_tui::keys`. With `RUSTY_MUX_KEYLOG` set to a new file's path,
//! every keyboard read is written to it as one line of hex, to see what a
//! terminal really sends. That is everything typed, passwords included:
//! the file is created owner-only on Unix (on Windows it takes its
//! directory's permissions), and an existing file is refused.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args.len() > 2 {
        eprintln!("usage: rusty-mux <socket> [block]");
        return ExitCode::from(2);
    }
    match client::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rusty-mux: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod client {
    pub fn run(_args: &[std::ffi::OsString]) -> Result<(), String> {
        Err("the client runs on Linux and Windows only".into())
    }
}

#[cfg(any(target_os = "linux", windows))]
mod client {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io::Write;
    use std::net::Shutdown;
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver as Inbox, RecvTimeoutError, Sender};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use platform::term::Terminal;
    #[cfg(target_os = "linux")]
    use platform_linux::LinuxTerminal as Term;
    #[cfg(windows)]
    use platform_windows::{WindowsTerminal as Term, WindowsUnixStream as UnixStream};
    use rusty_mux_core::{BlockId, Event, Layout, Rect, SessionId, Side, Size, WindowId};
    use rusty_mux_proto::{Entry, Features, Launch, Message};
    use rusty_mux_server::checkpoint::Receiver;
    use rusty_mux_tui::keys::{Action, Keys};
    use rusty_mux_tui::screen::{self, Frame, View};
    use rusty_mux_tui::{Conn, NoWin32Input, RESTORE, modes, pass};
    use rusty_term_core::{AnsiParser, Grid};
    use rusty_uuid::Uuid;

    /// How often the loop wakes with nothing to do, to notice the local
    /// terminal changing size.
    const TICK: Duration = Duration::from_millis(100);

    /// What the control connection asks for; a server without `LAYOUT`
    /// shows only the block asked for.
    const CONTROL: Features = Features::CHECKPOINT_V5
        .union(Features::INSPECT)
        .union(Features::LAYOUT);

    /// How long a layout change that drops shown blocks waits for their own
    /// close, so their last output shows first.
    const GRACE: Duration = Duration::from_millis(500);

    /// A block's connection: the block, and which of the connections the
    /// client opened for it, so a stale one's messages are told apart.
    type Tag = (BlockId, u64);

    /// Something for the main loop.
    enum Input {
        Keys(Vec<u8>),
        Control(Message),
        Pane(Tag, Message),
        /// A connection ended: the server's control one (`None`), or a
        /// block's.
        Ended(Option<Tag>),
    }

    /// Why the client stopped.
    enum Ended {
        Detached,
        /// The window's last block closed, or the window did.
        Closed,
        /// The server went away.
        Disconnected,
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

    pub fn run(args: &[OsString]) -> Result<(), String> {
        let socket = PathBuf::from(&args[0]);
        let stream = connect(&socket)?;
        let mut conn = Conn::new(clone(&stream)?);
        let mut term = Term::new();
        let classic = Size::new(80, 24).map_err(|e| e.to_string())?;
        let area = window_size(&term, classic);
        conn.send(&Message::Hello {
            features: CONTROL,
            resume: None,
        })
        .map_err(|e| e.to_string())?;
        let mut req = Requests {
            conn: &mut conn,
            next: 1,
        };
        let features = req.welcome()?;
        let block = match args.get(1) {
            Some(id) => BlockId::from_uuid(parse_uuid(id)?),
            None => req.create(area)?,
        };
        let (window, layout, zoomed) = match features.contains(Features::INSPECT) {
            true => req.place(block)?,
            false => (None, Layout::Block(block), None),
        };
        let next = req.next;

        let (tx, inbox) = std::sync::mpsc::channel();
        let control = {
            let tx = tx.clone();
            std::thread::spawn(move || read(conn, &tx, Input::Control, None))
        };
        let keylog = keylog()?;
        term.enter_raw().map_err(|e| format!("raw mode: {e:?}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let keyboard = keyboard(tx.clone(), stop.clone(), keylog);
        let mut app = App {
            socket,
            tx,
            control: Conn::new(clone(&stream)?),
            next,
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
            no_win32: NoWin32Input::default(),
            pending: None,
            ended: None,
        };
        app.sync();
        let ended = app.serve(&term, &inbox);
        let focus = app.focus;
        // Stop every reader before touching the terminal, so nothing a
        // block prints lands after the restore. Shutting a socket ends
        // its read; an error only means the server already closed it.
        stop.store(true, Ordering::Release);
        let _ = stream.shutdown(Shutdown::Both);
        app.panes.values().for_each(Pane::hang_up);
        drop(app);
        let _ = (control.join(), keyboard.join());
        restore(&mut term);
        match ended {
            Ended::Closed => println!("\r\nrusty-mux: block {focus} closed"),
            Ended::Detached => println!("\r\nrusty-mux: detached from block {focus}"),
            Ended::Disconnected => {
                println!("\r\nrusty-mux: lost the server; block {focus} may still be running")
            }
            Ended::Stdout => eprintln!("rusty-mux: the terminal stopped taking output"),
            Ended::Failed(why) => return Err(why),
        }
        Ok(())
    }

    /// The local terminal's size, or `fallback` when it has none: a pty
    /// nobody sized reports 0x0.
    fn window_size(term: &Term, fallback: Size) -> Size {
        term.window_size()
            .ok()
            .and_then(|w| Size::new(w.cols, w.rows).ok())
            .unwrap_or(fallback)
    }

    /// Reads messages from `conn` into the main loop until it ends.
    fn read(
        mut conn: Conn<UnixStream>,
        tx: &Sender<Input>,
        wrap: impl Fn(Message) -> Input,
        pane: Option<Tag>,
    ) {
        while let Ok(message) = conn.recv() {
            if tx.send(wrap(message)).is_err() {
                return;
            }
        }
        let _ = tx.send(Input::Ended(pane));
    }

    /// The file `RUSTY_MUX_KEYLOG` names, created for the log; `None` when
    /// it is unset. It must not exist yet, so the keys never go to a file
    /// someone else may read; on Unix it is created owner-only.
    fn keylog() -> Result<Option<std::fs::File>, String> {
        let Some(path) = std::env::var_os("RUSTY_MUX_KEYLOG") else {
            return Ok(None);
        };
        let path = Path::new(&path);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options
            .open(path)
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

    /// Requests answered before the main loop starts.
    struct Requests<'a> {
        conn: &'a mut Conn<UnixStream>,
        next: u64,
    }

    impl Requests<'_> {
        fn welcome(&mut self) -> Result<Features, String> {
            match self.conn.recv().map_err(|e| e.to_string())? {
                Message::Welcome { features, .. } if features.contains(Features::CHECKPOINT_V5) => {
                    Ok(features)
                }
                Message::Welcome { .. } => Err("the server cannot send checkpoints".into()),
                other => Err(format!("expected Welcome, got {other:?}")),
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
                let message = self.conn.recv().map_err(|e| e.to_string())?;
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
            let (layout, zoomed) = entries
                .into_iter()
                .find_map(|m| match m {
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
                .unwrap_or_default();
            Ok((Some(window), layout.unwrap_or(Layout::Block(block)), zoomed))
        }
    }

    /// One block's connection and its local copy of the block's terminal.
    struct Pane {
        /// Which connection to its block this is.
        generation: u64,
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
    }

    impl Pane {
        fn send(&mut self, make: impl FnOnce(u64) -> Message) -> u64 {
            let req = self.next;
            self.next += 1;
            // A failed send ends the reader too, which reports it.
            let _ = self.conn.send(&make(req));
            req
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
                    return Ok(Change::Ready);
                }
                Message::Output { block: b, data } if b == block => {
                    if let Some((grid, parser)) = self.term.as_mut() {
                        parser.advance(grid, &data);
                    }
                    return Ok(Change::Output(data));
                }
                Message::Event {
                    event: Event::BlockResized { block: b, size },
                    ..
                } if b == block => {
                    if let Some((grid, _)) = self.term.as_mut() {
                        grid.resize(size.cols().into(), size.rows().into());
                    }
                    self.sizing = None;
                }
                Message::Event {
                    event: Event::BlockClosed { block: b },
                    ..
                } if b == block => return Ok(Change::Closed),
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
        tx: Sender<Input>,
        control: Conn<UnixStream>,
        next: u64,
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
                Input::Control(message) => self.on_control(message),
                Input::Pane(tag, message) => self.on_pane(tag, message),
                // The server closes every connection when it goes; a
                // block's own is left to report it, after its last output.
                Input::Ended(None) if !self.panes.is_empty() => {}
                Input::Ended(None) => self.end(Ended::Disconnected),
                Input::Ended(Some(tag)) if self.current(tag) => {
                    let block = tag.0;
                    let leaving = self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.leaving.contains(&block));
                    let closed = self.panes.get(&block).is_some_and(|p| p.closed);
                    match leaving || closed {
                        true => self.drained(block),
                        // A block this client still shows lost its connection.
                        false => self.end(Ended::Disconnected),
                    }
                }
                // A connection this client already hung up on.
                Input::Ended(Some(_)) => {}
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
            let req = self.next;
            self.next += 1;
            if self.control.send(&make(req)).is_err() {
                self.end(Ended::Disconnected);
            }
            req
        }

        fn act(&mut self, action: Action) {
            let focus = self.focus;
            match action {
                Action::Input(data) => {
                    if let Some(pane) = self.panes.get_mut(&focus).filter(|p| p.leased) {
                        pane.send(|req| Message::Input {
                            req,
                            block: focus,
                            data,
                        });
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
                        }
                    }
                    self.fit();
                }
                Ok(Change::Output(data)) => {
                    if self.shown == Shown::Passing(block) {
                        let bytes = self.no_win32.filter(&data);
                        write(&bytes, &mut self.ended);
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
            for block in gone {
                if let Some(pane) = self.panes.remove(&block) {
                    pane.hang_up();
                }
            }
            for (block, _) in &visible {
                if !self.panes.contains_key(block) {
                    match self.open(*block) {
                        Ok(pane) => {
                            self.panes.insert(*block, pane);
                        }
                        Err(why) => self.end(Ended::Failed(why)),
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

        /// Connects to the server for `block` and attaches to it.
        fn open(&mut self, block: BlockId) -> Result<Pane, String> {
            self.opened += 1;
            let tag = (block, self.opened);
            let stream = connect(&self.socket)?;
            let mut conn = Conn::new(clone(&stream)?);
            conn.send(&Message::Hello {
                features: Features::CHECKPOINT_V5,
                resume: None,
            })
            .map_err(|e| e.to_string())?;
            match conn.recv().map_err(|e| e.to_string())? {
                Message::Welcome { .. } => {}
                other => return Err(format!("expected Welcome, got {other:?}")),
            }
            let tx = self.tx.clone();
            let reader = std::thread::spawn(move || {
                read(
                    conn,
                    &tx,
                    move |message| Input::Pane(tag, message),
                    Some(tag),
                )
            });
            let mut pane = Pane {
                generation: tag.1,
                closed: false,
                conn: Conn::new(clone(&stream)?),
                stream,
                next: 1,
                attach: 0,
                receiver: Some(Receiver::default()),
                term: None,
                asking: None,
                leased: false,
                sizing: None,
                reader: Some(reader),
            };
            pane.attach = pane.send(|req| Message::Attach { req, block });
            Ok(pane)
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
            self.shown = shown;
        }

        /// Redraws the composed screen where it changed.
        fn draw(&mut self) {
            let Shown::Composed(last) = &self.shown else {
                return;
            };
            let visible = self.visible();
            let views: Vec<View<'_>> = visible
                .iter()
                .map(|(block, rect)| View {
                    block: *block,
                    rect: *rect,
                    grid: self
                        .panes
                        .get(block)
                        .and_then(|p| p.term.as_ref())
                        .map(|(g, _)| g),
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
}
