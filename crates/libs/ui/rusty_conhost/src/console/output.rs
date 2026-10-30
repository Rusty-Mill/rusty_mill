//! The screen APIs over the host's Grid (design §5.1–§5.3, §5.7):
//! cursor, attributes, screen buffer info, font and window, and resize.
//! Calls that change the screen emit VT for the terminal through the
//! write queue, so they are ordered with the text around them.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/server/ApiDispatchers.cpp`, `src/host/getset.cpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use platform::term::WinSize;

use super::screen::{self, VALID_ATTRIBUTES};
use super::{Console, Effect, Kind, Origin, OutputQueue, Packet, Reply, Text, mode};
use crate::input::Record;
use crate::msg;

/// conhost's default cursor size, in percent of the cell.
pub(super) const DEFAULT_CURSOR_SIZE: u32 = 25;
/// `CONSOLE_GETCURSORINFO_MSG` and `CONSOLE_SETCURSORINFO_MSG`: a ULONG
/// and a BOOLEAN, padded.
const CURSOR_INFO_LEN: usize = 8;
/// `CONSOLE_GETCONSOLEWINDOW_MSG`: one HWND.
const CONSOLE_WINDOW_LEN: usize = 8;
/// `CONSOLE_SCREENBUFFERINFO_MSG`: four COORDs and a USHORT around the
/// window sizes, then the colour table at 28.
const SBI_LEN: usize = 92;
const SBI_COLOR_TABLE: usize = 28;
/// conhost's default popup colours: magenta on white.
const POPUP_ATTRIBUTES: u16 = 0xF5;
/// `CONSOLE_CURRENTFONT_MSG`: the face name, 32 units, at 20.
const FONT_LEN: usize = 84;
const FONT_FACE: usize = 20;
/// What GetCurrentConsoleFont reports: the terminal owns the real font,
/// so these are a plausible cell (8×16 Consolas, `FF_MODERN` with
/// `TMPF_VECTOR | TMPF_TRUETYPE`, normal weight).
const FONT_SIZE: (u16, u16) = (8, 16);
const FONT_FAMILY: u32 = 0x36;
const FONT_WEIGHT: u32 = 400;
const FONT_FACE_NAME: &str = "Consolas";

impl Console {
    /// The output object behind `object`, if the handle has the access.
    fn output_object(&self, object: usize, write: bool) -> Result<(), i32> {
        let Some(o) = self.objects.get(&object) else {
            return Err(msg::STATUS_FROM_HANDLE);
        };
        let access = if write { o.access.write } else { o.access.read };
        if !access {
            return Err(msg::STATUS_FROM_ACCESS_DENIED);
        }
        if o.kind != Kind::Output {
            return Err(msg::STATUS_FROM_HANDLE);
        }
        Ok(())
    }

    /// `CONSOLE_GETCURSORINFO_MSG { OUT CursorSize; OUT Visible }`.
    pub(super) fn get_cursor_info(&self, object: usize, fixed: &mut [u8]) -> i32 {
        if fixed.len() < CURSOR_INFO_LEN {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        if let Err(status) = self.output_object(object, true) {
            return status;
        }
        msg::put_u32(fixed, 0, self.cursor_size);
        fixed[4] = u8::from(self.screen.cursor_visible());
        msg::STATUS_SUCCESS
    }

    /// `CONSOLE_SETCURSORINFO_MSG { IN CursorSize; IN Visible }`: a size
    /// of 1 to 100, mirrored to the terminal as DECTCEM.
    pub(super) fn set_cursor_info(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(size), Some(visible), true) = (
            msg::u32_at(&fixed, 0),
            msg::bool_at(&fixed, 4),
            fixed.len() >= CURSOR_INFO_LEN,
        ) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        if !(1..=100).contains(&size) {
            return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx);
        }
        self.cursor_size = size;
        let vt = if visible { b"\x1b[?25h" } else { b"\x1b[?25l" };
        self.write_host(p, fixed, vt.to_vec(), out, fx);
    }

    /// `CONSOLE_SCREENBUFFERINFO_MSG`. The buffer is the window (§5.1,
    /// V1); CurrentWindowSize is conhost's inclusive extent, one less
    /// than the size.
    pub(super) fn get_screen_buffer_info(&self, object: usize, fixed: &mut [u8]) -> i32 {
        if fixed.len() < SBI_LEN {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        if let Err(status) = self.output_object(object, false) {
            return status;
        }
        let WinSize { rows, cols } = self.screen.size();
        let (x, y) = self.screen.cursor();
        let fields: [(usize, u16); 14] = [
            (0, cols),
            (2, rows),
            (4, x),
            (6, y),
            (8, 0),
            (10, 0),
            (12, self.screen.attributes()),
            (14, cols.saturating_sub(1)),
            (16, rows.saturating_sub(1)),
            (18, cols),
            (20, rows),
            (22, POPUP_ATTRIBUTES),
            // FullscreenSupported and the padding before the table.
            (24, 0),
            (26, 0),
        ];
        for (at, v) in fields {
            msg::put_u16(fixed, at, v);
        }
        for (i, c) in self.screen.color_table().into_iter().enumerate() {
            msg::put_u32(fixed, SBI_COLOR_TABLE + 4 * i, c);
        }
        msg::STATUS_SUCCESS
    }

    /// `CONSOLE_SETCURSORPOSITION_MSG { IN COORD }`: on the screen,
    /// mirrored to the terminal as CUP.
    pub(super) fn set_cursor_position(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(x), Some(y)) = (msg::u16_at(&fixed, 0), msg::u16_at(&fixed, 2)) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        // COORD halves are signed: a negative one is past any size.
        let size = self.screen.size();
        if x >= size.cols || y >= size.rows || x > i16::MAX as u16 || y > i16::MAX as u16 {
            return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx);
        }
        self.write_host(p, fixed, self.absolute_cup(x, y), out, fx);
    }

    /// CUP to screen position (`x`, `y`). The Win32 position is absolute,
    /// so under the client's origin mode (DECOM) it is made relative to the
    /// region; a position outside the region can't be reached with DECOM
    /// on, so origin mode is turned off first.
    fn absolute_cup(&self, x: u16, y: u16) -> Vec<u8> {
        let cup = |col: u16, row: u16| format!("\x1b[{};{}H", row + 1, col + 1);
        match self.screen.origin_region() {
            None => cup(x, y),
            Some((l, t, r, b)) if (l..=r).contains(&x) && (t..=b).contains(&y) => cup(x - l, y - t),
            Some(_) => format!("\x1b[?6l{}", cup(x, y)),
        }
        .into_bytes()
    }

    /// `CONSOLE_SETTEXTATTRIBUTE_MSG { IN Attributes }`, mirrored to the
    /// terminal as SGR (§5.2).
    pub(super) fn set_text_attribute(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let Some(attr) = msg::u16_at(&fixed, 0) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        if attr & !VALID_ATTRIBUTES != 0 {
            return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx);
        }
        self.write_host(p, fixed, screen::sgr(attr), out, fx);
    }

    /// `CONSOLE_CURRENTFONT_MSG { IN MaximumWindow; OUT FontIndex,
    /// FontSize, FontFamily, FontWeight, FaceName }`. For the maximum
    /// window, conhost reports that window's size in cells as FontSize.
    pub(super) fn get_current_font(&self, object: usize, fixed: &mut [u8]) -> i32 {
        if fixed.len() < FONT_LEN {
            return msg::STATUS_ILLEGAL_FUNCTION;
        }
        if let Err(status) = self.output_object(object, false) {
            return status;
        }
        let (x, y) = if fixed[0] != 0 {
            let size = self.screen.size();
            (size.cols, size.rows)
        } else {
            FONT_SIZE
        };
        msg::put_u32(fixed, 4, 0);
        msg::put_u16(fixed, 8, x);
        msg::put_u16(fixed, 10, y);
        msg::put_u32(fixed, 12, FONT_FAMILY);
        msg::put_u32(fixed, 16, FONT_WEIGHT);
        let face = FONT_FACE_NAME.encode_utf16().chain(std::iter::repeat(0));
        for (i, unit) in face.take((FONT_LEN - FONT_FACE) / 2).enumerate() {
            msg::put_u16(fixed, FONT_FACE + 2 * i, unit);
        }
        msg::STATUS_SUCCESS
    }

    /// The terminal's new size (§5.7). The Grid reflows; the terminal
    /// does its own, so nothing is emitted. A client that asked for
    /// WINDOW_INPUT gets a WINDOW_BUFFER_SIZE_EVENT.
    pub(super) fn resize(&mut self, size: WinSize, out: &mut OutputQueue, fx: &mut Vec<Effect>) {
        if size.rows == 0 || size.cols == 0 || size == self.screen.size() {
            return;
        }
        self.screen.resize(size);
        if self.input_mode & mode::WINDOW_INPUT == 0 {
            return;
        }
        let record = Record::BufferSize {
            cols: size.cols,
            rows: size.rows,
        };
        self.store(record, fx);
        self.serve_reads(out, fx);
    }

    /// Reply once `vt`, made by the host, is emitted in order.
    fn write_host(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        vt: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        self.write(reply, Text::Bytes(vt), Origin::Host, out, fx);
    }
}

/// `CONSOLE_GETCONSOLEWINDOW_MSG { OUT HWND }`: there is no window (V5).
pub(super) fn get_console_window(fixed: &mut [u8]) -> i32 {
    if fixed.len() < CONSOLE_WINDOW_LEN {
        return msg::STATUS_ILLEGAL_FUNCTION;
    }
    fixed.fill(0);
    msg::STATUS_SUCCESS
}

fn reply(p: &Packet<'_>, status: i32, fixed: Vec<u8>, fx: &mut Vec<Effect>) {
    let mut reply = Reply::status(p.id, status);
    reply.write_back = fixed;
    fx.push(Effect::Reply(reply));
}

#[cfg(test)]
mod tests {
    use rusty_term_core::{AnsiParser, Grid};

    use super::super::Event;
    use super::super::tests::Host;
    use super::*;
    use crate::msg::api;

    fn coord(x: u16, y: u16) -> Vec<u8> {
        [x.to_le_bytes(), y.to_le_bytes()].concat()
    }

    /// The VT the next call emits.
    fn emitted(h: &mut Host, call: impl FnOnce(&mut Host) -> Reply) -> (Reply, Vec<u8>) {
        let mark = h.vt.len();
        let r = call(h);
        (r, h.vt[mark..].to_vec())
    }

    fn sbi(h: &mut Host, object: usize) -> Reply {
        h.call(object, api::GET_SCREEN_BUFFER_INFO, &[0; SBI_LEN], &[], 0)
    }

    fn u16s(r: &Reply, at: &[usize]) -> Vec<u16> {
        at.iter()
            .map(|&a| msg::u16_at(&r.write_back, a).expect("in the struct"))
            .collect()
    }

    fn set_cursor(h: &mut Host, output: usize, x: u16, y: u16) -> Reply {
        h.call(output, api::SET_CURSOR_POSITION, &coord(x, y), &[], 0)
    }

    #[test]
    fn cursor_info_round_trips_and_emits_dectcem() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let get = |h: &mut Host| {
            let r = h.call(output, api::GET_CURSOR_INFO, &[0; 8], &[], 0);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
            (msg::u32_at(&r.write_back, 0), r.write_back[4])
        };
        assert_eq!(get(&mut h), (Some(DEFAULT_CURSOR_SIZE), 1));
        let mut fixed = 50u32.to_le_bytes().to_vec();
        fixed.extend([0, 0, 0, 0]);
        let (r, vt) = emitted(&mut h, |h| {
            h.call(output, api::SET_CURSOR_INFO, &fixed, &[], 0)
        });
        assert_eq!(
            (r.status, vt.as_slice()),
            (msg::STATUS_SUCCESS, &b"\x1b[?25l"[..])
        );
        assert_eq!(get(&mut h), (Some(50), 0));
        for size in [0u32, 101] {
            let mut bad = size.to_le_bytes().to_vec();
            bad.extend([1, 0, 0, 0]);
            let r = h.call(output, api::SET_CURSOR_INFO, &bad, &[], 0);
            assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
        }
        let r = h.call(input, api::GET_CURSOR_INFO, &[0; 8], &[], 0);
        assert_eq!(r.status, msg::STATUS_FROM_HANDLE);
    }

    #[test]
    fn screen_buffer_info_reads_the_grid() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        h.write_w(output, "ab\x1b[31;44m");
        let r = sbi(&mut h, output);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        // Size, cursor, scroll position, attributes, window, maximum, popup.
        let at = [0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22];
        let want = [80, 24, 2, 0, 0, 0, 0x14, 79, 23, 80, 24, POPUP_ATTRIBUTES];
        assert_eq!(u16s(&r, &at), want);
        let palette = *h.c.screen.palette();
        let red = palette[1];
        let colorref = (red & 0xFF) << 16 | red & 0xFF00 | red >> 16;
        assert_eq!(
            msg::u32_at(&r.write_back, SBI_COLOR_TABLE + 4 * 4),
            Some(colorref)
        );
        assert_eq!(sbi(&mut h, input).status, msg::STATUS_FROM_HANDLE);
        let short = h.call(output, api::GET_SCREEN_BUFFER_INFO, &[0; 88], &[], 0);
        assert_eq!(short.status, msg::STATUS_ILLEGAL_FUNCTION);
    }

    #[test]
    fn screen_buffer_info_needs_read_and_cursor_moves_need_write() {
        let mut h = Host::new();
        let [process, ..] = h.connect(1, "");
        let reader = h.create(process, msg::OBJECT_CURRENT_OUTPUT, msg::GENERIC_READ);
        let reader = reader.information;
        assert_eq!(sbi(&mut h, reader).status, msg::STATUS_SUCCESS);
        let r = set_cursor(&mut h, reader, 0, 0);
        assert_eq!(r.status, msg::STATUS_FROM_ACCESS_DENIED);
        let writer = h.create(process, msg::OBJECT_CURRENT_OUTPUT, msg::GENERIC_WRITE);
        let r = sbi(&mut h, writer.information);
        assert_eq!(r.status, msg::STATUS_FROM_ACCESS_DENIED);
    }

    #[test]
    fn set_cursor_position_emits_cup_inside_the_screen_only() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let (r, vt) = emitted(&mut h, |h| set_cursor(h, output, 4, 2));
        assert_eq!(
            (r.status, vt.as_slice()),
            (msg::STATUS_SUCCESS, &b"\x1b[3;5H"[..])
        );
        assert_eq!(u16s(&sbi(&mut h, output), &[4, 6]), [4, 2]);
        for (x, y) in [(80, 0), (0, 24), (0xFFFF, 0)] {
            let (r, vt) = emitted(&mut h, |h| set_cursor(h, output, x, y));
            assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG, "({x}, {y})");
            assert!(vt.is_empty());
        }
    }

    #[test]
    fn set_cursor_position_stays_absolute_under_origin_mode() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        assert_eq!(h.set_mode(output, 0x5), msg::STATUS_SUCCESS);
        h.packet(msg::RAW_WRITE, 1, output, b"\x1b[5;20r\x1b[?6h", 0);
        let (_, vt) = emitted(&mut h, |h| set_cursor(h, output, 3, 6));
        assert_eq!(vt, b"\x1b[3;4H");
        assert_eq!(u16s(&sbi(&mut h, output), &[4, 6]), [3, 6]);
        let (r, vt) = emitted(&mut h, |h| set_cursor(h, output, 0, 0));
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(vt, b"\x1b[?6l\x1b[1;1H");
        assert_eq!(u16s(&sbi(&mut h, output), &[4, 6]), [0, 0]);
    }

    #[test]
    fn set_text_attribute_emits_sgr_and_reads_back() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let attr = 0x1Eu16 | 0x8000;
        let (r, vt) = emitted(&mut h, |h| {
            h.call(output, api::SET_TEXT_ATTRIBUTE, &attr.to_le_bytes(), &[], 0)
        });
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(vt, b"\x1b[0;93;44;4m");
        assert_eq!(u16s(&sbi(&mut h, output), &[12]), [attr]);
        let r = h.call(
            output,
            api::SET_TEXT_ATTRIBUTE,
            &0x2007u16.to_le_bytes(),
            &[],
            0,
        );
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
    }

    #[test]
    fn host_vt_ignores_the_output_mode() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        assert_eq!(h.set_mode(output, 0), msg::STATUS_SUCCESS);
        let (_, vt) = emitted(&mut h, |h| set_cursor(h, output, 1, 1));
        assert_eq!(vt, b"\x1b[2;2H");
        let (_, vt) = emitted(&mut h, |h| h.write_w(output, "\x1b\n"));
        assert_eq!(vt, "←◙".as_bytes());
    }

    #[test]
    fn a_parked_write_keeps_the_output_mode_it_was_written_under() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let fill = vec![b'x'; crate::sched::PACKET_INPUT_CAP];
        h.packet(msg::RAW_WRITE, 1, output, &fill, 0);
        h.packet(msg::RAW_WRITE, 1, output, &fill, 0);
        let (r, _) = h.packet(msg::RAW_WRITE, 1, output, b"\x1b[m", 0);
        assert_eq!(r.status, i32::MIN, "parked behind the budget");
        // PROCESSED and WRAP_AT_EOL: VT off, and no DECAWM to emit.
        assert_eq!(h.set_mode(output, 0x3), msg::STATUS_SUCCESS);
        let mark = h.vt.len();
        while h.c.waiting() > 0 {
            h.drain();
        }
        assert!(h.vt[mark..].ends_with(b"\x1b[m"), "emitted as VT");
    }

    #[test]
    fn answered_queries_count_against_the_output_budget() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let on = h.mode(input) | mode::VIRTUAL_TERMINAL_INPUT;
        assert_eq!(h.set_mode(input, on), msg::STATUS_SUCCESS);
        let fill = vec![b'x'; crate::sched::PACKET_INPUT_CAP];
        h.packet(msg::RAW_WRITE, 1, output, &fill, 0);
        h.packet(msg::RAW_WRITE, 1, output, &fill, 0);
        let queries = b"\x1b[?6n".repeat(crate::sched::PACKET_INPUT_CAP / 5);
        for _ in 0..15 {
            h.packet(msg::RAW_WRITE, 1, output, &queries, 0);
        }
        let before = h.c.waiting();
        h.drain();
        assert!(h.c.waiting() > 0, "one drain does not run every write");
        assert!(
            before - h.c.waiting() < 4,
            "{} writes ran",
            before - h.c.waiting()
        );
    }

    #[test]
    fn an_lf_after_a_cursor_move_gains_its_cr() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "ab\r");
        set_cursor(&mut h, output, 5, 3);
        let (_, vt) = emitted(&mut h, |h| h.write_w(output, "\n"));
        assert_eq!(vt, b"\r\n");
        assert_eq!(h.c.screen.cursor(), (0, 4));
    }

    #[test]
    fn a_held_prefix_goes_out_at_hang_up() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let (_, vt) = emitted(&mut h, |h| h.write_w(output, "x\x1b"));
        assert_eq!(vt, b"x");
        h.ev(Event::HangUp);
        assert!(h.vt.ends_with(b"x\x1b"));
    }

    #[test]
    fn short_descriptors_are_refused() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let r = h.call(output, api::GET_CURSOR_INFO, &[0; 7], &[], 0);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
        let r = h.call(
            output,
            api::SET_CURSOR_INFO,
            &[25, 0, 0, 0, 1, 0, 0],
            &[],
            0,
        );
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
        let r = h.call(output, api::GET_CONSOLE_WINDOW, &[0; 4], &[], 0);
        assert_eq!(r.status, msg::STATUS_ILLEGAL_FUNCTION);
    }

    #[test]
    fn current_font_and_console_window() {
        let mut h = Host::new();
        let [_, _, output] = h.connect(1, "");
        let r = h.call(output, api::GET_CURRENT_FONT, &[0; FONT_LEN], &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(u16s(&r, &[8, 10]), [8, 16]);
        assert_eq!(msg::u32_at(&r.write_back, 12), Some(FONT_FAMILY));
        let face = msg::utf16_units(&r.write_back[FONT_FACE..]);
        assert_eq!(String::from_utf16_lossy(&face[..9]), "Consolas\0");
        let mut maximum = [0u8; FONT_LEN];
        maximum[0] = 1;
        let r = h.call(output, api::GET_CURRENT_FONT, &maximum, &[], 0);
        assert_eq!(u16s(&r, &[8, 10]), [80, 24], "the maximum window, in cells");
        let r = h.call(output, api::GET_CONSOLE_WINDOW, &[0xAA; 8], &[], 0);
        assert_eq!((r.status, r.write_back), (msg::STATUS_SUCCESS, vec![0; 8]));
    }

    #[test]
    fn resize_queues_a_buffer_size_event_under_window_input() {
        let mut h = Host::new();
        let [_, input, _] = h.connect(1, "");
        let mark = h.vt.len();
        h.ev(Event::Resize(WinSize {
            rows: 30,
            cols: 100,
        }));
        assert!(h.c.input.is_empty(), "WINDOW_INPUT is off");
        assert_eq!(
            h.c.size(),
            WinSize {
                rows: 30,
                cols: 100
            }
        );
        let on = h.mode(input) | mode::WINDOW_INPUT;
        assert_eq!(h.set_mode(input, on), msg::STATUS_SUCCESS);
        h.ev(Event::Resize(WinSize { rows: 0, cols: 50 }));
        h.ev(Event::Resize(WinSize {
            rows: 30,
            cols: 100,
        }));
        assert!(
            h.c.input.is_empty(),
            "no size, and no change, queue nothing"
        );
        h.ev(Event::Resize(WinSize { rows: 20, cols: 60 }));
        let records: Vec<_> = h.c.input.iter().copied().collect();
        assert_eq!(records, [Record::BufferSize { cols: 60, rows: 20 }]);
        assert_eq!(h.vt.len(), mark, "a resize emits nothing");
    }

    #[test]
    fn dsr_and_cpr_answer_a_vt_client_from_the_grid() {
        let mut h = Host::new();
        let [_, input, output] = h.connect(1, "");
        let (_, vt) = emitted(&mut h, |h| h.write_w(output, "ab\x1b[6n"));
        assert_eq!(vt, b"ab", "the terminal never sees the query");
        assert!(
            h.c.input.is_empty(),
            "dropped without VIRTUAL_TERMINAL_INPUT"
        );
        let on = h.mode(input) | mode::VIRTUAL_TERMINAL_INPUT;
        assert_eq!(h.set_mode(input, on), msg::STATUS_SUCCESS);
        h.write_w(output, "\x1b[5n\x1b[6n");
        let typed: Vec<u16> =
            h.c.input
                .iter()
                .map(|r| match r {
                    Record::Key(k) => k.ch,
                    _ => 0,
                })
                .collect();
        assert_eq!(String::from_utf16_lossy(&typed), "\x1b[0n\x1b[1;3R");
    }

    /// The terminal's side of a V3 test: a Grid with scrollback, fed what
    /// the host sent.
    struct Terminal {
        parser: AnsiParser,
        grid: Grid,
        seen: usize,
    }

    impl Terminal {
        fn new(size: WinSize) -> Terminal {
            Terminal {
                parser: AnsiParser::new(),
                grid: Grid::new(usize::from(size.cols), usize::from(size.rows)),
                seen: 0,
            }
        }

        fn sync(&mut self, h: &Host) {
            self.parser.advance(&mut self.grid, &h.vt[self.seen..]);
            self.seen = h.vt.len();
        }
    }

    fn rows(g: &Grid) -> Vec<String> {
        (0..g.rows)
            .map(|y| {
                g.row(y)
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Resize both sides, as the terminal and the host would.
    fn resize(h: &mut Host, t: &mut Terminal, size: WinSize) {
        h.ev(Event::Resize(size));
        t.grid
            .resize(usize::from(size.cols), usize::from(size.rows));
    }

    /// SetConsoleCursorPosition then WriteConsole, and a relative write:
    /// the VT each emits.
    fn positioned_then_relative(h: &mut Host, output: usize, y: u16) -> [Vec<u8>; 2] {
        let (_, mut cup) = emitted(h, |h| set_cursor(h, output, 0, y));
        let (_, d) = emitted(h, |h| h.write_w(output, "D"));
        cup.extend(d);
        let (_, rel) = emitted(h, |h| h.write_w(output, "\r\nE"));
        [cup, rel]
    }

    #[test]
    fn v3_shrink_then_grow_diverges_as_documented() {
        let size = |rows| WinSize { rows, cols: 4 };
        let mut h = Host::sized(size(3));
        let mut t = Terminal::new(size(3));
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "A\r\nB\r\nC");
        t.sync(&h);
        assert_eq!(rows(h.c.screen.grid()), rows(&t.grid));
        resize(&mut h, &mut t, size(2));
        resize(&mut h, &mut t, size(3));
        assert_eq!(rows(&t.grid), ["A", "B", "C"]);
        assert_eq!(t.grid.cursor.1, 2);
        assert_eq!(rows(h.c.screen.grid()), ["B", "C", ""]);
        assert_eq!(h.c.screen.cursor().1, 1);
        let [cup, rel] = positioned_then_relative(&mut h, output, 2);
        assert_eq!(cup, b"\x1b[3;1HD");
        assert_eq!(rel, b"\r\nE");
        t.sync(&h);
        // The relative write scrolls both, from different screens.
        assert_eq!(rows(h.c.screen.grid()), ["C", "D", "E"]);
        assert_eq!(rows(&t.grid), ["B", "D", "E"]);
    }

    #[test]
    fn v3_narrow_then_widen_diverges_on_the_primary_screen() {
        let size = |cols| WinSize { rows: 3, cols };
        let mut h = Host::sized(size(4));
        let mut t = Terminal::new(size(4));
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "x\r\nabcdef");
        t.sync(&h);
        resize(&mut h, &mut t, size(2));
        resize(&mut h, &mut t, size(4));
        assert_eq!(rows(&t.grid), ["x", "abcd", "ef"]);
        assert_eq!(rows(h.c.screen.grid()), ["abcd", "ef", ""]);
        let [cup, rel] = positioned_then_relative(&mut h, output, 2);
        assert_eq!(cup, b"\x1b[3;1HD");
        assert_eq!(rel, b"\r\nE");
        t.sync(&h);
        // The terminal's D lands on the rejoined "ef" row.
        assert_eq!(rows(h.c.screen.grid()), ["ef", "D", "E"]);
        assert_eq!(rows(&t.grid), ["abcd", "Df", "E"]);
    }

    #[test]
    fn v3_narrow_then_widen_on_the_alternate_screen() {
        let size = |cols| WinSize { rows: 3, cols };
        let mut h = Host::sized(size(4));
        let mut t = Terminal::new(size(4));
        let [_, _, output] = h.connect(1, "");
        h.write_w(output, "\x1b[?1049hx\r\nabcdef");
        t.sync(&h);
        resize(&mut h, &mut t, size(2));
        resize(&mut h, &mut t, size(4));
        // The alternate screen keeps no history and does not rejoin
        // rows, so both sides clip alike and stay in step.
        assert_eq!(rows(h.c.screen.grid()), ["x", "ab", "ef"]);
        assert_eq!(rows(&t.grid), rows(h.c.screen.grid()));
        let [cup, rel] = positioned_then_relative(&mut h, output, 2);
        assert_eq!(cup, b"\x1b[3;1HD");
        assert_eq!(rel, b"\r\nE");
        t.sync(&h);
        assert_eq!(rows(h.c.screen.grid()), ["ab", "Df", "E"]);
        assert_eq!(rows(&t.grid), rows(h.c.screen.grid()));
    }
}
