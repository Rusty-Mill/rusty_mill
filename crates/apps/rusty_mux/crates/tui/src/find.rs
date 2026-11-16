//! Find in a block's scrollback: Ctrl-] `/`, then type; Enter or Ctrl-N is
//! the next match, Ctrl-P the previous, Esc leaves. The search runs over the
//! client's own copy of the block (history it holds plus the screen), as the
//! GUI's find bar does, case-insensitive and across soft-wrapped rows.

use rusty_term_core::{Cell, Grid, WIDE_TRAILER};

/// Matches kept; more are not looked for.
const MAX_HITS: usize = 10_000;

/// One match: absolute row (0 = oldest history row), first column, columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub abs: usize,
    pub col: usize,
    pub len: usize,
}

/// What a read of keyboard bytes did to the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The bar stays open.
    Open,
    /// Leave find.
    Leave,
}

/// An open find bar: the query, its matches and the active one.
#[derive(Debug, Default)]
pub struct Find {
    query: String,
    hits: Vec<Hit>,
    current: usize,
    /// The start of a character a read cut short.
    tail: Vec<u8>,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Every match of `query` in `grid`, oldest first. A logical line is the
/// rows joined where the program wrapped, so a match may cross a wrap.
pub fn search(grid: &Grid, query: &str) -> Vec<Hit> {
    let q: Vec<char> = query.chars().map(fold).collect();
    let mut hits = Vec::new();
    if q.is_empty() {
        return hits;
    }
    let (mut text, mut at): (Vec<char>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
    let mut abs = 0;
    while let Some((cells, wrapped)) = grid.line_cells(abs) {
        for (col, cell) in cells.iter().enumerate().filter(|(_, c)| !is_trailer(c)) {
            text.push(fold(cell.ch));
            at.push((abs, col));
        }
        abs += 1;
        if wrapped {
            continue;
        }
        scan(&text, &at, &q, &mut hits);
        text.clear();
        at.clear();
        if hits.len() >= MAX_HITS {
            return hits;
        }
    }
    scan(&text, &at, &q, &mut hits);
    hits
}

fn is_trailer(cell: &Cell) -> bool {
    cell.flags & WIDE_TRAILER != 0
}

/// The non-overlapping matches of `q` in one logical line. A match that
/// crosses a wrap is kept as its part on the first row.
fn scan(text: &[char], at: &[(usize, usize)], q: &[char], hits: &mut Vec<Hit>) {
    let end = text.iter().rposition(|&c| c != ' ').map_or(0, |i| i + 1);
    let mut i = 0;
    while i + q.len() <= end {
        if text[i..i + q.len()] != *q {
            i += 1;
            continue;
        }
        let (abs, col) = at[i];
        let len = at[i..i + q.len()].iter().take_while(|p| p.0 == abs).count();
        hits.push(Hit { abs, col, len });
        i += q.len();
    }
}

impl Find {
    /// The query so far.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The active match's number (from 1) and how many there are.
    pub fn position(&self) -> Option<(usize, usize)> {
        (!self.hits.is_empty()).then_some((self.current + 1, self.hits.len()))
    }

    /// The matches.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The active match.
    pub fn active(&self) -> Option<Hit> {
        self.hits.get(self.current).copied()
    }

    /// Takes keyboard `bytes` while the bar is open; scrolls `grid` to the
    /// active match. Keys that mean nothing here are dropped, so they never
    /// reach the block.
    pub fn keys(&mut self, grid: &mut Grid, bytes: &[u8]) -> Step {
        if bytes == [0x1b] {
            return Step::Leave;
        }
        // A longer escape (an arrow key, say) is no text: drop it whole.
        if bytes.first() == Some(&0x1b) {
            return Step::Open;
        }
        let mut typed = Vec::new();
        for &b in bytes {
            match b {
                b'\r' | b'\n' | 0x0e => self.step(true),
                0x10 => self.step(false),
                0x08 | 0x7f => {
                    self.query.pop();
                    self.refresh(grid);
                }
                b if b >= 0x20 && b != 0x7f => typed.push(b),
                _ => {}
            }
        }
        if !typed.is_empty() {
            self.tail.extend(typed);
            let whole = match std::str::from_utf8(&self.tail) {
                Ok(_) => self.tail.len(),
                // An incomplete character at the end waits for its rest.
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => self.tail.len(),
            };
            let rest = self.tail.split_off(whole);
            let text =
                String::from_utf8_lossy(&std::mem::replace(&mut self.tail, rest)).into_owned();
            self.query.push_str(&text);
            self.refresh(grid);
        }
        self.show(grid);
        Step::Open
    }

    fn step(&mut self, forward: bool) {
        let n = self.hits.len();
        if n > 0 {
            self.current = (self.current + if forward { 1 } else { n - 1 }) % n;
        }
    }

    fn refresh(&mut self, grid: &Grid) {
        self.hits = search(grid, &self.query);
        self.current = 0;
    }

    /// Searches again, keeping the active match's place, for output that
    /// arrived since: on a full history rows drop off the top and every hit
    /// moves. Call before drawing.
    pub fn follow(&mut self, grid: &Grid) {
        if self.query.is_empty() {
            return;
        }
        let at = self.current;
        self.refresh(grid);
        self.current = at.min(self.hits.len().saturating_sub(1));
    }

    /// Scrolls `grid` so the active match is a third of the way down.
    fn show(&self, grid: &mut Grid) {
        let Some(hit) = self.active() else {
            return;
        };
        let history = grid.scrollback_len();
        let want = (history + grid.rows / 3).saturating_sub(hit.abs);
        let target = want.min(history);
        match target.cmp(&grid.view_offset) {
            std::cmp::Ordering::Greater => grid.scroll_view_up(target - grid.view_offset),
            std::cmp::Ordering::Less => grid.scroll_view_down(grid.view_offset - target),
            std::cmp::Ordering::Equal => false,
        };
    }
}

#[cfg(test)]
mod tests {
    use rusty_term_core::AnsiParser;

    use super::*;

    fn grid(cols: usize, rows: usize, input: &str) -> Grid {
        let mut grid = Grid::new(cols, rows);
        AnsiParser::new().advance(&mut grid, input.as_bytes());
        grid
    }

    #[test]
    fn finds_every_match_in_history_and_on_screen_ignoring_case() {
        let g = grid(8, 2, "Foo one\r\nfoo two\r\nbar FOO\r\nzzz");
        let hits = search(&g, "foo");
        assert_eq!(hits.len(), 3);
        assert_eq!((hits[0].abs, hits[0].col, hits[0].len), (0, 0, 3));
        assert_eq!(hits[2].col, 4);
        assert!(search(&g, "").is_empty());
        assert!(search(&g, "nope").is_empty());
    }

    #[test]
    fn a_match_across_a_wrap_is_found_once() {
        let g = grid(4, 3, "abcdefgh");
        let hits = search(&g, "cdef");
        assert_eq!(
            hits,
            [Hit {
                abs: 0,
                col: 2,
                len: 2
            }]
        );
    }

    #[test]
    fn typing_scrolls_to_the_match_and_enter_steps() {
        let mut input = String::from("needle\r\n");
        input.push_str(&"x\r\n".repeat(30));
        input.push_str("needle");
        let mut g = grid(10, 5, &input);
        let mut find = Find::default();
        assert_eq!(find.keys(&mut g, b"needle"), Step::Open);
        assert_eq!(find.position(), Some((1, 2)));
        assert!(g.view_offset > 0, "scrolled up to the old match");
        find.keys(&mut g, b"\r");
        assert_eq!(find.position(), Some((2, 2)));
        find.keys(&mut g, &[0x10]);
        assert_eq!(find.position(), Some((1, 2)));
        find.keys(&mut g, &[0x7f]);
        assert_eq!(find.query(), "needl");
        assert_eq!(find.keys(&mut g, b"\x1b"), Step::Leave);
    }

    #[test]
    fn a_character_split_across_reads_is_typed_whole() {
        let mut g = grid(10, 2, "caf\u{e9}");
        let mut find = Find::default();
        let bytes = "\u{e9}".as_bytes();
        find.keys(&mut g, &[b'c', b'a', b'f', bytes[0]]);
        assert_eq!(find.query(), "caf");
        find.keys(&mut g, &bytes[1..]);
        assert_eq!(find.query(), "caf\u{e9}");
        assert_eq!(find.position(), Some((1, 1)));
    }

    #[test]
    fn hits_follow_a_history_that_moved() {
        let mut g = grid(10, 2, "");
        g.set_scrollback_max(3);
        AnsiParser::new().advance(&mut g, b"a\r\nb\r\nc\r\nfoo");
        let mut find = Find::default();
        find.keys(&mut g, b"foo");
        let before = find.active().unwrap().abs;
        // History is full: a row drops off the top for each new one.
        AnsiParser::new().advance(&mut g, b"\r\nx\r\ny");
        find.follow(&g);
        assert_eq!(find.active().unwrap().abs, before - 1);
        assert_eq!(find.position(), Some((1, 1)));
    }

    #[test]
    fn an_arrow_key_is_dropped_not_typed() {
        let mut g = grid(10, 2, "abc");
        let mut find = Find::default();
        assert_eq!(find.keys(&mut g, b"\x1b[A"), Step::Open);
        assert_eq!(find.query(), "");
    }
}
