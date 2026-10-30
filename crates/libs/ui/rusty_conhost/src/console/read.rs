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
    Char(u16, u32),
    /// A cooked-read editing key, by VK.
    Vkey(u16),
}

impl Console {
    /// Terminal input: decode, take out Ctrl+C and Ctrl+Break, store the
    /// rest as the input mode says, then retry the waiting reads.
    pub(super) fn terminal_input(
        &mut self,
        bytes: &[u8],
        end: bool,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let vt = self.input_mode & mode::VIRTUAL_TERMINAL_INPUT != 0;
        for piece in self.decoder.feed(bytes, end) {
            match (&piece.token, vt) {
                (Token::Win32(k), _) => self.key(*k, vt, fx),
                // Ctrl+C is the host's under PROCESSED_INPUT in either mode.
                (Token::Control('\x03'), true) => {
                    for k in input::key_events(&piece.token, self.key_for) {
                        self.key(k, true, fx);
                    }
                }
                (_, true) => {
                    for k in input::text_events(&piece.raw, self.key_for) {
                        self.store(Record::Key(k), fx);
                    }
                }
                (Token::Focus(on), false) => self.store(Record::Focus(*on), fx),
                (token, false) => {
                    for k in input::key_events(token, self.key_for) {
                        self.key(k, false, fx);
                    }
                }
            }
        }
        self.serve_reads(out, fx);
    }

    /// One key event: the host's own keys, or a record. A VT client gets
    /// the key as VT.
    fn key(&mut self, k: KeyEvent, vt: bool, fx: &mut Vec<Effect>) {
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

    fn store(&mut self, r: Record, fx: &mut Vec<Effect>) {
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
                self.ctrl_event(Ctrl::C, 0, fx);
                self.terminate_reads(Ctrl::C, fx);
                return true;
            }
            if k.vk == vk::CANCEL {
                self.clear_input();
                self.ctrl_event(Ctrl::Break, 0, fx);
                self.terminate_reads(Ctrl::Break, fx);
                return true;
            }
            return k.vk == vk::ESCAPE;
        }
        alt && k.vk == vk::ESCAPE
    }

    /// Queue `event` for the clients of `group` (0: every client).
    fn ctrl_event(&mut self, event: Ctrl, group: u32, fx: &mut Vec<Effect>) {
        let mut clients: Vec<(&usize, &super::Client)> = self.clients.iter().collect();
        clients.sort_by_key(|(cookie, _)| **cookie);
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

    pub(super) fn clear_input(&mut self) {
        self.echo_held = false;
        self.input.clear();
        self.cached_records.clear();
        self.held_unit = None;
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
                self.clear_input();
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
        msg::put_u32(fixed, 0, self.input.len() as u32);
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
        let count = (p.output_size as usize).saturating_sub(at) / input::RECORD_LEN;
        // Behind a cooked read whose echo waits, the queue is its input.
        let records = if count == 0 || self.echo_held {
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
        let capacity = (p.output_size as usize).saturating_sub(at);
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
        if self.echo_held {
            // Behind a read whose echo waits: it gets the input first.
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
    pub(super) fn serve_reads(&mut self, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        let mut kept = std::collections::VecDeque::new();
        let mut reads = std::mem::take(&mut self.reads);
        self.echo_held = false;
        while let Some(r) = reads.pop_front() {
            let retained = r.retained;
            match self.attempt(r, out, fx) {
                Attempt::Done(reply) => {
                    self.parked -= retained;
                    fx.push(Effect::Complete(reply));
                }
                Attempt::Wait(r) => kept.push_back(r),
                Attempt::Held(r) => {
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
                        out.push(&echo);
                        return Attempt::Held(r);
                    }
                    if line.erasing() {
                        line.erase_one(&mut echo);
                        continue;
                    }
                    match self.next_char(true) {
                        Some(Got::Char(ch, keys)) => line.char(
                            ch,
                            keys,
                            self.input_mode & mode::PROCESSED_INPUT != 0,
                            &mut echo,
                        ),
                        Some(Got::Vkey(vk)) => line.vkey(vk),
                        None => break,
                    }
                }
                if echoing {
                    out.push(&echo);
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
        let (text, keys) = line.finish(processed, &mut newline);
        if !wakeup {
            out.push(&newline);
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
            let Some(units) = self.next_text(unicode) else {
                break;
            };
            let data = self.consume(unicode, &units, capacity - out.len());
            out.extend(data.bytes);
        }
        out
    }

    /// The next character's units for a raw read. An ANSI read takes a
    /// surrogate pair whole, holding a high half until its low half
    /// arrives; a lone half reads as U+FFFD.
    fn next_text(&mut self, unicode: bool) -> Option<Vec<u16>> {
        let first = self.next_unit()?;
        if unicode || !input::is_high_surrogate(first) {
            return Some(vec![first]);
        }
        match self.next_unit() {
            Some(low) if input::is_low_surrogate(low) => Some(vec![first, low]),
            Some(other) => {
                self.held_unit = Some(other);
                Some(vec![first])
            }
            None => {
                self.held_unit = Some(first);
                None
            }
        }
    }

    fn next_unit(&mut self) -> Option<u16> {
        match self.next_char(false)? {
            Got::Char(ch, _) => Some(ch),
            Got::Vkey(_) => None,
        }
    }

    /// conhost `GetChar`: the next character from the queue, or for a
    /// cooked read an editing key. Records without either are consumed.
    fn next_char(&mut self, editing: bool) -> Option<Got> {
        if let Some(unit) = self.held_unit.take() {
            return Some(Got::Char(unit, 0));
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
                    return Some(Got::Char(k.ch, k.state));
                }
                if k.down && (vt || (k.vk != vk::ESCAPE && k.ch != 0x0a)) {
                    return Some(Got::Char(k.ch, k.state));
                }
            }
            if !k.down {
                continue;
            }
            if edit {
                return Some(Got::Vkey(k.vk));
            }
            if k.ch == 0 && self.types_nul(&k) {
                return Some(Got::Char(0, k.state));
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
        let mut taken = 0;
        // A peek leaves the waiting surrogate halves as they are.
        let mut utf8 = self.record_utf8;
        for r in self.input.iter() {
            if out.len() >= count {
                break;
            }
            taken += 1;
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
mod tests {
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

    #[test]
    fn a_hang_up_ends_reads_held_back_by_their_echo() {
        let mut h = Host::new();
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
        assert!(h.c.waiting() > 0, "held back by the stalled reader");
        h.ev(Event::HangUp);
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
        assert_eq!(ctrls(&fx), [(10, Ctrl::C), (11, Ctrl::C)]);
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
        assert_eq!(ctrls(&fx), [(10, Ctrl::Break), (11, Ctrl::Break)]);
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
