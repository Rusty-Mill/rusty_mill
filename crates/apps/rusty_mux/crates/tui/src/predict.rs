//! Predictive echo (C13, `docs/projects/mux/PREDICTIVE-ECHO.md`): guesses
//! what typed keys will show before the server's screen does, and checks
//! each guess when the server acks its key. Plain data in, cells to draw
//! out; no I/O.

use std::time::{Duration, Instant};

use rusty_mux_proto::Echo;
use rusty_term_core::{AnsiParser, Grid, WIDE_TRAILER, char_width};

/// Failed checks this many keys apart, or closer, turn predictions off
/// until the next Enter.
const CONFIDENCE: u64 = 10;

/// One predicted cell: what the key `key` should make column `x` show,
/// `None` for a blank left by Backspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Guess {
    x: usize,
    ch: Option<char>,
    key: u64,
}

/// A run of keys whose effect the client can model. Drawn only once one
/// of its guesses has been found on the screen.
#[derive(Debug, Default)]
struct Epoch {
    confirmed: bool,
    /// The row predicted on and the predicted cursor's column, from the
    /// epoch's first predicted key.
    at: Option<(usize, usize)>,
    /// The guesses not yet settled, at most one per column.
    guesses: Vec<Guess>,
}

/// What to draw over a block: its predicted cells, characters underlined
/// and `None` erased, and the cursor after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub row: usize,
    pub cells: Vec<(usize, Option<char>)>,
    pub cursor: usize,
}

/// One block's predictions, fed its keys, acks, echo modes and output.
#[derive(Debug)]
pub struct Predictor {
    epoch: Epoch,
    mode: Echo,
    /// Whether the grid showed the alternate screen at the last output.
    alt: bool,
    /// Keys typed so far, to space failed checks.
    typed: u64,
    /// `typed` at the last failed check.
    failed: Option<u64>,
    /// Off after two close failures, until the next Enter.
    off: bool,
}

impl Default for Predictor {
    /// Predicts nothing until the server tells an echo mode.
    fn default() -> Self {
        Self {
            epoch: Epoch::default(),
            mode: Echo::Unknown,
            alt: false,
            typed: 0,
            failed: None,
            off: false,
        }
    }
}

impl Predictor {
    /// Models the key sent as `Input { req, data }` on `grid`, the
    /// client's copy of the block's screen when it was sent.
    pub fn key(&mut self, req: u64, data: &[u8], grid: &Grid) {
        self.typed += 1;
        if data.contains(&b'\r') {
            self.off = false;
        }
        let modeled = match data {
            [0x7f] | [0x08] => self.erase(req),
            _ => std::str::from_utf8(data).is_ok_and(|text| self.text(req, text, grid)),
        };
        if !modeled {
            self.reset();
        }
    }

    /// Checks the characters `EchoAck { req }` settles against `grid`,
    /// once every Output before the ack is applied to it. Settled blanks
    /// leave unchecked: what erasing shows is the program's business, and
    /// only a character found on the screen confirms the epoch.
    pub fn ack(&mut self, req: u64, grid: &Grid) {
        let Some((row, _)) = self.epoch.at else {
            return;
        };
        let (settled, later): (Vec<Guess>, Vec<Guess>) =
            self.epoch.guesses.iter().partition(|g| g.key <= req);
        let typed: Vec<(usize, char)> = settled.iter().filter_map(|g| Some((g.x, g.ch?))).collect();
        let shown =
            |&(x, ch): &(usize, char)| grid.row_cells(row).nth(x).is_some_and(|c| c.ch == ch);
        if !typed.iter().all(shown) {
            return self.fail();
        }
        self.epoch.guesses = later;
        self.epoch.confirmed |= !typed.is_empty();
    }

    /// Takes the block's new echo mode.
    pub fn mode(&mut self, mode: Echo) {
        if mode != self.mode {
            self.mode = mode;
            self.reset();
        }
    }

    /// Applies the block's output `data` to `grid` through `parser`:
    /// leaving the predicted row or switching screens ends the epoch, even
    /// for a moment within `data`. So while an epoch has a row, `data` is
    /// parsed in steps, each up to and including a C0 control byte (ESC
    /// among them), and checked after each: only those bytes, and a wrap
    /// at the margin, move the cursor off its row or switch screens.
    pub fn output(&mut self, parser: &mut AnsiParser, grid: &mut Grid, data: &[u8]) {
        let mut rest = data;
        while self.epoch.at.is_some() && !rest.is_empty() {
            let end = rest
                .iter()
                .position(|b| *b < 0x20)
                .map_or(rest.len(), |i| i + 1);
            parser.advance(grid, &rest[..end]);
            self.follow(grid);
            rest = &rest[end..];
        }
        // Without a row to leave, the rest goes in one step.
        parser.advance(grid, rest);
        self.follow(grid);
    }

    fn follow(&mut self, grid: &Grid) {
        let alt = grid.in_alt_screen();
        let moved = self.epoch.at.is_some_and(|(row, _)| grid.cursor.1 != row);
        if alt != self.alt || moved {
            self.alt = alt;
            self.reset();
        }
    }

    /// Drops every guess and starts a tentative epoch: the screen or the
    /// client's place on it changed in a way the epoch did not model.
    pub fn reset(&mut self) {
        self.epoch = Epoch::default();
    }

    /// What to draw: nothing for a tentative epoch, while off, or unless
    /// the program's own echo is on.
    pub fn overlay(&self) -> Option<Overlay> {
        let drawing =
            self.epoch.confirmed && !self.off && matches!(self.mode, Echo::Kernel | Echo::Program);
        let (row, cursor) = self.epoch.at.filter(|_| drawing)?;
        let cells: Vec<(usize, Option<char>)> =
            self.epoch.guesses.iter().map(|g| (g.x, g.ch)).collect();
        (!cells.is_empty()).then_some(Overlay { row, cells, cursor })
    }

    /// Predicts `text` typed at the predicted cursor, if every character is
    /// one cell wide and lands on the blank tail of the row.
    fn text(&mut self, req: u64, text: &str, grid: &Grid) -> bool {
        if self.off || !matches!(self.mode, Echo::Kernel | Echo::Program) {
            return false;
        }
        let (row, mut x) = self.epoch.at.unwrap_or((grid.cursor.1, grid.cursor.0));
        let mut guesses = self.epoch.guesses.clone();
        for ch in text.chars() {
            if ch.is_control() || char_width(ch) != 1 || !self.blank_tail(grid, row, x) {
                return false;
            }
            // Over a blank Backspace left, which this replaces.
            guesses.retain(|g| g.x != x);
            guesses.push(Guess {
                x,
                ch: Some(ch),
                key: req,
            });
            x += 1;
        }
        self.epoch.guesses = guesses;
        self.epoch.at = Some((row, x));
        true
    }

    /// Backspace `req` over this epoch's own unsettled character just left
    /// of the predicted cursor blanks it until `req` settles, so an echo of
    /// the erased character does not show again meanwhile; anything else is
    /// the program's business.
    fn erase(&mut self, req: u64) -> bool {
        let Some((row, x)) = self.epoch.at else {
            return false;
        };
        let left = x.checked_sub(1);
        let Some(guess) = self
            .epoch
            .guesses
            .iter_mut()
            .find(|g| Some(g.x) == left && g.ch.is_some())
        else {
            return false;
        };
        // Its own ack no longer settles it: the screen may show it erased.
        *guess = Guess {
            ch: None,
            key: req,
            ..*guess
        };
        self.epoch.at = Some((row, x - 1));
        true
    }

    /// Whether column `x` and everything right of it on `row` is blank,
    /// on the grid or by this epoch's own erasing, with `x` inside the row.
    fn blank_tail(&self, grid: &Grid, row: usize, x: usize) -> bool {
        let erased = |at: usize| {
            self.epoch
                .guesses
                .iter()
                .any(|g| g.x == at && g.ch.is_none())
        };
        row < grid.rows
            && x < grid.cols
            && (grid.row_cells(row).enumerate().skip(x)).all(|(at, c)| {
                erased(at) || (c.ch == ' ' && c.cluster == 0 && c.flags & WIDE_TRAILER == 0)
            })
    }

    fn fail(&mut self) {
        self.reset();
        if self.failed.is_some_and(|at| self.typed - at <= CONFIDENCE) {
            self.off = true;
        }
        self.failed = Some(self.typed);
    }
}

/// Start drawing above this smoothed round trip.
const SLOW: Duration = Duration::from_millis(30);
/// Stop drawing below this one.
const FAST: Duration = Duration::from_millis(20);
/// The least time between round-trip samples.
const SAMPLE: Duration = Duration::from_secs(1);

/// Whether a block's path is slow enough for predictions to be worth
/// drawing, from `Sync` round trips timed on its connection.
#[derive(Debug, Default)]
pub struct Gate {
    /// The smoothed round trip, each sample weighted 1/8.
    srtt: Option<Duration>,
    slow: bool,
    /// When the last sample came.
    sampled: Option<Instant>,
    /// The `Sync` in flight and when it was sent.
    pending: Option<(u64, Instant)>,
}

impl Gate {
    /// Whether a key typed `now` should carry a `Sync`.
    pub fn due(&self, now: Instant) -> bool {
        self.pending.is_none() && self.sampled.is_none_or(|t| now - t >= SAMPLE)
    }

    pub fn sent(&mut self, req: u64, now: Instant) {
        self.pending = Some((req, now));
    }

    /// Whether `req` is the `Sync` in flight.
    pub fn awaits(&self, req: u64) -> bool {
        self.pending.is_some_and(|(r, _)| r == req)
    }

    /// Takes the answer to the `Sync` in flight, come `now`.
    pub fn answered(&mut self, now: Instant) {
        let Some((_, sent)) = self.pending.take() else {
            return;
        };
        let sample = now - sent;
        let srtt = self.srtt.map_or(sample, |s| (s * 7 + sample) / 8);
        self.srtt = Some(srtt);
        self.sampled = Some(now);
        self.slow = match self.slow {
            false => srtt > SLOW,
            true => srtt >= FAST,
        };
    }

    /// Whether predictions should be drawn: off until the first sample.
    pub fn slow(&self) -> bool {
        self.slow
    }
}

#[cfg(test)]
mod tests {
    use rusty_term_core::AnsiParser;

    use super::*;

    /// A shell at its prompt, `$ ` on the top row of a 10-column screen.
    struct Shell {
        grid: Grid,
        parser: AnsiParser,
        p: Predictor,
        next: u64,
    }

    impl Shell {
        fn new(mode: Echo) -> Self {
            let mut shell = Self {
                grid: Grid::new(10, 3),
                parser: AnsiParser::new(),
                p: Predictor::default(),
                next: 1,
            };
            shell.p.mode(mode);
            shell.echo("$ ");
            shell
        }

        /// Types `data`, returning its req.
        fn key(&mut self, data: &str) -> u64 {
            let req = self.next;
            self.next += 1;
            self.p.key(req, data.as_bytes(), &self.grid);
            req
        }

        fn echo(&mut self, data: &str) {
            (self.p).output(&mut self.parser, &mut self.grid, data.as_bytes());
        }

        fn ack(&mut self, req: u64) {
            self.p.ack(req, &self.grid);
        }

        /// Types `a` and has it echoed and acked: a confirmed epoch.
        fn confirmed(mode: Echo) -> Self {
            let mut shell = Self::new(mode);
            let a = shell.key("a");
            shell.echo("a");
            shell.ack(a);
            shell
        }

        fn drawn(&self) -> Vec<(usize, char)> {
            let cells = self.p.overlay().map(|o| o.cells).unwrap_or_default();
            cells
                .into_iter()
                .map(|(x, ch)| (x, ch.unwrap_or('_')))
                .collect()
        }
    }

    #[test]
    fn a_tentative_epoch_draws_nothing() {
        let mut shell = Shell::new(Echo::Kernel);
        shell.key("a");
        shell.key("b");
        assert_eq!(shell.p.overlay(), None);
    }

    #[test]
    fn a_confirmed_epoch_draws_later_keys_underlined_ahead() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.key("b");
        shell.key("c");
        let overlay = shell.p.overlay();
        assert_eq!(
            overlay,
            Some(Overlay {
                row: 0,
                cells: vec![(3, Some('b')), (4, Some('c'))],
                cursor: 5
            })
        );
    }

    #[test]
    fn coalesced_typing_checks_each_key_at_its_own_ack() {
        let mut shell = Shell::new(Echo::Program);
        let a = shell.key("a");
        let b = shell.key("b");
        shell.echo("ab");
        shell.ack(a);
        // `b` is left for its own ack, and drawn meanwhile.
        assert_eq!(shell.drawn(), [(3, 'b')]);
        shell.ack(b);
        assert_eq!(shell.p.overlay(), None);
        shell.key("c");
        assert_eq!(shell.drawn(), [(4, 'c')]);
    }

    #[test]
    fn an_ack_settling_only_erased_keys_confirms_nothing() {
        let mut shell = Shell::new(Echo::Kernel);
        let a = shell.key("a");
        shell.key("\x7f");
        shell.echo("a\x08 \x08");
        shell.ack(a);
        shell.key("c");
        assert_eq!(shell.p.overlay(), None, "still tentative");
        let d = shell.key("d");
        shell.echo("cd");
        shell.ack(d);
        assert!(shell.p.epoch.confirmed, "found on the screen");
    }

    #[test]
    fn backspace_over_its_own_guess_blanks_it() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.key("b");
        shell.key("c");
        shell.key("\x08");
        let overlay = shell.p.overlay().map(|o| (o.cells, o.cursor));
        assert_eq!(overlay, Some((vec![(3, Some('b')), (4, None)], 4)));
    }

    #[test]
    fn an_erased_echo_stays_blank_until_the_backspace_settles() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        let b = shell.key("b");
        shell.echo("b");
        let erase = shell.key("\x7f");
        // `b` is on the grid, but erased ahead of the server.
        assert_eq!(shell.drawn(), [(3, '_')]);
        assert_eq!(shell.p.overlay().map(|o| o.cursor), Some(3));
        shell.ack(b);
        assert_eq!(shell.drawn(), [(3, '_')], "b's ack settles nothing now");
        // Typed over the erased echo: predicted, replacing the blank.
        let c = shell.key("c");
        assert_eq!(shell.drawn(), [(3, 'c')]);
        shell.echo("\x08 \x08c");
        shell.ack(erase);
        assert_eq!(shell.drawn(), [(3, 'c')]);
        shell.ack(c);
        assert_eq!(shell.p.overlay(), None);
        assert!(shell.p.epoch.confirmed);
    }

    #[test]
    fn a_settled_blank_alone_confirms_nothing() {
        let mut shell = Shell::new(Echo::Kernel);
        shell.key("a");
        let erase = shell.key("\x7f");
        shell.echo("a\x08 \x08");
        shell.ack(erase);
        assert!(!shell.p.epoch.confirmed);
        assert!(shell.p.epoch.guesses.is_empty());
    }

    #[test]
    fn backspace_over_anything_else_starts_a_tentative_epoch() {
        // Over a confirmed character.
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.key("\x7f");
        shell.key("b");
        assert_eq!(shell.p.overlay(), None);
        // At the start of an epoch, over the prompt.
        let mut shell = Shell::new(Echo::Kernel);
        shell.key("\x7f");
        assert!(shell.p.epoch.at.is_none());
    }

    #[test]
    fn typing_in_the_middle_of_a_line_is_not_predicted() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        // Text right of the cursor, which stays at column 3.
        shell.echo("\x1b[6Gz\x1b[4G");
        shell.key("b");
        assert_eq!(shell.p.overlay(), None);
        assert!(shell.p.epoch.guesses.is_empty());
        shell.key("c");
        assert!(shell.p.epoch.guesses.is_empty(), "nor the next key");
    }

    #[test]
    fn a_partial_watermark_leaves_later_keys_for_the_next_ack() {
        let mut shell = Shell::new(Echo::Kernel);
        shell.key("a");
        let b = shell.key("b");
        let c = shell.key("c");
        shell.echo("ab");
        shell.ack(b);
        assert_eq!(shell.drawn(), [(4, 'c')]);
        shell.echo("c");
        shell.ack(c);
        assert!(shell.p.epoch.confirmed);
        assert!(shell.p.epoch.guesses.is_empty());
    }

    #[test]
    fn a_masked_field_shows_the_keys_typed_until_the_first_ack() {
        let mut shell = Shell::confirmed(Echo::Program);
        let p = shell.key("p");
        shell.key("w");
        shell.key("d");
        // Owner decision 3, option B: no lead cap.
        assert_eq!(shell.drawn(), [(3, 'p'), (4, 'w'), (5, 'd')]);
        shell.echo("***");
        shell.ack(p);
        assert_eq!(shell.p.overlay(), None);
        shell.key("x");
        assert_eq!(shell.p.overlay(), None, "nothing after the failed check");
    }

    #[test]
    fn a_typed_mask_passes_falsely_and_the_next_key_ends_it() {
        let mut shell = Shell::confirmed(Echo::Program);
        let star = shell.key("*");
        shell.echo("*");
        shell.ack(star);
        let q = shell.key("q");
        assert_eq!(shell.drawn(), [(4, 'q')], "the documented B behaviour");
        shell.echo("*");
        shell.ack(q);
        assert_eq!(shell.p.overlay(), None);
        shell.key("r");
        assert_eq!(shell.p.overlay(), None);
    }

    #[test]
    fn hidden_in_flight_erases_what_was_drawn_and_stops_drawing() {
        for mode in [Echo::Kernel, Echo::Program] {
            let mut shell = Shell::confirmed(mode);
            shell.key("p");
            shell.key("w");
            assert_eq!(shell.drawn().len(), 2);
            shell.p.mode(Echo::Hidden);
            assert_eq!(shell.p.overlay(), None, "{mode:?}");
            shell.key("d");
            assert!(shell.p.epoch.guesses.is_empty(), "{mode:?}");
        }
    }

    #[test]
    fn unknown_mode_predicts_nothing() {
        let mut shell = Shell::new(Echo::Unknown);
        shell.key("a");
        assert!(shell.p.epoch.guesses.is_empty());
    }

    #[test]
    fn epoch_breaking_keys_start_a_tentative_epoch() {
        for key in ["\r", "\t", "\x03", "\x1b", "\x1b[A", "\x1b[200~x\x1b[201~"] {
            let mut shell = Shell::confirmed(Echo::Kernel);
            shell.key(key);
            assert!(!shell.p.epoch.confirmed, "{key:?}");
        }
    }

    #[test]
    fn a_moment_off_the_row_or_screen_within_one_output_ends_the_epoch() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.echo("\x1b[3;1Hstatus\x1b[1;4H");
        assert!(!shell.p.epoch.confirmed, "a cursor excursion");
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.echo("\x1b[?1049h\x1b[?1049l\x1b[1;4H");
        assert!(!shell.p.epoch.confirmed, "the alternate screen and back");
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.echo("\x1b[0m\x1b[K");
        assert!(shell.p.epoch.confirmed, "staying on the row");
    }

    #[test]
    fn leaving_the_row_or_switching_screens_ends_the_epoch() {
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.key("b");
        shell.echo("\x1b[2;1H");
        assert!(!shell.p.epoch.confirmed);
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.echo("\x1b[?1049h\x1b[1;4H");
        assert!(!shell.p.epoch.confirmed);
    }

    #[test]
    fn two_failed_checks_within_ten_keys_turn_predictions_off_until_enter() {
        let mut shell = Shell::new(Echo::Kernel);
        for _ in 0..2 {
            let x = shell.key("x");
            shell.echo("y");
            shell.ack(x);
        }
        assert!(shell.p.off);
        shell.key("a");
        assert!(shell.p.epoch.guesses.is_empty());
        shell.key("\r");
        shell.echo("\r\n$ ");
        shell.key("a");
        assert_eq!(shell.p.epoch.guesses.len(), 1);
    }

    #[test]
    fn failed_checks_far_apart_keep_predictions_on() {
        let mut shell = Shell::new(Echo::Kernel);
        let x = shell.key("x");
        shell.echo("y");
        shell.ack(x);
        for _ in 0..CONFIDENCE {
            shell.key("\t");
        }
        let x = shell.key("x");
        shell.echo("\x1b[1;4Hy");
        shell.ack(x);
        assert!(!shell.p.off);
    }

    #[test]
    fn nothing_is_predicted_past_the_margin_or_wider_than_a_cell() {
        let mut shell = Shell::new(Echo::Kernel);
        shell.echo("\x1b[1;10H");
        let last = shell.key("a");
        shell.echo("a");
        shell.ack(last);
        // The cursor waits at the margin: no column 10 to predict on.
        shell.key("b");
        assert!(shell.p.epoch.guesses.is_empty());
        let mut shell = Shell::confirmed(Echo::Kernel);
        shell.key("中");
        assert!(shell.p.epoch.guesses.is_empty());
    }

    #[test]
    fn the_gate_draws_above_30_ms_and_stops_below_20_ms() {
        let mut gate = Gate::default();
        let start = Instant::now();
        assert!(
            gate.due(start) && !gate.slow(),
            "off until the first sample"
        );
        let sample = |gate: &mut Gate, at: Duration, rtt: Duration| {
            gate.sent(7, start + at);
            assert!(gate.awaits(7) && !gate.due(start + at + SAMPLE));
            gate.answered(start + at + rtt);
        };
        sample(&mut gate, Duration::ZERO, Duration::from_millis(100));
        assert!(gate.slow());
        assert!(
            !gate.due(start + Duration::from_millis(500)),
            "one per second"
        );
        let mut at = Duration::ZERO;
        let mut slow = Vec::new();
        for _ in 0..30 {
            at += SAMPLE * 2;
            sample(&mut gate, at, Duration::from_millis(10));
            slow.push(gate.slow());
        }
        // 100 ms decays by 7/8 a sample: below 20 ms after twelve.
        assert!(slow[..10].iter().all(|s| *s) && !slow[29]);
        let mut gate = Gate::default();
        sample(&mut gate, Duration::ZERO, Duration::from_millis(25));
        assert!(!gate.slow(), "between the two: stays as it was");
    }
}
