//! The engine thread and the state it shares with callers (design §7).
//!
//! One engine thread per console owns the [`Console`], the
//! [`ConsoleServer`] and the child's wait handle. Callers (`read`,
//! `write`, `resize`, `hang_up`, Drop) only touch [`Shared`]: the input
//! lane, the control lane, the output queue and the latches. No lock in
//! `Shared` is held across an OS call or across [`Console::handle`].
//!
//! A second thread, the ctrl worker, delivers Ctrl events: delivery can
//! block on the target's handler, so it never runs on the engine thread.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::term::WinSize;
use platform_windows::WindowsChild;
use platform_windows::condrv::{
    self, Attached, ClientProcess, ConsoleServer, CtrlEvent, IoComplete, IoDescriptor, Waker,
};
use platform_windows::sys::handle::OwnedWinHandle;

use crate::codepage::Codec;
use crate::console::{Console, Effect, Event, OutputQueue, POSTAMBLE, PREAMBLE, Packet, Reply};
use crate::ctrl::{self, Ctrl, Pushed};
use crate::fixture::{self, FIXTURE_CAP};
use crate::msg::STATUS_UNSUCCESSFUL;
use crate::sched::{self, Control, Grace, KILL_AFTER, PACKET_INPUT_CAP, View};

/// Code-page conversion by the OS (§5.9).
pub(crate) const NATIVE_CODEC: Codec = Codec {
    valid: condrv::is_valid_code_page,
    decode: |cp, bytes| condrv::decode(cp, bytes).ok(),
    encode: |cp, text| condrv::encode(cp, text).ok(),
};

/// Terminal input waiting for the engine; `write` blocks past this.
const INPUT_LANE_CAP: usize = 64 * 1024;
/// How long teardown waits for a cancelled READ_IO before closing anyway.
const CANCEL_GRACE: Duration = Duration::from_secs(1);
/// Names the file a console records its session to (design §8.2).
const RECORD_ENV: &str = "RUSTY_CONHOST_RECORD";

/// Writes the session as fixture steps, sanitised, up to the fixture cap.
struct Recorder {
    file: File,
    written: usize,
}

impl Recorder {
    /// A recorder when [`RECORD_ENV`] names a file that can be created.
    fn from_env() -> Option<Recorder> {
        let path = std::env::var_os(RECORD_ENV)?;
        match File::create(&path) {
            Ok(file) => Some(Recorder { file, written: 0 }),
            Err(e) => {
                crate::diag::log(&format!("not recording: {e}"));
                None
            }
        }
    }

    /// Append one line; `false` once recording has stopped.
    fn line(&mut self, line: &str) -> bool {
        let stop = "# recording stopped at the fixture cap\n";
        let fits = self.written + line.len() + 1 + stop.len() <= FIXTURE_CAP;
        let text = if fits {
            format!("{line}\n")
        } else {
            stop.to_owned()
        };
        self.written += text.len();
        match self.file.write_all(text.as_bytes()) {
            Ok(()) => fits,
            Err(e) => {
                crate::diag::log(&format!("recording stopped: {e}"));
                false
            }
        }
    }
}

/// What callers and the engine share.
pub(crate) struct Shared {
    lanes: Mutex<Lanes>,
    /// Signalled when the input lane frees space or closes.
    input_room: Condvar,
    output: Mutex<Output>,
    /// Signalled when output is queued or EOF is latched.
    output_ready: Condvar,
    exited: Mutex<bool>,
    exit_seen: Condvar,
    /// Latched once the console has closed and the clients still attached
    /// at its close have been ended (#1259).
    shut_down: Mutex<bool>,
    shut_down_seen: Condvar,
    waker: Waker,
}

struct Lanes {
    input: VecDeque<u8>,
    /// `write` calls part-way through their buffer: the input lane's
    /// last bytes are not where a terminal's write ended.
    writing: usize,
    /// Bytes taken from the lane so far, to place `esc_ends`.
    taken: u64,
    /// Where writes that ended with ESC ended, as byte counts from the
    /// lane's start: the engine cuts its slices there, so a write's last
    /// ESC is the Escape key however writes coalesce. At most one per
    /// queued byte.
    esc_ends: VecDeque<u64>,
    control: Control,
    /// Set by the first `hang_up`, and kept once the engine takes the
    /// control flag: `write` fails from then on.
    hung_up: bool,
    /// Set once the server is closed: `write` and `resize` fail.
    closed: bool,
}

struct Output {
    queue: VecDeque<u8>,
    eof: bool,
    /// The master is gone: nobody reads, so output is discarded rather
    /// than left to stall clients' handlers during the grace.
    detached: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn broken_pipe(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::BrokenPipe, OsCode::None, op)
}

impl Shared {
    fn new(waker: Waker) -> Shared {
        Shared {
            lanes: Mutex::new(Lanes {
                input: VecDeque::new(),
                writing: 0,
                taken: 0,
                esc_ends: VecDeque::new(),
                control: Control::default(),
                hung_up: false,
                closed: false,
            }),
            input_room: Condvar::new(),
            output: Mutex::new(Output {
                queue: VecDeque::new(),
                eof: false,
                detached: false,
            }),
            output_ready: Condvar::new(),
            exited: Mutex::new(false),
            exit_seen: Condvar::new(),
            shut_down: Mutex::new(false),
            shut_down_seen: Condvar::new(),
            waker,
        }
    }

    /// Queue terminal input, blocking while the lane is full. `BrokenPipe`
    /// after `hang_up` or once the server is closed.
    pub(crate) fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut rest = buf;
        let mut lanes = lock(&self.lanes);
        lanes.writing += 1;
        while !rest.is_empty() {
            if lanes.closed || lanes.hung_up {
                lanes.writing -= 1;
                return Err(broken_pipe("OwnPtyMaster::write"));
            }
            let room = INPUT_LANE_CAP.saturating_sub(lanes.input.len());
            if room == 0 {
                lanes = self
                    .input_room
                    .wait(lanes)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let (now, later) = rest.split_at(room.min(rest.len()));
            lanes.input.extend(now);
            rest = later;
            if rest.is_empty() {
                lanes.writing -= 1;
                if buf.last() == Some(&0x1b) {
                    let end = lanes.taken + lanes.input.len() as u64;
                    lanes.esc_ends.push_back(end);
                }
            }
            self.waker.wake();
        }
        if buf.is_empty() {
            lanes.writing -= 1;
        }
        Ok(buf.len())
    }

    /// Set the control lane's resize flag; the latest size wins.
    pub(crate) fn resize(&self, size: WinSize) -> Result<()> {
        let mut lanes = lock(&self.lanes);
        if lanes.closed {
            return Err(broken_pipe("OwnPtyMaster::resize"));
        }
        lanes.control.resize = Some(size);
        self.waker.wake();
        Ok(())
    }

    /// Set the control lane's hang-up flag and wake the engine, which
    /// sends CTRL_CLOSE and closes the server by its grace deadline
    /// (§7.3). Never blocks beyond the lane's lock; idempotent.
    pub(crate) fn hang_up(&self) {
        let mut lanes = lock(&self.lanes);
        if lanes.closed || lanes.hung_up {
            return;
        }
        lanes.hung_up = true;
        lanes.control.hang_up = true;
        // A blocked writer now fails instead of waiting for room.
        self.input_room.notify_all();
        self.waker.wake();
    }

    /// The master is gone (§7.3): drop queued output and discard what
    /// follows, so writes parked on the reader complete, then hang up.
    pub(crate) fn detach(&self) {
        {
            let mut out = lock(&self.output);
            out.detached = true;
            out.queue.clear();
        }
        lock(&self.lanes).control.output_drained = true;
        self.hang_up();
        // Wakes the engine for the drain even when it is hung up already.
        self.waker.wake();
    }

    /// Block until output is queued, then copy out up to `buf.len()`
    /// bytes. `Ok(0)` once EOF is latched and the queue is empty.
    pub(crate) fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut out = lock(&self.output);
        while out.queue.is_empty() && !out.eof {
            out = self
                .output_ready
                .wait(out)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let before = out.queue.len();
        let n = before.min(buf.len());
        for (dst, src) in buf.iter_mut().zip(out.queue.drain(..n)) {
            *dst = src;
        }
        drop(out);
        // A console call can decide to hold echo against an earlier
        // queue snapshot while this read drains it. Every real drain
        // must leave a wake, including drains starting below 64 KiB.
        if n > 0 {
            lock(&self.lanes).control.output_drained = true;
            self.waker.wake();
        }
        Ok(n)
    }

    /// Whether the master is gone and output is discarded.
    fn detached(&self) -> bool {
        lock(&self.output).detached
    }

    /// Whether a `read` would return without blocking.
    pub(crate) fn output_ready(&self) -> bool {
        let out = lock(&self.output);
        !out.queue.is_empty() || out.eof
    }

    /// Block until the direct child has exited or the engine has stopped.
    pub(crate) fn wait_exit(&self) {
        let mut exited = lock(&self.exited);
        while !*exited {
            exited = self
                .exit_seen
                .wait(exited)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Wait up to `timeout` for the shutdown latch; whether it is set.
    pub(crate) fn wait_shut_down(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut done = lock(&self.shut_down);
        while !*done {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            done = self
                .shut_down_seen
                .wait_timeout(done, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    // The engine's side.

    fn set_shut_down(&self) {
        *lock(&self.shut_down) = true;
        self.shut_down_seen.notify_all();
    }

    fn take_control(&self) -> (Control, usize) {
        let mut lanes = lock(&self.lanes);
        (lanes.control.take(), lanes.input.len())
    }

    /// Take up to `n` bytes of terminal input, and whether they end
    /// where a terminal's write ended: at the end of a write that ended
    /// with ESC (the slice stops there), or with the lane empty and no
    /// `write` part-way through.
    fn take_input(&self, n: usize) -> (Vec<u8>, bool) {
        let mut lanes = lock(&self.lanes);
        let mut n = n.min(lanes.input.len());
        let mut end = false;
        if let Some(&at) = lanes.esc_ends.front()
            && at - lanes.taken <= n as u64
        {
            n = (at - lanes.taken) as usize;
            end = true;
            lanes.esc_ends.pop_front();
        }
        let bytes: Vec<u8> = lanes.input.drain(..n).collect();
        lanes.taken += n as u64;
        end |= lanes.input.is_empty() && lanes.writing == 0;
        self.input_room.notify_all();
        (bytes, end)
    }

    fn input_len(&self) -> usize {
        lock(&self.lanes).input.len()
    }

    fn output_len(&self) -> usize {
        lock(&self.output).queue.len()
    }

    fn push_output(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let mut out = lock(&self.output);
        if out.detached {
            return;
        }
        out.queue.extend(bytes);
        drop(out);
        self.output_ready.notify_all();
    }

    fn set_eof(&self) {
        lock(&self.output).eof = true;
        self.output_ready.notify_all();
    }

    fn set_exited(&self) {
        *lock(&self.exited) = true;
        self.exit_seen.notify_all();
    }

    fn close_lanes(&self) {
        lock(&self.lanes).closed = true;
        self.input_room.notify_all();
    }

    /// The console is gone: `read` reaches EOF, waiters wake, writes fail.
    fn close(&self) {
        self.set_eof();
        self.set_exited();
        self.close_lanes();
    }
}

/// The ctrl worker's queue (§7.2): the engine pushes, the worker pops
/// and delivers. Closed when the engine's [`CtrlSender`] drops.
#[derive(Default)]
struct CtrlLane {
    state: Mutex<CtrlState>,
    ready: Condvar,
}

#[derive(Default)]
struct CtrlState {
    queue: ctrl::Queue,
    closed: bool,
    /// A full queue was logged once already.
    dropped_logged: bool,
}

impl CtrlLane {
    /// Deliver queued events until the lane closes. Then only CTRL_CLOSE
    /// still goes out: ConsoleControl doesn't need the server, and a
    /// client the closed server cut off may never exit by itself (S1),
    /// while Windows ends one that got CTRL_CLOSE within about 5 s.
    fn run(&self) {
        while let Some((pid, event)) = self.next() {
            if let Err(e) = condrv::send_ctrl(pid, ctrl_event(event)) {
                crate::diag::log(&format!("Ctrl delivery to {pid} failed: {e}"));
            }
        }
    }

    /// The next delivery, waiting for one; `None` once the lane is closed
    /// and no CTRL_CLOSE is left.
    fn next(&self) -> Option<(u32, Ctrl)> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return std::iter::from_fn(|| state.queue.pop()).find(|&(_, e)| e == Ctrl::Close);
            }
            if let Some(next) = state.queue.pop() {
                return Some(next);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn ctrl_event(event: Ctrl) -> CtrlEvent {
    match event {
        Ctrl::C => CtrlEvent::C,
        Ctrl::Break => CtrlEvent::Break,
        Ctrl::Close => CtrlEvent::Close,
    }
}

/// The engine's end of the [`CtrlLane`]; dropping it stops the worker,
/// also when the engine never runs.
struct CtrlSender(Arc<CtrlLane>);

impl CtrlSender {
    fn push(&self, pid: u32, event: Ctrl) {
        let mut state = lock(&self.0.state);
        match state.queue.push(pid, event) {
            Pushed::Queued => self.0.ready.notify_one(),
            Pushed::Coalesced => {}
            Pushed::Dropped if state.dropped_logged => {}
            Pushed::Dropped => {
                state.dropped_logged = true;
                crate::diag::log("the Ctrl queue is full; dropping events");
            }
        }
    }
}

impl CtrlSender {
    /// Drop what is still queued for `pid`, whose client has left.
    fn forget(&self, pid: u32) {
        lock(&self.0.state).queue.forget(pid);
    }
}

impl Drop for CtrlSender {
    fn drop(&mut self) {
        lock(&self.0.state).closed = true;
        self.0.ready.notify_all();
    }
}

/// A started engine thread waiting for its console. Everything that can
/// fail is done by [`prepare`], before the child exists; the handoff in
/// [`Starter::start`] cannot fail, so the caller always owns the child.
pub(crate) struct Starter {
    shared: Arc<Shared>,
    tx: mpsc::Sender<(ConsoleServer, OwnedWinHandle)>,
}

/// Create the shared state and start the ctrl worker and the engine
/// thread, before any child exists.
pub(crate) fn prepare(size: WinSize) -> Result<Starter> {
    prepare_with(size, |name, run| {
        std::thread::Builder::new()
            .name(name.into())
            .spawn(run)
            .map(drop)
    })
}

/// [`prepare`] with the thread spawn supplied, so tests can make it fail
/// or lose the thread.
fn prepare_with<S>(size: WinSize, mut spawn: S) -> Result<Starter>
where
    S: FnMut(&'static str, Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
{
    let shared = Arc::new(Shared::new(Waker::new()?));
    let lane = Arc::new(CtrlLane::default());
    let worker = Arc::clone(&lane);
    spawn("rusty_conhost-ctrl", Box::new(move || worker.run()))
        .map_err(|_| not_started("rusty_conhost: the ctrl worker could not start"))?;
    // From here the worker stops when `ctrl` drops, run or not.
    let ctrl = CtrlSender(lane);
    let (tx, rx) = mpsc::channel::<(ConsoleServer, OwnedWinHandle)>();
    let engine_shared = Arc::clone(&shared);
    let run = Box::new(move || {
        // No console means the spawn failed: nothing to serve.
        let Ok((server, wait)) = rx.recv() else {
            return;
        };
        let console = Console::new(size, condrv::key_for, NATIVE_CODEC);
        Engine::new(console, server, wait, engine_shared, ctrl).run();
    });
    spawn("rusty_conhost-engine", run)
        .map_err(|_| not_started("rusty_conhost: the engine thread could not start"))?;
    Ok(Starter { shared, tx })
}

fn not_started(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::Other, OsCode::None, op)
}

impl Starter {
    /// Hand the attached console to the engine and the child to the
    /// caller. If the engine thread is gone, the console closes at once,
    /// as after a panic (§6.2): `read` reaches EOF and the caller cleans
    /// up the child as for any closed console.
    pub(crate) fn start(self, attached: Attached) -> (Arc<Shared>, WindowsChild) {
        let Attached {
            server,
            child,
            wait,
        } = attached;
        if let Err(mpsc::SendError((server, _wait))) = self.tx.send((server, wait)) {
            crate::diag::log("the engine thread is gone; closing the console");
            server.close();
            self.shared.close();
        }
        (self.shared, child)
    }
}

struct Engine {
    console: Console,
    /// `None` once closed.
    server: Option<ConsoleServer>,
    wait: OwnedWinHandle,
    shared: Arc<Shared>,
    ctrl: CtrlSender,
    /// The console's output for the current `handle` call.
    out: OutputQueue,
    read_in_flight: bool,
    /// The reply to send with the next READ_IO.
    reply: Option<Reply>,
    /// Ctrl events that came with [`Engine::reply`]: queued once it is
    /// sent, so a client hears CTRL_CLOSE only after its CONNECT returns.
    after_reply: Vec<(u32, Ctrl)>,
    /// A wait reported the child's exit.
    exited: bool,
    exit_handled: bool,
    /// The CTRL_CLOSE deadline, armed by the first hang-up or child exit.
    grace: Grace,
    /// Attached client processes by pid, ended if still attached at the
    /// deadline (#1259). One that couldn't be opened is missing.
    clients: HashMap<u32, ClientProcess>,
    /// The client whose CONNECT [`Engine::reply`] answers, opened and
    /// checked against its CONNECT thread; held only once that reply is
    /// delivered (design §7.3).
    joining: Option<Joining>,
    /// A client that couldn't be opened was logged once already.
    open_failed_logged: bool,
    /// Clients still attached when the grace ran out, ended at `kill_at`
    /// once the console has closed (#1259).
    doomed: Vec<(u32, ClientProcess)>,
    kill_at: Option<Instant>,
    /// When CTRL_CLOSE went out: the grace was armed.
    close_started: Option<Instant>,
    /// Calls the console holds without a reply yet, for the panic path.
    pending: Vec<[u8; 8]>,
    /// `handle` panicked; nothing more reaches the console.
    dead: bool,
    recorder: Option<Recorder>,
    /// The next slice a streamed write asked for ([`Effect::Fetch`]),
    /// read in the next turn, after its control lane (§7.2).
    fetch: Option<Fetch>,
    /// Whole inputs of calls a test made up, read in place of the driver.
    #[cfg(test)]
    test_inputs: HashMap<[u8; 8], Vec<u8>>,
}

/// A slice of call `id`'s input: `len` bytes from `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fetch {
    id: [u8; 8],
    at: usize,
    len: usize,
}

impl Engine {
    fn new(
        console: Console,
        server: ConsoleServer,
        wait: OwnedWinHandle,
        shared: Arc<Shared>,
        ctrl: CtrlSender,
    ) -> Engine {
        Engine {
            console,
            server: Some(server),
            wait,
            shared,
            ctrl,
            out: OutputQueue::default(),
            read_in_flight: false,
            reply: None,
            after_reply: Vec::new(),
            joining: None,
            exited: false,
            exit_handled: false,
            grace: Grace::default(),
            clients: HashMap::new(),
            open_failed_logged: false,
            doomed: Vec::new(),
            kill_at: None,
            close_started: None,
            pending: Vec::new(),
            dead: false,
            recorder: Recorder::from_env(),
            fetch: None,
            #[cfg(test)]
            test_inputs: HashMap::new(),
        }
    }

    fn run(mut self) {
        self.shared.push_output(PREAMBLE.to_vec());
        while self.server.is_some() {
            if let Err(e) = self.turn() {
                crate::diag::log(&format!("engine stopped: {e}"));
                self.teardown();
            }
        }
        self.shared.close();
        self.reap();
        self.shared.set_shut_down();
    }

    /// End the clients still attached at the grace, at [`KILL_AFTER`]
    /// from CTRL_CLOSE. The console is closed and every reader and exit
    /// waiter released, so only this thread and a caller of
    /// `OwnPtyMaster::wait_shut_down` wait; a client that exited
    /// meanwhile, as Windows ends a handler at about the same bound, is
    /// left alone.
    fn reap(&mut self) {
        let Some(at) = self.kill_at else {
            return;
        };
        std::thread::sleep(at.saturating_duration_since(Instant::now()));
        for (pid, client) in self.doomed.drain(..) {
            if let Err(e) = client.terminate() {
                crate::diag::log(&format!("ending client {pid} failed: {e}"));
            }
        }
    }

    /// One turn of design §7.2's loop.
    fn turn(&mut self) -> Result<()> {
        self.work()?;
        // Decided on what the turn's events left, not on its plan.
        let until = self.grace.until(self.must_poll(), Instant::now());
        let Some(server) = self.server.as_mut() else {
            return Ok(());
        };
        let exit = (!self.exited).then_some(&self.wait);
        let ready = server.wait(&self.shared.waker, exit, until);
        let ready = match ready {
            Ok(ready) => ready,
            Err(e) => {
                // A read that completed with an error is no longer in
                // flight; `close` guards the other case.
                self.read_in_flight = false;
                return Err(e);
            }
        };
        if ready.exited && !self.exited {
            self.exited = true;
            self.shared.set_exited();
        }
        if let Some(d) = ready.io {
            self.read_in_flight = false;
            self.serve(d)?;
        }
        Ok(())
    }

    /// The turn up to its wait: plan, handle the events in order, and
    /// submit the next READ_IO or complete the held reply.
    fn work(&mut self) -> Result<()> {
        let now = Instant::now();
        if self.grace.expire(now) {
            self.handle(Event::GraceExpired);
        }
        let (mut control, input) = self.shared.take_control();
        // Discarded output drains at once, but each drain releases only one
        // client budget of parked writes and no reader announces the next.
        control.output_drained |= self.drains_itself();
        let plan = sched::turn(&self.view(control, input));
        if plan.control.hang_up {
            self.handle(Event::HangUp);
            self.arm_grace(now);
        }
        if let Some(size) = plan.control.resize {
            self.handle(Event::Resize(size));
        }
        if plan.control.output_drained || plan.pump_writes {
            self.handle(Event::OutputDrained);
        }
        if plan.child_exited {
            self.exit_handled = true;
            self.handle(Event::ChildExited);
            self.arm_grace(now);
        }
        self.serve_fetch();
        if plan.serve_reads {
            self.handle(Event::ServeReads);
        }
        if plan.input > 0 {
            let (bytes, end) = self.shared.take_input(plan.input);
            if end {
                self.handle(Event::Input(&bytes));
            } else {
                self.handle(Event::InputMore(&bytes));
            }
        }
        let Some(server) = self.server.as_mut() else {
            return Ok(());
        };
        let mut reply = self.reply.take().map(|r| deliverable(server, r));
        if let Some(r) = &reply
            && let Some(joining) = self.joining.take()
        {
            if let Some(j) = confirm_join(server, r, joining) {
                j.admit(&mut self.clients, &mut self.doomed);
            }
            reply = None;
        }
        if plan.submit_read {
            let complete = reply.as_ref().map(Reply::as_complete);
            server.submit_read(complete.as_ref())?;
            self.read_in_flight = true;
        } else if let Some(r) = reply {
            // No READ_IO goes out this turn, so the reply can't ride on
            // one: complete it on its own rather than hold the client.
            server.complete_io(&r.as_complete())?;
        }
        self.release_ctrl();
        Ok(())
    }

    /// Queue the Ctrl events held behind a reply that has now gone out.
    fn release_ctrl(&mut self) {
        if self.reply.is_some() {
            return;
        }
        for (pid, event) in self.after_reply.drain(..) {
            self.ctrl.push(pid, event);
        }
    }

    /// Start the grace once the console has sent CTRL_CLOSE; a console
    /// with no client closed at once, and needs none.
    fn arm_grace(&mut self, now: Instant) {
        if self.server.is_some() {
            self.grace.arm(now);
            self.close_started.get_or_insert(now);
        }
    }

    /// Whether work is left that the next turn can do at once, so the
    /// wait must not block. The waker is auto-reset and its wake may be
    /// spent: neither reads left ready nor input left behind an ESC cut
    /// signals it again.
    fn must_poll(&self) -> bool {
        self.drains_itself()
            || sched::must_poll(&self.view(Control::default(), self.shared.input_len()))
    }

    /// Calls wait on output that nobody will read.
    fn drains_itself(&self) -> bool {
        self.console.output_blocked() && self.shared.detached()
    }

    fn view(&self, control: Control, input: usize) -> View {
        View {
            control,
            exited: self.exited && !self.exit_handled,
            output: self.shared.output_len(),
            input,
            read_in_flight: self.read_in_flight,
            waiting: self.console.waiting(),
            parked: self.console.parked(),
            closed: false,
            reads_ready: self.console.reads_ready(),
            input_paused: self.console.input_paused(),
            writes_ready: self.console.writes_ready(),
            streaming: self.console.streaming(),
            fetch_due: self.fetch.is_some(),
        }
    }

    /// Read a packet's variable input, at most [`PACKET_INPUT_CAP`] of
    /// it, and hand it to the console: a write past the cap fetches the
    /// rest a slice at a time, and any other such packet is refused.
    fn serve(&mut self, d: IoDescriptor) -> Result<()> {
        let Some(server) = self.server.as_mut() else {
            return Ok(());
        };
        let len = d.input_size as usize;
        let mut payload = vec![0u8; len.min(PACKET_INPUT_CAP)];
        let got = match server.read_input(d.id, 0, &mut payload) {
            Ok(n) => n,
            Err(e) => {
                crate::diag::log(&format!("READ_INPUT failed: {e}"));
                self.reply = Some(Reply::status(d.id, STATUS_UNSUCCESSFUL));
                return Ok(());
            }
        };
        self.handle(Event::Packet(Packet {
            id: d.id,
            process: d.process,
            object: d.object,
            function: d.function,
            input_size: d.input_size,
            output_size: d.output_size,
            payload: &payload[..got],
        }));
        Ok(())
    }

    /// Run one event through the console, keeping the slice of a
    /// streamed write it asks for until the next turn (§7.4).
    fn handle(&mut self, ev: Event<'_>) {
        if let Some(fetch) = self.handle_one(ev) {
            self.fetch = Some(fetch);
        }
    }

    /// Hand the console the slice its streamed write asked for, if any:
    /// one per turn, so a write of any size leaves the control lane, the
    /// child's exit and the deadline their turns (§7.2).
    fn serve_fetch(&mut self) {
        let Some(f) = self.fetch.take() else {
            return;
        };
        let bytes = self.read_slice(f);
        self.handle(Event::Fetched {
            id: f.id,
            bytes: bytes.as_deref(),
        });
    }

    /// Read a slice of a call's input; `None`, logged, when the driver
    /// can't, which fails the call.
    fn read_slice(&mut self, f: Fetch) -> Option<Vec<u8>> {
        #[cfg(test)]
        if let Some(input) = self.test_inputs.get(&f.id) {
            return input
                .get(f.at..(f.at + f.len).min(input.len()))
                .map(<[u8]>::to_vec);
        }
        let server = self.server.as_mut()?;
        let mut bytes = vec![0u8; f.len];
        match server.read_input(f.id, f.at as u32, &mut bytes) {
            Ok(n) => {
                bytes.truncate(n);
                Some(bytes)
            }
            Err(e) => {
                crate::diag::log(&format!("READ_INPUT of a streamed write failed: {e}"));
                None
            }
        }
    }

    /// Run one event through the console, inside the panic boundary, and
    /// apply its effects in order; returns the fetch it asked for.
    fn handle_one(&mut self, ev: Event<'_>) -> Option<Fetch> {
        let packet = match &ev {
            Event::Packet(p) => Some(p.id),
            _ => None,
        };
        if self.dead {
            if let Some(id) = packet {
                self.reply = Some(Reply::status(id, STATUS_UNSUCCESSFUL));
            }
            return None;
        }
        let held = self.shared.output_len();
        self.out.set_held(held);
        if let Some(r) = self.recorder.as_mut()
            && !r.line(&fixture::event_line(&ev, held))
        {
            self.recorder = None;
        }
        let console = &mut self.console;
        let out = &mut self.out;
        let effects = match catch_unwind(AssertUnwindSafe(|| console.handle(ev, out))) {
            Ok(effects) => effects,
            Err(_) => {
                self.out.take();
                self.die(packet);
                return None;
            }
        };
        self.shared.push_output(self.out.take());
        let mut replied = false;
        let mut fetch = None;
        for e in effects {
            match e {
                Effect::Reply(ref r) => replied |= Some(r.id) == packet,
                // Answered once the effects are applied, as the next event.
                Effect::Fetch { id, at, len } => {
                    fetch = Some(Fetch { id, at, len });
                    continue;
                }
                _ => {}
            }
            self.apply(e);
        }
        if let Some(id) = packet
            && !replied
            && self.server.is_some()
        {
            self.pending.push(id);
        }
        fetch.filter(|_| self.server.is_some())
    }

    fn apply(&mut self, e: Effect) {
        if self.server.is_none() {
            // The clients are disconnected; nothing can be completed.
            if !matches!(e, Effect::Eof | Effect::Log(_)) {
                crate::diag::log("dropped an effect after CloseServer");
                return;
            }
        }
        match e {
            Effect::Reply(r) => {
                if let Some(old) = self.reply.replace(r) {
                    self.complete(old);
                }
            }
            Effect::Complete(r) => {
                self.pending.retain(|id| *id != r.id);
                self.complete(r);
            }
            Effect::SetInputAvailable(on) => self.set_input_available(on),
            Effect::Ctrl { pid, event } if self.reply.is_some() => {
                self.after_reply.push((pid, event));
            }
            Effect::Ctrl { pid, event } => self.ctrl.push(pid, event),
            Effect::Joined { pid, tid } => self.track(pid, tid),
            Effect::Left { pid } => {
                self.after_reply.retain(|&(p, _)| p != pid);
                self.ctrl.forget(pid);
                self.clients.remove(&pid);
            }
            Effect::Kill { pid } => self.kill(pid),
            Effect::CloseServer => self.teardown(),
            Effect::Eof => self.shared.set_eof(),
            // `handle` answers it; it never gets here.
            Effect::Fetch { .. } => {}
            Effect::Log(d) => crate::diag::log(&d.to_string()),
        }
    }

    /// Open a joining client through its CONNECT thread `tid`, to hold it
    /// once its CONNECT reply is delivered ([`confirm_join`]). Fails
    /// closed: a client that can't be opened that way (gone already, its
    /// pid now another process's, or a protected process) is only logged,
    /// and never ended; CTRL_CLOSE still reaches it.
    fn track(&mut self, pid: u32, tid: u32) {
        if self.clients.contains_key(&pid) {
            return;
        }
        match ClientProcess::open(pid, tid) {
            Ok(client) => {
                self.joining = Some(Joining {
                    pid,
                    client,
                    doomed: false,
                });
            }
            Err(e) if !self.open_failed_logged => {
                self.open_failed_logged = true;
                crate::diag::log(&format!(
                    "client {pid} can't be ended at the close deadline: {e}"
                ));
            }
            Err(_) => {}
        }
    }

    /// Doom a client still attached at the grace deadline: [`Engine::reap`]
    /// ends it [`KILL_AFTER`] from CTRL_CLOSE, after the console closes.
    /// One whose CONNECT reply is still held is doomed once that reply is
    /// delivered, which may be in the teardown this deadline starts.
    fn kill(&mut self, pid: u32) {
        let started = *self.close_started.get_or_insert_with(Instant::now);
        if let Some(joining) = self.joining.as_mut().filter(|j| j.pid == pid) {
            joining.doomed = true;
        } else if let Some(client) = self.clients.remove(&pid) {
            self.doomed.push((pid, client));
        } else {
            return;
        }
        self.kill_at.get_or_insert(started + KILL_AFTER);
    }

    /// Signal or reset the input-available event, best effort: a failure
    /// only leaves a waiting client unwoken.
    fn set_input_available(&mut self, on: bool) {
        let Some(server) = self.server.as_mut() else {
            return;
        };
        if let Err(e) = server.set_input_available(on) {
            crate::diag::log(&format!("set_input_available failed: {e}"));
        }
    }

    /// Complete one call now, best effort: a failure only means its client
    /// is gone.
    fn complete(&mut self, r: Reply) {
        let Some(server) = self.server.as_mut() else {
            return;
        };
        let r = deliverable(server, r);
        if let Err(e) = server.complete_io(&r.as_complete()) {
            crate::diag::log(&format!("COMPLETE_IO failed: {e}"));
        }
    }

    /// The console panicked (§6.2): fail every call it held, then close.
    fn die(&mut self, packet: Option<[u8; 8]>) {
        crate::diag::log("the console panicked; closing it");
        self.dead = true;
        let ids: Vec<[u8; 8]> = self.pending.drain(..).chain(packet).collect();
        for id in ids {
            self.complete(Reply::status(id, STATUS_UNSUCCESSFUL));
        }
        self.teardown();
        self.shared.set_eof();
    }

    /// Close the server (§7.3): answer the held reply, cancel and drain
    /// the in-flight read, then close.
    fn teardown(&mut self) {
        self.fetch = None;
        if let Some(r) = self.reply.take() {
            match (self.joining.take(), self.server.as_mut()) {
                (Some(joining), Some(server)) => {
                    let r = deliverable(server, r);
                    if let Some(j) = confirm_join(server, &r, joining) {
                        j.admit(&mut self.clients, &mut self.doomed);
                    }
                }
                _ => self.complete(r),
            }
        }
        self.release_ctrl();
        let Some(mut server) = self.server.take() else {
            return;
        };
        if self.read_in_flight {
            server.cancel_read();
            drain_cancelled(&mut server, &self.shared.waker);
            self.read_in_flight = false;
        }
        server.close();
        self.shared.push_output(POSTAMBLE.to_vec());
        self.shared.close_lanes();
    }
}

/// A client opened through its CONNECT thread, whose reply is held.
struct Joining {
    pid: u32,
    client: ClientProcess,
    /// The grace ran out before its reply went out (review of f24f50b3).
    doomed: bool,
}

impl Joining {
    /// Hold the confirmed client, or doom it if the deadline already
    /// passed it by.
    fn admit(
        self,
        clients: &mut HashMap<u32, ClientProcess>,
        doomed: &mut Vec<(u32, ClientProcess)>,
    ) {
        if self.doomed {
            doomed.push((self.pid, self.client));
        } else {
            clients.insert(self.pid, self.client);
        }
    }
}

/// Deliver the CONNECT reply `r` on its own, returning its client only if
/// it is delivered. ConDrv aborts a CONNECT whose thread dies before its
/// reply (ERROR_OPERATION_ABORTED, Beast 2026-10-31), and a thread can't
/// finish exiting while its CONNECT is pending; so a delivered reply
/// proves the thread `joining` was opened through was the client's own,
/// not a reuse of its id (design §7.3). Failing only drops the client.
fn confirm_join(server: &mut ConsoleServer, r: &Reply, joining: Joining) -> Option<Joining> {
    match server.complete_io(&r.as_complete()) {
        Ok(()) => Some(joining),
        Err(e) => {
            let pid = joining.pid;
            crate::diag::log(&format!("client {pid} left before its CONNECT reply: {e}"));
            None
        }
    }
}

/// Wait for a cancelled read's completion, up to [`CANCEL_GRACE`]. A read
/// that completed successfully in the race is failed, best effort.
fn drain_cancelled(server: &mut ConsoleServer, waker: &Waker) {
    let deadline = Instant::now() + CANCEL_GRACE;
    while Instant::now() < deadline {
        match server.wait(waker, None, Some(deadline)) {
            Ok(ready) => {
                if let Some(d) = ready.io {
                    let _ =
                        server.complete_io(&Reply::status(d.id, STATUS_UNSUCCESSFUL).as_complete());
                    return;
                }
            }
            // Cancelled, or failed: either way it is no longer in flight.
            Err(_) => return,
        }
    }
}

/// The reply to complete once its variable output, if any, is written at
/// its offset. Output ConDrv refuses, such as more than the client's
/// buffer holds, fails that one call; it never closes the console.
fn deliverable(server: &mut ConsoleServer, r: Reply) -> Reply {
    if r.output.is_empty() {
        return r;
    }
    match server.write_output(r.id, r.output_at, &r.output) {
        Ok(()) => r,
        Err(e) => {
            crate::diag::log(&format!("WRITE_OUTPUT failed: {e}"));
            Reply::status(r.id, STATUS_UNSUCCESSFUL)
        }
    }
}

impl Reply {
    fn as_complete(&self) -> IoComplete<'_> {
        IoComplete {
            id: self.id,
            status: self.status,
            information: self.information,
            write_back: &self.write_back,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::console::PANIC_FOR_TEST;
    use crate::msg;
    use platform::process::{Child, Command, ExitStatus, Signal};

    #[test]
    fn a_panicking_console_fails_its_calls_and_closes() {
        let cwd = std::env::current_dir().expect("current_dir");
        let mut cmd = Command::new("cmd.exe", cwd);
        cmd.argv = vec!["/c".into(), "ping -n 30 127.0.0.1".into()];
        let attached = condrv::spawn_attached(&cmd).expect("spawn_attached");
        let shared = Arc::new(Shared::new(Waker::new().expect("Waker")));
        let size = WinSize {
            rows: 30,
            cols: 120,
        };
        let mut engine = Engine::new(
            Console::new(size, condrv::key_for, NATIVE_CODEC),
            attached.server,
            attached.wait,
            Arc::clone(&shared),
            CtrlSender(Arc::default()),
        );
        // A call the console held, as a parked WriteConsole would be.
        engine.pending.push([9; 8]);
        let mut payload = PANIC_FOR_TEST.to_le_bytes().to_vec();
        payload.extend(0u32.to_le_bytes());
        engine.handle(Event::Packet(Packet {
            id: [7; 8],
            process: 1,
            object: 0,
            function: msg::USER_DEFINED,
            input_size: payload.len() as u32,
            output_size: 0,
            payload: &payload,
        }));
        assert!(engine.dead && engine.server.is_none());
        assert!(engine.pending.is_empty());
        // The teardown's postamble, then EOF.
        let mut out = Vec::new();
        let mut buf = [0u8; 8];
        loop {
            let n = shared.read(&mut buf).expect("read");
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(out, crate::console::POSTAMBLE);
        let err = shared.write(b"x").expect_err("write after close");
        assert_eq!(err.kind, ErrorKind::BrokenPipe);
        // Later events no longer reach the console.
        engine.handle(Event::HangUp);
        let _ = attached.child.kill_single(Signal::Kill);
    }

    fn ping_for_a_while() -> Command {
        let cwd = std::env::current_dir().expect("current_dir");
        let mut cmd = Command::new("cmd.exe", cwd);
        cmd.argv = vec!["/c".into(), "ping -n 30 127.0.0.1".into()];
        cmd
    }

    const SIZE: WinSize = WinSize {
        rows: 30,
        cols: 120,
    };

    #[test]
    fn an_engine_that_cannot_start_fails_before_any_child_exists() {
        let spawned = AtomicBool::new(false);
        let failed = prepare_with(SIZE, |_, _| {
            spawned.store(true, Ordering::Relaxed);
            Err(std::io::Error::other("no threads"))
        });
        let err = failed.err().expect("prepare fails");
        assert!(spawned.load(Ordering::Relaxed));
        assert_eq!(err.op, "rusty_conhost: the ctrl worker could not start");
        // OwnPty::spawn_concrete calls spawn_attached only after this.
    }

    #[test]
    fn the_ctrl_worker_stops_when_the_engine_cannot_start() {
        let mut worker = None;
        let failed = prepare_with(SIZE, |name, run| {
            if name != "rusty_conhost-ctrl" {
                return Err(std::io::Error::other("no threads"));
            }
            worker = Some(std::thread::spawn(run));
            Ok(())
        });
        let err = failed.err().expect("prepare fails");
        assert_eq!(err.op, "rusty_conhost: the engine thread could not start");
        worker
            .expect("the worker was started")
            .join()
            .expect("the worker returns once the engine is gone");
    }

    /// An engine on an attached `ping`, serving `console`.
    fn engine_on(console: Console) -> (Engine, WindowsChild) {
        let attached = condrv::spawn_attached(&ping_for_a_while()).expect("spawn_attached");
        let shared = Arc::new(Shared::new(Waker::new().expect("Waker")));
        let engine = Engine::new(
            console,
            attached.server,
            attached.wait,
            shared,
            CtrlSender(Arc::default()),
        );
        (engine, attached.child)
    }

    /// Run turns as the engine does once the writers' one wake is spent:
    /// after the first, a turn runs only if the last one's wait would not
    /// block. Returns how many ran.
    fn turns_without_another_wake(engine: &mut Engine) -> usize {
        for turns in 1..=1000 {
            engine.work().expect("turn");
            if !engine.must_poll() {
                return turns;
            }
        }
        panic!("the engine never settles");
    }

    #[test]
    fn reads_a_slice_leaves_ready_are_served_without_another_wake() {
        let (console, input) = crate::console::tests::owing_large_reads(8, u32::MAX - 64);
        assert!(input.len() <= sched::INPUT_SLICE, "one slice takes it all");
        let (mut engine, child) = engine_on(console);
        engine.shared.write(&input).expect("write");
        let turns = turns_without_another_wake(&mut engine);
        assert!(turns > 1, "the slice left reads ready");
        assert_eq!(engine.shared.input_len(), 0);
        assert!(!engine.console.reads_ready());
        assert_eq!(engine.console.waiting(), 0, "every read was served");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn a_joined_client_is_held_from_its_connect_and_ended_by_kill() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        // The CONNECT is answered but its reply not yet sent.
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        assert_eq!(
            engine.joining.as_ref().map(|j| j.pid),
            Some(pid),
            "opened before its reply"
        );
        assert!(engine.clients.is_empty(), "held only once it is delivered");
        engine.work().expect("send the reply");
        assert!(engine.clients.contains_key(&pid), "held once delivered");
        let started = Instant::now();
        engine.arm_grace(started);
        engine.apply(Effect::Kill { pid });
        assert!(engine.clients.is_empty());
        assert_eq!(engine.doomed.len(), 1, "ended later, not now");
        assert_eq!(
            engine.kill_at,
            Some(started + KILL_AFTER),
            "from CTRL_CLOSE"
        );
        engine.kill_at = Some(Instant::now());
        engine.reap();
        let status = Box::new(child).wait().expect("wait");
        assert_eq!(status, ExitStatus::Code(0xC000_013A_u32 as i32));
        engine.teardown();
    }

    /// The grace runs out while a CONNECT reply is held, so the deadline's
    /// Kill finds the client still joining, and its reply goes out in the
    /// teardown that follows: it is ended all the same (review of
    /// f24f50b3).
    #[test]
    fn a_client_joining_at_the_deadline_is_ended_once_confirmed() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        let started = Instant::now();
        engine.arm_grace(started);
        engine.apply(Effect::Kill { pid });
        assert!(engine.doomed.is_empty(), "not confirmed yet");
        assert_eq!(engine.kill_at, Some(started + KILL_AFTER));
        engine.apply(Effect::CloseServer);
        assert!(engine.clients.is_empty(), "never held as attached");
        assert_eq!(engine.doomed.len(), 1, "doomed once its reply went out");
        engine.kill_at = Some(Instant::now());
        engine.reap();
        let status = Box::new(child).wait().expect("wait");
        assert_eq!(status, ExitStatus::Code(0xC000_013A_u32 as i32));
    }

    /// As above, but the client dies before the teardown delivers its
    /// reply: the delivery fails, and an unconfirmed client is never
    /// ended, so nothing is doomed (fails closed).
    #[test]
    fn a_client_joining_at_the_deadline_that_dies_first_is_never_ended() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        engine.arm_grace(Instant::now());
        engine.apply(Effect::Kill { pid });
        child.kill_single(Signal::Kill).expect("kill");
        let _ = Box::new(child).wait().expect("wait");
        engine.apply(Effect::CloseServer);
        assert!(engine.joining.is_none());
        assert!(engine.clients.is_empty(), "never held");
        assert!(engine.doomed.is_empty(), "nothing to end");
    }

    /// A pid whose CONNECT thread isn't its own, as when the client died
    /// and its pid was reused, is never held, so never ended.
    #[test]
    fn a_client_not_reached_through_its_connect_thread_is_never_ended() {
        let (mut engine, mut child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        engine.apply(Effect::Joined {
            pid,
            tid: u32::MAX - 3,
        });
        assert!(engine.clients.is_empty(), "fails closed");
        engine.apply(Effect::Kill { pid });
        assert!(engine.doomed.is_empty(), "nothing to end");
        assert!(matches!(child.try_wait(), Ok(None)), "still running");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    /// A client that dies after its CONNECT is read and checked but before
    /// the reply is never held: its thread and pid may already be reused,
    /// and ConDrv aborts the reply (design §7.3).
    #[test]
    fn a_client_that_dies_before_its_connect_reply_is_never_held() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        assert!(engine.joining.is_some(), "checked against its thread");
        child.kill_single(Signal::Kill).expect("kill");
        let _ = Box::new(child).wait().expect("wait");
        engine
            .work()
            .expect("a failed reply is not an engine error");
        assert!(engine.joining.is_none());
        assert!(engine.clients.is_empty(), "never held");
        engine.apply(Effect::Kill { pid });
        assert!(engine.doomed.is_empty(), "nothing to end");
        engine.teardown();
    }

    #[test]
    fn the_shutdown_latch_waits_for_the_kill_and_is_bounded() {
        let shared = Shared::new(Waker::new().expect("Waker"));
        let started = Instant::now();
        assert!(!shared.wait_shut_down(Duration::from_millis(50)), "not yet");
        assert!(started.elapsed() >= Duration::from_millis(50));
        shared.set_shut_down();
        assert!(shared.wait_shut_down(Duration::ZERO), "latched");
    }

    #[test]
    fn a_client_that_left_is_let_go_and_never_ended() {
        let (mut engine, mut child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        let pid = child.id();
        // Served as the engine serves it: opened through its CONNECT.
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        engine.work().expect("send the reply");
        assert!(engine.clients.contains_key(&pid));
        engine.apply(Effect::Joined { pid, tid: 0 });
        assert_eq!(engine.clients.len(), 1, "one handle per process");
        engine.apply(Effect::Left { pid });
        assert!(engine.clients.is_empty());
        engine.apply(Effect::Kill { pid });
        assert!(engine.doomed.is_empty(), "nothing to end");
        assert!(matches!(child.try_wait(), Ok(None)), "still running");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn ctrl_events_behind_a_held_reply_are_queued_once_it_is_sent() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        // Serve until ping's CONNECT is answered but its reply not sent,
        // then add a CTRL_CLOSE, as a late CONNECT leaves them.
        while engine.reply.is_none() {
            engine.turn().expect("turn");
        }
        engine.apply(Effect::Ctrl {
            pid: 7,
            event: Ctrl::Close,
        });
        assert_eq!(lock(&engine.ctrl.0.state).queue.len(), 0, "held");
        engine.work().expect("turn");
        assert!(engine.reply.is_none(), "the reply went out");
        assert_eq!(
            lock(&engine.ctrl.0.state).queue.pop(),
            Some((7, Ctrl::Close))
        );
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    /// Design §7.2: a streamed write reads one slice a turn, so the
    /// control lane is served between its slices, and the next slice
    /// needs no other wake. DSR queries only: no output to stall on.
    #[test]
    fn a_streamed_write_reads_one_slice_a_turn_and_yields_to_hang_up() {
        let queries = b"\x1b[5n".repeat(8 * PACKET_INPUT_CAP / 4);
        let (console, id, at, len) = crate::console::tests::streaming_write(&queries);
        let (mut engine, child) = engine_on(console);
        engine.test_inputs.insert(id, queries.clone());
        engine.fetch = Some(Fetch { id, at, len });
        // Eight slices: still streaming after three turns only if each
        // turn read one at most.
        let mut last = at;
        for turn in 0..3 {
            assert!(engine.must_poll(), "the next turn needs no other wake");
            engine.work().expect("turn");
            if let Some(f) = engine.fetch {
                assert!(
                    f.at <= last + PACKET_INPUT_CAP,
                    "turn {turn} read more than a slice"
                );
                last = f.at;
            }
            assert!(engine.console.streaming(), "turn {turn}");
        }
        lock(&engine.shared.lanes).control.hang_up = true;
        engine.work().expect("turn");
        assert_ne!(
            engine.grace,
            Grace::Unarmed,
            "the hang-up was served mid-stream"
        );
        assert!(engine.console.streaming(), "the write goes on in the grace");
        turns_without_another_wake(&mut engine);
        assert!(!engine.console.streaming() && engine.fetch.is_none());
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn input_left_behind_an_esc_cut_is_taken_without_another_wake() {
        let (mut engine, child) = engine_on(Console::new(SIZE, condrv::key_for, NATIVE_CODEC));
        engine.shared.write(b"\x1b").expect("write");
        engine.shared.write(b"x").expect("write");
        let turns = turns_without_another_wake(&mut engine);
        assert_eq!(turns, 2, "Escape, then the typing after it");
        assert_eq!(engine.shared.input_len(), 0);
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn input_that_frees_a_waiting_slot_rearms_the_driver_read() {
        // One unit each: every read takes one character.
        let console = crate::console::tests::holding_raw_reads(sched::MAX_WAITING, 2);
        let (mut engine, child) = engine_on(console);
        assert!(!sched::admits_read(engine.console.waiting(), 0));
        engine
            .shared
            .write(&[b'a'; sched::MAX_WAITING])
            .expect("write");
        turns_without_another_wake(&mut engine);
        assert_eq!(engine.console.waiting(), 0, "every read was served");
        assert!(engine.read_in_flight, "the driver read is armed again");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn a_drain_below_the_parked_cap_rearms_the_driver_read() {
        let calls = sched::PARKED_CAP / sched::PACKET_INPUT_CAP - 1;
        let console = crate::console::tests::parking_writes(calls);
        let parked = console.parked();
        assert!(!sched::admits_read(console.waiting(), parked));
        let (mut engine, child) = engine_on(console);
        lock(&engine.shared.lanes).control.output_drained = true;
        turns_without_another_wake(&mut engine);
        assert!(
            engine.console.parked() < parked,
            "the drain completed writes"
        );
        assert!(engine.read_in_flight, "the driver read is armed again");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    /// Design §7.3: closing while a client is calling, so the cancelled
    /// READ_IO may complete successfully in the race. Teardown always
    /// drains it and returns.
    #[test]
    fn teardown_while_a_client_calls_drains_the_cancelled_read_every_time() {
        const ROUNDS: usize = 1_000;
        let cwd = std::env::current_dir().expect("current_dir");
        let mut cmd = Command::new("cmd.exe", cwd);
        // Writes without end: a packet is always about to arrive.
        cmd.argv = vec![
            "/d".into(),
            "/c".into(),
            "for /l %i in (0,0,1) do @echo x".into(),
        ];
        for round in 0..ROUNDS {
            let attached = condrv::spawn_attached(&cmd).expect("spawn_attached");
            let shared = Arc::new(Shared::new(Waker::new().expect("Waker")));
            let mut engine = Engine::new(
                Console::new(SIZE, condrv::key_for, NATIVE_CODEC),
                attached.server,
                attached.wait,
                Arc::clone(&shared),
                CtrlSender(Arc::default()),
            );
            // Vary the moment of the close across rounds.
            for _ in 0..=round % 7 {
                engine.turn().expect("turn");
            }
            let started = Instant::now();
            engine.teardown();
            assert!(
                engine.server.is_none() && !engine.read_in_flight,
                "round {round}"
            );
            assert!(started.elapsed() < CANCEL_GRACE * 2, "round {round}");
            let _ = attached.child.kill_single(Signal::Kill);
        }
    }

    #[test]
    fn a_detached_master_releases_every_parked_write() {
        // Several client budgets' worth: one drain releases only one.
        let console = crate::console::tests::parking_writes(8);
        let (mut engine, child) = engine_on(console);
        engine.shared.detach();
        turns_without_another_wake(&mut engine);
        assert_eq!(engine.console.waiting(), 0, "every write completed");
        assert_eq!(engine.shared.output_len(), 0, "the output was discarded");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn a_detached_master_releases_reads_held_by_their_echo() {
        let (mut engine, child) = engine_on(crate::console::tests::echo_held_reads());
        assert!(engine.console.output_blocked());
        engine.shared.detach();
        turns_without_another_wake(&mut engine);
        assert!(!engine.console.output_blocked(), "the echo went out");
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn a_drain_below_64k_leaves_a_wake_for_a_concurrent_echo_hold() {
        let shared = Shared::new(Waker::new().expect("Waker"));
        shared.push_output(vec![b'x'; 61_440]);
        // A console call has already sampled 61,440. Draining during
        // that call must leave a flag even though it starts below 64K.
        let (control, _) = shared.take_control();
        assert!(!control.output_drained);
        assert_eq!(shared.read(&mut vec![0; 17_921]).expect("drain"), 17_921);
        assert!(shared.take_control().0.output_drained);
        assert!(!shared.take_control().0.output_drained, "coalesced once");
    }

    #[test]
    fn a_detached_master_with_an_idle_read_waits_for_the_grace_not_a_poll() {
        let (mut engine, child) = engine_on(crate::console::tests::holding_raw_reads(1, 8));
        engine.shared.detach();
        engine.work().expect("turn");
        assert_eq!(
            engine.console.waiting(),
            1,
            "the read still waits for input"
        );
        assert!(!engine.drains_itself());
        let now = Instant::now();
        let Some(until) = engine.grace.until(engine.must_poll(), now) else {
            panic!("the grace deadline bounds the wait");
        };
        assert!(until > now, "no immediate poll");
        assert!(until <= now + sched::GRACE);
        engine.teardown();
        let _ = child.kill_single(Signal::Kill);
    }

    #[test]
    fn a_write_ending_in_esc_stays_its_own_slice_when_writes_coalesce() {
        let shared = Arc::new(Shared::new(Waker::new().expect("Waker")));
        // Escape, then typing, both written before the engine drains.
        shared.write(b"\x1b").expect("write");
        shared.write(b"x").expect("write");
        shared.write(b"\x1b[A").expect("write");
        assert_eq!(shared.take_input(16), (b"\x1b".to_vec(), true));
        assert_eq!(shared.take_input(16), (b"x\x1b[A".to_vec(), true));
        // A write that only has an ESC inside is one slice.
        shared.write(b"a\x1bb").expect("write");
        shared.write(b"c\x1b").expect("write");
        assert_eq!(shared.take_input(16), (b"a\x1bbc\x1b".to_vec(), true));
    }

    #[test]
    fn input_ends_where_a_write_ended_not_where_the_engine_cut() {
        let shared = Arc::new(Shared::new(Waker::new().expect("Waker")));
        shared.write(b"ab\x1b").expect("write");
        assert_eq!(
            shared.take_input(2),
            (b"ab".to_vec(), false),
            "more is queued"
        );
        assert_eq!(shared.take_input(16), (b"\x1b".to_vec(), true));
        // A write larger than the lane is still going on when the lane
        // empties: its ESC at the cut is not the end of it.
        let mut big = vec![b'x'; INPUT_LANE_CAP - 1];
        big.extend(b"\x1b[A");
        let writer = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || shared.write(&big))
        };
        while lock(&shared.lanes).input.len() < INPUT_LANE_CAP {
            std::thread::yield_now();
        }
        let (first, end) = shared.take_input(INPUT_LANE_CAP);
        assert_eq!(first.last(), Some(&0x1b));
        assert!(!end, "the writer has more");
        writer.join().expect("writer").expect("write");
        assert_eq!(shared.take_input(16), (b"[A".to_vec(), true));
    }

    #[test]
    fn departed_clients_free_their_ctrl_queue_entries_under_churn() {
        let lane = Arc::new(CtrlLane::default());
        let sender = CtrlSender(Arc::clone(&lane));
        // The worker is stuck in one handler; clients come and go.
        for pid in 0..ctrl::MAX_QUEUED as u32 {
            sender.push(pid, Ctrl::Close);
        }
        for pid in 0..ctrl::MAX_QUEUED as u32 - 1 {
            sender.forget(pid);
        }
        for pid in 1000..1000 + ctrl::MAX_QUEUED as u32 - 1 {
            sender.push(pid, Ctrl::Close);
        }
        let state = lock(&lane.state);
        assert!(!state.dropped_logged, "every live client's Close fits");
        assert_eq!(state.queue.len(), ctrl::MAX_QUEUED);
    }

    #[test]
    fn a_closed_lane_still_hands_out_its_ctrl_close() {
        let lane = Arc::new(CtrlLane::default());
        let sender = CtrlSender(Arc::clone(&lane));
        sender.push(2, Ctrl::Close);
        sender.push(4, Ctrl::Close);
        // Typed during the grace, after the Close events.
        sender.push(3, Ctrl::Break);
        drop(sender);
        assert_eq!(lane.next(), Some((2, Ctrl::Close)));
        assert_eq!(lane.next(), Some((4, Ctrl::Close)));
        assert_eq!(lane.next(), None, "Break is dropped once closed");
    }

    #[test]
    fn a_full_ctrl_queue_drops_new_events() {
        let lane = Arc::new(CtrlLane::default());
        let sender = CtrlSender(Arc::clone(&lane));
        for pid in 0..ctrl::MAX_QUEUED as u32 + 5 {
            sender.push(pid, Ctrl::C);
        }
        let state = lock(&lane.state);
        assert!(state.dropped_logged);
        assert_eq!(state.queue.len(), ctrl::MAX_QUEUED);
    }

    #[test]
    fn a_lost_engine_closes_the_console_and_leaves_the_child_with_the_caller() {
        // The engine closure is dropped unrun, as if its thread had died.
        let starter = prepare_with(SIZE, |_, run| {
            drop(run);
            Ok(())
        })
        .expect("prepare");
        let attached = condrv::spawn_attached(&ping_for_a_while()).expect("spawn_attached");
        let (shared, child) = starter.start(attached);
        let mut buf = [0u8; 8];
        assert_eq!(shared.read(&mut buf).expect("read at EOF"), 0);
        let err = shared.write(b"x").expect_err("write after close");
        assert_eq!(err.kind, ErrorKind::BrokenPipe);
        shared.wait_exit();
        // The caller still holds the child and cleans it up as usual.
        let _ = child.kill_single(Signal::Kill);
        Box::new(child)
            .wait()
            .expect("the caller can reap the child");
    }

    #[test]
    fn refused_reply_output_fails_only_that_call() {
        let attached = condrv::spawn_attached(&ping_for_a_while()).expect("spawn_attached");
        let mut server = attached.server;
        // No packet has this id, so ConDrv refuses the output.
        let mut r = Reply::status([0xEE; 8], msg::STATUS_SUCCESS);
        r.output = vec![b'x'; 8];
        r.output_at = 8;
        let r = deliverable(&mut server, r);
        assert_eq!(r.status, STATUS_UNSUCCESSFUL);
        assert!(r.output.is_empty());
        server.close();
        let _ = attached.child.kill_single(Signal::Kill);
    }
}
