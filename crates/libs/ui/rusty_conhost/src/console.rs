//! The sans-I/O console (design §3.3): [`Console::handle`] takes one
//! [`Event`] and returns the [`Effect`]s for the engine to apply, in
//! order. It makes no OS call. VT for the terminal goes straight into the
//! [`OutputQueue`], so output order is the order calls were handled.
//!
//! It covers connection and objects, the output APIs and stubs of
//! Appendix A, the output caps of §7.4, and input (PR 3, the `read`
//! child module): terminal input, the input APIs, waiting reads and Ctrl
//! events. Hang-up or child exit starts the close (§7.3): every client
//! gets CTRL_CLOSE and the console keeps serving them until the last one
//! disconnects or the engine's grace deadline passes.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/server/IoDispatchers.cpp`, `ApiDispatchers.cpp`,
//! `ObjectHandle.cpp`, `ObjectHeader.cpp`, `src/host/getset.cpp`,
//! `srvinit.cpp`, `inputBuffer.cpp`, `screenInfo.hpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use platform::term::WinSize;

use crate::ctrl::Ctrl;
use crate::input::{self, KeyFor};
use crate::msg::{self, ApiMsg, api};
use crate::sched::{MAX_RETAINED_PER_CALL, PACKET_INPUT_CAP, WRITE_BACK_CAP};

mod read;

/// What the host sends the terminal before anything else (§5.5):
/// win32-input-mode, for exact key records and Ctrl+Break, and focus
/// reports, as inbox conhost asks for them.
pub(crate) const PREAMBLE: &[u8] = b"\x1b[?9001h\x1b[?1004h";

/// A call that produces VT is admitted only while the output queue holds
/// at most this much (§7.4).
pub(crate) const CLIENT_BUDGET: usize = 64 * 1024;
/// The most VT one emission step adds.
pub(crate) const SLICE: usize = 64 * 1024;
/// UTF-16 units per slice: each unit becomes at most 3 bytes of UTF-8.
const WIDE_SLICE: usize = SLICE / 3;
/// Connected clients.
pub(crate) const MAX_CLIENTS: usize = 256;
// Every client's CTRL_CLOSE fits the Ctrl queue.
const _: () = assert!(MAX_CLIENTS <= crate::ctrl::MAX_QUEUED);
/// Objects per client, the two from CONNECT included.
pub(crate) const MAX_OBJECTS: usize = 64;
/// Objects in all. Objects a client opened outlive its DISCONNECT until
/// ConDrv closes them, so the per-client cap alone does not bound them.
const MAX_ALL_OBJECTS: usize = MAX_CLIENTS * MAX_OBJECTS;

/// A test-only API number that panics inside `handle`, for the panic
/// path's tests.
#[cfg(test)]
pub(crate) const PANIC_FOR_TEST: u32 = 0xFFFF_FFFF;

/// Title units mirrored to the terminal as OSC 0 (§7.4). The stored
/// title is not capped, as in conhost; this keeps a title's OSC far
/// below one packet's input when it parks.
const MAX_OSC_TITLE: usize = 4096;

/// The only code page until PR 9 (§5.9, V4).
const UTF8: u32 = 65001;

/// `ENABLE_*` input mode bits.
mod mode {
    pub(super) const PROCESSED_INPUT: u32 = 0x1;
    pub(super) const LINE_INPUT: u32 = 0x2;
    pub(super) const ECHO_INPUT: u32 = 0x4;
    pub(super) const WINDOW_INPUT: u32 = 0x8;
    pub(super) const MOUSE_INPUT: u32 = 0x10;
    pub(super) const INSERT_MODE: u32 = 0x20;
    pub(super) const QUICK_EDIT_MODE: u32 = 0x40;
    pub(super) const EXTENDED_FLAGS: u32 = 0x80;
    pub(super) const AUTO_POSITION: u32 = 0x100;
    pub(super) const VIRTUAL_TERMINAL_INPUT: u32 = 0x200;
    pub(super) const WRAP_AT_EOL_OUTPUT: u32 = 0x2;
}

/// Input modes the input buffer stores.
const INPUT_MODES: u32 = mode::LINE_INPUT
    | mode::PROCESSED_INPUT
    | mode::ECHO_INPUT
    | mode::WINDOW_INPUT
    | mode::MOUSE_INPUT
    | mode::VIRTUAL_TERMINAL_INPUT;
/// Input modes that are console-wide settings, not stored per buffer.
const PRIVATE_MODES: u32 =
    mode::INSERT_MODE | mode::QUICK_EDIT_MODE | mode::AUTO_POSITION | mode::EXTENDED_FLAGS;
/// PROCESSED_OUTPUT, WRAP_AT_EOL_OUTPUT, VIRTUAL_TERMINAL_PROCESSING,
/// DISABLE_NEWLINE_AUTO_RETURN and LVB_GRID_WORLDWIDE.
const OUTPUT_MODES: u32 = 0x1F;
/// `INPUT_BUFFER_DEFAULT_INPUT_MODE`: LINE, PROCESSED, ECHO and MOUSE.
const DEFAULT_INPUT_MODE: u32 =
    mode::LINE_INPUT | mode::PROCESSED_INPUT | mode::ECHO_INPUT | mode::MOUSE_INPUT;
/// PROCESSED, WRAP_AT_EOL and VIRTUAL_TERMINAL_PROCESSING: what ConPTY
/// reports (Beast, build 26300). Inbox conhost's own default
/// (`SCREEN_INFORMATION::OutputMode`) lacks the VT bit; ConPTY turns it on.
const DEFAULT_OUTPUT_MODE: u32 = 0x7;

/// One packet, its variable input already read (capped, §7.4).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Packet<'a> {
    pub id: [u8; 8],
    pub process: usize,
    pub object: usize,
    pub function: u32,
    pub input_size: u32,
    pub output_size: u32,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event<'a> {
    Packet(Packet<'a>),
    /// VT bytes from the terminal, ending where its write ended.
    Input(&'a [u8]),
    /// VT bytes from the terminal that is still writing: more follow, so
    /// an ESC at their end is not yet the Escape key.
    InputMore(&'a [u8]),
    /// Serve the waiting reads the last call left at its reply budget
    /// ([`Console::reads_ready`]).
    ServeReads,
    Resize(WinSize),
    /// The reader took bytes out of the output queue.
    OutputDrained,
    ChildExited,
    HangUp,
    /// The CTRL_CLOSE grace deadline passed (§7.3).
    GraceExpired,
}

/// The answer to one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reply {
    pub id: [u8; 8],
    pub status: i32,
    pub information: usize,
    /// The fixed API struct, returned through `CD_IO_COMPLETE.Write`.
    pub write_back: Vec<u8>,
    /// Variable output, written at `output_at` before completion.
    pub output: Vec<u8>,
    pub output_at: u32,
}

impl Reply {
    pub(crate) fn status(id: [u8; 8], status: i32) -> Reply {
        Reply {
            id,
            status,
            information: 0,
            write_back: Vec::new(),
            output: Vec::new(),
            output_at: 0,
        }
    }

    fn ok(id: [u8; 8]) -> Reply {
        Reply::status(id, msg::STATUS_SUCCESS)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Sent with the next READ_IO.
    Reply(Reply),
    /// A held call finishes now, with COMPLETE_IO.
    Complete(Reply),
    /// Set or reset ConDrv's input-available event: it is set while the
    /// input queue holds records.
    SetInputAvailable(bool),
    /// Deliver a Ctrl event to a client process, off the engine thread.
    Ctrl {
        pid: u32,
        event: Ctrl,
    },
    /// Disconnect every client.
    CloseServer,
    /// The reader gets `Ok(0)` once the queue drains.
    Eof,
    Log(Diag),
}

/// Something worth one log line. Each kind is logged once per console,
/// or once per (client, API) for unknown APIs, up to [`MAX_LOGGED`]
/// distinct lines; then one [`Diag::Suppressed`] and silence.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Diag {
    UnknownApi { pid: u32, number: u32 },
    UnknownFunction(u32),
    InputDropped,
    PendingDropped,
    TooManyClients,
    Suppressed,
}

/// Distinct diagnostics a console remembers. Clients choose the API
/// numbers, so the set is capped rather than grown per client.
pub(crate) const MAX_LOGGED: usize = 64;

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Diag::UnknownApi { pid, number } => {
                write!(f, "pid {pid}: API {number:#010X} is not implemented")
            }
            Diag::UnknownFunction(n) => write!(f, "ConDrv function {n} is not implemented"),
            Diag::InputDropped => write!(
                f,
                "the input queue holds {} records: the oldest are dropped",
                input::MAX_RECORDS
            ),
            Diag::PendingDropped => write!(
                f,
                "handles keep {} units of read lines: a line's rest was dropped",
                read::MAX_PENDING_UNITS
            ),
            Diag::TooManyClients => write!(f, "a client was refused: {MAX_CLIENTS} are connected"),
            Diag::Suppressed => write!(f, "{MAX_LOGGED} diagnostics logged; the rest are dropped"),
        }
    }
}

/// VT the console produced for the current `handle` call, on top of what
/// the queue already held when the call began.
#[derive(Debug, Default)]
pub(crate) struct OutputQueue {
    held: usize,
    new: Vec<u8>,
}

impl OutputQueue {
    /// Bytes the shared queue holds before this call's output.
    pub(crate) fn set_held(&mut self, held: usize) {
        self.held = held;
    }

    pub(crate) fn len(&self) -> usize {
        self.held + self.new.len()
    }

    /// This call's output, leaving the queue empty.
    pub(crate) fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.new)
    }

    fn push(&mut self, bytes: &[u8]) {
        self.new.extend_from_slice(bytes);
    }
}

/// The object a handle refers to; the discriminant indexes
/// `Console::shares`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Input = 0,
    Output = 1,
}

struct Client {
    pid: u32,
    /// CONNECT's `ProcessGroupId`.
    group: u32,
    /// The input and output objects CONNECT opened, which DISCONNECT
    /// closes.
    own: [usize; 2],
    /// Objects open in this client's name, `own` included.
    objects: usize,
}

/// conhost's console-wide input settings (`CONSOLE_USE_PRIVATE_FLAGS`
/// and the three it gates), reported by GetConsoleMode on input.
#[derive(Clone, Copy, Debug)]
struct Private {
    on: bool,
    insert: bool,
    quick_edit: bool,
    auto_position: bool,
}

impl Private {
    /// Set at console allocation, with conhost's default settings.
    const DEFAULT: Private = Private {
        on: true,
        insert: true,
        quick_edit: true,
        auto_position: true,
    };

    fn bits(self) -> u32 {
        if !self.on {
            return 0;
        }
        [
            (true, mode::EXTENDED_FLAGS),
            (self.insert, mode::INSERT_MODE),
            (self.quick_edit, mode::QUICK_EDIT_MODE),
            (self.auto_position, mode::AUTO_POSITION),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |m, (_, bit)| m | bit)
    }
}

struct Object {
    kind: Kind,
    /// The client cookie that opened it.
    owner: usize,
    access: Access,
    /// What a cooked read on this input handle returned beyond its
    /// client's buffer, for the next read (conhost
    /// `INPUT_READ_HANDLE_DATA`).
    pending: Vec<u16>,
}

/// What a handle counts towards its object's sharing. Only the literal
/// GENERIC_READ and GENERIC_WRITE bits are access, and only
/// FILE_SHARE_READ and FILE_SHARE_WRITE are sharing: GENERIC_ALL alone
/// is neither (conhost `ConsoleHandleData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Access {
    read: bool,
    write: bool,
    read_shared: bool,
    write_shared: bool,
}

impl Access {
    /// The handles CONNECT opens: read, write and share both.
    const CONNECT: Access = Access {
        read: true,
        write: true,
        read_shared: true,
        write_shared: true,
    };

    fn new(desired_access: u32, share_mode: u32) -> Access {
        Access {
            read: desired_access & msg::GENERIC_READ != 0,
            write: desired_access & msg::GENERIC_WRITE != 0,
            read_shared: share_mode & msg::FILE_SHARE_READ != 0,
            write_shared: share_mode & msg::FILE_SHARE_WRITE != 0,
        }
    }
}

/// One object's open counts (conhost `ConsoleObjectHeader`).
#[derive(Clone, Copy, Debug, Default)]
struct Share {
    open: usize,
    readers: usize,
    read_shares: usize,
    writers: usize,
    write_shares: usize,
}

impl Share {
    /// Whether a new handle with `a` is compatible with the open ones.
    fn admits(&self, a: Access) -> bool {
        let read_conflict = a.read && self.open > self.read_shares;
        let refuses_readers = !a.read_shared && self.readers > 0;
        let write_conflict = a.write && self.open > self.write_shares;
        let refuses_writers = !a.write_shared && self.writers > 0;
        !(read_conflict || refuses_readers || write_conflict || refuses_writers)
    }

    /// Count a handle in (`open`) or out.
    fn count(&mut self, a: Access, open: bool) {
        let step = |n: &mut usize, on: bool| {
            if on {
                *n = if open { *n + 1 } else { n.saturating_sub(1) };
            }
        };
        step(&mut self.open, true);
        step(&mut self.readers, a.read);
        step(&mut self.read_shares, a.read_shared);
        step(&mut self.writers, a.write);
        step(&mut self.write_shares, a.write_shared);
    }
}

/// Text a write still has to emit.
enum Text {
    Wide(Vec<u16>),
    Bytes(Vec<u8>),
}

impl Text {
    fn len(&self) -> usize {
        match self {
            Text::Wide(units) => units.len(),
            Text::Bytes(bytes) => bytes.len(),
        }
    }

    /// Payload bytes the write retains while parked.
    fn retained(&self) -> usize {
        match self {
            Text::Wide(units) => units.len() * 2,
            Text::Bytes(bytes) => bytes.len(),
        }
    }

    /// Emit up to one slice from `at`; returns the new position.
    fn emit(&self, at: usize, out: &mut OutputQueue) -> usize {
        match self {
            Text::Bytes(bytes) => {
                let end = bytes.len().min(at + SLICE);
                out.push(&bytes[at..end]);
                end
            }
            Text::Wide(units) => {
                let mut end = units.len().min(at + WIDE_SLICE);
                // Keep a surrogate pair in one slice.
                if end < units.len() && (0xD800..0xDC00).contains(&units[end - 1]) {
                    end -= 1;
                }
                let text: String = char::decode_utf16(units[at..end].iter().copied())
                    .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                    .collect();
                out.push(text.as_bytes());
                end
            }
        }
    }
}

/// A call whose VT is being emitted, with its reply ready for when it is
/// done.
struct Write {
    reply: Reply,
    text: Text,
    at: usize,
    /// Bytes this call holds while queued: text and reply buffers.
    retained: usize,
}

pub(crate) struct Console {
    size: WinSize,
    clients: HashMap<usize, Client>,
    objects: HashMap<usize, Object>,
    /// Sharing of the input buffer and the screen buffer.
    shares: [Share; 2],
    next_cookie: usize,
    input_mode: u32,
    output_mode: u32,
    private: Private,
    title: Vec<u16>,
    /// The first client's title (`GetConsoleOriginalTitle`).
    original_title: Vec<u16>,
    /// Writes in arrival order; only the front emits.
    writes: VecDeque<Write>,
    /// Payload bytes `writes` retain.
    parked: usize,
    connected_once: bool,
    /// Hang-up or child exit was seen: clients have been sent CTRL_CLOSE.
    closing: bool,
    closed: bool,
    /// At most [`MAX_LOGGED`] entries, plus `Suppressed` once full.
    logged: HashSet<Diag>,
    key_for: KeyFor,
    decoder: input::Decoder,
    input: input::Queue,
    /// The last value of the input-available event.
    input_available: bool,
    /// Reads waiting for input, in arrival order.
    reads: VecDeque<read::Read>,
    /// A cooked read's echo waits for the reader with input queued: reads
    /// behind it wait too, so input keeps its order.
    echo_held: bool,
    /// The last pass over waiting reads stopped at its reply budget.
    reads_cut: bool,
    /// UTF-8 bytes of a character an ANSI read could not fit, for the
    /// next ANSI read (conhost `_cachedTextReaderA`).
    cached_text: VecDeque<u8>,
    /// Records an ANSI GetConsoleInput expanded beyond what it asked for.
    cached_records: VecDeque<input::Record>,
    /// A unit taken from the queue but not yet read: the one after a high
    /// surrogate that turned out to be alone, or a high surrogate an ANSI
    /// read holds until its low half arrives.
    held_unit: Option<u16>,
    /// ANSI GetConsoleInput's surrogate halves waiting for their partner,
    /// for key-ups and key-downs apart.
    record_utf8: [input::Utf8Units; 2],
    /// A win32-input-mode surrogate half waiting for its partner before
    /// it goes to a VT client as UTF-8.
    vt_utf8: input::Utf8Units,
}

impl Console {
    pub(crate) fn new(size: WinSize, key_for: KeyFor) -> Console {
        Console {
            size,
            clients: HashMap::new(),
            objects: HashMap::new(),
            shares: [Share::default(); 2],
            next_cookie: 0,
            input_mode: DEFAULT_INPUT_MODE,
            output_mode: DEFAULT_OUTPUT_MODE,
            private: Private::DEFAULT,
            title: Vec::new(),
            original_title: Vec::new(),
            writes: VecDeque::new(),
            parked: 0,
            connected_once: false,
            closing: false,
            closed: false,
            logged: HashSet::new(),
            key_for,
            decoder: input::Decoder::default(),
            input: input::Queue::default(),
            input_available: false,
            reads: VecDeque::new(),
            echo_held: false,
            reads_cut: false,
            cached_text: VecDeque::new(),
            cached_records: VecDeque::new(),
            held_unit: None,
            record_utf8: [input::Utf8Units::default(); 2],
            vt_utf8: input::Utf8Units::default(),
        }
    }

    /// Waiting reads can go on at once: the last call stopped at its
    /// reply budget with input left.
    pub(crate) fn reads_ready(&self) -> bool {
        self.reads_cut
    }

    /// Calls held without a reply: parked writes and waiting reads.
    pub(crate) fn waiting(&self) -> usize {
        self.writes.len() + self.reads.len()
    }

    /// Payload bytes those calls retain.
    pub(crate) fn parked(&self) -> usize {
        self.parked
    }

    /// The window and buffer size, which GetConsoleScreenBufferInfo
    /// reports from PR 4.
    #[cfg(test)]
    pub(crate) fn size(&self) -> WinSize {
        self.size
    }

    /// Handle one event. Pure: every effect is returned.
    pub(crate) fn handle(&mut self, ev: Event<'_>, out: &mut OutputQueue) -> Vec<Effect> {
        let mut fx = Vec::new();
        match ev {
            Event::Packet(p) => self.packet(p, out, &mut fx),
            Event::Input(bytes) => self.terminal_input(bytes, true, out, &mut fx),
            Event::InputMore(bytes) => self.terminal_input(bytes, false, out, &mut fx),
            Event::Resize(size) => self.size = size,
            Event::ServeReads => self.serve_reads(out, &mut fx),
            Event::OutputDrained => {
                self.pump(None, out, &mut fx);
                // Cooked reads held back by their echo go on.
                self.serve_reads(out, &mut fx);
            }
            Event::ChildExited | Event::HangUp => self.start_close(&mut fx),
            Event::GraceExpired => self.close(&mut fx),
        }
        self.sync_input_available(&mut fx);
        fx
    }

    /// Emit the input-available event's new value when it changed.
    fn sync_input_available(&mut self, fx: &mut Vec<Effect>) {
        let available = !self.closed && !self.input.is_empty();
        if available != self.input_available {
            self.input_available = available;
            fx.push(Effect::SetInputAvailable(available));
        }
    }

    fn log(&mut self, d: Diag, fx: &mut Vec<Effect>) {
        if self.logged.len() >= MAX_LOGGED {
            if self.logged.insert(Diag::Suppressed) {
                fx.push(Effect::Log(Diag::Suppressed));
            }
            return;
        }
        if self.logged.insert(d.clone()) {
            fx.push(Effect::Log(d));
        }
    }

    /// Send every client CTRL_CLOSE, once. The console keeps serving them
    /// while their handlers run; it closes on the last DISCONNECT or at
    /// the engine's deadline. With no client it closes now.
    fn start_close(&mut self, fx: &mut Vec<Effect>) {
        if self.closing || self.closed {
            return;
        }
        self.closing = true;
        if self.clients.is_empty() {
            self.close(fx);
            return;
        }
        self.ctrl_event(Ctrl::Close, 0, fx);
    }

    fn close(&mut self, fx: &mut Vec<Effect>) {
        if self.closed {
            return;
        }
        self.closed = true;
        // Their clients are about to be disconnected.
        self.writes.clear();
        self.reads.clear();
        self.echo_held = false;
        self.reads_cut = false;
        self.parked = 0;
        fx.push(Effect::CloseServer);
        fx.push(Effect::Eof);
    }

    fn packet(&mut self, p: Packet<'_>, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        if self.closed {
            fx.push(Effect::Reply(Reply::status(p.id, msg::STATUS_UNSUCCESSFUL)));
            return;
        }
        if p.input_size as usize > PACKET_INPUT_CAP {
            fx.push(Effect::Reply(Reply::status(
                p.id,
                msg::STATUS_INVALID_PARAMETER,
            )));
            return;
        }
        let reply = match p.function {
            msg::CONNECT => self.connect(&p, fx),
            msg::DISCONNECT => {
                self.end_reads(
                    |r| r.process == p.process,
                    msg::STATUS_THREAD_IS_TERMINATING,
                    fx,
                );
                fx.push(Effect::Reply(self.disconnect(&p)));
                if self.clients.is_empty() {
                    // The last client is gone: nothing will use the console.
                    self.close(fx);
                }
                return;
            }
            msg::CREATE_OBJECT => self.create_object(&p),
            msg::CLOSE_OBJECT => {
                self.end_reads(|r| r.object == p.object, msg::STATUS_ALERTED, fx);
                self.close_object(&p)
            }
            msg::RAW_WRITE => {
                let mut reply = Reply::ok(p.id);
                reply.information = p.payload.len();
                return self.write(reply, Text::Bytes(p.payload.to_vec()), out, fx);
            }
            msg::RAW_READ => return self.read_console(&p, None, Vec::new(), out, fx),
            msg::RAW_FLUSH => self.flush_input(&p),
            // The fixed struct is written back, so it must fit the
            // client's output; ConDrv refuses a larger write-back.
            msg::USER_DEFINED => match msg::api_msg(p.payload) {
                Some(m) if m.fixed.len() <= p.output_size as usize => {
                    return self.api(&p, m, out, fx);
                }
                _ => Reply::status(p.id, msg::STATUS_ILLEGAL_FUNCTION),
            },
            f => {
                self.log(Diag::UnknownFunction(f), fx);
                Reply::status(p.id, msg::STATUS_UNSUCCESSFUL)
            }
        };
        fx.push(Effect::Reply(reply));
    }

    fn cookie(&mut self) -> usize {
        self.next_cookie += 1;
        self.next_cookie
    }

    /// CONNECT: the first client's title becomes the console's.
    fn connect(&mut self, p: &Packet<'_>, fx: &mut Vec<Effect>) -> Reply {
        // The cookies are written back, so the client must have room.
        if (p.output_size as usize) < msg::CONNECTION_INFORMATION_LEN {
            return Reply::status(p.id, msg::STATUS_INVALID_BUFFER_SIZE);
        }
        let info = match msg::server_msg_info(p.payload) {
            Ok(info) => info,
            Err(status) => return Reply::status(p.id, status),
        };
        if self.clients.len() >= MAX_CLIENTS || self.objects.len() + 2 > MAX_ALL_OBJECTS {
            self.log(Diag::TooManyClients, fx);
            return Reply::status(p.id, msg::STATUS_NO_MEMORY);
        }
        if !self.shares.iter().all(|s| s.admits(Access::CONNECT)) {
            return Reply::status(p.id, msg::STATUS_FROM_SHARING_VIOLATION);
        }
        if !self.connected_once {
            self.connected_once = true;
            self.original_title = info.title.clone();
            self.title = info.title;
        }
        let process = self.cookie();
        let input = self.cookie();
        let output = self.cookie();
        self.clients.insert(
            process,
            Client {
                // On CONNECT, Process carries the client's pid.
                pid: p.process as u32,
                group: info.group,
                own: [input, output],
                objects: 0,
            },
        );
        for (cookie, kind) in [(input, Kind::Input), (output, Kind::Output)] {
            self.add_object(cookie, kind, process, Access::CONNECT);
        }
        if self.closing {
            // A late client is told too, or it would hold the console
            // open until the deadline.
            fx.push(Effect::Ctrl {
                pid: p.process as u32,
                event: Ctrl::Close,
            });
        }
        let mut reply = Reply::ok(p.id);
        reply.write_back = msg::connection_information(process, input, output);
        reply.information = reply.write_back.len();
        reply
    }

    /// DISCONNECT closes the two objects CONNECT opened. Objects the
    /// client opened itself stay valid until their own CLOSE_OBJECT,
    /// which may arrive after this with Process 0.
    fn disconnect(&mut self, p: &Packet<'_>) -> Reply {
        if let Some(client) = self.clients.remove(&p.process) {
            for cookie in client.own {
                self.remove_object(cookie);
            }
        }
        Reply::ok(p.id)
    }

    /// A GENERIC object resolves by access: exactly GENERIC_READ is the
    /// input, exactly GENERIC_WRITE the output, anything else is invalid.
    /// On success Information is the object's cookie.
    fn create_object(&mut self, p: &Packet<'_>) -> Reply {
        let Some(info) = msg::create_object(p.payload) else {
            return Reply::status(p.id, msg::STATUS_INVALID_PARAMETER);
        };
        let rw = info.desired_access & (msg::GENERIC_READ | msg::GENERIC_WRITE);
        let kind = match info.object_type {
            msg::OBJECT_CURRENT_INPUT => Kind::Input,
            msg::OBJECT_CURRENT_OUTPUT => Kind::Output,
            msg::OBJECT_GENERIC if rw == msg::GENERIC_READ => Kind::Input,
            msg::OBJECT_GENERIC if rw == msg::GENERIC_WRITE => Kind::Output,
            // NEW_OUTPUT: multiple screen buffers are V6.
            msg::OBJECT_NEW_OUTPUT => return Reply::status(p.id, msg::STATUS_NOT_IMPLEMENTED),
            _ => return Reply::status(p.id, msg::STATUS_INVALID_PARAMETER),
        };
        let Some(client) = self.clients.get(&p.process) else {
            return Reply::status(p.id, msg::STATUS_INVALID_HANDLE);
        };
        if client.objects >= MAX_OBJECTS || self.objects.len() >= MAX_ALL_OBJECTS {
            return Reply::status(p.id, msg::STATUS_NO_MEMORY);
        }
        let access = Access::new(info.desired_access, info.share_mode);
        if !self.shares[kind as usize].admits(access) {
            return Reply::status(p.id, msg::STATUS_FROM_SHARING_VIOLATION);
        }
        let cookie = self.cookie();
        self.add_object(cookie, kind, p.process, access);
        let mut reply = Reply::ok(p.id);
        reply.information = cookie;
        reply
    }

    fn close_object(&mut self, p: &Packet<'_>) -> Reply {
        self.remove_object(p.object);
        Reply::ok(p.id)
    }

    fn add_object(&mut self, cookie: usize, kind: Kind, owner: usize, access: Access) {
        self.shares[kind as usize].count(access, true);
        if let Some(c) = self.clients.get_mut(&owner) {
            c.objects += 1;
        }
        self.objects.insert(
            cookie,
            Object {
                kind,
                owner,
                access,
                pending: Vec::new(),
            },
        );
    }

    /// Unknown cookies are ignored: a second close is harmless.
    fn remove_object(&mut self, cookie: usize) {
        let Some(o) = self.objects.remove(&cookie) else {
            return;
        };
        self.shares[o.kind as usize].count(o.access, false);
        if let Some(c) = self.clients.get_mut(&o.owner) {
            c.objects -= 1;
        }
    }

    fn kind(&self, object: usize) -> Option<Kind> {
        self.objects.get(&object).map(|o| o.kind)
    }

    fn api(&mut self, p: &Packet<'_>, m: ApiMsg<'_>, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        // Only the served struct goes back, however large the descriptor.
        let mut fixed = m.fixed[..m.fixed.len().min(WRITE_BACK_CAP)].to_vec();
        let status = match m.number {
            api::GET_CP => self.get_cp(&mut fixed),
            api::SET_CP => set_cp(&fixed),
            api::GET_MODE => self.get_mode(p.object, &mut fixed),
            api::SET_MODE => return self.set_mode(p, fixed, out, fx),
            api::GET_LANG_ID => get_lang_id(),
            api::GET_NUMBER_OF_INPUT_EVENTS => {
                self.get_number_of_input_events(p.object, &mut fixed)
            }
            api::FLUSH_INPUT_BUFFER => match self.input_object(p.object, true) {
                Ok(()) => {
                    self.clear_input();
                    msg::STATUS_SUCCESS
                }
                Err(status) => status,
            },
            api::GENERATE_CTRL_EVENT => self.generate_ctrl_event(&fixed, fx),
            api::GET_CONSOLE_INPUT => return self.get_console_input(p, m, fixed, fx),
            api::READ_CONSOLE => return self.read_console(p, Some(m), fixed, out, fx),
            api::WRITE_CONSOLE => return self.write_console(p, m, fixed, out, fx),
            api::SET_TITLE => return self.set_title(p, m, fixed, out, fx),
            #[cfg(test)]
            PANIC_FOR_TEST => panic!("a test asked the console to panic"),
            api::GET_TITLE => {
                let reply = self.get_title(p, m, fixed);
                fx.push(Effect::Reply(reply));
                return;
            }
            number => {
                let pid = self.clients.get(&p.process).map_or(0, |c| c.pid);
                self.log(Diag::UnknownApi { pid, number }, fx);
                msg::STATUS_ILLEGAL_FUNCTION
            }
        };
        let mut reply = Reply::status(p.id, status);
        reply.write_back = fixed;
        fx.push(Effect::Reply(reply));
    }

    /// `CONSOLE_GETCP_MSG { OUT CodePage; IN Output }`.
    fn get_cp(&self, fixed: &mut [u8]) -> i32 {
        if fixed.len() < 4 {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        msg::put_u32(fixed, 0, UTF8);
        msg::STATUS_SUCCESS
    }

    /// `CONSOLE_MODE_MSG { IN OUT Mode }`. On input, the console-wide
    /// settings are added to the stored mode.
    fn get_mode(&self, object: usize, fixed: &mut [u8]) -> i32 {
        if fixed.len() < 4 {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        let mode = match self.kind(object) {
            Some(Kind::Input) => self.input_mode | self.private.bits(),
            Some(Kind::Output) => self.output_mode,
            None => return msg::STATUS_INVALID_HANDLE,
        };
        msg::put_u32(fixed, 0, mode);
        msg::STATUS_SUCCESS
    }

    fn set_mode(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let Some(mode) = msg::u32_at(&fixed, 0) else {
            fx.push(Effect::Reply(Reply::status(
                p.id,
                msg::STATUS_ILLEGAL_FUNCTION,
            )));
            return;
        };
        let (status, vt) = match self.kind(p.object) {
            Some(Kind::Input) => (self.set_input_mode(mode), None),
            Some(Kind::Output) => self.set_output_mode(mode),
            None => (msg::STATUS_INVALID_HANDLE, None),
        };
        let mut reply = Reply::status(p.id, status);
        reply.write_back = fixed;
        match vt {
            Some(vt) => self.write(reply, Text::Bytes(vt), out, fx),
            None => fx.push(Effect::Reply(reply)),
        }
    }

    /// conhost stores the mode first and only then rejects unknown bits,
    /// or ECHO without LINE: PSReadLine sets 0x1E4 and relies on it.
    fn set_input_mode(&mut self, mode: u32) -> i32 {
        self.private = if mode & PRIVATE_MODES == 0 {
            Private {
                on: false,
                ..self.private
            }
        } else {
            Private {
                on: true,
                insert: mode & mode::INSERT_MODE != 0,
                quick_edit: mode & mode::QUICK_EDIT_MODE != 0,
                auto_position: mode & mode::AUTO_POSITION != 0,
            }
        };
        self.input_mode = mode & !PRIVATE_MODES;
        let unknown = mode & !(INPUT_MODES | PRIVATE_MODES) != 0;
        let echo_without_line = mode & mode::ECHO_INPUT != 0 && mode & mode::LINE_INPUT == 0;
        if unknown || echo_without_line {
            return msg::STATUS_FROM_INVALIDARG;
        }
        msg::STATUS_SUCCESS
    }

    /// Output modes are checked before they are stored. A change of
    /// WRAP_AT_EOL is mirrored to the terminal as DECAWM.
    fn set_output_mode(&mut self, mode: u32) -> (i32, Option<Vec<u8>>) {
        if mode & !OUTPUT_MODES != 0 {
            return (msg::STATUS_FROM_INVALIDARG, None);
        }
        let diff = self.output_mode ^ mode;
        self.output_mode = mode;
        let vt = (diff & mode::WRAP_AT_EOL_OUTPUT != 0).then(|| {
            let on = mode & mode::WRAP_AT_EOL_OUTPUT != 0;
            if on { b"\x1b[?7h" } else { b"\x1b[?7l" }.to_vec()
        });
        (msg::STATUS_SUCCESS, vt)
    }

    /// `CONSOLE_WRITECONSOLE_MSG { OUT NumBytes; IN Unicode }`, the text
    /// in the variable input. NumBytes and Information are the bytes
    /// consumed: all of them.
    fn write_console(
        &mut self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(unicode), true) = (msg::bool_at(&fixed, 4), fixed.len() >= 8) else {
            fx.push(Effect::Reply(Reply::status(
                p.id,
                msg::STATUS_ILLEGAL_FUNCTION,
            )));
            return;
        };
        if self.kind(p.object) != Some(Kind::Output) {
            fx.push(Effect::Reply(Reply::status(
                p.id,
                msg::STATUS_INVALID_HANDLE,
            )));
            return;
        }
        let consumed = if unicode {
            m.variable.len() & !1
        } else {
            m.variable.len()
        };
        msg::put_u32(&mut fixed, 0, consumed as u32);
        let mut reply = Reply::ok(p.id);
        reply.information = consumed;
        reply.write_back = fixed;
        let text = if unicode {
            Text::Wide(msg::utf16_units(m.variable))
        } else {
            // 65001 is the only code page (§5.9): the bytes are UTF-8.
            Text::Bytes(m.variable.to_vec())
        };
        self.write(reply, text, out, fx);
    }

    /// `CONSOLE_SETTITLE_MSG { IN Unicode }`, the title in the variable
    /// input. Mirrored to the terminal as OSC 0, as ConPTY does.
    fn set_title(
        &mut self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let Some(unicode) = msg::bool_at(&fixed, 0) else {
            fx.push(Effect::Reply(Reply::status(
                p.id,
                msg::STATUS_ILLEGAL_FUNCTION,
            )));
            return;
        };
        // Stored as given, as conhost does; the packet cap bounds it.
        let title: Vec<u16> = if unicode {
            msg::utf16_units(m.variable)
        } else {
            String::from_utf8_lossy(m.variable).encode_utf16().collect()
        };
        let mut osc = b"\x1b]0;".to_vec();
        // Controls would end or escape the OSC; the terminal never sees them.
        let shown: String = char::decode_utf16(title.iter().copied().take(MAX_OSC_TITLE))
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .filter(|c| !c.is_control())
            .collect();
        osc.extend(shown.bytes());
        osc.push(0x07);
        self.title = title;
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        self.write(reply, Text::Bytes(osc), out, fx);
    }

    /// `CONSOLE_GETTITLE_MSG { OUT TitleLength; IN Unicode; IN Original }`,
    /// the title being the variable output. See [`title_w`] and
    /// [`title_a`] for conhost's truncation rules.
    fn get_title(&self, p: &Packet<'_>, m: ApiMsg<'_>, mut fixed: Vec<u8>) -> Reply {
        let (Some(unicode), Some(original)) = (msg::bool_at(&fixed, 4), msg::bool_at(&fixed, 5))
        else {
            return Reply::status(p.id, msg::STATUS_ILLEGAL_FUNCTION);
        };
        let at = m.fixed.len();
        let capacity = (p.output_size as usize).saturating_sub(at);
        let title = if original {
            &self.original_title
        } else {
            &self.title
        };
        let t = if unicode {
            title_w(title, capacity / 2)
        } else {
            title_a(String::from_utf16_lossy(title).as_bytes(), capacity)
        };
        msg::put_u32(&mut fixed, 0, t.needed as u32);
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        reply.information = t.information;
        reply.output = t.output;
        reply.output_at = at as u32;
        reply
    }

    /// Queue a write behind any parked ones and emit what the budget
    /// allows. It completes in the next READ_IO if it finishes now, and
    /// with COMPLETE_IO once the reader makes room otherwise.
    fn write(&mut self, reply: Reply, text: Text, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        let id = reply.id;
        let retained = text.retained() + reply.write_back.len() + reply.output.len();
        debug_assert!(
            retained <= MAX_RETAINED_PER_CALL,
            "a call retains {retained} bytes"
        );
        self.parked += retained;
        self.writes.push_back(Write {
            reply,
            text,
            at: 0,
            retained,
        });
        self.pump(Some(id), out, fx);
    }

    /// Emit from the front write while the output queue is within the
    /// client budget. `current` is the packet being handled, whose reply
    /// rides on the next READ_IO.
    fn pump(&mut self, current: Option<[u8; 8]>, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        while let Some(w) = self.writes.front_mut() {
            if w.at < w.text.len() {
                if out.len() > CLIENT_BUDGET {
                    return;
                }
                w.at = w.text.emit(w.at, out);
            }
            if w.at < w.text.len() {
                continue;
            }
            let Some(done) = self.writes.pop_front() else {
                return;
            };
            self.parked -= done.retained;
            if Some(done.reply.id) == current {
                fx.push(Effect::Reply(done.reply));
            } else {
                fx.push(Effect::Complete(done.reply));
            }
        }
    }
}

/// A GetConsoleTitle answer: TitleLength, Information, and the bytes
/// for the client's buffer.
#[derive(Debug, PartialEq, Eq)]
struct TitleOut {
    needed: usize,
    information: usize,
    output: Vec<u8>,
}

/// The W rules: TitleLength is the whole title in units. The buffer of
/// `size` units gets at most `size - 1` of them and a NUL, yet
/// Information reports `min(size, len)` units, in bytes.
fn title_w(title: &[u16], size: usize) -> TitleOut {
    let needed = title.len();
    if size == 0 {
        return TitleOut {
            needed,
            information: 0,
            output: Vec::new(),
        };
    }
    let copied = needed.min(size - 1);
    let output = title[..copied]
        .iter()
        .chain([&0])
        .flat_map(|u| u.to_le_bytes())
        .collect();
    TitleOut {
        needed,
        information: needed.min(size) * 2,
        output,
    }
}

/// The legacy A rules. No buffer gets nothing. A nonempty buffer shorter
/// than the title gets one NUL,
/// Information 1 and TitleLength 0. Otherwise it gets the title and a
/// NUL if there is room, the title's last byte in place of the NUL if
/// there is not, and Information counts the NUL.
fn title_a(title: &[u8], size: usize) -> TitleOut {
    // No buffer: nothing is written, whatever the title.
    if size == 0 {
        return TitleOut {
            needed: 0,
            information: 0,
            output: Vec::new(),
        };
    }
    if size < title.len() {
        return TitleOut {
            needed: 0,
            information: 1,
            output: vec![0],
        };
    }
    let mut output = title.to_vec();
    if size > title.len() {
        output.push(0);
    }
    TitleOut {
        needed: title.len(),
        information: size.min(title.len() + 1),
        output,
    }
}

/// `CONSOLE_SETCP_MSG { IN CodePage; IN Output }`: only 65001 (§5.9).
fn set_cp(fixed: &[u8]) -> i32 {
    match msg::u32_at(fixed, 0) {
        Some(UTF8) => msg::STATUS_SUCCESS,
        Some(_) => msg::STATUS_NOT_SUPPORTED,
        None => msg::STATUS_ILLEGAL_FUNCTION,
    }
}

/// `CONSOLE_LANGID_MSG { OUT LangId }`. conhost answers only for the CJK
/// code pages; under 65001 the call fails and the client falls back to
/// its own UI language.
fn get_lang_id() -> i32 {
    msg::from_hresult(0xD000_00BB)
}

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(windows)]
    pub(crate) use super::read::tests::{holding_raw_reads, owing_large_reads};
    use super::*;

    /// Drives a console as the engine would, with the shared output
    /// queue modelled by a byte count nobody reads until `drain`.
    pub(super) struct Host {
        pub(super) c: Console,
        queued: usize,
        pub(super) high_water: usize,
        pub(super) vt: Vec<u8>,
        next_id: u64,
        /// Each packet's output size, to check its write-back fits.
        output_sizes: HashMap<[u8; 8], u32>,
    }

    impl Host {
        pub(super) fn new() -> Host {
            Host {
                c: Console::new(WinSize { rows: 24, cols: 80 }, input::us_key_for),
                queued: 0,
                high_water: 0,
                vt: Vec::new(),
                next_id: 0,
                output_sizes: HashMap::new(),
            }
        }

        pub(super) fn ev(&mut self, ev: Event<'_>) -> Vec<Effect> {
            let mut out = OutputQueue::default();
            out.set_held(self.queued);
            let fx = self.c.handle(ev, &mut out);
            let new = out.take();
            self.queued += new.len();
            self.high_water = self.high_water.max(self.queued);
            self.vt.extend(new);
            // ConDrv refuses a write-back larger than the client's output.
            for e in &fx {
                if let Effect::Reply(r) | Effect::Complete(r) = e {
                    let room = self.output_sizes.get(&r.id).copied().unwrap_or(0);
                    assert!(r.write_back.len() <= room as usize, "{r:?}");
                }
            }
            fx
        }

        /// The reader takes everything queued.
        pub(super) fn drain(&mut self) -> Vec<Effect> {
            self.queued = 0;
            self.ev(Event::OutputDrained)
        }

        pub(super) fn packet(
            &mut self,
            function: u32,
            process: usize,
            object: usize,
            payload: &[u8],
            output_size: u32,
        ) -> (Reply, Vec<Effect>) {
            self.next_id += 1;
            let id = self.next_id.to_le_bytes();
            self.output_sizes.insert(id, output_size);
            let fx = self.ev(Event::Packet(Packet {
                id,
                process,
                object,
                function,
                input_size: payload.len() as u32,
                output_size,
                payload,
            }));
            // A held call has no reply yet: i32::MIN stands in.
            let reply = fx
                .iter()
                .find_map(|e| match e {
                    Effect::Reply(r) if r.id == id => Some(r.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| Reply::status(id, i32::MIN));
            (reply, fx)
        }

        pub(super) fn call(
            &mut self,
            object: usize,
            number: u32,
            fixed: &[u8],
            variable: &[u8],
            output: u32,
        ) -> Reply {
            let mut payload = Vec::new();
            payload.extend(number.to_le_bytes());
            payload.extend((fixed.len() as u32).to_le_bytes());
            payload.extend(fixed);
            payload.extend(variable);
            let output_size = fixed.len() as u32 + output;
            self.packet(msg::USER_DEFINED, 1, object, &payload, output_size)
                .0
        }

        /// CONNECT as `pid`: the (process, input, output) cookies.
        pub(super) fn connect(&mut self, pid: usize, title: &str) -> [usize; 3] {
            let info = msg::CONNECTION_INFORMATION_LEN as u32;
            let (r, _) = self.packet(msg::CONNECT, pid, 0, &msg::server_msg(title), info);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
            let cookie = |i: usize| {
                let bytes: [u8; 8] = r.write_back[i * 8..i * 8 + 8].try_into().expect("8 bytes");
                u64::from_le_bytes(bytes) as usize
            };
            [cookie(0), cookie(1), cookie(2)]
        }

        pub(super) fn create(&mut self, process: usize, object_type: u32, access: u32) -> Reply {
            self.create_shared(process, object_type, access, 3)
        }

        fn create_shared(
            &mut self,
            process: usize,
            object_type: u32,
            access: u32,
            share: u32,
        ) -> Reply {
            let mut info = Vec::new();
            info.extend(object_type.to_le_bytes());
            info.extend(share.to_le_bytes());
            info.extend(access.to_le_bytes());
            self.packet(msg::CREATE_OBJECT, process, 0, &info, 0).0
        }

        pub(super) fn mode(&mut self, object: usize) -> u32 {
            let r = self.call(object, api::GET_MODE, &[0; 4], &[], 0);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
            msg::u32_at(&r.write_back, 0).expect("Mode")
        }

        pub(super) fn set_mode(&mut self, object: usize, mode: u32) -> i32 {
            self.call(object, api::SET_MODE, &mode.to_le_bytes(), &[], 0)
                .status
        }

        fn write_w(&mut self, output: usize, text: &str) -> Reply {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            self.call(
                output,
                api::WRITE_CONSOLE,
                &[0, 0, 0, 0, 1, 0, 0, 0],
                &bytes,
                0,
            )
        }

        fn get_title(&mut self, unicode: bool, original: bool, capacity: u32) -> Reply {
            let fixed = [0, 0, 0, 0, u8::from(unicode), u8::from(original), 0, 0];
            self.call(1, api::GET_TITLE, &fixed, &[], capacity)
        }
    }

    pub(super) fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    pub(super) fn logs(fx: &[Effect]) -> usize {
        fx.iter().filter(|e| matches!(e, Effect::Log(_))).count()
    }

    #[test]
    fn connect_returns_three_cookies_and_takes_the_first_title() {
        let mut h = Host::new();
        let [process, input, output] = h.connect(100, "cmd.exe");
        assert!(process != input && input != output);
        h.connect(101, "other");
        assert_eq!(h.c.title, units("cmd.exe"));
        assert_eq!(h.c.original_title, units("cmd.exe"));
        let (r, _) = h.packet(msg::CONNECT, 102, 0, &[0; 12], 0);
        assert_eq!(r.status, msg::STATUS_INVALID_BUFFER_SIZE);
    }

    #[test]
    fn connect_reply_carries_the_connection_information() {
        let mut h = Host::new();
        let (r, _) = h.packet(msg::CONNECT, 7, 0, &msg::server_msg(""), 24);
        assert_eq!(r.information, 24);
        assert_eq!(r.write_back.len(), 24);
    }

    #[test]
    fn the_257th_client_is_refused_and_logged_once() {
        let mut h = Host::new();
        for pid in 0..MAX_CLIENTS {
            h.connect(pid, "");
        }
        let (r, fx) = h.packet(msg::CONNECT, 999, 0, &msg::server_msg(""), 24);
        assert_eq!(r.status, msg::STATUS_NO_MEMORY);
        assert!(fx.contains(&Effect::Log(Diag::TooManyClients)));
        let (_, fx) = h.packet(msg::CONNECT, 1000, 0, &msg::server_msg(""), 24);
        assert_eq!(logs(&fx), 0);
    }

    #[test]
    fn create_object_resolves_generic_by_access() {
        let mut h = Host::new();
        let [process, ..] = h.connect(1, "");
        let r = h.create(process, msg::OBJECT_GENERIC, msg::GENERIC_READ);
        assert_eq!(h.c.kind(r.information), Some(Kind::Input));
        let r = h.create(process, msg::OBJECT_GENERIC, msg::GENERIC_WRITE | 0x1);
        assert_eq!(h.c.kind(r.information), Some(Kind::Output));
        let rw = msg::GENERIC_READ | msg::GENERIC_WRITE;
        let status =
            |h: &mut Host, object_type, access| h.create(process, object_type, access).status;
        assert_eq!(
            status(&mut h, msg::OBJECT_GENERIC, rw),
            msg::STATUS_INVALID_PARAMETER
        );
        assert_eq!(
            status(&mut h, msg::OBJECT_GENERIC, 0),
            msg::STATUS_INVALID_PARAMETER
        );
        assert_eq!(
            status(&mut h, msg::OBJECT_NEW_OUTPUT, rw),
            msg::STATUS_NOT_IMPLEMENTED
        );
        assert_eq!(status(&mut h, 9, rw), msg::STATUS_INVALID_PARAMETER);
        let stranger = h.create(12345, msg::OBJECT_CURRENT_OUTPUT, rw);
        assert_eq!(stranger.status, msg::STATUS_INVALID_HANDLE);
        let (r, _) = h.packet(msg::CREATE_OBJECT, process, 0, &[0; 11], 0);
        assert_eq!(r.status, msg::STATUS_INVALID_PARAMETER);
    }

    #[test]
    fn create_object_enforces_share_modes() {
        let mut h = Host::new();
        let [process, ..] = h.connect(1, "");
        let rw = msg::GENERIC_READ | msg::GENERIC_WRITE;
        // CONNECT's handles read and write: an exclusive open must fail.
        let r = h.create_shared(process, msg::OBJECT_CURRENT_OUTPUT, rw, 0);
        assert_eq!(r.status as u32, 0xC007_0020);
        let r = h.create_shared(
            process,
            msg::OBJECT_CURRENT_OUTPUT,
            msg::GENERIC_WRITE,
            msg::FILE_SHARE_WRITE,
        );
        assert_eq!(
            r.status,
            msg::STATUS_FROM_SHARING_VIOLATION,
            "it refuses the open readers"
        );
        // GENERIC_ALL alone is neither read nor write.
        let r = h.create_shared(process, msg::OBJECT_CURRENT_INPUT, 0x1000_0000, 3);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(h.c.objects[&r.information].access, Access::new(0, 3));
    }

    #[test]
    fn share_counts_admit_again_once_a_handle_closes() {
        let exclusive = Access::new(msg::GENERIC_READ | msg::GENERIC_WRITE, 0);
        let reader = Access::new(msg::GENERIC_READ, msg::FILE_SHARE_READ);
        let mut share = Share::default();
        assert!(share.admits(exclusive));
        share.count(exclusive, true);
        assert!(!share.admits(reader));
        assert!(
            !share.admits(Access::new(0, 0)),
            "it refuses the open reader"
        );
        share.count(exclusive, false);
        assert!(share.admits(reader));
        share.count(reader, true);
        assert!(!share.admits(exclusive));
        assert!(share.admits(Access::new(msg::GENERIC_READ, 3)));
        assert!(
            !share.admits(Access::new(msg::GENERIC_WRITE, 3)),
            "the reader does not share write"
        );
    }

    #[test]
    fn a_client_holds_at_most_64_objects() {
        let mut h = Host::new();
        let [process, ..] = h.connect(1, "");
        for _ in 2..MAX_OBJECTS {
            let r = h.create(process, msg::OBJECT_CURRENT_OUTPUT, 0);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
        }
        let over = h.create(process, msg::OBJECT_CURRENT_INPUT, 0);
        assert_eq!(over.status, msg::STATUS_NO_MEMORY);
        let any = *h.c.objects.keys().max().expect("objects");
        h.packet(msg::CLOSE_OBJECT, process, any, &[], 0);
        let again = h.create(process, msg::OBJECT_CURRENT_OUTPUT, 0);
        assert_eq!(again.status, msg::STATUS_SUCCESS);
    }

    #[test]
    fn created_objects_outlive_disconnect_until_closed() {
        let mut h = Host::new();
        let [a, a_in, _] = h.connect(1, "");
        h.connect(2, "");
        let created = h.create(a, msg::OBJECT_CURRENT_OUTPUT, 0).information;
        let (r, fx) = h.packet(msg::DISCONNECT, a, 0, &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert!(
            !fx.contains(&Effect::CloseServer),
            "client 2 is still connected"
        );
        assert_eq!(h.c.kind(a_in), None, "CONNECT's objects go with the client");
        assert_eq!(h.mode(created), DEFAULT_OUTPUT_MODE);
        // ConDrv closes it later, with Process 0.
        let (r, _) = h.packet(msg::CLOSE_OBJECT, 0, created, &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(h.c.kind(created), None);
        let (r, _) = h.packet(msg::CLOSE_OBJECT, 0, created, &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS, "a second close is harmless");
    }

    #[test]
    fn the_last_disconnect_replies_then_closes_the_console() {
        let mut h = Host::new();
        let [a, ..] = h.connect(1, "");
        let (r, fx) = h.packet(msg::DISCONNECT, a, 0, &[], 0);
        assert_eq!(fx, [Effect::Reply(r), Effect::CloseServer, Effect::Eof]);
        let (r, _) = h.packet(msg::CONNECT, 2, 0, &msg::server_msg(""), 24);
        assert_eq!(r.status, msg::STATUS_UNSUCCESSFUL);
        assert!(h.ev(Event::HangUp).is_empty(), "closing is idempotent");
    }

    fn close_to(pid: u32) -> Effect {
        Effect::Ctrl {
            pid,
            event: Ctrl::Close,
        }
    }

    #[test]
    fn hang_up_and_child_exit_send_ctrl_close_once_then_the_grace_closes() {
        for ev in [Event::HangUp, Event::ChildExited] {
            let mut h = Host::new();
            h.connect(1, "");
            h.connect(2, "");
            assert_eq!(h.ev(ev), [close_to(1), close_to(2)]);
            assert!(h.ev(Event::HangUp).is_empty(), "sent once");
            assert!(h.ev(Event::ChildExited).is_empty());
            assert_eq!(
                h.ev(Event::GraceExpired),
                [Effect::CloseServer, Effect::Eof]
            );
            assert!(h.ev(Event::GraceExpired).is_empty(), "closes once");
        }
    }

    #[test]
    fn the_console_serves_handlers_during_the_grace_and_closes_on_the_last_disconnect() {
        let mut h = Host::new();
        let [a, _, a_out] = h.connect(1, "");
        let [b, ..] = h.connect(2, "");
        h.ev(Event::HangUp);
        // A handler's WriteConsole is answered while the console closes.
        let r = h.write_w(a_out, "bye");
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert!(h.vt.ends_with(b"bye"));
        let (_, fx) = h.packet(msg::DISCONNECT, a, 0, &[], 0);
        assert!(!fx.contains(&Effect::CloseServer), "client 2 remains");
        let (r, fx) = h.packet(msg::DISCONNECT, b, 0, &[], 0);
        assert_eq!(fx, [Effect::Reply(r), Effect::CloseServer, Effect::Eof]);
        assert!(h.ev(Event::GraceExpired).is_empty(), "already closed");
    }

    #[test]
    fn a_client_connecting_during_the_grace_gets_ctrl_close() {
        let mut h = Host::new();
        h.connect(1, "");
        h.ev(Event::ChildExited);
        let info = msg::CONNECTION_INFORMATION_LEN as u32;
        let (r, fx) = h.packet(msg::CONNECT, 7, 0, &msg::server_msg(""), info);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(fx, [close_to(7), Effect::Reply(r)]);
    }

    #[test]
    fn hang_up_with_no_client_closes_at_once() {
        for ev in [Event::HangUp, Event::ChildExited] {
            let mut h = Host::new();
            assert_eq!(h.ev(ev), [Effect::CloseServer, Effect::Eof]);
            assert!(h.ev(Event::GraceExpired).is_empty());
        }
    }

    #[test]
    fn a_hang_up_between_a_packet_and_its_completion_keeps_the_call() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let chunk = vec![b'z'; PACKET_INPUT_CAP];
        for _ in 0..4 {
            h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
        }
        let parked = h.c.waiting();
        assert!(parked > 0);
        h.ev(Event::HangUp);
        assert_eq!(h.c.waiting(), parked, "parked writes go on in the grace");
        let fx = h.drain();
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Complete(_))),
            "a drain completes them as before"
        );
    }

    #[test]
    fn default_modes_match_conhost() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        assert_eq!(h.mode(input), 0x1F7);
        assert_eq!(h.mode(output), 0x7);
        let stranger = h.call(12345, api::GET_MODE, &[0; 4], &[], 0);
        assert_eq!(stranger.status, msg::STATUS_INVALID_HANDLE);
    }

    #[test]
    fn an_invalid_input_mode_is_stored_and_then_rejected() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        // PSReadLine's ECHO without LINE.
        assert_eq!(h.set_mode(input, 0x1E4), msg::STATUS_FROM_INVALIDARG);
        assert_eq!(h.mode(input), 0x1E4);
        assert_eq!(h.set_mode(input, 0x403), msg::STATUS_FROM_INVALIDARG);
        assert_eq!(
            h.mode(input),
            0x403,
            "unknown bits are kept; no private bit, none reported"
        );
        assert_eq!(h.set_mode(input, 0x87), msg::STATUS_SUCCESS);
        assert_eq!(
            h.mode(input),
            0x87,
            "EXTENDED_FLAGS alone turns the others off"
        );
    }

    #[test]
    fn an_invalid_output_mode_is_rejected_before_it_is_stored() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        assert_eq!(h.set_mode(output, 0x20), msg::STATUS_FROM_INVALIDARG);
        assert_eq!(h.mode(output), 0x7);
        assert_eq!(h.set_mode(output, 0x1F), msg::STATUS_SUCCESS);
        assert!(h.vt.is_empty(), "WRAP did not change");
        assert_eq!(h.set_mode(output, 0x1D), msg::STATUS_SUCCESS);
        assert_eq!(h.set_mode(output, 0x1F), msg::STATUS_SUCCESS);
        assert_eq!(h.vt, b"\x1b[?7l\x1b[?7h");
    }

    #[test]
    fn write_console_emits_text_and_reports_bytes_consumed() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let r = h.write_w(output, "h\u{e9}\u{1F600}");
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(r.information, 8);
        assert_eq!(msg::u32_at(&r.write_back, 0), Some(8));
        let r = h.call(output, api::WRITE_CONSOLE, &[0; 8], b"!", 0);
        assert_eq!(r.information, 1);
        assert_eq!(h.vt, "h\u{e9}\u{1F600}!".as_bytes());
        assert_eq!(h.write_w(input, "x").status, msg::STATUS_INVALID_HANDLE);
        let short = h.call(output, api::WRITE_CONSOLE, &[0; 4], b"x", 0);
        assert_eq!(short.status, msg::STATUS_ILLEGAL_FUNCTION);
    }

    #[test]
    fn raw_write_passes_bytes_through() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let (r, _) = h.packet(msg::RAW_WRITE, 1, output, b"\x1b[1mhi", 0);
        assert_eq!(r.information, 6);
        assert!(r.write_back.is_empty());
        assert_eq!(h.vt, b"\x1b[1mhi");
        // RAW_FLUSH flushes the input buffer, so it needs the input handle.
        let (r, _) = h.packet(msg::RAW_FLUSH, 1, output, &[], 0);
        assert_eq!(r.status, msg::STATUS_FROM_HANDLE);
        let (r, _) = h.packet(msg::RAW_FLUSH, 1, input, &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
    }

    #[test]
    fn set_title_stores_the_title_and_sends_a_clean_osc() {
        let mut h = Host::new();
        h.connect(1, "first");
        let title = "a\u{1b}]b\u{7}c";
        let bytes: Vec<u8> = title.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            h.call(1, api::SET_TITLE, &[1], &bytes, 0).status,
            msg::STATUS_SUCCESS
        );
        assert_eq!(h.vt, b"\x1b]0;a]bc\x07");
        assert_eq!(h.c.title, units(title));
        h.call(1, api::SET_TITLE, &[0], "\u{e9}t\u{e9}".as_bytes(), 0);
        assert_eq!(h.c.title, units("\u{e9}t\u{e9}"));
        assert_eq!(h.c.original_title, units("first"));
    }

    #[test]
    fn get_title_answers_from_the_current_or_original_title() {
        let mut h = Host::new();
        h.connect(1, "orig");
        h.call(1, api::SET_TITLE, &[0], b"now", 0);
        let r = h.get_title(true, false, 64);
        assert_eq!(msg::u32_at(&r.write_back, 0), Some(3));
        assert_eq!(r.information, 6);
        assert_eq!(r.output, b"n\0o\0w\0\0\0");
        assert_eq!(r.output_at, 8);
        let r = h.get_title(false, true, 64);
        assert_eq!(r.output, b"orig\0");
        assert_eq!(r.information, 5);
    }

    #[test]
    fn title_w_truncates_like_string_cch_copy_n() {
        let t = units("abcd");
        let fits = title_w(&t, 10);
        assert_eq!(
            (fits.needed, fits.information, fits.output.len()),
            (4, 8, 10)
        );
        // Exactly the length: three units and a NUL, yet 4 are reported.
        let exact = title_w(&t, 4);
        assert_eq!((exact.needed, exact.information), (4, 8));
        assert_eq!(exact.output, b"a\0b\0c\0\0\0");
        let short = title_w(&t, 2);
        assert_eq!((short.needed, short.information), (4, 4));
        assert_eq!(short.output, b"a\0\0\0");
        let none = TitleOut {
            needed: 4,
            information: 0,
            output: Vec::new(),
        };
        assert_eq!(title_w(&t, 0), none);
        assert_eq!(title_w(&[], 1).output, b"\0\0");
    }

    #[test]
    fn a_zero_capacity_title_query_writes_nothing() {
        assert_eq!(
            title_a(b"abcd", 0),
            TitleOut {
                needed: 0,
                information: 0,
                output: Vec::new()
            }
        );
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "a title");
        for unicode in [false, true] {
            let r = h.get_title(unicode, false, 0);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
            assert_eq!((r.information, r.output.len()), (0, 0));
        }
        // The console is still serving.
        assert_eq!(h.mode(output), DEFAULT_OUTPUT_MODE);
    }

    #[test]
    fn title_a_follows_the_legacy_rules() {
        let fits = title_a(b"abcd", 10);
        assert_eq!((fits.needed, fits.information), (4, 5));
        assert_eq!(fits.output, b"abcd\0");
        // Exactly the length: the whole title and no NUL.
        let exact = title_a(b"abcd", 4);
        assert_eq!((exact.needed, exact.information), (4, 4));
        assert_eq!(exact.output, b"abcd");
        // Shorter: one NUL, and TitleLength 0.
        let short = title_a(b"abcd", 3);
        assert_eq!(
            (short.needed, short.information, short.output),
            (0, 1, vec![0])
        );
        assert_eq!(title_a(b"", 0).information, 0);
        assert_eq!(title_a(b"", 1).information, 1);
    }

    #[test]
    fn code_page_is_65001_only_and_lang_id_fails_like_conhost() {
        let mut h = Host::new();
        h.connect(1, "");
        let r = h.call(1, api::GET_CP, &[0; 8], &[], 0);
        assert_eq!(msg::u32_at(&r.write_back, 0), Some(65001));
        let set = |h: &mut Host, cp: u32| h.call(1, api::SET_CP, &cp.to_le_bytes(), &[], 0).status;
        assert_eq!(set(&mut h, 65001), msg::STATUS_SUCCESS);
        assert_eq!(set(&mut h, 437), msg::STATUS_NOT_SUPPORTED);
        // HRESULT_FROM_NT(STATUS_NOT_SUPPORTED) through NTSTATUS_FROM_HRESULT.
        let lang = h.call(1, api::GET_LANG_ID, &[0; 4], &[], 0);
        assert_eq!(lang.status as u32, 0xC007_00BB);
    }

    #[test]
    fn an_unknown_api_fails_and_is_logged_once_per_client() {
        let mut h = Host::new();
        h.connect(77, "");
        let [p2, ..] = h.connect(78, "");
        let unknown: u32 = 0x0300_0042;
        let mut payload = unknown.to_le_bytes().to_vec();
        payload.extend(0u32.to_le_bytes());
        let (r, fx) = h.packet(msg::USER_DEFINED, 1, 0, &payload, 0);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
        assert_eq!(logs(&fx), 1);
        assert_eq!(logs(&h.packet(msg::USER_DEFINED, 1, 0, &payload, 0).1), 0);
        assert_eq!(logs(&h.packet(msg::USER_DEFINED, p2, 0, &payload, 0).1), 1);
        let (r, fx) = h.packet(42, p2, 0, &[], 0);
        assert_eq!(r.status, msg::STATUS_UNSUCCESSFUL, "an unknown function");
        assert_eq!(logs(&fx), 1);
    }

    #[test]
    fn a_fixed_struct_larger_than_the_output_fails_only_that_call() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let mut payload = api::GET_MODE.to_le_bytes().to_vec();
        payload.extend(4u32.to_le_bytes());
        payload.extend([0; 4]);
        let (r, _) = h.packet(msg::USER_DEFINED, 1, output, &payload, 0);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
        assert!(r.write_back.is_empty());
        let r = h.call(output, api::GET_MODE, &[0; 4], &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS, "the console still serves");
        assert_eq!(msg::u32_at(&r.write_back, 0), Some(DEFAULT_OUTPUT_MODE));
    }

    #[test]
    fn a_connect_without_room_for_the_cookies_is_refused_and_changes_nothing() {
        let mut h = Host::new();
        let (r, _) = h.packet(msg::CONNECT, 1, 0, &msg::server_msg("x"), 23);
        assert_eq!(r.status, msg::STATUS_INVALID_BUFFER_SIZE);
        assert!(r.write_back.is_empty());
        assert!(h.c.clients.is_empty() && h.c.objects.is_empty());
        h.connect(2, "cmd.exe");
        assert_eq!(h.c.title, units("cmd.exe"), "the first real client's title");
    }

    fn unknown_api(h: &mut Host, process: usize, number: u32) -> Vec<Effect> {
        let mut payload = number.to_le_bytes().to_vec();
        payload.extend(0u32.to_le_bytes());
        let (r, fx) = h.packet(msg::USER_DEFINED, process, 0, &payload, 0);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
        fx
    }

    #[test]
    fn many_unknown_apis_log_a_bounded_set_then_one_summary() {
        let mut h = Host::new();
        h.connect(1, "");
        let total: usize = (0..10_000)
            .map(|n| logs(&unknown_api(&mut h, 1, 0x0300_0000 + n)))
            .sum();
        assert_eq!(total, MAX_LOGGED + 1);
        assert_eq!(h.c.logged.len(), MAX_LOGGED + 1);
        assert!(h.c.logged.contains(&Diag::Suppressed));
        assert_eq!(logs(&unknown_api(&mut h, 1, 0x0300_0000)), 0);
    }

    #[test]
    fn client_churn_does_not_grow_the_log_set() {
        let mut h = Host::new();
        h.connect(1, "");
        for pid in 2..2_000 {
            let [process, ..] = h.connect(pid, "");
            unknown_api(&mut h, process, 0x0300_0042);
            let (r, _) = h.packet(msg::DISCONNECT, process, 0, &[], 0);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
        }
        assert_eq!(h.c.logged.len(), MAX_LOGGED + 1);
    }

    #[test]
    #[should_panic(expected = "a test asked the console to panic")]
    fn the_test_api_panics() {
        let mut h = Host::new();
        h.connect(1, "");
        h.call(1, PANIC_FOR_TEST, &[], &[], 0);
    }

    #[test]
    fn an_oversized_packet_is_refused() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let big = vec![b'x'; PACKET_INPUT_CAP + 1];
        let (r, _) = h.packet(msg::RAW_WRITE, 1, output, &big, 0);
        assert_eq!(r.status, msg::STATUS_INVALID_PARAMETER);
        assert!(h.vt.is_empty());
        let (r, _) = h.packet(msg::RAW_WRITE, 1, output, &big[1..], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
    }

    #[test]
    fn a_stalled_reader_parks_writes_within_the_caps() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let chunk = vec![b'y'; PACKET_INPUT_CAP];
        let mut replied = 0;
        let mut parked = Vec::new();
        for _ in 0..16 {
            let (r, fx) = h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
            if r.status == msg::STATUS_SUCCESS {
                replied += 1;
            } else {
                parked.push(r.id);
            }
            assert!(fx.iter().all(|e| !matches!(e, Effect::Complete(_))));
        }
        // The first slice fits the budget and the second tips it over.
        assert_eq!(replied, 2);
        let cap = CLIENT_BUDGET + SLICE + 16 * 1024;
        assert!(h.high_water <= cap);
        assert_eq!(h.c.waiting(), 14);
        assert_eq!(h.c.parked(), 14 * PACKET_INPUT_CAP);
        // Each drain completes parked writes in arrival order.
        let mut completed = Vec::new();
        for _ in 0..16 {
            for e in h.drain() {
                if let Effect::Complete(r) = e {
                    assert_eq!(r.information, PACKET_INPUT_CAP);
                    completed.push(r.id);
                }
            }
            assert!(h.high_water <= cap);
        }
        assert_eq!(completed, parked);
        assert_eq!((h.c.waiting(), h.c.parked()), (0, 0));
        assert_eq!(h.vt.len(), 16 * PACKET_INPUT_CAP);
    }

    /// A console holding `writes` raw writes of one packet's input each,
    /// parked behind a stalled reader.
    #[cfg(windows)]
    pub(crate) fn parking_writes(writes: usize) -> Console {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        stall(&mut h, output);
        let chunk = vec![b'y'; PACKET_INPUT_CAP];
        for _ in 0..writes {
            h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
        }
        assert_eq!(h.c.waiting(), writes);
        h.c
    }

    /// Fill the queue past the client budget so the next writes park.
    fn stall(h: &mut Host, output: usize) {
        let chunk = vec![b'q'; PACKET_INPUT_CAP];
        h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
        h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
        assert_eq!(h.c.waiting(), 0);
        assert!(h.queued > CLIENT_BUDGET);
    }

    #[test]
    fn an_oversized_descriptor_is_not_retained_while_parked() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        stall(&mut h, output);
        // A descriptor that fills the packet, and one byte of text.
        let fixed = vec![0u8; PACKET_INPUT_CAP - 64];
        for _ in 0..crate::sched::MAX_WAITING {
            let r = h.call(output, api::WRITE_CONSOLE, &fixed, b"x", 0);
            assert_eq!(r.status, i32::MIN, "parked");
        }
        let calls = crate::sched::MAX_WAITING;
        assert_eq!(h.c.parked(), calls * (WRITE_BACK_CAP + 1));
        assert!(h.c.parked() <= crate::sched::PARKED_CAP);
        for _ in 0..4 {
            for e in h.drain() {
                if let Effect::Complete(r) = e {
                    assert_eq!(r.write_back.len(), WRITE_BACK_CAP);
                    assert_eq!(msg::u32_at(&r.write_back, 0), Some(1));
                }
            }
        }
        assert_eq!((h.c.waiting(), h.c.parked()), (0, 0));
    }

    #[test]
    fn a_parked_title_retains_at_most_one_call_reservation() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        stall(&mut h, output);
        // The largest W title a packet carries, every unit 3 bytes in UTF-8.
        // 8 bytes of CONSOLE_MSG_HEADER and the 1-byte descriptor.
        let units = (PACKET_INPUT_CAP - 8 - 1) / 2;
        let bytes: Vec<u8> = std::iter::repeat_n(0x0800u16, units)
            .flat_map(u16::to_le_bytes)
            .collect();
        let r = h.call(output, api::SET_TITLE, &[1], &bytes, 0);
        assert_eq!(r.status, i32::MIN, "parked");
        let osc = 4 + 3 * MAX_OSC_TITLE + 1;
        assert_eq!(h.c.parked(), osc + 1);
        assert!(h.c.parked() <= MAX_RETAINED_PER_CALL);
        assert_eq!(h.c.title.len(), units, "the stored title is whole");
        h.drain();
        h.drain();
        assert_eq!(h.c.parked(), 0);
        assert!(h.vt.ends_with(b"\x07"));
    }

    #[test]
    fn a_long_wide_write_keeps_surrogate_pairs_whole() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        // Two units first, so the first slice ends on a high surrogate.
        let text = format!("ab{}", "\u{1F600}".repeat(16_000));
        assert!(text.encode_utf16().count() * 2 <= PACKET_INPUT_CAP);
        let r = h.write_w(output, &text);
        assert_eq!(
            r.status,
            msg::STATUS_SUCCESS,
            "one call within the budget emits it all"
        );
        assert_eq!(String::from_utf8(h.vt.clone()).expect("UTF-8"), text);
    }

    #[test]
    fn close_drops_parked_writes() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let chunk = vec![b'z'; PACKET_INPUT_CAP];
        for _ in 0..4 {
            h.packet(msg::RAW_WRITE, 1, output, &chunk, 0);
        }
        assert!(h.c.waiting() > 0);
        h.ev(Event::HangUp);
        h.ev(Event::GraceExpired);
        assert_eq!((h.c.waiting(), h.c.parked()), (0, 0));
        assert!(h.drain().is_empty());
    }

    #[test]
    fn resize_records_the_size() {
        let mut h = Host::new();
        let size = WinSize {
            rows: 50,
            cols: 132,
        };
        assert!(h.ev(Event::Resize(size)).is_empty());
        assert_eq!(h.c.size(), size);
    }
}
