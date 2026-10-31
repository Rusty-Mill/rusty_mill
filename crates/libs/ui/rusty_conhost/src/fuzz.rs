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

use std::collections::HashSet;

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
const APIS: [u32; 20] = [
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
];

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

    /// A cookie: ConDrv echoes the console's own, which count up from 1,
    /// so mostly a small one.
    fn cookie(&mut self) -> usize {
        match self.u8().unwrap_or(0) {
            n @ 0..=223 => usize::from(n % 8),
            n => usize::from(n),
        }
    }

    /// A terminal size, as the terminal reports one.
    fn size(&mut self) -> Option<WinSize> {
        let rows = u16::from(self.u8()?) + 1;
        let cols = u16::from(self.u8()?) + 1;
        Some(WinSize { rows, cols })
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
    /// Calls without a reply, with their output size.
    owed: Vec<([u8; 8], u32)>,
    answered: HashSet<[u8; 8]>,
    logs: usize,
}

impl Engine {
    fn new(size: WinSize) -> Engine {
        Engine {
            c: Console::new(size, crate::input::us_key_for),
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
            answered: HashSet::new(),
            logs: 0,
        }
    }

    /// One script step: a packet, terminal input, or a control event.
    /// Steps that end the console are rare, so most scripts get deep.
    fn step(&mut self, op: u8, s: &mut Script<'_>) {
        match op {
            0..=15 => self.connect(s),
            16..=23 => self.send(msg::DISCONNECT, s.cookie(), 0, &[], 0),
            24..=39 => {
                let process = s.cookie();
                let info = s.bytes(12);
                self.send(msg::CREATE_OBJECT, process, 0, info, 0);
            }
            40..=47 => self.send(msg::CLOSE_OBJECT, s.cookie(), s.cookie(), &[], 0),
            48..=79 | 232..=243 => {
                let object = s.cookie();
                // The high ops write up to a whole packet.
                let n = if op >= 232 {
                    usize::from(s.u16())
                } else {
                    s.len()
                };
                self.send(msg::RAW_WRITE, 1, object, &s.blob(n), 0);
            }
            80..=95 => {
                let object = s.cookie();
                let output = s.output(0);
                self.send(msg::RAW_READ, 1, object, &[], output);
            }
            96..=175 => self.api(s),
            176..=183 => {
                let function = s.u32();
                let n = s.len();
                self.send(function, s.cookie(), s.cookie(), &s.blob(n), s.u32());
            }
            184..=231 => {
                let n = s.len().min(INPUT_LANE - self.lane.len());
                self.lane.extend(s.blob(n));
            }
            244..=251 => self.control.resize = s.size(),
            252 => self.control.hang_up = true,
            253 => self.exited = true,
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
        let pid = s.cookie();
        let n = s.len().min(260);
        let title: String = s.bytes(n).iter().map(|b| char::from(*b)).collect();
        let group = u32::from(s.u8().unwrap_or(0));
        let m = msg::server_msg_in_group(&title, group);
        let info = msg::CONNECTION_INFORMATION_LEN as u32;
        self.send(msg::CONNECT, pid, pid, &m, info);
    }

    /// A USER_DEFINED call: header, fixed struct, variable data.
    fn api(&mut self, s: &mut Script<'_>) {
        let process = s.cookie();
        let object = s.cookie();
        let number = match s.u8().unwrap_or(0) {
            n @ 0..=239 => APIS[usize::from(n) % APIS.len()],
            _ => s.u32(),
        };
        let fixed = s.len().min(128);
        let variable = s.len();
        let output = s.output(fixed);
        let mut payload = Vec::with_capacity(8 + fixed + variable);
        payload.extend(number.to_le_bytes());
        payload.extend((fixed as u32).to_le_bytes());
        payload.extend(s.bytes(fixed));
        payload.resize(8 + fixed, 0);
        payload.extend(s.blob(variable));
        self.send(msg::USER_DEFINED, process, object, &payload, output);
    }

    /// Deliver a packet if the engine would have a READ_IO out for it.
    /// A payload past the per-packet cap arrives empty, as the engine
    /// reads none of it.
    fn send(&mut self, function: u32, process: usize, object: usize, payload: &[u8], output: u32) {
        if self.closed || !sched::admits_read(self.c.waiting(), self.c.parked()) {
            return;
        }
        let input_size = payload.len() as u32;
        let payload = if payload.len() > PACKET_INPUT_CAP {
            &[][..]
        } else {
            payload
        };
        self.next_id += 1;
        let id = self.next_id.to_le_bytes();
        self.owed.push((id, output));
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
        let control = std::mem::take(&mut self.control);
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
        self.queued += out.take().len();
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
                }
                Effect::Eof => self.eof = true,
                Effect::Log(_) => {
                    self.logs += 1;
                    assert!(self.logs <= MAX_LOGS, "{} log lines", self.logs);
                }
                Effect::Kill { .. } => assert!(!self.closed, "Kill after CloseServer"),
                Effect::SetInputAvailable(_)
                | Effect::Ctrl { .. }
                | Effect::Joined { .. }
                | Effect::Left { .. } => {}
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
    }

    /// Check one answer against its call; returns its variable output.
    fn answer(&mut self, r: &Reply) -> usize {
        assert!(self.answered.insert(r.id), "{:?} answered twice", r.id);
        let at = self.owed.iter().position(|(id, _)| *id == r.id);
        let Some(at) = at else {
            panic!("{:?} was never asked", r.id)
        };
        let (_, room) = self.owed.remove(at);
        let room = room as usize;
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
}
