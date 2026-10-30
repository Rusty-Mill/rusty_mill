//! The console's input side (design §5.5): terminal input into records,
//! Ctrl events, and the read APIs with their waiting reads.
//!
//! A read that finds nothing waits, holding only its descriptor, its
//! write-back and, for a cooked read, the line so far. Every arrival of
//! input retries the waiting reads in order; each finishes exactly once,
//! by its data, by Ctrl+C or Ctrl+Break, by its handle closing or by its
//! client disconnecting.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/server/ApiDispatchers.cpp`, `ApiDispatchersInternal.cpp`,
//! `IoSorter.cpp`, `ObjectHandle.cpp`, `src/host/input.cpp`,
//! `inputBuffer.cpp`, `stream.cpp`, `directio.cpp`, `readDataRaw.cpp`,
//! `readDataDirect.cpp`, `readDataCooked.cpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use super::{Console, Diag, Effect, Kind, OutputQueue, Packet, Reply, mode};
use crate::cooked::{Done, Line};
use crate::ctrl::{self, Ctrl};
use crate::input::{self, KeyEvent, Record, Token, state, vk};
use crate::msg::{self, ApiMsg};
use crate::sched::{INPUT_PAUSE_ABOVE as ECHO_ABOVE, MAX_RETAINED_PER_CALL};

/// `CONSOLE_READ_NOREMOVE` and `CONSOLE_READ_NOWAIT`.
const READ_NOREMOVE: u16 = 0x1;
const READ_NOWAIT: u16 = 0x2;
/// `CONSOLE_READCONSOLE_MSG`'s size, and its OUT fields.
const READ_CONSOLE_LEN: usize = 20;
const CONTROL_KEY_STATE_AT: usize = 12;
const NUM_BYTES_AT: usize = 16;
/// Reply data one read returns, and one call builds across the reads it
/// completes (§7.4); a read's buffer beyond it goes unused.
pub(super) const READ_REPLY_CAP: usize = 1024 * 1024;
/// UTF-16 units of finished lines that all handles together keep for
/// their next read (§7.4); a line's rest past it is dropped.
pub(super) const MAX_PENDING_UNITS: usize = 512 * 1024;
/// Ctrl+Z, which ends a RAW_READ's data under PROCESSED_INPUT.
const CTRL_Z: u8 = 0x1a;

/// What one try at a read came to.
enum Attempt {
    Done(Reply),
    /// Not enough input yet: the read, to wait.
    Wait(Read),
    /// Input is queued but the read's echo waits for the reader: it and
    /// every read behind it wait, so none takes its input.
    Held(Read),
}

/// A read waiting for input (conhost's wait blocks).
pub(super) struct Read {
    id: [u8; 8],
    pub(super) process: usize,
    pub(super) object: usize,
    /// The fixed struct to write back; empty for RAW_READ.
    fixed: Vec<u8>,
    /// Where the data goes in the client's output, and how much fits.
    at: u32,
    capacity: usize,
    kind: ReadKind,
    /// Bytes the read retains, counted in `Console::parked`.
    pub(super) retained: usize,
}

enum ReadKind {
    /// GetConsoleInput: up to `count` records.
    Records { count: usize, unicode: bool },
    /// ReadConsole without LINE_INPUT, or RAW_READ.
    Chars { unicode: bool, ctrl_z: bool },
    /// ReadConsole with LINE_INPUT.
    Line {
        line: Line,
        unicode: bool,
        ctrl_z: bool,
    },
}

/// What `next_char` found.
enum Got {
    Char(KeyEvent),
    /// A cooked-read editing key, by VK.
    Vkey(u16),
}

impl Console {
    /// Admit one engine slice. Keep its unread bytes while echo or the
    /// reply budget holds a read; the engine then stops taking input.
    pub(super) fn terminal_input(
        &mut self,
        bytes: &[u8],
        end: bool,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        debug_assert!(!self.input_pending(), "input slices must not overtake");
        debug_assert!(bytes.len() <= crate::sched::INPUT_SLICE);
        self.input_bytes.extend(bytes);
        self.input_end = end;
        self.resume_input(out, fx);
    }

    /// Decode a byte at a time so a long text token cannot fill the
    /// record queue before its waiting read runs. Retain only one slice
    /// and one decoded sequence, never an unbounded decoded paste.
    pub(super) fn resume_input(&mut self, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        while !self.reads_behind() {
            if let Some((k, vt)) = self.input_keys.pop_front() {
                self.key(k, vt, out, fx);
                if self
                    .reads
                    .front()
                    .is_some_and(|r| matches!(r.kind, ReadKind::Line { .. }))
                    || self.input.len() >= input::MAX_RECORDS
                {
                    self.serve_reads(out, fx);
                }
                continue;
            }
            let vt = self.input_mode & mode::VIRTUAL_TERMINAL_INPUT != 0;
            let byte = self.input_bytes.pop_front();
            let end = self.input_bytes.is_empty() && self.input_end;
            if byte.is_none() && !end {
                break;
            }
            if end {
                self.input_end = false;
            }
            for piece in self.decoder.feed(byte.as_slice(), end) {
                match (&piece.token, vt) {
                    (Token::Win32(k), _) => self.input_keys.push_back((*k, vt)),
                    // Ctrl+C is the host's under PROCESSED_INPUT in either mode.
                    (Token::Control('\x03'), true) => {
                        for k in input::key_events(&piece.token, self.key_for) {
                            self.input_keys.push_back((k, true));
                        }
                    }
                    (_, true) => {
                        for k in input::text_events(&piece.raw, self.key_for) {
                            self.input_keys.push_back((k, false));
                        }
                    }
                    (Token::Focus(on), false) => self.store(Record::Focus(*on), fx),
                    (token, false) => {
                        for k in input::key_events(token, self.key_for) {
                            self.input_keys.push_back((k, false));
                        }
                    }
                }
            }
        }
        if !self.reads_behind() {
            self.serve_reads(out, fx);
        }
    }

    /// One key event: the host's own keys, or a record. A VT client gets
    /// the key as VT.
    fn key(&mut self, k: KeyEvent, vt: bool, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        if k.down
            && k.state & state::CTRL != 0
            && k.state & state::ALT == 0
            && (k.vk == vk::CANCEL
                || (k.vk == vk::C && self.input_mode & mode::PROCESSED_INPUT != 0))
        {
            // Even when a raw read precedes the cooked one, earlier
            // text must reach the waiting readers before cancellation.
            self.serve_reads(out, fx);
        }
        if self.intercept(&k, fx) {
            return;
        }
        if !vt {
            self.store(Record::Key(k), fx);
            return;
        }
        let vt = input::vt_for(&k, &mut self.vt_utf8);
        for k in input::text_events(&vt, self.key_for) {
            self.store(Record::Key(k), fx);
        }
    }

    /// The host's answers to DSR 5 and CPR (§5.6): input for a VT
    /// client, dropped otherwise.
    pub(super) fn host_replies(
        &mut self,
        replies: &[u8],
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        if replies.is_empty() || self.input_mode & mode::VIRTUAL_TERMINAL_INPUT == 0 {
            return;
        }
        for k in input::text_events(replies, self.key_for) {
            self.store(Record::Key(k), fx);
        }
        self.serve_reads(out, fx);
    }

    pub(super) fn store(&mut self, r: Record, fx: &mut Vec<Effect>) {
        if self.input.push(r) {
            self.log(Diag::InputDropped, fx);
        }
    }

    /// conhost `HandleGenericKeyEvent`: Ctrl+C under PROCESSED_INPUT and
    /// Ctrl+Break go to every client and end the reads they end; Ctrl+Esc
    /// and Alt+Esc are dropped. `true` when the key goes no further.
    fn intercept(&mut self, k: &KeyEvent, fx: &mut Vec<Effect>) -> bool {
        if !k.down {
            return false;
        }
        let ctrl = k.state & state::CTRL != 0;
        let alt = k.state & state::ALT != 0;
        if ctrl && !alt {
            if k.vk == vk::C && self.input_mode & mode::PROCESSED_INPUT != 0 {
                let cooked_owns_input = self.echo_held
                    || (self.reads_cut
                        && self
                            .reads
                            .front()
                            .is_some_and(|r| matches!(r.kind, ReadKind::Line { .. })));
                if cooked_owns_input {
                    // Echo or the reply budget can defer a cooked read
                    // before it consumes its prefix. Discard that prefix
                    // on cancellation, but preserve input owed to a raw
                    // or record read at the front of a budget cut.
                    self.clear_input(false);
                }
                self.ctrl_event(Ctrl::C, 0, fx);
                self.terminate_reads(Ctrl::C, fx);
                return true;
            }
            if k.vk == vk::CANCEL {
                self.clear_input(false);
                self.ctrl_event(Ctrl::Break, 0, fx);
                self.terminate_reads(Ctrl::Break, fx);
                return true;
            }
            return k.vk == vk::ESCAPE;
        }
        alt && k.vk == vk::ESCAPE
    }

    /// Queue `event` for the clients of `group` (0: every client), newest
    /// first, as Windows has always dispatched them (conhost
    /// `ProcessList.cpp`): a child's handler runs before its parent's.
    pub(super) fn ctrl_event(&mut self, event: Ctrl, group: u32, fx: &mut Vec<Effect>) {
        let mut clients: Vec<(&usize, &super::Client)> = self.clients.iter().collect();
        clients.sort_by_key(|(cookie, _)| std::cmp::Reverse(**cookie));
        let pids = ctrl::recipients(clients.into_iter().map(|(_, c)| (c.pid, c.group)), group);
        fx.extend(pids.into_iter().map(|pid| Effect::Ctrl { pid, event }));
    }

    /// Ctrl+C ends cooked reads, Ctrl+Break cooked and raw reads, both
    /// with STATUS_ALERTED. GetConsoleInput keeps waiting.
    fn terminate_reads(&mut self, by: Ctrl, fx: &mut Vec<Effect>) {
        self.end_reads(
            |r| match r.kind {
                ReadKind::Line { .. } => true,
                ReadKind::Chars { .. } => by == Ctrl::Break,
                ReadKind::Records { .. } => false,
            },
            msg::STATUS_ALERTED,
            fx,
        );
    }

    /// Complete every waiting read that `which` picks with `status` and no
    /// data: a closed handle (STATUS_ALERTED) or a departed client
    /// (STATUS_THREAD_IS_TERMINATING).
    pub(super) fn end_reads(
        &mut self,
        which: impl Fn(&Read) -> bool,
        status: i32,
        fx: &mut Vec<Effect>,
    ) {
        let mut kept = std::collections::VecDeque::new();
        for r in std::mem::take(&mut self.reads) {
            if !which(&r) {
                kept.push_back(r);
                continue;
            }
            self.parked -= r.retained;
            let mut reply = Reply::status(r.id, status);
            reply.write_back = r.fixed;
            fx.push(Effect::Complete(reply));
        }
        self.reads = kept;
        // Only a cooked read's echo holds the others back.
        self.echo_held &= self
            .reads
            .iter()
            .any(|r| matches!(r.kind, ReadKind::Line { .. }));
    }

    /// Waiting reads are owed the queued input: a cooked read's echo
    /// waits for the reader, or the last call stopped at its reply budget.
    fn reads_behind(&self) -> bool {
        self.echo_held || self.reads_cut
    }

    /// Records a new record read or peek may take now. Owed reads get
    /// first claim; the event and count use the same availability rule.
    pub(super) fn ready_records(&self) -> usize {
        if self.reads_behind() {
            return 0;
        }
        self.input.len() + self.cached_records.len() + usize::from(self.held_unit.is_some())
    }

    pub(super) fn clear_input(&mut self, discard_slice: bool) {
        self.echo_held = false;
        self.input.clear();
        self.cached_records.clear();
        self.held_unit = None;
        if discard_slice {
            self.input_bytes.clear();
            self.input_keys.clear();
            self.input_end = false;
        }
        self.record_utf8 = [input::Utf8Units::default(); 2];
        self.vt_utf8.clear();
    }

    /// conhost `GetInputBuffer`: the object exists, has the access, and
    /// is the input buffer.
    pub(super) fn input_object(&self, object: usize, write: bool) -> Result<(), i32> {
        let Some(o) = self.objects.get(&object) else {
            return Err(msg::STATUS_FROM_HANDLE);
        };
        let access = if write { o.access.write } else { o.access.read };
        if !access {
            return Err(msg::STATUS_FROM_ACCESS_DENIED);
        }
        if o.kind != Kind::Input {
            return Err(msg::STATUS_FROM_HANDLE);
        }
        Ok(())
    }

    /// RAW_FLUSH: FlushConsoleInputBuffer by another route.
    pub(super) fn flush_input(&mut self, p: &Packet<'_>) -> Reply {
        match self.input_object(p.object, true) {
            Ok(()) => {
                self.clear_input(true);
                Reply::ok(p.id)
            }
            Err(status) => Reply::status(p.id, status),
        }
    }

    /// `CONSOLE_GETNUMBEROFINPUTEVENTS_MSG { OUT ReadyEvents }`.
    pub(super) fn get_number_of_input_events(&self, object: usize, fixed: &mut [u8]) -> i32 {
        if fixed.len() < 4 {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        if let Err(status) = self.input_object(object, false) {
            return status;
        }
        msg::put_u32(fixed, 0, self.ready_records() as u32);
        msg::STATUS_SUCCESS
    }

    /// `CONSOLE_CTRLEVENT_MSG { IN CtrlEvent; IN ProcessGroupId }`. A
    /// nonzero group must be a client's; conhost's fallback to a child of
    /// a client, found by its parent pid, is not supported.
    pub(super) fn generate_ctrl_event(&mut self, fixed: &[u8], fx: &mut Vec<Effect>) -> i32 {
        let (Some(event), Some(group)) = (msg::u32_at(fixed, 0), msg::u32_at(fixed, 4)) else {
            return msg::STATUS_ILLEGAL_FUNCTION;
        };
        if group != 0 && !self.clients.values().any(|c| c.group == group) {
            return msg::STATUS_FROM_INVALIDARG;
        }
        // CTRL_CLOSE comes with PR 8; other numbers do nothing, as in conhost.
        if let Some(event) = Ctrl::from_event(event) {
            self.ctrl_event(event, group, fx);
        }
        msg::STATUS_SUCCESS
    }

    /// `CONSOLE_GETCONSOLEINPUT_MSG { OUT NumRecords; IN Flags; IN
    /// Unicode }`, the records being the variable output.
    pub(super) fn get_console_input(
        &mut self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        mut fixed: Vec<u8>,
        fx: &mut Vec<Effect>,
    ) {
        let reply = |status| Effect::Reply(Reply::status(p.id, status));
        let (Some(flags), Some(unicode), true) = (
            fixed.get(4..6).map(|b| u16::from_le_bytes([b[0], b[1]])),
            msg::bool_at(&fixed, 6),
            fixed.len() >= 8,
        ) else {
            fx.push(reply(msg::STATUS_ILLEGAL_FUNCTION));
            return;
        };
        msg::put_u32(&mut fixed, 0, 0);
        if flags & !(READ_NOREMOVE | READ_NOWAIT) != 0 {
            fx.push(reply(msg::STATUS_FROM_INVALIDARG));
            return;
        }
        if let Err(status) = self.input_object(p.object, false) {
            fx.push(reply(status));
            return;
        }
        let at = m.fixed.len();
        let count = (p.output_size as usize)
            .saturating_sub(at)
            .min(READ_REPLY_CAP)
            / input::RECORD_LEN;
        // Behind reads still to be served, the queue is their input.
        let records = if count == 0 || self.reads_behind() {
            Vec::new()
        } else {
            self.read_records(count, unicode, flags & READ_NOREMOVE != 0)
        };
        let read = Read {
            id: p.id,
            process: p.process,
            object: p.object,
            retained: fixed.len(),
            fixed,
            at: at as u32,
            capacity: count * input::RECORD_LEN,
            // A waiting peek removes what it finds, as conhost's does.
            kind: ReadKind::Records { count, unicode },
        };
        if records.is_empty() && count > 0 && flags & READ_NOWAIT == 0 {
            self.wait(read);
            return;
        }
        fx.push(Effect::Reply(records_reply(read, &records)));
    }

    /// ReadConsole (`m` given) and RAW_READ (`m` None, read as ANSI with
    /// Ctrl+Z processing): pending data first, then a cooked read under
    /// LINE_INPUT, otherwise characters as they come.
    pub(super) fn read_console(
        &mut self,
        p: &Packet<'_>,
        m: Option<ApiMsg<'_>>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let reply = |status| Effect::Reply(Reply::status(p.id, status));
        if m.is_some() && fixed.len() < READ_CONSOLE_LEN {
            fx.push(reply(msg::STATUS_ILLEGAL_FUNCTION));
            return;
        }
        if let Err(status) = self.input_object(p.object, false) {
            fx.push(reply(status));
            return;
        }
        let processed = self.input_mode & mode::PROCESSED_INPUT != 0;
        let (at, unicode, ctrl_z, initial, wakeup_mask) = match m {
            None => (0, false, processed, Vec::new(), 0),
            Some(m) => {
                msg::put_u32(&mut fixed, NUM_BYTES_AT, 0);
                match read_args(m, &fixed, p.output_size) {
                    Ok((unicode, ctrl_z, initial, mask)) => {
                        (m.fixed.len(), unicode, ctrl_z && processed, initial, mask)
                    }
                    Err(status) => {
                        fx.push(reply(status));
                        return;
                    }
                }
            }
        };
        let capacity = (p.output_size as usize)
            .saturating_sub(at)
            .min(READ_REPLY_CAP);
        let char_size = if unicode { 2 } else { 1 };
        if capacity < 1 {
            fx.push(reply(msg::STATUS_BUFFER_TOO_SMALL));
            return;
        }
        let mut read = Read {
            id: p.id,
            process: p.process,
            object: p.object,
            retained: fixed.len() + initial.len() * 2,
            fixed,
            at: at as u32,
            capacity,
            kind: ReadKind::Chars { unicode, ctrl_z },
        };
        if let Some(pending) = self.take_pending(p.object) {
            let data = self.consume(unicode, &pending, capacity);
            self.keep_pending(p.object, &pending[data.units..]);
            fx.push(Effect::Reply(chars_reply(read, data.bytes, 0, ctrl_z)));
            return;
        }
        if self.input_mode & mode::LINE_INPUT != 0 {
            let line = Line::new(initial, wakeup_mask);
            // The line's most: its initial data came from the payload.
            read.retained = read.fixed.len() + line.reserved();
            read.kind = ReadKind::Line {
                line,
                unicode,
                ctrl_z,
            };
        } else if capacity < char_size {
            fx.push(reply(msg::STATUS_BUFFER_TOO_SMALL));
            return;
        }
        if self.reads_behind() {
            // Behind reads still to be served: they get the input first.
            self.wait(read);
            return;
        }
        match self.attempt(read, out, fx) {
            Attempt::Done(reply) => fx.push(Effect::Reply(reply)),
            Attempt::Wait(read) => self.wait(read),
            Attempt::Held(read) => {
                self.echo_held = true;
                self.wait(read);
            }
        }
    }

    fn wait(&mut self, read: Read) {
        debug_assert!(
            read.retained <= MAX_RETAINED_PER_CALL,
            "a read retains {} bytes",
            read.retained
        );
        self.parked += read.retained;
        self.reads.push_back(read);
    }

    /// Retry every waiting read in order; those that finish complete now.
    /// At most [`READ_REPLY_CAP`] of reply data is built per call (§7.4):
    /// the rest of the reads wait for [`Console::reads_ready`]'s turn.
    pub(super) fn serve_reads(&mut self, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        let mut kept = std::collections::VecDeque::new();
        let mut reads = std::mem::take(&mut self.reads);
        self.echo_held = false;
        self.reads_cut = false;
        while let Some(mut r) = reads.pop_front() {
            let remaining = READ_REPLY_CAP - self.read_built;
            // Bound the actual reply, rather than postponing a large
            // declared buffer after a tiny reply. Preserve its capacity
            // if it has to wait instead of completing.
            let unit = match &r.kind {
                ReadKind::Records { .. } => input::RECORD_LEN,
                ReadKind::Chars { unicode: true, .. } | ReadKind::Line { unicode: true, .. } => 2,
                _ => 1,
            };
            if remaining < unit {
                self.reads_cut = true;
                kept.push_back(r);
                kept.append(&mut reads);
                break;
            }
            let capacity = r.capacity;
            r.capacity = r.capacity.min(remaining);
            if let ReadKind::Records { count, .. } = &mut r.kind {
                *count = r.capacity / input::RECORD_LEN;
            }
            let retained = r.retained;
            match self.attempt(r, out, fx) {
                Attempt::Done(reply) => {
                    self.parked -= retained;
                    self.read_built += reply.output.len();
                    fx.push(Effect::Complete(reply));
                }
                Attempt::Wait(mut r) => {
                    r.capacity = capacity;
                    if let ReadKind::Records { count, .. } = &mut r.kind {
                        *count = capacity / input::RECORD_LEN;
                    }
                    kept.push_back(r);
                }
                Attempt::Held(mut r) => {
                    r.capacity = capacity;
                    self.echo_held = true;
                    kept.push_back(r);
                    kept.append(&mut reads);
                }
            }
        }
        self.reads = kept;
    }

    /// Read what is there: the reply when the read is done, the read back
    /// when it must wait.
    fn attempt(&mut self, mut r: Read, out: &mut OutputQueue, fx: &mut Vec<Effect>) -> Attempt {
        match &mut r.kind {
            ReadKind::Records { count, unicode } => {
                let records = self.read_records(*count, *unicode, false);
                if records.is_empty() {
                    return Attempt::Wait(r);
                }
                Attempt::Done(records_reply(r, &records))
            }
            ReadKind::Chars { unicode, ctrl_z } => {
                let (unicode, ctrl_z) = (*unicode, *ctrl_z);
                let bytes = self.read_chars(unicode, r.capacity);
                if bytes.is_empty() {
                    return Attempt::Wait(r);
                }
                Attempt::Done(chars_reply(r, bytes, 0, ctrl_z))
            }
            ReadKind::Line {
                line,
                unicode,
                ctrl_z,
            } => {
                let (unicode, ctrl_z) = (*unicode, *ctrl_z);
                let echoing = self.input_mode & mode::ECHO_INPUT != 0;
                let mut echo = Vec::new();
                while line.done().is_none() {
                    // Echo uses the host reserve and no more (§7.4): the
                    // read goes on once the reader drains.
                    if echoing && out.len() + echo.len() > ECHO_ABOVE {
                        self.screen.host(&echo, out);
                        return Attempt::Held(r);
                    }
                    if line.erasing() {
                        line.erase_one(&mut echo);
                        continue;
                    }
                    match self.next_char(true) {
                        Some(Got::Char(k)) => line.char(
                            k.ch,
                            k.state,
                            self.input_mode & mode::PROCESSED_INPUT != 0,
                            &mut echo,
                        ),
                        Some(Got::Vkey(vk)) => line.vkey(vk),
                        None => break,
                    }
                }
                if echoing {
                    self.screen.host(&echo, out);
                }
                if line.done().is_none() {
                    return Attempt::Wait(r);
                }
                let kind = std::mem::replace(&mut r.kind, ReadKind::Chars { unicode, ctrl_z });
                let ReadKind::Line { line, .. } = kind else {
                    return Attempt::Wait(r);
                };
                Attempt::Done(self.finish_line(r, line, unicode, ctrl_z, out, fx))
            }
        }
    }

    /// A finished cooked read: its text up to the client's buffer, the
    /// rest kept on the handle for the next read.
    fn finish_line(
        &mut self,
        r: Read,
        line: Line,
        unicode: bool,
        ctrl_z: bool,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) -> Reply {
        let processed = self.input_mode & mode::PROCESSED_INPUT != 0;
        let wakeup = line.done() == Some(Done::Wakeup);
        let mut newline = Vec::new();
        let (text, keys) = line.finish(
            processed,
            self.input_mode & mode::ECHO_INPUT != 0,
            &mut newline,
        );
        if !wakeup {
            self.screen.host(&newline, out);
        }
        let data = self.consume(unicode, &text, r.capacity);
        let rest = &text[data.units..];
        let held: usize = self.objects.values().map(|o| o.pending.len()).sum();
        if held + rest.len() > MAX_PENDING_UNITS {
            self.log(Diag::PendingDropped, fx);
        } else {
            self.keep_pending(r.object, rest);
        }
        chars_reply(r, data.bytes, keys, ctrl_z)
    }

    /// conhost `ReadCharacterInput`: cached bytes first, then characters
    /// while a whole one still fits. Empty means wait.
    fn read_chars(&mut self, unicode: bool, capacity: usize) -> Vec<u8> {
        let char_size = if unicode { 2 } else { 1 };
        let mut out = self.consume_cached(unicode, capacity);
        while capacity - out.len() >= char_size {
            let Some((units, n)) = self.next_text(unicode) else {
                break;
            };
            let data = self.consume(unicode, &units[..n], capacity - out.len());
            out.extend(data.bytes);
        }
        out
    }

    /// The next character's units for a raw read. An ANSI read takes a
    /// surrogate pair whole, holding a high half until its low half
    /// arrives; a lone half reads as U+FFFD.
    fn next_text(&mut self, unicode: bool) -> Option<([u16; 2], usize)> {
        let first = self.next_unit()?;
        if unicode || !input::is_high_surrogate(first.ch) {
            return Some(([first.ch, 0], 1));
        }
        match self.next_unit() {
            Some(low) if input::is_low_surrogate(low.ch) => Some(([first.ch, low.ch], 2)),
            Some(other) => {
                self.held_unit = Some(other);
                Some(([first.ch, 0], 1))
            }
            None => {
                self.held_unit = Some(first);
                None
            }
        }
    }

    fn next_unit(&mut self) -> Option<KeyEvent> {
        match self.next_char(false)? {
            Got::Char(k) => Some(k),
            Got::Vkey(_) => None,
        }
    }

    /// conhost `GetChar`: the next character from the queue, or for a
    /// cooked read an editing key. Records without either are consumed.
    fn next_char(&mut self, editing: bool) -> Option<Got> {
        if let Some(unit) = self.held_unit.take() {
            return Some(Got::Char(unit));
        }
        let vt = self.input_mode & mode::VIRTUAL_TERMINAL_INPUT != 0;
        loop {
            let Record::Key(k) = self.input.pop_stream()? else {
                continue;
            };
            let edit = editing && is_editing_key(&k);
            if k.ch != 0 && !edit {
                // An Alt+numpad character arrives on the Alt key's release.
                if !k.down && k.vk == vk::MENU {
                    return Some(Got::Char(k));
                }
                if k.down && (vt || (k.vk != vk::ESCAPE && k.ch != 0x0a)) {
                    return Some(Got::Char(k));
                }
            }
            if !k.down {
                continue;
            }
            if edit {
                return Some(Got::Vkey(k.vk));
            }
            if k.ch == 0 && self.types_nul(&k) {
                return Some(Got::Char(k));
            }
        }
    }

    /// Whether a key types NUL: it is the key and modifiers that
    /// `key_for('\0')` names (Ctrl+@ on US English).
    fn types_nul(&self, k: &KeyEvent) -> bool {
        let Some((key, mods)) = (self.key_for)('\0') else {
            return false;
        };
        let held = |mask: u32| k.state & mask != 0;
        k.vk == key
            && held(state::ALT) == (mods & state::ALT != 0)
            && held(state::CTRL) == (mods & state::CTRL != 0)
            && held(state::SHIFT) == (mods & state::SHIFT != 0)
    }

    /// conhost `InputBuffer::Read` without streaming: up to `count`
    /// records. An ANSI read gets one record per UTF-8 byte of a key's
    /// character, and keeps what it expanded past `count` for the next.
    /// A surrogate pair's UTF-8 comes with its low half's record, so the
    /// high half's yields none; a lone half is U+FFFD.
    fn read_records(&mut self, count: usize, unicode: bool, peek: bool) -> Vec<Record> {
        let mut out: Vec<Record> = if peek {
            self.cached_records.iter().take(count).copied().collect()
        } else {
            let n = count.min(self.cached_records.len());
            self.cached_records.drain(..n).collect()
        };
        // A raw ANSI read may have looked one UTF-16 unit ahead. It is
        // still the next key, ahead of all records remaining in input.
        let held = self.held_unit.map(Record::Key);
        let mut taken = 0;
        let mut took_held = false;
        // A peek leaves the waiting surrogate halves as they are.
        let mut utf8 = self.record_utf8;
        for (index, r) in held.iter().chain(self.input.iter()).enumerate() {
            if out.len() >= count {
                break;
            }
            if index == 0 && held.is_some() {
                took_held = true;
            } else {
                taken += 1;
            }
            match r {
                Record::Key(k) if !unicode => {
                    let mut bytes = Vec::new();
                    utf8[usize::from(k.down)].push(k.ch, &mut bytes);
                    out.extend(bytes.into_iter().map(|b| {
                        Record::Key(KeyEvent {
                            ch: u16::from(b),
                            ..*k
                        })
                    }));
                }
                r => out.push(*r),
            }
        }
        let extra = out.split_off(count.min(out.len()));
        if !peek {
            if took_held {
                self.held_unit = None;
            }
            self.input.drain_front(taken);
            self.cached_records.extend(extra);
            self.record_utf8 = utf8;
        }
        out
    }

    /// conhost `ConsumeCached`: an ANSI read first gets the bytes of a
    /// character the last one could not fit; a Unicode read drops them.
    fn consume_cached(&mut self, unicode: bool, capacity: usize) -> Vec<u8> {
        if unicode {
            self.cached_text.clear();
            return Vec::new();
        }
        let n = capacity.min(self.cached_text.len());
        self.cached_text.drain(..n).collect()
    }

    /// conhost `InputBuffer::Consume`: `units` into at most `capacity`
    /// bytes, as UTF-16 or as UTF-8 (the only code page, §5.9). An ANSI
    /// character that doesn't fit is split, the rest cached.
    fn consume(&mut self, unicode: bool, units: &[u16], capacity: usize) -> Consumed {
        let mut bytes = self.consume_cached(unicode, capacity);
        let mut used = 0;
        if unicode {
            for u in units {
                if capacity - bytes.len() < 2 {
                    break;
                }
                bytes.extend(u.to_le_bytes());
                used += 1;
            }
            return Consumed { bytes, units: used };
        }
        for c in char::decode_utf16(units.iter().copied()) {
            let room = capacity - bytes.len();
            if room == 0 {
                break;
            }
            let ch = c.as_ref().map_or(char::REPLACEMENT_CHARACTER, |ch| *ch);
            used += c.as_ref().map_or(1, |ch| ch.len_utf16());
            let mut buf = [0u8; 4];
            let enc = ch.encode_utf8(&mut buf).as_bytes();
            let fit = enc.len().min(room);
            bytes.extend_from_slice(&enc[..fit]);
            self.cached_text.extend(&enc[fit..]);
        }
        Consumed { bytes, units: used }
    }

    fn take_pending(&mut self, object: usize) -> Option<Vec<u16>> {
        let o = self.objects.get_mut(&object)?;
        (!o.pending.is_empty()).then(|| std::mem::take(&mut o.pending))
    }

    fn keep_pending(&mut self, object: usize, rest: &[u16]) {
        if rest.is_empty() {
            return;
        }
        if let Some(o) = self.objects.get_mut(&object) {
            o.pending = rest.to_vec();
        }
    }
}

struct Consumed {
    bytes: Vec<u8>,
    /// UTF-16 units used up, a split character's included.
    units: usize,
}

/// ReadConsole's inputs (conhost `ServerReadConsole`): Unicode,
/// ProcessControlZ, the initial data that follows the exe name, and
/// CtrlWakeupMask.
fn read_args(
    m: ApiMsg<'_>,
    fixed: &[u8],
    output_size: u32,
) -> Result<(bool, bool, Vec<u16>, u32), i32> {
    let unicode = msg::bool_at(fixed, 0).unwrap_or(false);
    let ctrl_z = msg::bool_at(fixed, 1).unwrap_or(false);
    let exe_units = usize::from(u16::from_le_bytes([fixed[2], fixed[3]]));
    let initial_bytes = msg::u32_at(fixed, 4).unwrap_or(0) as usize;
    let mask = msg::u32_at(fixed, 8).unwrap_or(0);
    let capacity = (output_size as usize).saturating_sub(m.fixed.len());
    if initial_bytes > capacity || (initial_bytes > 0 && !unicode) {
        return Err(msg::STATUS_FROM_INVALIDARG);
    }
    let initial = m
        .variable
        .get(exe_units * 2..exe_units * 2 + initial_bytes)
        .ok_or(msg::STATUS_FROM_INVALIDARG)?;
    Ok((unicode, ctrl_z, msg::utf16_units(initial), mask))
}

fn records_reply(r: Read, records: &[Record]) -> Reply {
    let mut fixed = r.fixed;
    msg::put_u32(&mut fixed, 0, records.len() as u32);
    let mut reply = Reply::ok(r.id);
    reply.write_back = fixed;
    reply.output = records.iter().flat_map(Record::encode).collect();
    reply.information = reply.output.len();
    reply.output_at = r.at;
    reply
}

/// A ReadConsole or RAW_READ answer. Under Ctrl+Z processing, data that
/// starts with Ctrl+Z reads as nothing.
fn chars_reply(r: Read, bytes: Vec<u8>, keys: u32, ctrl_z: bool) -> Reply {
    let empty = ctrl_z && bytes.first() == Some(&CTRL_Z);
    let n = if empty { 0 } else { bytes.len() };
    let mut fixed = r.fixed;
    if fixed.len() >= READ_CONSOLE_LEN {
        msg::put_u32(&mut fixed, CONTROL_KEY_STATE_AT, keys);
        msg::put_u32(&mut fixed, NUM_BYTES_AT, n as u32);
    }
    let mut reply = Reply::ok(r.id);
    reply.write_back = fixed;
    reply.information = n;
    reply.output = if empty { Vec::new() } else { bytes };
    reply.output_at = r.at;
    reply
}

/// conhost `IsCommandLineEditingKey`.
fn is_editing_key(k: &KeyEvent) -> bool {
    let ctrl = k.state & state::CTRL != 0;
    let alt = k.state & state::ALT != 0;
    if !ctrl && !alt {
        return matches!(
            k.vk,
            vk::ESCAPE
                | vk::PRIOR
                | vk::NEXT
                | vk::END
                | vk::HOME
                | vk::LEFT
                | vk::UP
                | vk::RIGHT
                | vk::DOWN
        ) || k.vk == vk::INSERT
            || k.vk == vk::DELETE
            || (vk::F1..vk::F1 + 9).contains(&k.vk);
    }
    if ctrl && matches!(k.vk, vk::END | vk::HOME | vk::LEFT | vk::RIGHT) {
        return true;
    }
    alt && (k.vk == vk::F1 + 6 || k.vk == vk::F1 + 9)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::tests::Host;
    use super::super::{Effect, Event};
    use super::*;
    use crate::msg::api;

    const RAW: u32 = mode::PROCESSED_INPUT;

    /// A `CONSOLE_READCONSOLE_MSG`.
    fn read_msg(unicode: bool, initial_bytes: u32, wakeup_mask: u32) -> Vec<u8> {
        let mut m = vec![u8::from(unicode), 0, 0, 0];
        m.extend(initial_bytes.to_le_bytes());
        m.extend(wakeup_mask.to_le_bytes());
        m.extend([0; 8]);
        m
    }

    fn read_w(h: &mut Host, input: usize, capacity: u32) -> Reply {
        h.call(
            input,
            api::READ_CONSOLE,
            &read_msg(true, 0, 0),
            &[],
            capacity,
        )
    }

    fn get_input(h: &mut Host, input: usize, flags: u16, unicode: bool, records: u32) -> Reply {
        let mut m = vec![0; 4];
        m.extend(flags.to_le_bytes());
        m.extend([u8::from(unicode), 0]);
        h.call(
            input,
            api::GET_CONSOLE_INPUT,
            &m,
            &[],
            records * input::RECORD_LEN as u32,
        )
    }

    fn completes(fx: &[Effect]) -> Vec<&Reply> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Complete(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    fn ctrls(fx: &[Effect]) -> Vec<(u32, Ctrl)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Ctrl { pid, event } => Some((*pid, *event)),
                _ => None,
            })
            .collect()
    }

    fn wide(r: &Reply) -> String {
        String::from_utf16_lossy(&msg::utf16_units(&r.output))
    }

    fn held(r: &Reply) -> bool {
        r.status == i32::MIN
    }

    #[test]
    fn cancelled_same_slice_text_never_reaches_the_next_read() {
        for split in [false, true] {
            let mut h = Host::new();
            let [_, input, _] = h.connect(1, "");
            let cancelled = read_w(&mut h, input, 512);
            let fx = if split {
                h.ev(Event::Input(b"del x"));
                h.ev(Event::Input(b"\x03"))
            } else {
                h.ev(Event::Input(b"del x\x03"))
            };
            let done = completes(&fx);
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].id, cancelled.id);
            assert_eq!(done[0].status, msg::STATUS_ALERTED);
            assert!(done[0].output.is_empty());
            let next = read_w(&mut h, input, 512);
            let fx = h.ev(Event::Input(b"\r"));
            let done = completes(&fx);
            assert_eq!(done[0].id, next.id);
            assert_eq!(wide(done[0]), "\r\n");
        }
    }

    #[test]
    fn cancellation_after_a_backpressure_pause_does_not_submit_the_cancelled_line() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.queued = ECHO_ABOVE - 2;
        let cancelled = read_w(&mut h, input, 512);
        assert!(completes(&h.ev(Event::Input(b"del x\x03"))).is_empty());
        assert!(h.c.echo_held && h.c.input_paused());
        let fx = h.drain();
        let done = completes(&fx);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, cancelled.id);
        assert_eq!(done[0].status, msg::STATUS_ALERTED);
        read_w(&mut h, input, 512);
        assert_eq!(wide(completes(&h.ev(Event::Input(b"\r")))[0]), "\r\n");
    }

    #[test]
    fn cancellation_serves_text_even_when_a_raw_read_precedes_the_cooked_read() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let raw = read_w(&mut h, input, 2);
        h.set_mode(
            input,
            mode::LINE_INPUT | mode::ECHO_INPUT | mode::PROCESSED_INPUT,
        );
        let cooked = read_w(&mut h, input, 512);
        let fx = h.ev(Event::Input(b"xdel x\x03"));
        let done = completes(&fx);
        assert_eq!(done[0].id, raw.id);
        assert_eq!(wide(done[0]), "x");
        assert_eq!(done[1].id, cooked.id);
        assert_eq!(done[1].status, msg::STATUS_ALERTED);
        read_w(&mut h, input, 512);
        assert_eq!(wide(completes(&h.ev(Event::Input(b"\r")))[0]), "\r\n");
    }

    #[test]
    fn cancellation_at_the_reply_cap_discards_only_the_cooked_readers_prefix() {
        for successor in ["cooked", "raw", "records"] {
            let mut h = Host::new();
            let [_, input, _] = h.connect(1, "");
            h.set_mode(input, RAW);
            let raw = read_w(&mut h, input, READ_REPLY_CAP as u32);
            let kept = match successor {
                "raw" => Some(read_w(&mut h, input, 512)),
                "records" => Some(get_input(&mut h, input, 0, true, 64)),
                _ => None,
            };
            h.set_mode(
                input,
                mode::LINE_INPUT | mode::ECHO_INPUT | mode::PROCESSED_INPUT,
            );
            let cooked = read_w(&mut h, input, 512);
            let mut bytes = b"\x1b[65;0;97;1;0;65535_".repeat(8);
            bytes.extend(b"\x1b[65;0;97;1;0;8_del x\x03");
            let fx = h.ev(Event::Input(&bytes));
            assert_eq!(reply_bytes(&fx), READ_REPLY_CAP);
            let done = completes(&fx);
            let raw_done = done
                .iter()
                .find(|r| r.id == raw.id)
                .expect("original raw read completed");
            assert_eq!(raw_done.status, msg::STATUS_SUCCESS);
            assert_eq!(raw_done.output.len(), READ_REPLY_CAP);
            assert!(
                msg::utf16_units(&raw_done.output)
                    .iter()
                    .all(|&u| u == u16::from(b'a'))
            );
            let cancelled = done
                .iter()
                .find(|r| r.id == cooked.id)
                .expect("cooked read cancelled");
            assert_eq!(cancelled.status, msg::STATUS_ALERTED);
            assert!(cancelled.output.is_empty());
            let resumed = h.ev(Event::ServeReads);
            if let Some(kept) = kept {
                let done = completes(&resumed);
                let kept_done = done
                    .iter()
                    .find(|r| r.id == kept.id)
                    .expect("unrelated reader resumed");
                assert_eq!(kept_done.status, msg::STATUS_SUCCESS);
                if successor == "raw" {
                    assert_eq!(wide(kept_done), "del x");
                } else {
                    let chars: Vec<u16> = kept_done
                        .output
                        .as_chunks::<{ input::RECORD_LEN }>()
                        .0
                        .iter()
                        .map(|r| u16::from_le_bytes([r[14], r[15]]))
                        .filter(|&ch| ch != 0)
                        .collect();
                    assert_eq!(String::from_utf16_lossy(&chars), "del x");
                }
            }
            let next = read_w(&mut h, input, 512);
            assert!(held(&next));
            let fx = h.ev(Event::Input(b"\r"));
            assert!(reply_bytes(&fx) <= READ_REPLY_CAP);
            let done = completes(&fx);
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].id, next.id);
            assert_eq!(wide(done[0]), "\r\n");
        }
    }

    #[test]
    fn ctrl_break_flushes_preceding_input_but_keeps_later_same_slice_text() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        read_w(&mut h, input, 512);
        let mut bytes = b"cancel".to_vec();
        bytes.extend(b"\x1b[3;0;0;1;8;1_");
        bytes.extend(b"next");
        h.ev(Event::Input(&bytes));
        read_w(&mut h, input, 512);
        assert_eq!(wide(completes(&h.ev(Event::Input(b"\r")))[0]), "next\r\n");
    }

    #[test]
    fn flush_discards_the_remainder_of_a_held_slice() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.queued = ECHO_ABOVE + 1;
        read_w(&mut h, input, 512);
        h.ev(Event::Input(b"discard"));
        assert!(h.c.input_pending());
        h.call(input, api::FLUSH_INPUT_BUFFER, &[], &[], 0);
        assert!(!h.c.input_pending());
        assert!(completes(&h.drain()).is_empty());
        assert_eq!(wide(completes(&h.ev(Event::Input(b"\r")))[0]), "\r\n");
    }

    #[test]
    fn a_single_slice_paste_reaches_its_waiting_read_before_the_queue_cap() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        read_w(&mut h, input, 16_384);
        let text = format!("{}\r", "a".repeat(6000));
        let fx = h.ev(Event::Input(text.as_bytes()));
        assert_eq!(wide(completes(&fx)[0]), format!("{}\r\n", "a".repeat(6000)));
        assert!(!fx.contains(&Effect::Log(Diag::InputDropped)));
        // With no reader the original drop-oldest cap still applies.
        h.ev(Event::Input(&vec![b'x'; 6000]));
        assert_eq!(h.c.input.len(), input::MAX_RECORDS);
    }

    #[test]
    fn empty_rest_from_an_overlapping_read_keeps_the_first_reads_rest() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        read_w(&mut h, input, 4);
        read_w(&mut h, input, 512);
        let fx = h.ev(Event::Input(b"abcd\r\r"));
        assert_eq!(completes(&fx).len(), 2);
        assert_eq!(wide(&read_w(&mut h, input, 512)), "cd\r\n");
    }

    #[test]
    fn echo_off_never_flushes_a_dangling_surrogate_to_the_terminal() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, mode::LINE_INPUT | mode::PROCESSED_INPUT);
        read_w(&mut h, input, 512);
        let mut bytes = win32_down(0, 0xd83d);
        bytes.push(b'\r');
        let fx = h.ev(Event::Input(&bytes));
        assert_eq!(
            msg::utf16_units(&completes(&fx)[0].output),
            [0xd83d, 13, 10]
        );
        assert_eq!(h.vt, b"\r\n");
    }

    #[test]
    fn a_wide_backspace_erases_both_display_columns() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        read_w(&mut h, input, 512);
        let fx = h.ev(Event::Input("界\x08\r".as_bytes()));
        assert_eq!(wide(completes(&fx)[0]), "\r\n");
        assert_eq!(h.vt, "界\x08 \x08\x08 \x08\r\n".as_bytes());
    }

    #[test]
    fn tab_erase_keeps_the_existing_one_column_behavior_before_a_tab_stop() {
        for erase in *b"\x08\x1b" {
            let mut h = Host::new();
            let [_, input, output] = h.connect(1, "");
            h.write_w(output, "       ");
            assert_eq!(h.c.screen.cursor().0, 7);
            read_w(&mut h, input, 512);
            let fx = h.ev(Event::Input(&[b'\t', erase]));
            assert!(completes(&fx).is_empty());
            assert_eq!(h.c.screen.cursor().0, 7);
            assert_eq!(h.vt, b"       \t\x08 \x08");
            assert_eq!(wide(completes(&h.ev(Event::Input(b"\r")))[0]), "\r\n");
        }
    }

    #[test]
    fn a_raw_ansi_lookahead_stays_available_to_record_reads() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let mut bytes = win32_down(0, 0xd83d);
        bytes.extend(b"\x1b[65;30;97;1;16;1_");
        h.ev(Event::Input(&bytes));
        let r = h.call(input, api::READ_CONSOLE, &read_msg(false, 0, 0), &[], 3);
        assert_eq!(r.output, "�".as_bytes());
        let count = h.call(input, api::GET_NUMBER_OF_INPUT_EVENTS, &[0; 4], &[], 0);
        assert_eq!(msg::u32_at(&count.write_back, 0), Some(1));
        assert!(h.c.input_available);
        let peek = get_input(&mut h, input, READ_NOREMOVE | READ_NOWAIT, true, 1);
        assert_eq!(&peek.output[14..16], &u16::from(b'a').to_le_bytes());
        assert_eq!(&peek.output[10..12], &65u16.to_le_bytes());
        assert_eq!(&peek.output[12..14], &30u16.to_le_bytes());
        assert_eq!(&peek.output[16..20], &state::SHIFT.to_le_bytes());
        let read = get_input(&mut h, input, 0, true, 1);
        assert_eq!(read.output, peek.output);
        h.ev(Event::Input(b"y"));
        let read = get_input(&mut h, input, 0, true, 1);
        assert_eq!(&read.output[14..16], &u16::from(b'y').to_le_bytes());
    }

    #[test]
    fn tiny_then_large_declared_read_use_the_same_actual_reply_budget() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        read_w(&mut h, input, 2);
        read_w(&mut h, input, READ_REPLY_CAP as u32);
        let fx = h.ev(Event::Input(b"ab"));
        assert_eq!(
            completes(&fx).iter().map(|r| wide(r)).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(!h.c.reads_ready());
    }

    fn assert_peek_count_agree(h: &mut Host, input: usize) {
        let count = h.call(input, api::GET_NUMBER_OF_INPUT_EVENTS, &[0; 4], &[], 0);
        let peek = get_input(h, input, READ_NOREMOVE | READ_NOWAIT, true, 4096);
        assert_eq!(
            msg::u32_at(&count.write_back, 0),
            msg::u32_at(&peek.write_back, 0)
        );
        assert_eq!(h.c.input_available, peek.information > 0);
    }

    #[test]
    fn peek_count_and_event_agree_while_echo_or_reply_budget_owns_input() {
        let mut h = Host::new();
        hold_reads_on_their_echo(&mut h);
        let input = h.c.reads.front().expect("held read").object;
        assert_peek_count_agree(&mut h, input);
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        large_reads(&mut h, input, 4, READ_REPLY_CAP as u32);
        assert_peek_count_agree(&mut h, input);
    }

    #[test]
    fn a_held_slice_resumes_without_evicting_any_of_its_input() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.queued = ECHO_ABOVE - 500;
        read_w(&mut h, input, 16_384);
        let text = format!("{}{}\r", "a".repeat(4000), "b".repeat(3000));
        let mut done: Vec<String> = completes(&h.ev(Event::Input(text.as_bytes())))
            .iter()
            .map(|r| wide(r))
            .collect();
        assert!(h.c.input_paused());
        assert!(h.c.input.len() <= input::MAX_RECORDS);
        // A small drain leaves echo held; no new input may be admitted.
        h.queued -= 1024;
        done.extend(
            completes(&h.ev(Event::OutputDrained))
                .iter()
                .map(|r| wide(r)),
        );
        assert!(h.c.input_paused());
        done.extend(completes(&h.drain()).iter().map(|r| wide(r)));
        assert_eq!(done, [text.replace('\r', "\r\n")]);
        assert!(!h.c.input_paused());
    }

    #[test]
    fn closing_or_disconnecting_a_held_reader_resumes_the_successor_now() {
        for disconnect in [false, true] {
            let mut h = Host::new();
            let [process, input, _] = h.connect(1, "");
            let [process2, other, _] = h.connect(2, "");
            h.queued = ECHO_ABOVE + 1;
            read_w(&mut h, input, 512);
            h.ev(Event::Input(b"abc"));
            h.set_mode(other, RAW);
            let mut payload = api::READ_CONSOLE.to_le_bytes().to_vec();
            payload.extend(20u32.to_le_bytes());
            payload.extend(read_msg(true, 0, 0));
            let (successor, _) = h.packet(msg::USER_DEFINED, process2, other, &payload, 532);
            assert!(held(&successor));
            let (_, fx) = if disconnect {
                h.packet(msg::DISCONNECT, process, 0, &[], 0)
            } else {
                h.packet(msg::CLOSE_OBJECT, process, input, &[], 0)
            };
            let done = completes(&fx);
            let resumed = done
                .iter()
                .find(|r| r.id == successor.id)
                .expect("successor completed in close call");
            assert_eq!(wide(resumed), "abc");
            assert_eq!(done.iter().filter(|r| r.id == successor.id).count(), 1);
            assert!(completes(&h.drain()).is_empty());
        }
    }

    #[test]
    fn text_becomes_records_and_drives_the_input_event() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let fx = h.ev(Event::Input(b"hi"));
        assert_eq!(fx, [Effect::SetInputAvailable(true)]);
        let r = h.call(input, api::GET_NUMBER_OF_INPUT_EVENTS, &[0; 4], &[], 0);
        assert_eq!(msg::u32_at(&r.write_back, 0), Some(2));
        // A peek leaves them; a read takes them and resets the event.
        let peek = get_input(&mut h, input, READ_NOREMOVE | READ_NOWAIT, true, 8);
        assert_eq!(msg::u32_at(&peek.write_back, 0), Some(2));
        let (r, fx) = {
            let mut m = vec![0; 4];
            m.extend(0u16.to_le_bytes());
            m.extend([1, 0]);
            let mut payload = api::GET_CONSOLE_INPUT.to_le_bytes().to_vec();
            payload.extend((m.len() as u32).to_le_bytes());
            payload.extend(&m);
            h.packet(msg::USER_DEFINED, 1, input, &payload, 8 + 8 * 20)
        };
        assert_eq!(r.information, 2 * input::RECORD_LEN);
        assert_eq!(r.output[14..16], u16::from(b'h').to_le_bytes());
        assert!(fx.contains(&Effect::SetInputAvailable(false)));
    }

    #[test]
    fn a_waiting_record_read_completes_when_input_arrives() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let r = get_input(&mut h, input, 0, true, 4);
        assert!(held(&r));
        assert_eq!(h.c.waiting(), 1);
        let fx = h.ev(Event::Input(b"x"));
        let done = completes(&fx);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, r.id);
        assert_eq!(msg::u32_at(&done[0].write_back, 0), Some(1));
        assert_eq!(h.c.waiting(), 0);
        assert_eq!(h.c.parked(), 0);
    }

    #[test]
    fn a_no_wait_read_of_nothing_answers_at_once() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let r = get_input(&mut h, input, READ_NOWAIT, true, 4);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(r.information, 0);
        let r = get_input(&mut h, input, 0x4, true, 4);
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG, "an unknown flag");
    }

    #[test]
    fn a_raw_read_waits_then_takes_characters_up_to_its_buffer() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let r = read_w(&mut h, input, 4);
        assert!(held(&r));
        let fx = h.ev(Event::Input(b"abc"));
        let done = completes(&fx);
        assert_eq!(wide(done[0]), "ab");
        assert_eq!(msg::u32_at(&done[0].write_back, NUM_BYTES_AT), Some(4));
        let r = read_w(&mut h, input, 8);
        assert_eq!(wide(&r), "c", "the rest stays queued");
    }

    #[test]
    fn a_cooked_read_echoes_and_returns_the_line() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let r = read_w(&mut h, input, 512);
        assert!(held(&r));
        let fx = h.ev(Event::Input(b"dir"));
        assert!(completes(&fx).is_empty());
        let fx = h.ev(Event::Input(b"x\x7f\r"));
        let done = completes(&fx);
        assert_eq!(wide(done[0]), "dir\r\n");
        assert_eq!(h.vt, b"dirx\x08 \x08\r\n");
    }

    /// A win32-input-mode key-down record typing `ch`.
    fn win32_down(vk: u16, ch: u16) -> Vec<u8> {
        format!("\x1b[{vk};0;{ch};1;0;1_").into_bytes()
    }

    #[test]
    fn cooked_echo_stays_within_the_host_reserve_and_resumes_on_drain() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        // 60 waiting cooked reads, each prefilled with 8,192 units.
        let initial: Vec<u8> = [b'a', 0].repeat(crate::cooked::MAX_LINE);
        let msg = read_msg(true, initial.len() as u32, 0);
        let reads: Vec<Reply> = (0..60)
            .map(|_| {
                h.call(
                    input,
                    api::READ_CONSOLE,
                    &msg,
                    &initial,
                    2 * initial.len() as u32,
                )
            })
            .collect();
        assert!(reads.iter().all(held));
        // One slice: Escape then Enter for every read, 24 KiB of erase each.
        let mut keys = Vec::new();
        for _ in 0..60 {
            keys.extend(win32_down(vk::ESCAPE, 0x1b));
            keys.push(b'\r');
        }
        let mut done = completes(&h.ev(Event::Input(&keys))).len();
        let reserve = ECHO_ABOVE + 16;
        assert!(h.high_water <= reserve, "{} queued", h.high_water);
        assert!(done < 60, "echo waits for the reader");
        let mut drains = 0;
        while done < 60 {
            let fx = h.drain();
            assert!(h.high_water <= reserve, "{} queued", h.high_water);
            for r in completes(&fx) {
                assert_eq!(wide(r), "\r\n", "the line was erased");
            }
            done += completes(&fx).len();
            drains += 1;
            assert!(drains < 200, "no progress");
        }
        assert_eq!(h.c.waiting(), 0);
        assert_eq!(h.c.parked(), 0);
    }

    #[test]
    fn a_read_behind_held_echo_does_not_take_its_input() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let initial: Vec<u8> = [b'a', 0].repeat(crate::cooked::MAX_LINE);
        let msg = read_msg(true, initial.len() as u32, 0);
        for _ in 0..4 {
            h.call(
                input,
                api::READ_CONSOLE,
                &msg,
                &initial,
                2 * initial.len() as u32,
            );
        }
        let mut keys = Vec::new();
        for _ in 0..4 {
            keys.extend(win32_down(vk::ESCAPE, 0x1b));
            keys.extend(b"x\r");
        }
        let mut lines: Vec<String> = completes(&h.ev(Event::Input(&keys)))
            .iter()
            .map(|r| wide(r))
            .collect();
        assert!(lines.len() < 4, "held back by the stalled reader");
        // GetConsoleInput now must not take the cooked reads' keys.
        let r = get_input(&mut h, input, READ_NOWAIT, true, 64);
        assert_eq!(r.information, 0, "the queue belongs to the held reads");
        let r = read_w(&mut h, input, 64);
        assert!(held(&r), "a raw read waits behind them");
        for _ in 0..8 {
            lines.extend(completes(&h.drain()).iter().map(|r| wide(r)));
        }
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l == "x\r\n"), "{lines:?}");
    }

    /// Built reply data in `fx`.
    fn reply_bytes(fx: &[Effect]) -> usize {
        fx.iter()
            .map(|e| match e {
                Effect::Reply(r) | Effect::Complete(r) => r.output.len(),
                _ => 0,
            })
            .sum()
    }

    /// Raw W reads of `declared` bytes each, then repeated win32 'a'
    /// records enough to fill them all.
    fn large_reads(h: &mut Host, input: usize, reads: usize, declared: u32) -> Vec<Effect> {
        hold_raw_reads(h, input, reads, declared);
        h.ev(Event::Input(&large_input()))
    }

    fn hold_raw_reads(h: &mut Host, input: usize, reads: usize, declared: u32) {
        h.set_mode(input, RAW);
        for _ in 0..reads {
            assert!(held(&read_w(h, input, declared)));
        }
    }

    /// 513 win32-input-mode records repeating `a` 65535 times: more than
    /// one reply budget, in well under one input slice.
    fn large_input() -> Vec<u8> {
        b"\x1b[65;0;97;1;0;65535_".repeat(513)
    }

    /// A console holding `reads` raw reads of `declared` bytes each.
    #[cfg(windows)]
    pub(crate) fn holding_raw_reads(reads: usize, declared: u32) -> Console {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        hold_raw_reads(&mut h, input, reads, declared);
        h.c
    }

    /// A console holding `reads` raw reads of `declared` bytes each, and
    /// the input that completes the first and leaves the rest owed.
    #[cfg(windows)]
    pub(crate) fn owing_large_reads(reads: usize, declared: u32) -> (Console, Vec<u8>) {
        (holding_raw_reads(reads, declared), large_input())
    }

    #[test]
    fn one_call_builds_at_most_one_reply_budget_and_the_rest_follows() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let reads = 8;
        let fx = large_reads(&mut h, input, reads, u32::MAX - 64);
        assert!(reply_bytes(&fx) <= READ_REPLY_CAP, "{}", reply_bytes(&fx));
        let mut done = completes(&fx).len();
        assert_eq!(done, 1, "the first read fills its capped buffer");
        assert!(completes(&fx)[0].output.len() <= READ_REPLY_CAP);
        while h.c.reads_ready() {
            let fx = h.ev(Event::ServeReads);
            assert!(reply_bytes(&fx) <= READ_REPLY_CAP, "{}", reply_bytes(&fx));
            done += completes(&fx).len();
        }
        assert_eq!(done, reads, "every read is served without more input");
    }

    #[test]
    fn reads_behind_the_budget_keep_their_order_and_closing_ends_them() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        large_reads(&mut h, input, 4, 1 << 20);
        assert!(h.c.reads_ready());
        // A new read queues behind the ones still owed input.
        let r = get_input(&mut h, input, READ_NOWAIT, true, 8);
        assert_eq!(r.information, 0);
        h.ev(Event::HangUp);
        h.ev(Event::GraceExpired);
        assert_eq!(h.c.waiting(), 0);
        assert!(!h.c.reads_ready(), "nothing is left to serve");
    }

    /// Cooked reads whose input is queued, held back because their echo
    /// waits for the reader.
    fn hold_reads_on_their_echo(h: &mut Host) {
        let [_, input, _] = h.connect(1, "");
        let initial: Vec<u8> = [b'a', 0].repeat(crate::cooked::MAX_LINE);
        let msg = read_msg(true, initial.len() as u32, 0);
        for _ in 0..8 {
            h.call(
                input,
                api::READ_CONSOLE,
                &msg,
                &initial,
                2 * initial.len() as u32,
            );
        }
        let mut keys = Vec::new();
        for _ in 0..8 {
            keys.extend(win32_down(vk::ESCAPE, 0x1b));
            keys.push(b'\r');
        }
        h.ev(Event::Input(&keys));
        assert!(
            h.c.waiting() > 0 && h.c.echo_held,
            "held back by the stalled reader"
        );
    }

    /// A console whose cooked reads wait on their echo's output.
    #[cfg(windows)]
    pub(crate) fn echo_held_reads() -> Console {
        let mut h = Host::new();
        hold_reads_on_their_echo(&mut h);
        h.c
    }

    #[test]
    fn closing_ends_reads_held_back_by_their_echo() {
        let mut h = Host::new();
        hold_reads_on_their_echo(&mut h);
        h.ev(Event::HangUp);
        h.ev(Event::GraceExpired);
        assert_eq!(h.c.waiting(), 0);
    }

    #[test]
    fn an_esc_at_a_slice_end_waits_for_the_rest_of_its_sequence() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let r = read_w(&mut h, input, 64);
        h.ev(Event::Input(b"ab"));
        // The engine cut a cursor key after its ESC: not an Escape.
        h.ev(Event::InputMore(b"\x1b"));
        let fx = h.ev(Event::Input(b"[Ac\r"));
        assert_eq!(wide(completes(&fx)[0]), "abc\r\n");
        assert!(held(&r));
        // Where the write ended, a lone ESC is the Escape key.
        let r = read_w(&mut h, input, 64);
        h.ev(Event::Input(b"xy"));
        h.ev(Event::Input(b"\x1b"));
        let done = h.ev(Event::Input(b"z\r"));
        assert_eq!(wide(completes(&done)[0]), "z\r\n");
        assert!(held(&r));
    }

    #[test]
    fn ansi_raw_reads_keep_a_supplementary_character_whole() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let read_a = |h: &mut Host, capacity| {
            h.call(
                input,
                api::READ_CONSOLE,
                &read_msg(false, 0, 0),
                &[],
                capacity,
            )
        };
        h.ev(Event::Input("😀".as_bytes()));
        // Two bytes at a time: the character is split, not replaced.
        let first = read_a(&mut h, 2);
        let second = read_a(&mut h, 2);
        assert_eq!([first.output, second.output].concat(), "😀".as_bytes());
        // Halves in separate records and separate inputs.
        let r = read_a(&mut h, 8);
        assert!(held(&r));
        assert!(completes(&h.ev(Event::Input(&win32_down(0, 0xD83D)))).is_empty());
        let fx = h.ev(Event::Input(&win32_down(0, 0xDE00)));
        assert_eq!(completes(&fx)[0].output, "😀".as_bytes());
        // A lone half is U+FFFD.
        h.ev(Event::Input(&win32_down(0, 0xDE00)));
        assert_eq!(read_a(&mut h, 8).output, "\u{FFFD}".as_bytes());
        // A Unicode read gets the units as they are.
        h.ev(Event::Input("😀".as_bytes()));
        assert_eq!(wide(&read_w(&mut h, input, 8)), "😀");
    }

    #[test]
    fn ansi_records_and_cooked_echo_keep_a_supplementary_character_whole() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.ev(Event::Input("😀".as_bytes()));
        let r = get_input(&mut h, input, 0, false, 8);
        let chars: Vec<u8> = r.output.chunks(input::RECORD_LEN).map(|c| c[14]).collect();
        assert_eq!(chars, "😀".as_bytes());
        // Cooked echo, the halves arriving in separate inputs.
        let r = read_w(&mut h, input, 64);
        h.vt.clear();
        h.ev(Event::Input(&win32_down(0, 0xD83D)));
        h.ev(Event::Input(&win32_down(0, 0xDE00)));
        let done = h.ev(Event::Input(b"\r"));
        assert_eq!(h.vt, "😀\r\n".as_bytes());
        assert_eq!(wide(completes(&done)[0]), "😀\r\n");
        assert!(held(&r));
    }

    #[test]
    fn a_vt_client_gets_a_win32_surrogate_pair_as_one_character() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW | mode::VIRTUAL_TERMINAL_INPUT);
        let up = |ch: u16| format!("\x1b[0;0;{ch};0;0;1_").into_bytes();
        let mut keys = win32_down(0, 0xD83D);
        keys.extend(up(0xD83D));
        keys.extend(win32_down(0, 0xDE00));
        keys.extend(up(0xDE00));
        h.ev(Event::Input(&keys));
        let r = h.call(input, api::READ_CONSOLE, &read_msg(false, 0, 0), &[], 8);
        assert_eq!(r.output, "😀".as_bytes());
    }

    #[test]
    fn a_line_rest_past_the_pending_cap_is_dropped_and_logged() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let other = h.c.objects.get_mut(&output).expect("output object");
        other.pending = vec![u16::from(b'z'); MAX_PENDING_UNITS - 1];
        let r = read_w(&mut h, input, 2);
        let fx = h.ev(Event::Input(b"ab\r"));
        assert_eq!(wide(completes(&fx)[0]), "a");
        assert!(fx.contains(&Effect::Log(Diag::PendingDropped)));
        assert!(held(&r));
        let r = read_w(&mut h, input, 2);
        assert!(held(&r), "nothing was kept for the next read");
    }

    #[test]
    fn a_line_longer_than_the_buffer_is_returned_over_several_reads() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.ev(Event::Input(b"abcdef\r"));
        let r = read_w(&mut h, input, 4);
        assert_eq!(wide(&r), "ab");
        // The rest comes from the handle, whatever the mode now is.
        h.set_mode(input, RAW);
        assert_eq!(wide(&read_w(&mut h, input, 6)), "cde");
        assert_eq!(wide(&read_w(&mut h, input, 64)), "f\r\n");
    }

    #[test]
    fn ctrl_c_goes_to_every_client_and_ends_cooked_reads() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(10, "");
        h.connect(11, "");
        let r = read_w(&mut h, input, 64);
        h.ev(Event::Input(b"ab"));
        let fx = h.ev(Event::Input(b"\x03"));
        assert_eq!(ctrls(&fx), [(11, Ctrl::C), (10, Ctrl::C)], "newest first");
        let done = completes(&fx);
        assert_eq!((done[0].id, done[0].status), (r.id, msg::STATUS_ALERTED));
        // Only the key-up is queued.
        assert_eq!(h.c.input.len(), 1);
    }

    #[test]
    fn without_processed_input_ctrl_c_is_a_key() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, 0);
        let fx = h.ev(Event::Input(b"\x03"));
        assert!(ctrls(&fx).is_empty());
        let r = read_w(&mut h, input, 8);
        assert_eq!(msg::utf16_units(&r.output), [3]);
    }

    #[test]
    fn ctrl_break_flushes_and_ends_raw_and_cooked_reads() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        h.ev(Event::Input(b"\x1b[65;0;97;1;0;1_"));
        h.c.input.drain_front(1);
        let records = get_input(&mut h, input, 0, true, 4);
        let raw = read_w(&mut h, input, 8);
        assert!(held(&records) && held(&raw));
        h.ev(Event::Input(b"\x1b[65;0;97;0;0;1_"));
        // Ctrl+Break: VK_CANCEL with Ctrl, as win32-input-mode sends it.
        let fx = h.ev(Event::Input(b"\x1b[3;70;0;1;8;1_"));
        assert_eq!(ctrls(&fx), [(1, Ctrl::Break)]);
        let done = completes(&fx);
        assert_eq!(done.len(), 1, "GetConsoleInput keeps waiting");
        assert_eq!((done[0].id, done[0].status), (raw.id, msg::STATUS_ALERTED));
    }

    #[test]
    fn generate_ctrl_event_targets_a_process_group() {
        let mut h = Host::new();
        let group = |h: &mut Host, pid: usize, g: u32| {
            let info = msg::CONNECTION_INFORMATION_LEN as u32;
            h.packet(msg::CONNECT, pid, 0, &msg::server_msg_in_group("", g), info)
        };
        group(&mut h, 10, 10);
        group(&mut h, 11, 10);
        group(&mut h, 12, 12);
        let ctrl = |h: &mut Host, event: u32, g: u32| {
            let mut fixed = event.to_le_bytes().to_vec();
            fixed.extend(g.to_le_bytes());
            let mut payload = api::GENERATE_CTRL_EVENT.to_le_bytes().to_vec();
            payload.extend(8u32.to_le_bytes());
            payload.extend(fixed);
            h.packet(msg::USER_DEFINED, 1, 0, &payload, 8)
        };
        let (r, fx) = ctrl(&mut h, 1, 10);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(ctrls(&fx), [(11, Ctrl::Break), (10, Ctrl::Break)]);
        let (_, fx) = ctrl(&mut h, 0, 0);
        assert_eq!(ctrls(&fx).len(), 3);
        let (r, fx) = ctrl(&mut h, 0, 99);
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
        assert!(ctrls(&fx).is_empty());
    }

    #[test]
    fn vt_input_passes_raw_bytes_and_turns_win32_records_into_vt() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW | mode::VIRTUAL_TERMINAL_INPUT);
        h.ev(Event::Input(b"\x1b[A\x1b[?62c"));
        // `a` down, `a` up, then Left with Ctrl.
        h.ev(Event::Input(
            b"\x1b[65;30;97;1;0;1_\x1b[65;30;97;0;0;1_\x1b[37;75;0;1;8;1_",
        ));
        let r = read_w(&mut h, input, 128);
        assert_eq!(wide(&r), "\x1b[A\x1b[?62ca\x1b[1;5D");
    }

    #[test]
    fn replies_and_unknown_sequences_are_dropped_for_a_win32_client() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let fx = h.ev(Event::Input(b"\x1b[?62c\x1b]11;rgb:0/0/0\x07\x1b[5x"));
        assert!(fx.is_empty());
        assert!(h.c.input.is_empty());
        // A focus report is a FOCUS_EVENT record.
        h.ev(Event::Input(b"\x1b[I"));
        let r = get_input(&mut h, input, READ_NOWAIT, true, 4);
        assert_eq!(&r.output[..2], &0x10u16.to_le_bytes());
    }

    #[test]
    fn closing_a_handle_or_disconnecting_ends_its_reads() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let [p2, ..] = h.connect(2, "");
        let a = read_w(&mut h, input, 8);
        let mut payload = api::READ_CONSOLE.to_le_bytes().to_vec();
        payload.extend(20u32.to_le_bytes());
        payload.extend(read_msg(true, 0, 0));
        let (b, _) = h.packet(msg::USER_DEFINED, p2, input, &payload, 28);
        assert!(held(&a) && held(&b));
        let (_, fx) = h.packet(msg::DISCONNECT, p2, 0, &[], 0);
        let done = completes(&fx);
        assert_eq!(
            (done[0].id, done[0].status),
            (b.id, msg::STATUS_THREAD_IS_TERMINATING)
        );
        let (_, fx) = h.packet(msg::CLOSE_OBJECT, 1, input, &[], 0);
        let done = completes(&fx);
        assert_eq!((done[0].id, done[0].status), (a.id, msg::STATUS_ALERTED));
        assert_eq!(h.c.waiting(), 0);
    }

    #[test]
    fn raw_read_is_ansi_and_honours_ctrl_z() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        h.ev(Event::Input(b"\x1a"));
        let (r, _) = h.packet(msg::RAW_READ, 1, input, &[], 16);
        assert_eq!((r.status, r.information), (msg::STATUS_SUCCESS, 0));
        h.ev(Event::Input(b"ok"));
        let (r, _) = h.packet(msg::RAW_READ, 1, input, &[], 16);
        assert_eq!(r.output, b"ok");
        assert!(r.write_back.is_empty());
    }

    #[test]
    fn an_ansi_read_splits_a_character_that_does_not_fit() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        h.ev(Event::Input("é".as_bytes()));
        let read_a =
            |h: &mut Host, cap| h.call(input, api::READ_CONSOLE, &read_msg(false, 0, 0), &[], cap);
        assert_eq!(read_a(&mut h, 1).output, [0xC3]);
        assert_eq!(read_a(&mut h, 4).output, [0xA9], "the cached byte");
    }

    #[test]
    fn an_ansi_record_read_expands_utf8_and_keeps_the_surplus() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.ev(Event::Input("é".as_bytes()));
        let r = get_input(&mut h, input, READ_NOWAIT, false, 1);
        assert_eq!(r.output[14], 0xC3);
        let r = get_input(&mut h, input, READ_NOWAIT, false, 4);
        assert_eq!(r.information, input::RECORD_LEN);
        assert_eq!(r.output[14], 0xA9);
    }

    #[test]
    fn a_wakeup_character_ends_a_cooked_read_with_initial_data() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let initial: Vec<u8> = "cd".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let r = h.call(
            input,
            api::READ_CONSOLE,
            &read_msg(true, initial.len() as u32, 1 << 9),
            &initial,
            64,
        );
        assert!(held(&r));
        let fx = h.ev(Event::Input(b" w\t"));
        let done = completes(&fx);
        assert_eq!(wide(done[0]), "cd w\t");
        assert_eq!(h.vt, b" w", "neither the initial data nor Tab is echoed");
    }

    #[test]
    fn read_console_checks_its_arguments_like_conhost() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let r = h.call(
            input,
            api::READ_CONSOLE,
            &read_msg(true, 16, 0),
            &[0; 16],
            8,
        );
        assert_eq!(
            r.status,
            msg::STATUS_FROM_INVALIDARG,
            "initial data past the buffer"
        );
        let r = h.call(input, api::READ_CONSOLE, &read_msg(false, 2, 0), &[0; 2], 8);
        assert_eq!(
            r.status,
            msg::STATUS_FROM_INVALIDARG,
            "initial data needs Unicode"
        );
        assert_eq!(read_w(&mut h, output, 8).status, msg::STATUS_FROM_HANDLE);
        assert_eq!(
            read_w(&mut h, input, 0).status,
            msg::STATUS_BUFFER_TOO_SMALL
        );
        let r = h.call(input, api::READ_CONSOLE, &[0; 8], &[], 8);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION, "a short descriptor");
    }

    #[test]
    fn flushing_needs_write_access_to_the_input() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let read_only = h
            .create(1, msg::OBJECT_GENERIC, msg::GENERIC_READ)
            .information;
        h.ev(Event::Input(b"x"));
        let flush = |h: &mut Host, object| h.call(object, api::FLUSH_INPUT_BUFFER, &[], &[], 0);
        assert_eq!(
            flush(&mut h, read_only).status,
            msg::STATUS_FROM_ACCESS_DENIED
        );
        assert_eq!(h.c.input.len(), 1);
        assert_eq!(flush(&mut h, input).status, msg::STATUS_SUCCESS);
        assert!(h.c.input.is_empty());
    }

    /// After every event, the last input-available value emitted is
    /// whether the queue holds records (§7.2).
    #[test]
    fn the_input_event_always_matches_the_queue() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        h.set_mode(input, RAW);
        let mut on = false;
        let mut check = |h: &mut Host, fx: Vec<Effect>| {
            for e in fx {
                if let Effect::SetInputAvailable(v) = e {
                    on = v;
                }
            }
            assert_eq!(on, !h.c.input.is_empty());
        };
        let fx = h.ev(Event::Input(b"ab"));
        check(&mut h, fx);
        let (_, fx) = h.packet(msg::RAW_READ, 1, input, &[], 1);
        check(&mut h, fx);
        let mut payload = api::READ_CONSOLE.to_le_bytes().to_vec();
        payload.extend(20u32.to_le_bytes());
        payload.extend(read_msg(true, 0, 0));
        let (_, fx) = h.packet(msg::USER_DEFINED, 1, input, &payload, 64);
        check(&mut h, fx);
        let (_, fx) = h.packet(msg::USER_DEFINED, 1, input, &payload, 64);
        check(&mut h, fx);
        let fx = h.ev(Event::Input(b"c"));
        check(&mut h, fx);
        let fx = h.ev(Event::HangUp);
        check(&mut h, fx);
    }
}
