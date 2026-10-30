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

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::term::WinSize;
use platform_windows::WindowsChild;
use platform_windows::condrv::{
    self, Attached, ConsoleServer, CtrlEvent, IoComplete, IoDescriptor, Waker,
};
use platform_windows::sys::handle::OwnedWinHandle;

use crate::console::{Console, Effect, Event, OutputQueue, PREAMBLE, Packet, Reply};
use crate::ctrl::{self, Ctrl, Pushed};
use crate::fixture::{self, FIXTURE_CAP};
use crate::msg::STATUS_UNSUCCESSFUL;
use crate::sched::{self, Control, PACKET_INPUT_CAP, View};

/// Terminal input waiting for the engine; `write` blocks past this.
const INPUT_LANE_CAP: usize = 64 * 1024;
/// How long teardown waits for a cancelled READ_IO before closing anyway.
const CANCEL_GRACE: Duration = Duration::from_secs(1);
/// Readers wake the engine only when they free space it may be waiting
/// for: the client budget (§7.4).
const DRAIN_WAKE_ABOVE: usize = 64 * 1024;

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
    /// Set once the server is closed: `write` and `resize` fail.
    closed: bool,
}

struct Output {
    queue: VecDeque<u8>,
    eof: bool,
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
                closed: false,
            }),
            input_room: Condvar::new(),
            output: Mutex::new(Output {
                queue: VecDeque::new(),
                eof: false,
            }),
            output_ready: Condvar::new(),
            exited: Mutex::new(false),
            exit_seen: Condvar::new(),
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
            if lanes.closed || lanes.control.hang_up {
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

    /// Set the control lane's hang-up flag and wake the engine. Never
    /// blocks beyond the lane's lock; idempotent.
    pub(crate) fn hang_up(&self) {
        let mut lanes = lock(&self.lanes);
        if lanes.closed || lanes.control.hang_up {
            return;
        }
        lanes.control.hang_up = true;
        // A blocked writer now fails instead of waiting for room.
        self.input_room.notify_all();
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
        if before > DRAIN_WAKE_ABOVE && n > 0 {
            lock(&self.lanes).control.output_drained = true;
            self.waker.wake();
        }
        Ok(n)
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

    // The engine's side.

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

    fn output_len(&self) -> usize {
        lock(&self.output).queue.len()
    }

    fn push_output(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        lock(&self.output).queue.extend(bytes);
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
    /// Deliver queued events until the lane closes; queued events are
    /// dropped then, as their clients are gone.
    fn run(&self) {
        loop {
            let mut state = lock(&self.state);
            let next = loop {
                if state.closed {
                    return;
                }
                if let Some(next) = state.queue.pop() {
                    break next;
                }
                state = self
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            };
            drop(state);
            let (pid, event) = next;
            if let Err(e) = condrv::send_ctrl(pid, ctrl_event(event)) {
                crate::diag::log(&format!("Ctrl delivery to {pid} failed: {e}"));
            }
        }
    }
}

fn ctrl_event(event: Ctrl) -> CtrlEvent {
    match event {
        Ctrl::C => CtrlEvent::C,
        Ctrl::Break => CtrlEvent::Break,
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
        let console = Console::new(size, condrv::key_for);
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
    /// A wait reported the child's exit.
    exited: bool,
    exit_handled: bool,
    /// Calls the console holds without a reply yet, for the panic path.
    pending: Vec<[u8; 8]>,
    /// `handle` panicked; nothing more reaches the console.
    dead: bool,
    recorder: Option<Recorder>,
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
            exited: false,
            exit_handled: false,
            pending: Vec::new(),
            dead: false,
            recorder: Recorder::from_env(),
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
    }

    /// One turn of design §7.2's loop.
    fn turn(&mut self) -> Result<()> {
        let (control, input) = self.shared.take_control();
        let plan = sched::turn(&View {
            control,
            exited: self.exited && !self.exit_handled,
            output: self.shared.output_len(),
            input,
            read_in_flight: self.read_in_flight,
            waiting: self.console.waiting(),
            parked: self.console.parked(),
            closed: false,
            reads_ready: self.console.reads_ready(),
        });
        if plan.control.hang_up {
            self.handle(Event::HangUp);
        }
        if let Some(size) = plan.control.resize {
            self.handle(Event::Resize(size));
        }
        if plan.control.output_drained {
            self.handle(Event::OutputDrained);
        }
        if plan.child_exited {
            self.exit_handled = true;
            self.handle(Event::ChildExited);
        }
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
        if plan.submit_read {
            let reply = self.reply.take().map(|r| deliverable(server, r));
            let complete = reply.as_ref().map(Reply::as_complete);
            server.submit_read(complete.as_ref())?;
            self.read_in_flight = true;
        } else if let Some(r) = self.reply.take() {
            // No READ_IO goes out this turn, so the reply can't ride on
            // one: complete it on its own rather than hold the client.
            let r = deliverable(server, r);
            server.complete_io(&r.as_complete())?;
        }
        let until = plan.poll.then(Instant::now);
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

    /// Read a packet's variable input, capped, and hand it to the console.
    fn serve(&mut self, d: IoDescriptor) -> Result<()> {
        let Some(server) = self.server.as_mut() else {
            return Ok(());
        };
        let len = d.input_size as usize;
        let mut payload = vec![0u8; if len > PACKET_INPUT_CAP { 0 } else { len }];
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

    /// Run one event through the console, inside the panic boundary, and
    /// apply its effects in order.
    fn handle(&mut self, ev: Event<'_>) {
        let packet = match &ev {
            Event::Packet(p) => Some(p.id),
            _ => None,
        };
        if self.dead {
            if let Some(id) = packet {
                self.reply = Some(Reply::status(id, STATUS_UNSUCCESSFUL));
            }
            return;
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
                return;
            }
        };
        self.shared.push_output(self.out.take());
        let mut replied = false;
        for e in effects {
            if let Effect::Reply(r) = &e {
                replied |= Some(r.id) == packet;
            }
            self.apply(e);
        }
        if let Some(id) = packet
            && !replied
            && self.server.is_some()
        {
            self.pending.push(id);
        }
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
            Effect::Ctrl { pid, event } => self.ctrl.push(pid, event),
            Effect::CloseServer => self.teardown(),
            Effect::Eof => self.shared.set_eof(),
            Effect::Log(d) => crate::diag::log(&d.to_string()),
        }
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
        if let Some(r) = self.reply.take() {
            self.complete(r);
        }
        let Some(mut server) = self.server.take() else {
            return;
        };
        if self.read_in_flight {
            server.cancel_read();
            drain_cancelled(&mut server, &self.shared.waker);
            self.read_in_flight = false;
        }
        server.close();
        self.shared.close_lanes();
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
    use platform::process::{Child, Command, Signal};

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
            Console::new(size, condrv::key_for),
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
        let mut buf = [0u8; 8];
        assert_eq!(shared.read(&mut buf).expect("read at EOF"), 0);
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
