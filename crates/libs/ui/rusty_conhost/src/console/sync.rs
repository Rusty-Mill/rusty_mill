//! The cursor resync after a resize (#1375, design §5.7). The terminal
//! reflows with its scrollback and the host's Grid has none, so after a
//! resize the two can put the cursor on different rows (V3). A client
//! that positions absolutely from GetConsoleScreenBufferInfo's cursor,
//! as PSReadLine does, then writes to the wrong rows.
//!
//! As ConPTY does, a resize marks the cursor as possibly wrong. The next
//! GetConsoleScreenBufferInfo asks the terminal with DSR 6 and is held,
//! together with every packet after it, until the terminal's CPR arrives
//! or [`CURSOR_WAIT`] passes. The CPR moves the host's cursor to the
//! terminal's; the held calls are then answered. Only the cursor is
//! resynced: the screen's contents may still differ (V3).
//!
//! Behaviour follows microsoft/terminal's conhost (GH#18725,
//! `src/host/screenInfo.cpp` `WaitForConptyCursorPositionToBeSynchronized`,
//! `src/server/ApiDispatchers.cpp` `ServerGetConsoleScreenBufferInfo`,
//! `src/terminal/parser/InputStateMachineEngine.cpp`
//! `CaptureNextCursorPositionReport`; see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use std::time::Duration;

use super::output::{SBI_LEN, reply};
use super::{Console, Effect, OutputQueue, Packet, Reply};

/// How long a held GetConsoleScreenBufferInfo waits for the terminal's
/// CPR before it is answered from the host's model, as in ConPTY.
pub(crate) const CURSOR_WAIT: Duration = Duration::from_millis(500);
/// What asks the terminal for its cursor: DSR 6.
const DSR_CPR: &[u8] = b"\x1b[6n";

/// Whether the host's cursor matches the terminal's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CursorSync {
    /// No resize since the last resync.
    #[default]
    Clean,
    /// Resized: the next GetConsoleScreenBufferInfo asks the terminal.
    Dirty,
    /// DSR 6 went out and its CPR is awaited. `stale` when the size
    /// changed again since: the CPR may predate that resize, so the
    /// terminal is asked once more.
    Asked { stale: bool },
}

/// A GetConsoleScreenBufferInfo held for the terminal's CPR.
#[derive(Debug)]
pub(super) struct HeldInfo {
    id: [u8; 8],
    object: usize,
    fixed: Vec<u8>,
}

impl Console {
    /// `CONSOLE_SCREENBUFFERINFO_MSG`, after the cursor resync a resize
    /// asks for.
    pub(super) fn screen_buffer_info(
        &mut self,
        p: &Packet<'_>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let ready = fixed.len() < SBI_LEN
            || self.output_object(p.object, false).is_err()
            || self.cursor_sync == CursorSync::Clean;
        if ready {
            let status = self.get_screen_buffer_info(p.object, &mut fixed);
            return reply(p, status, fixed, fx);
        }
        if self.cursor_sync == CursorSync::Dirty {
            self.ask_cursor(out, fx);
        }
        self.held_info.push_back(HeldInfo {
            id: p.id,
            object: p.object,
            fixed,
        });
    }

    /// The size changed: the cursor may no longer match the terminal's.
    pub(super) fn cursor_maybe_wrong(&mut self) {
        self.cursor_sync = match self.cursor_sync {
            CursorSync::Clean | CursorSync::Dirty => CursorSync::Dirty,
            CursorSync::Asked { .. } => CursorSync::Asked { stale: true },
        };
    }

    /// A CPR is awaited: the next one in the terminal's input is the
    /// answer, not a key, and no packet is taken until it comes.
    pub(crate) fn awaiting_cursor(&self) -> bool {
        matches!(self.cursor_sync, CursorSync::Asked { .. })
    }

    /// The terminal's CPR, 1-based: the host's cursor moves there, unless
    /// a resize since makes the terminal be asked again.
    pub(super) fn cursor_reported(
        &mut self,
        (row, col): (u16, u16),
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        match self.cursor_sync {
            CursorSync::Asked { stale: true } => self.ask_cursor(out, fx),
            CursorSync::Asked { stale: false } => {
                self.screen
                    .place_cursor(col.saturating_sub(1), row.saturating_sub(1));
                self.answer_held_info(fx);
            }
            CursorSync::Clean | CursorSync::Dirty => {}
        }
    }

    /// No CPR came in time: answer from the host's model, as ConPTY does.
    pub(super) fn cursor_wait_expired(&mut self, fx: &mut Vec<Effect>) {
        if self.awaiting_cursor() {
            self.answer_held_info(fx);
        }
    }

    /// Calls held for the CPR; the console is closing.
    pub(super) fn drop_held_info(&mut self) {
        self.held_info.clear();
        self.cursor_sync = CursorSync::Clean;
    }

    pub(super) fn held_info(&self) -> usize {
        self.held_info.len()
    }

    fn ask_cursor(&mut self, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        self.screen.host(DSR_CPR, out);
        self.cursor_sync = CursorSync::Asked { stale: false };
        fx.push(Effect::AwaitCursor);
    }

    fn answer_held_info(&mut self, fx: &mut Vec<Effect>) {
        self.cursor_sync = CursorSync::Clean;
        while let Some(HeldInfo {
            id,
            object,
            mut fixed,
        }) = self.held_info.pop_front()
        {
            let status = self.get_screen_buffer_info(object, &mut fixed);
            let mut r = Reply::status(id, status);
            r.write_back = fixed;
            fx.push(Effect::Complete(r));
        }
    }
}

/// `ESC [ row ; col R`, a CPR, as 1-based (row, col).
pub(crate) fn cursor_report(raw: &[u8]) -> Option<(u16, u16)> {
    let params = raw.strip_prefix(b"\x1b[")?.strip_suffix(b"R")?;
    let (row, col) = std::str::from_utf8(params).ok()?.split_once(';')?;
    Some((row.parse().ok()?, col.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use platform::term::WinSize;

    use super::super::tests::Host;
    use super::super::{Effect, Event};
    use super::*;
    use crate::msg::{self, api};

    fn sbi_packet(h: &mut Host, output: usize) -> Reply {
        h.call(output, api::GET_SCREEN_BUFFER_INFO, &[0; SBI_LEN], &[], 0)
    }

    fn cursor_of(r: &Reply) -> (u16, u16) {
        let at = |i| msg::u16_at(&r.write_back, i).expect("COORD");
        (at(4), at(6))
    }

    fn completed(fx: &[Effect]) -> Vec<Reply> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Complete(r) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }

    fn size(rows: u16) -> WinSize {
        WinSize { rows, cols: 10 }
    }

    #[test]
    fn cursor_report_parses_only_a_cpr() {
        assert_eq!(cursor_report(b"\x1b[3;14R"), Some((3, 14)));
        for raw in [
            &b"\x1b[3R"[..],
            b"\x1b[?3;1R",
            b"\x1b[3;1~",
            b"\x1b[;1R",
            b"x",
        ] {
            assert_eq!(cursor_report(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn without_a_resize_the_info_is_answered_at_once() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        let mark = h.vt.len();
        let r = sbi_packet(&mut h, output);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(h.vt.len(), mark, "nothing asked");
        assert!(!h.c.awaiting_cursor());
    }

    #[test]
    fn after_a_resize_the_info_waits_for_the_terminals_cursor() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "a\r\nb");
        h.ev(Event::Resize(size(6)));
        let mark = h.vt.len();
        let first = sbi_packet(&mut h, output);
        assert_eq!(first.status, i32::MIN, "held");
        assert_eq!(&h.vt[mark..], DSR_CPR);
        assert!(h.c.awaiting_cursor());
        let second = sbi_packet(&mut h, output);
        assert_eq!(second.status, i32::MIN, "held behind the first");
        assert_eq!(&h.vt[mark..], DSR_CPR, "asked once");
        assert_eq!(h.c.waiting(), 2);
        // The terminal pulled a row back from its scrollback.
        let fx = h.ev(Event::Input(b"\x1b[3;2R"));
        let done = completed(&fx);
        assert_eq!(
            done.iter().map(|r| r.id).collect::<Vec<_>>(),
            [first.id, second.id]
        );
        for r in &done {
            assert_eq!((r.status, cursor_of(r)), (msg::STATUS_SUCCESS, (1, 2)));
        }
        assert!(h.c.input.is_empty(), "the CPR is no key");
        assert_eq!(h.c.waiting(), 0);
        let r = sbi_packet(&mut h, output);
        assert_eq!(cursor_of(&r), (1, 2), "clean until the next resize");
    }

    #[test]
    fn a_cpr_at_the_cursor_keeps_its_pending_wrap() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        h.ev(Event::Resize(size(5)));
        h.write_w(output, "0123456789");
        sbi_packet(&mut h, output);
        h.ev(Event::Input(b"\x1b[1;10R"));
        h.write_w(output, "x");
        assert_eq!(h.c.screen.cursor(), (1, 1), "x wrapped to the next row");
    }

    #[test]
    fn a_resize_while_asking_asks_again() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        h.ev(Event::Resize(size(6)));
        let mark = h.vt.len();
        let held = sbi_packet(&mut h, output);
        h.ev(Event::Resize(size(3)));
        let fx = h.ev(Event::Input(b"\x1b[5;1R"));
        assert!(
            completed(&fx).is_empty(),
            "the answer may predate the resize"
        );
        assert!(fx.contains(&Effect::AwaitCursor));
        assert_eq!(&h.vt[mark..], [DSR_CPR, DSR_CPR].concat());
        let fx = h.ev(Event::Input(b"\x1b[2;1R"));
        let done = completed(&fx);
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].id, cursor_of(&done[0])), (held.id, (0, 1)));
    }

    #[test]
    fn with_no_answer_the_model_is_reported() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "ab");
        h.ev(Event::Resize(size(6)));
        let held = sbi_packet(&mut h, output);
        let fx = h.ev(Event::CursorWaitExpired);
        let done = completed(&fx);
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].id, cursor_of(&done[0])), (held.id, (2, 0)));
        assert!(!h.c.awaiting_cursor());
        assert!(h.ev(Event::CursorWaitExpired).is_empty(), "a late timer");
    }

    #[test]
    fn a_cpr_nobody_asked_for_is_still_input() {
        let mut h = Host::sized(size(4));
        h.connect(1, "");
        h.ev(Event::Input(b"\x1b[1;5R"));
        assert!(!h.c.input.is_empty(), "Ctrl+F3, as before");
    }

    #[test]
    fn bad_calls_fail_without_waiting() {
        let mut h = Host::sized(size(4));
        let [_, input, output] = h.connect(1, "");
        h.ev(Event::Resize(size(6)));
        let r = h.call(input, api::GET_SCREEN_BUFFER_INFO, &[0; SBI_LEN], &[], 0);
        assert_eq!(r.status, msg::STATUS_FROM_HANDLE);
        let short = h.call(output, api::GET_SCREEN_BUFFER_INFO, &[0; 88], &[], 0);
        assert_eq!(short.status, msg::STATUS_ILLEGAL_FUNCTION);
        assert!(!h.c.awaiting_cursor());
    }

    #[test]
    fn closing_drops_the_held_calls() {
        let mut h = Host::sized(size(4));
        let [_, _, output] = h.connect(1, "");
        h.ev(Event::Resize(size(6)));
        sbi_packet(&mut h, output);
        h.ev(Event::HangUp);
        h.ev(Event::GraceExpired);
        assert_eq!(h.c.waiting(), 0);
        assert!(!h.c.awaiting_cursor());
    }
}
