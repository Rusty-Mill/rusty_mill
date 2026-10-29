//! The engine thread and the state it shares with callers (design §7).
//!
//! One engine thread per console owns the [`Console`], the
//! [`ConsoleServer`] and the child's wait handle. Callers (`read`,
//! `write`, `resize`, `hang_up`, Drop) only touch [`Shared`]: the input
//! lane, the control lane, the output queue and the latches. No lock in
//! `Shared` is held across an OS call or across [`Console::handle`].

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::term::WinSize;
use platform_windows::condrv::{ConsoleServer, IoComplete, IoDescriptor, Waker};
use platform_windows::sys::handle::OwnedWinHandle;

use crate::console::{Console, Effect, Event, OutputQueue, Packet, Reply};
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
        while !rest.is_empty() {
            if lanes.closed || lanes.control.hang_up {
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
            self.waker.wake();
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

    /// Drop up to `n` bytes of terminal input: there is no input path
    /// until PR 3.
    fn discard_input(&self, n: usize) {
        let mut lanes = lock(&self.lanes);
        let n = n.min(lanes.input.len());
        lanes.input.drain(..n);
        self.input_room.notify_all();
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
}

/// Start the engine thread for an attached child. On failure the server
/// is closed here and the caller still holds the child.
pub(crate) fn start(
    server: ConsoleServer,
    wait: OwnedWinHandle,
    size: WinSize,
) -> Result<Arc<Shared>> {
    let shared = Arc::new(Shared::new(Waker::new()?));
    let engine = Engine::new(Console::new(size), server, wait, Arc::clone(&shared));
    std::thread::Builder::new()
        .name("rusty_conhost-engine".into())
        .spawn(move || engine.run())
        .map_err(|_| {
            PlatformError::new(
                ErrorKind::Other,
                OsCode::None,
                "rusty_conhost: the engine thread could not start",
            )
        })?;
    Ok(shared)
}

struct Engine {
    console: Console,
    /// `None` once closed.
    server: Option<ConsoleServer>,
    wait: OwnedWinHandle,
    shared: Arc<Shared>,
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
    ) -> Engine {
        Engine {
            console,
            server: Some(server),
            wait,
            shared,
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
        while self.server.is_some() {
            if let Err(e) = self.turn() {
                crate::diag::log(&format!("engine stopped: {e}"));
                self.teardown();
            }
        }
        self.shared.set_eof();
        self.shared.set_exited();
        self.shared.close_lanes();
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
        if plan.input > 0 {
            self.shared.discard_input(plan.input);
            self.handle(Event::Input);
        }
        let Some(server) = self.server.as_mut() else {
            return Ok(());
        };
        if plan.submit_read {
            let reply = self.reply.take();
            let complete = reply.as_ref().map(Reply::as_complete);
            if let Some(r) = &reply {
                write_payload(server, r)?;
            }
            server.submit_read(complete.as_ref())?;
            self.read_in_flight = true;
        } else if let Some(r) = self.reply.take() {
            // No READ_IO goes out this turn, so the reply can't ride on
            // one: complete it on its own rather than hold the client.
            write_payload(server, &r)?;
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
                    self.complete(&old);
                }
            }
            Effect::Complete(r) => {
                self.pending.retain(|id| *id != r.id);
                self.complete(&r);
            }
            Effect::CloseServer => self.teardown(),
            Effect::Eof => self.shared.set_eof(),
            Effect::Log(d) => crate::diag::log(&d.to_string()),
        }
    }

    /// Complete one call now, best effort: a failure only means its client
    /// is gone.
    fn complete(&mut self, r: &Reply) {
        let Some(server) = self.server.as_mut() else {
            return;
        };
        if let Err(e) = write_payload(server, r).and_then(|()| server.complete_io(&r.as_complete()))
        {
            crate::diag::log(&format!("COMPLETE_IO failed: {e}"));
        }
    }

    /// The console panicked (§6.2): fail every call it held, then close.
    fn die(&mut self, packet: Option<[u8; 8]>) {
        crate::diag::log("the console panicked; closing it");
        self.dead = true;
        let ids: Vec<[u8; 8]> = self.pending.drain(..).chain(packet).collect();
        for id in ids {
            self.complete(&Reply::status(id, STATUS_UNSUCCESSFUL));
        }
        self.teardown();
        self.shared.set_eof();
    }

    /// Close the server (§7.3): answer the held reply, cancel and drain
    /// the in-flight read, then close.
    fn teardown(&mut self) {
        if let Some(r) = self.reply.take() {
            self.complete(&r);
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

/// Write a reply's variable output, if any, at its offset.
fn write_payload(server: &mut ConsoleServer, r: &Reply) -> Result<()> {
    if r.output.is_empty() {
        return Ok(());
    }
    server.write_output(r.id, r.output_at, &r.output)
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
    use platform::process::{Child, Command, Signal};
    use platform_windows::condrv;

    use super::*;
    use crate::console::PANIC_FOR_TEST;
    use crate::msg;

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
            Console::new(size),
            attached.server,
            attached.wait,
            Arc::clone(&shared),
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
}
