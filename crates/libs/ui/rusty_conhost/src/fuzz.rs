//! Entry points for fuzzing hostile ConDrv traffic (design §6.1, §8.1,
//! PR 13). Built only with the `fuzz` feature, for the cargo-fuzz crate
//! in `fuzz/`, and in this crate's tests, which run a seeded sample on
//! every PR so the driver can't rot between nightly runs.
//!
//! Both entry points panic on a broken invariant, which is what a fuzzer
//! reports:
//!
//! - [`decode`] runs the packet decoders on raw bytes;
//! - [`packets`] reads the bytes as a script of packets, terminal input
//!   and control events, drives a [`Console`] the way the engine does
//!   (admission and turn order from `sched`), and checks every effect
//!   against the limits of §6.1 and §7.4 after each event.

use std::collections::{HashMap, HashSet};

use platform::term::WinSize;

use crate::console::{Console, Effect, Event, OutputQueue, Packet, READ_REPLY_CAP, Reply};
use crate::msg::{self, api};
use crate::sched::{self, Control, PACKET_INPUT_CAP, View, WRITE_BACK_CAP};

/// The output queue's bound (§7.4): the client budget, the host reserve
/// and one emission step.
const OUTPUT_BOUND: usize = (64 + 16 + 64) * 1024;
/// Terminal input the input lane holds before `write` blocks.
const INPUT_LANE: usize = 64 * 1024;
/// Script steps per input, so one input can't run for long.
const MAX_STEPS: usize = 4096;
/// Diagnostics a console may log: [`crate::console::MAX_LOGGED`] lines
/// and one `Suppressed`.
const MAX_LOGS: usize = crate::console::MAX_LOGGED + 1;

/// API numbers the script picks from by index; any other index sends a
/// raw number.
const APIS: [u32; 30] = [
    api::GET_CP,
    api::GET_MODE,
    api::SET_MODE,
    api::GET_NUMBER_OF_INPUT_EVENTS,
    api::GET_CONSOLE_INPUT,
    api::READ_CONSOLE,
    api::WRITE_CONSOLE,
    api::GET_LANG_ID,
    api::GENERATE_CTRL_EVENT,
    api::FLUSH_INPUT_BUFFER,
    api::SET_CP,
    api::GET_CURSOR_INFO,
    api::SET_CURSOR_INFO,
    api::GET_SCREEN_BUFFER_INFO,
    api::SET_CURSOR_POSITION,
    api::SET_TEXT_ATTRIBUTE,
    api::GET_TITLE,
    api::SET_TITLE,
    api::GET_CURRENT_FONT,
    api::GET_CONSOLE_WINDOW,
    api::FILL_OUTPUT,
    api::SCROLL_SCREEN_BUFFER,
    api::READ_OUTPUT,
    api::WRITE_OUTPUT,
    api::READ_OUTPUT_STRING,
    api::WRITE_OUTPUT_STRING,
    api::SET_SCREEN_BUFFER_SIZE,
    api::SET_WINDOW_INFO,
    api::GET_LARGEST_WINDOW_SIZE,
    api::SET_SCREEN_BUFFER_INFO,
];

/// Code pages the driver's codec (`codepage::table`) knows.
const CODE_PAGES: [u32; 4] = [437, 850, 932, 65001];

/// Run every packet decoder on `data`; panics if one breaks its contract.
pub fn decode(data: &[u8]) {
    if let Some(m) = msg::api_msg(data) {
        assert_eq!(8 + m.fixed.len() + m.variable.len(), data.len());
        assert_eq!(msg::u32_at(data, 4), Some(m.fixed.len() as u32));
    }
    let _ = msg::create_object(data);
    if let Ok(info) = msg::server_msg_info(data) {
        // Title holds 261 units, one of them the NUL.
        assert!(
            info.title.len() <= 260,
            "title of {} units",
            info.title.len()
        );
    }
    assert_eq!(msg::utf16_units(data).len(), data.len() / 2);
    let at = data.first().map_or(0, |b| usize::from(*b));
    let _ = (
        msg::u32_at(data, at),
        msg::u16_at(data, at),
        msg::bool_at(data, at),
    );
}

/// Drive a console with the script in `data`; panics on any effect
/// outside the limits.
pub fn packets(data: &[u8]) {
    let mut s = Script(data);
    let size = s.size().unwrap_or(WinSize { rows: 24, cols: 80 });
    let mut e = Engine::new(size);
    for _ in 0..MAX_STEPS {
        let Some(op) = s.u8() else { break };
        e.step(op, &mut s);
        e.turn();
    }
    e.finish();
}

/// The fuzz input, read front to back. Each read takes what it needs, or
/// what is left.
struct Script<'a>(&'a [u8]);

impl<'a> Script<'a> {
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let (head, tail) = self.0.split_at(n.min(self.0.len()));
        self.0 = tail;
        head
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).first().copied()
    }

    fn u16(&mut self) -> u16 {
        let b = self.bytes(2);
        u16::from_le_bytes([
            b.first().copied().unwrap_or(0),
            b.get(1).copied().unwrap_or(0),
        ])
    }

    fn u32(&mut self) -> u32 {
        u32::from(self.u16()) | u32::from(self.u16()) << 16
    }

    /// A length: mostly small, sometimes up to a packet, sometimes past
    /// the per-packet cap.
    fn len(&mut self) -> usize {
        match self.u8().unwrap_or(0) {
            n @ 0..=239 => usize::from(n),
            240..=251 => usize::from(self.u16()),
            _ => usize::from(self.u16()) * 4,
        }
    }

    /// `len` bytes of a short pattern from the script, repeated, so a
    /// short input can still fill a packet or the input lane.
    fn blob(&mut self, len: usize) -> Vec<u8> {
        let n = usize::from(self.u8().unwrap_or(0) % 64) + 1;
        let pattern = self.bytes(n);
        if pattern.is_empty() {
            return vec![0; len];
        }
        pattern.iter().copied().cycle().take(len).collect()
    }

    /// An output size: mostly `base` plus a small length, sometimes any.
    fn output(&mut self, base: usize) -> u32 {
        match self.u8().unwrap_or(0) {
            0..=223 => (base + self.len()) as u32,
            _ => self.u32(),
        }
    }

    /// A terminal size, as the terminal reports one.
    fn size(&mut self) -> Option<WinSize> {
        let rows = u16::from(self.u8()?) + 1;
        let cols = u16::from(self.u8()?) + 1;
        Some(WinSize { rows, cols })
    }
}

/// A call still owed an answer.
struct Owed {
    id: [u8; 8],
    function: u32,
    /// The client's output capacity.
    room: usize,
    /// The most its write-back may hold: CONNECT's connection
    /// information, a USER_DEFINED call's own fixed descriptor (none if
    /// the header doesn't decode), nothing for any other function.
    write_back: usize,
}

impl Owed {
    /// The call as it arrives: `payload` is what the console will see,
    /// so a header that lies about its size bounds nothing.
    fn new(id: [u8; 8], function: u32, payload: &[u8], room: u32) -> Owed {
        let write_back = match function {
            msg::CONNECT => msg::CONNECTION_INFORMATION_LEN,
            msg::USER_DEFINED => msg::api_msg(payload).map_or(0, |m| m.fixed.len()),
            _ => 0,
        };
        Owed {
            id,
            function,
            room: room as usize,
            write_back,
        }
    }
}

/// The engine of `runtime.rs` without the OS: an output queue nobody
/// reads until the script drains it, an input lane, the control lane, and
/// every call still owed an answer.
struct Engine {
    c: Console,
    /// Bytes in the shared output queue.
    queued: usize,
    lane: Vec<u8>,
    control: Control,
    exited: bool,
    exit_handled: bool,
    grace_armed: bool,
    closed: bool,
    eof: bool,
    next_id: u64,
    /// Calls without a reply, with their output size and function.
    owed: Vec<Owed>,
    /// The whole input of each call too large to deliver at once.
    inputs: HashMap<[u8; 8], Vec<u8>>,
    /// The slice a streamed write asked for, answered next turn.
    fetch: Option<([u8; 8], usize, usize)>,
    answered: HashSet<[u8; 8]>,
    logs: usize,
    /// Every cookie the console handed out, live or not, for the script
    /// to name.
    cookies: Vec<usize>,
    /// Pids that joined and haven't left.
    attached: HashSet<u32>,
    killed: HashSet<u32>,
    /// The grace deadline has passed: the next turn handles it first.
    grace_due: bool,
    /// No reader: output is discarded, and blocked calls drain each turn.
    detached: bool,
    /// The last answer's status, for tests.
    last_status: i32,
}

impl Engine {
    fn new(size: WinSize) -> Engine {
        Engine {
            c: Console::new(
                size,
                crate::input::us_key_for,
                crate::codepage::table::CODEC,
            ),
            queued: 0,
            lane: Vec::new(),
            control: Control::default(),
            exited: false,
            exit_handled: false,
            grace_armed: false,
            closed: false,
            eof: false,
            next_id: 0,
            owed: Vec::new(),
            inputs: HashMap::new(),
            fetch: None,
            answered: HashSet::new(),
            logs: 0,
            cookies: Vec::new(),
            attached: HashSet::new(),
            killed: HashSet::new(),
            grace_due: false,
            detached: false,
            last_status: 0,
        }
    }

    /// A cookie to name: mostly one the console handed out, sometimes a
    /// small or any number.
    fn cookie(&self, s: &mut Script<'_>) -> usize {
        match s.u8().unwrap_or(0) {
            0..=223 if !self.cookies.is_empty() => {
                self.cookies[usize::from(s.u16()) % self.cookies.len()]
            }
            n @ 0..=239 => usize::from(n % 16),
            _ => s.u32() as usize,
        }
    }

    /// One script step: a packet, terminal input, or a control event.
    /// Steps that end the console are rare, so most scripts get deep.
    fn step(&mut self, op: u8, s: &mut Script<'_>) {
        match op {
            0..=15 => self.connect(s),
            16..=23 => {
                let process = self.cookie(s);
                self.send(msg::DISCONNECT, process, 0, &[], 0);
            }
            24..=39 => {
                let process = self.cookie(s);
                let info = s.bytes(12);
                self.send(msg::CREATE_OBJECT, process, 0, info, 0);
            }
            40..=47 => {
                let (process, object) = (self.cookie(s), self.cookie(s));
                self.send(msg::CLOSE_OBJECT, process, object, &[], 0);
            }
            48..=79 | 232..=243 => {
                let object = self.cookie(s);
                // The high ops write up to a whole packet.
                let n = if op >= 232 {
                    usize::from(s.u16())
                } else {
                    s.len()
                };
                self.send(msg::RAW_WRITE, 1, object, &s.blob(n), 0);
            }
            80..=95 => {
                let object = self.cookie(s);
                let output = s.output(0);
                self.send(msg::RAW_READ, 1, object, &[], output);
            }
            96..=175 => self.api(s),
            176..=183 => {
                let function = s.u32();
                let n = s.len();
                let (process, object) = (self.cookie(s), self.cookie(s));
                let payload = s.blob(n);
                self.send(function, process, object, &payload, s.u32());
            }
            184..=231 => {
                let n = s.len().min(INPUT_LANE - self.lane.len());
                self.lane.extend(s.blob(n));
            }
            244..=249 => self.control.resize = s.size(),
            250 => self.control.hang_up = true,
            251 => self.exited = true,
            // The deadline passes with whatever is still in flight.
            252 => self.grace_due = self.grace_armed,
            // The terminal goes away: `OwnPtyMaster::detach`.
            253 => {
                self.detached = true;
                self.queued = 0;
                self.control.output_drained = true;
                self.control.hang_up = true;
            }
            // The reader takes everything queued and says so.
            _ => {
                self.queued = 0;
                self.control.output_drained = true;
            }
        }
    }

    /// CONNECT with a well-formed message, so the script gets past it;
    /// malformed ones come from step 10.
    fn connect(&mut self, s: &mut Script<'_>) {
        let pid = usize::from(s.u8().unwrap_or(0));
        let n = s.len().min(260);
        let title: String = s.bytes(n).iter().map(|b| char::from(*b)).collect();
        let group = u32::from(s.u8().unwrap_or(0));
        let m = msg::server_msg_in_group(&title, group);
        let info = msg::CONNECTION_INFORMATION_LEN as u32;
        self.send(msg::CONNECT, pid, pid, &m, info);
    }

    /// A USER_DEFINED call: header, fixed struct, variable data.
    fn api(&mut self, s: &mut Script<'_>) {
        let process = self.cookie(s);
        let object = self.cookie(s);
        let number = match s.u8().unwrap_or(0) {
            n @ 0..=239 => APIS[usize::from(n) % APIS.len()],
            _ => s.u32(),
        };
        let fixed = s.len().min(128);
        let variable = s.len();
        let output = s.output(fixed);
        // Sometimes the header lies about its fixed size.
        let claim = match s.u8() {
            Some(255) => s.u32(),
            _ => fixed as u32,
        };
        let mut payload = Vec::with_capacity(8 + fixed + variable);
        payload.extend(number.to_le_bytes());
        payload.extend(claim.to_le_bytes());
        payload.extend(s.bytes(fixed));
        payload.resize(8 + fixed, 0);
        payload.extend(s.blob(variable));
        // Half the time a SET_CP names a code page the codec knows, so
        // scripts reach the DBCS and lead-byte paths.
        if number == api::SET_CP
            && fixed >= 4
            && let Some(cp) = CODE_PAGES.get(usize::from(payload[8] % 8))
        {
            payload[8..12].copy_from_slice(&cp.to_le_bytes());
        }
        self.send(msg::USER_DEFINED, process, object, &payload, output);
    }

    /// Deliver a packet if the engine would have a READ_IO out for it.
    /// Only the first [`PACKET_INPUT_CAP`] bytes arrive with it, as the
    /// engine reads no more; the rest is kept to answer fetches.
    fn send(&mut self, function: u32, process: usize, object: usize, payload: &[u8], output: u32) {
        if self.closed
            || self.c.streaming()
            || !sched::admits_read(self.c.waiting(), self.c.parked())
        {
            return;
        }
        let input_size = payload.len() as u32;
        self.next_id += 1;
        let id = self.next_id.to_le_bytes();
        if payload.len() > PACKET_INPUT_CAP {
            self.inputs.insert(id, payload.to_vec());
        }
        let payload = &payload[..payload.len().min(PACKET_INPUT_CAP)];
        self.owed.push(Owed::new(id, function, payload, output));
        self.handle(Event::Packet(Packet {
            id,
            process,
            object,
            function,
            input_size,
            output_size: output,
            payload,
        }));
    }

    /// One engine turn up to its wait, in `Engine::work`'s order.
    fn turn(&mut self) {
        if std::mem::take(&mut self.grace_due) && !self.closed {
            self.handle(Event::GraceExpired);
        }
        let mut control = std::mem::take(&mut self.control);
        // Calls wait on output nobody will read: `drains_itself`.
        control.output_drained |= self.detached && self.c.output_blocked();
        let plan = sched::turn(&self.view(control));
        if plan.control.hang_up {
            self.handle(Event::HangUp);
            self.grace_armed = true;
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
            self.grace_armed = true;
        }
        self.serve_fetch();
        if plan.serve_reads {
            self.handle(Event::ServeReads);
        }
        if plan.input > 0 {
            let bytes: Vec<u8> = self.lane.drain(..plan.input).collect();
            let ev = if self.lane.is_empty() {
                Event::Input(&bytes)
            } else {
                Event::InputMore(&bytes)
            };
            self.handle(ev);
        }
    }

    fn view(&self, control: Control) -> View {
        View {
            control,
            exited: self.exited && !self.exit_handled,
            output: self.queued,
            input: self.lane.len(),
            read_in_flight: false,
            waiting: self.c.waiting(),
            parked: self.c.parked(),
            closed: self.closed,
            reads_ready: self.c.reads_ready(),
            input_paused: self.c.input_paused(),
            writes_ready: self.c.writes_ready(),
            streaming: self.c.streaming(),
            fetch_due: self.fetch.is_some(),
        }
    }

    /// Run one event and check the console's state and effects.
    fn handle(&mut self, ev: Event<'_>) {
        let packet = match &ev {
            Event::Packet(p) => Some(p.id),
            _ => None,
        };
        let mut out = OutputQueue::default();
        out.set_held(self.queued);
        let fx = self.c.handle(ev, &mut out);
        let new = out.take().len();
        if !self.detached {
            self.queued += new;
        }
        assert!(
            self.queued <= OUTPUT_BOUND,
            "output queue at {} bytes",
            self.queued
        );
        assert!(
            self.c.waiting() <= sched::MAX_WAITING,
            "{} calls wait",
            self.c.waiting()
        );
        assert!(
            self.c.parked() <= sched::PARKED_CAP,
            "{} bytes parked",
            self.c.parked()
        );
        let mut built = 0;
        let mut fetch = None;
        for e in fx {
            match e {
                Effect::Reply(r) => {
                    assert_eq!(Some(r.id), packet, "a Reply answers the packet in hand");
                    built += self.answer(&r);
                }
                Effect::Complete(r) => built += self.answer(&r),
                Effect::CloseServer => {
                    assert!(!self.closed, "CloseServer twice");
                    self.closed = true;
                    self.fetch = None;
                }
                Effect::Eof => self.eof = true,
                Effect::Log(_) => {
                    self.logs += 1;
                    assert!(self.logs <= MAX_LOGS, "{} log lines", self.logs);
                }
                Effect::Kill { pid } => {
                    assert!(!self.closed, "Kill after CloseServer");
                    assert!(
                        self.attached.contains(&pid),
                        "Kill for pid {pid}, not attached"
                    );
                    assert!(self.killed.insert(pid), "pid {pid} killed twice");
                }
                Effect::Joined { pid, .. } => {
                    self.attached.insert(pid);
                }
                Effect::Left { pid } => {
                    assert!(self.attached.remove(&pid), "pid {pid} left, never joined");
                }
                // Two Ctrl+C in one input slice signal a pid twice; the
                // engine's queue coalesces them, so only the target is checked.
                Effect::Ctrl { pid, .. } => {
                    assert!(
                        self.attached.contains(&pid),
                        "Ctrl to pid {pid}, not attached"
                    );
                }
                Effect::SetInputAvailable(_) => {}
                Effect::Fetch { id, at, len } => {
                    assert!(!self.closed, "Fetch after CloseServer");
                    assert!(fetch.is_none(), "two fetches from one event");
                    assert!((1..=PACKET_INPUT_CAP).contains(&len), "fetch of {len}");
                    fetch = Some((id, at, len));
                }
            }
        }
        assert!(
            built <= READ_REPLY_CAP,
            "one call built {built} bytes of replies"
        );
        if !self.closed {
            assert_eq!(
                self.owed.len(),
                self.c.waiting(),
                "calls owed vs. calls held"
            );
        }
        if fetch.is_some() {
            assert!(
                self.fetch.is_none(),
                "a second fetch before the first is read"
            );
            self.fetch = fetch;
        }
    }

    /// The slice `Engine::serve_fetch` reads this turn, if one is due.
    fn serve_fetch(&mut self) {
        let Some((id, at, len)) = self.fetch.take() else {
            return;
        };
        let input = self.inputs.get(&id).expect("a fetch for a streamed call");
        assert!(at + len <= input.len(), "fetch past the input");
        let bytes = input[at..at + len].to_vec();
        self.handle(Event::Fetched {
            id,
            bytes: Some(&bytes),
        });
    }

    /// Check one answer against its call; returns its variable output.
    fn answer(&mut self, r: &Reply) -> usize {
        assert!(self.answered.insert(r.id), "{:?} answered twice", r.id);
        self.inputs.remove(&r.id);
        let at = self.owed.iter().position(|o| o.id == r.id);
        let Some(at) = at else {
            panic!("{:?} was never asked", r.id)
        };
        let Owed {
            function,
            room,
            write_back,
            ..
        } = self.owed.remove(at);
        self.last_status = r.status;
        if r.status == msg::STATUS_SUCCESS {
            self.learn_cookies(function, r);
        }
        // Past its descriptor, a write-back would run into the variable
        // output region (msg.rs).
        assert!(
            r.write_back.len() <= write_back,
            "write-back of {} for a {write_back}-byte descriptor (function {function})",
            r.write_back.len()
        );
        assert!(
            r.write_back.len() <= WRITE_BACK_CAP,
            "write-back of {}",
            r.write_back.len()
        );
        assert!(
            r.write_back.len() <= room,
            "write-back of {} into {room}",
            r.write_back.len()
        );
        let end = r.output_at as usize + r.output.len();
        assert!(
            r.output.is_empty() || end <= room,
            "output to {end} of {room}"
        );
        r.output.len()
    }

    /// Keep the cookies a successful CONNECT or CREATE_OBJECT handed out.
    fn learn_cookies(&mut self, function: u32, r: &Reply) {
        match function {
            msg::CONNECT => {
                let (words, _) = r.write_back.as_chunks::<8>();
                let cookies = words.iter().map(|w| u64::from_le_bytes(*w) as usize);
                self.cookies.extend(cookies);
            }
            msg::CREATE_OBJECT => self.cookies.push(r.information),
            _ => {}
        }
    }

    /// Hang up, drain and let the grace run out: the console must close,
    /// with its queue still bounded.
    fn finish(&mut self) {
        self.control.hang_up = true;
        self.control.output_drained = true;
        self.queued = 0;
        self.turn();
        if !self.closed && self.grace_armed {
            self.handle(Event::GraceExpired);
        }
        assert!(self.closed && self.eof, "the console did not close");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*: a seeded sample of scripts, enough to exercise every
    /// step on stable without a fuzzer.
    fn scripts(seed: u64, n: usize) -> impl Iterator<Item = Vec<u8>> {
        let mut x = seed;
        let mut next = move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        (0..n).map(move |_| {
            let len = (next() % 4096) as usize;
            (0..len).map(|_| next() as u8).collect()
        })
    }

    #[test]
    fn seeded_scripts_stay_within_limits() {
        for data in scripts(0x1222, 300) {
            decode(&data);
            packets(&data);
        }
    }

    #[test]
    fn an_empty_script_still_closes() {
        packets(&[]);
    }

    fn engine() -> Engine {
        Engine::new(WinSize { rows: 24, cols: 80 })
    }

    /// CONNECT as `pid`: its (process, input, output) cookies.
    fn connect(e: &mut Engine, pid: usize) -> Option<[usize; 3]> {
        let m = msg::server_msg_in_group("", 0);
        let info = msg::CONNECTION_INFORMATION_LEN as u32;
        e.send(msg::CONNECT, pid, pid, &m, info);
        let c = &e.cookies;
        (e.last_status == msg::STATUS_SUCCESS)
            .then(|| [c[c.len() - 3], c[c.len() - 2], c[c.len() - 1]])
    }

    #[test]
    fn the_257th_client_is_refused() {
        let mut e = engine();
        for pid in 0..256 {
            assert!(connect(&mut e, pid).is_some(), "client {pid}");
        }
        assert!(connect(&mut e, 256).is_none());
        assert_eq!(e.last_status, msg::STATUS_NO_MEMORY);
        e.finish();
    }

    #[test]
    fn the_65th_object_of_a_client_is_refused() {
        let mut e = engine();
        connect(&mut e, 1);
        let [process, ..] = connect(&mut e, 2).expect("connects");
        let info: Vec<u8> = [msg::OBJECT_CURRENT_INPUT, 3, 0]
            .iter()
            .flat_map(|v: &u32| v.to_le_bytes())
            .collect();
        // CONNECT's own two count towards the 64.
        for _ in 0..62 {
            e.send(msg::CREATE_OBJECT, process, 0, &info, 0);
            assert_eq!(e.last_status, msg::STATUS_SUCCESS);
        }
        e.send(msg::CREATE_OBJECT, process, 0, &info, 0);
        assert_eq!(e.last_status, msg::STATUS_NO_MEMORY);
        e.finish();
    }

    #[test]
    fn the_grace_expires_with_calls_in_flight() {
        let mut e = engine();
        let [_, input, output] = connect(&mut e, 7).expect("connects");
        e.send(msg::RAW_READ, 1, input, &[], 64);
        e.control.hang_up = true;
        e.turn();
        for _ in 0..4 {
            e.send(msg::RAW_WRITE, 1, output, &[b'x'; 60 * 1024], 0);
        }
        assert!(e.c.waiting() >= 2, "the read and a write wait");
        e.grace_due = e.grace_armed;
        e.turn();
        assert!(e.closed && e.eof);
        assert_eq!(e.killed, HashSet::from([7]));
    }

    /// Turns until the streamed write in flight completes, the reader
    /// draining everything each turn; panics past `limit`.
    fn stream_turns(e: &mut Engine, limit: usize) -> usize {
        for turns in 1..=limit {
            assert!(
                sched::must_poll(&e.view(Control::default())),
                "a due slice needs no other wake"
            );
            e.queued = 0;
            e.control.output_drained = true;
            e.turn();
            if !e.c.streaming() {
                return turns;
            }
        }
        panic!("still streaming after {limit} turns");
    }

    #[test]
    fn a_streamed_write_reads_one_slice_per_turn() {
        let mut e = engine();
        let [_, _, output] = connect(&mut e, 7).expect("connects");
        let slices = 8;
        e.send(
            msg::RAW_WRITE,
            1,
            output,
            &vec![b'x'; slices * PACKET_INPUT_CAP],
            0,
        );
        assert!(e.c.streaming() && e.fetch.is_some());
        // An always-draining reader still gets one slice a turn.
        let turns = stream_turns(&mut e, 100);
        assert!(turns >= slices - 1, "{turns} turns for {slices} slices");
        assert_eq!((e.c.waiting(), e.c.parked()), (0, 0));
        assert_eq!(e.last_status, msg::STATUS_SUCCESS);
        e.finish();
    }

    #[test]
    fn a_query_only_stream_yields_to_hang_up_and_the_deadline() {
        let mut e = engine();
        let [_, _, output] = connect(&mut e, 7).expect("connects");
        // DSR queries alone: answers, no terminal output to stall on.
        let queries = b"\x1b[5n".repeat(8 * PACKET_INPUT_CAP / 4);
        e.send(msg::RAW_WRITE, 1, output, &queries, 0);
        assert!(e.c.streaming());
        e.queued = 0;
        e.turn();
        assert!(e.c.streaming(), "one slice a turn");
        // Hang-up between slices is served in the next turn.
        e.control.hang_up = true;
        e.turn();
        assert!(e.grace_armed, "the hang-up was handled mid-stream");
        assert!(e.c.streaming(), "the write goes on during the grace");
        // So is the deadline: the console closes with the write unfinished.
        e.grace_due = true;
        e.turn();
        assert!(e.closed && e.eof);
        assert!(e.fetch.is_none(), "no slice is read after the close");
        assert_eq!(e.killed, HashSet::from([7]));
    }

    #[test]
    fn a_header_that_overstates_its_fixed_size_is_refused() {
        let mut e = engine();
        let [_, _, output] = connect(&mut e, 7).expect("connects");
        let mut payload = Vec::new();
        payload.extend(api::WRITE_CONSOLE.to_le_bytes());
        payload.extend(200u32.to_le_bytes());
        payload.extend([0; 16]);
        e.send(msg::USER_DEFINED, 1, output, &payload, 256);
        assert_eq!(e.last_status, msg::STATUS_ILLEGAL_FUNCTION);
        assert_eq!((e.c.waiting(), e.c.parked()), (0, 0));
        e.finish();
    }

    /// A USER_DEFINED payload: header, `fixed` and no variable part.
    fn api_payload(number: u32, fixed: &[u8]) -> Vec<u8> {
        let mut p = number.to_le_bytes().to_vec();
        p.extend((fixed.len() as u32).to_le_bytes());
        p.extend(fixed);
        p
    }

    fn reply(id: [u8; 8], write_back: usize) -> Reply {
        Reply {
            id,
            status: msg::STATUS_SUCCESS,
            information: 0,
            write_back: vec![0; write_back],
            output: Vec::new(),
            output_at: 0,
        }
    }

    /// A `CONSOLE_READCONSOLE_MSG` for an ANSI read: 20 bytes.
    const READ_CONSOLE_MSG: [u8; 20] = [0; 20];

    #[test]
    #[should_panic(expected = "write-back of 8 for a 4-byte descriptor")]
    fn a_write_back_past_its_descriptor_is_caught() {
        let mut e = engine();
        let id = 99u64.to_le_bytes();
        let payload = api_payload(api::GET_CP, &[0; 4]);
        // 256 bytes of room and 8 <= WRITE_BACK_CAP: only the descriptor
        // bound can catch it.
        e.owed.push(Owed::new(id, msg::USER_DEFINED, &payload, 256));
        e.answer(&reply(id, 8));
    }

    #[test]
    fn a_header_that_lies_bounds_no_write_back() {
        let mut lying = api_payload(api::GET_CP, &[0; 4]);
        lying[4..8].copy_from_slice(&200u32.to_le_bytes());
        let o = Owed::new([0; 8], msg::USER_DEFINED, &lying, 256);
        assert_eq!(o.write_back, 0);
        let past_cap = Owed::new([0; 8], msg::USER_DEFINED, &[], 256);
        assert_eq!(past_cap.write_back, 0);
    }

    /// A cooked READ_CONSOLE waits for its line.
    fn deferred_read(e: &mut Engine) -> [u8; 8] {
        let [_, input, _] = connect(e, 7).expect("connects");
        let payload = api_payload(api::READ_CONSOLE, &READ_CONSOLE_MSG);
        e.send(msg::USER_DEFINED, 1, input, &payload, 256);
        assert_eq!(e.c.waiting(), 1, "the read waits");
        assert_eq!(e.owed[0].write_back, READ_CONSOLE_MSG.len());
        e.owed[0].id
    }

    #[test]
    fn a_deferred_read_completes_within_its_descriptor() {
        let mut e = engine();
        let id = deferred_read(&mut e);
        e.lane.extend(b"hi\r");
        e.turn();
        assert!(e.answered.contains(&id), "the line completes the read");
        assert!(e.owed.is_empty());
        e.finish();
    }

    #[test]
    #[should_panic(expected = "write-back of 21 for a 20-byte descriptor")]
    fn a_deferred_completion_keeps_its_bound() {
        let mut e = engine();
        let id = deferred_read(&mut e);
        e.answer(&reply(id, 21));
    }

    #[test]
    fn a_dbcs_pair_split_across_writes_stays_within_limits() {
        let mut e = engine();
        let [process, _, output] = connect(&mut e, 7).expect("connects");
        let mut fixed = 932u32.to_le_bytes().to_vec();
        fixed.extend([1, 0, 0, 0]);
        e.send(
            msg::USER_DEFINED,
            process,
            output,
            &api_payload(api::SET_CP, &fixed),
            8,
        );
        assert_eq!(e.last_status, msg::STATUS_SUCCESS);
        e.send(msg::RAW_WRITE, process, output, b"a\x82", 0);
        e.send(msg::RAW_WRITE, process, output, b"\xa0b", 0);
        assert_eq!(e.last_status, msg::STATUS_SUCCESS);
        e.finish();
    }

    #[test]
    fn a_detached_reader_drains_parked_writes() {
        let mut e = engine();
        let [_, _, output] = connect(&mut e, 7).expect("connects");
        while sched::admits_read(e.c.waiting(), e.c.parked()) {
            e.send(msg::RAW_WRITE, 1, output, &[b'x'; 60 * 1024], 0);
        }
        assert!(e.c.parked() > 0);
        e.detached = true;
        e.queued = 0;
        for _ in 0..64 {
            e.turn();
        }
        assert_eq!((e.c.waiting(), e.c.parked()), (0, 0));
        e.finish();
    }
}
