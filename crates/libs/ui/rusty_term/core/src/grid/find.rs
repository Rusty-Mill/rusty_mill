//! Text search and prompt marks over a grid's whole history, for headless
//! readers (the mux server's `terminal.find` and `terminal.prompts`, the
//! terminal client's find bar). The windowed renderers keep their own
//! highlighting search (`Grid::search_with`) behind the `gui` feature.

use super::Grid;
use crate::cell::{Cell, WIDE_TRAILER};

/// One match of [`Grid::find_text`]: the absolute row (0 is the oldest
/// scrollback row, as for [`Grid::line_text`]), the first column and how
/// many columns it spans on that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextHit {
    pub abs: usize,
    pub col: usize,
    pub len: usize,
}

/// Simple one-char case fold; full folding would change lengths and so the
/// char-to-column mapping.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_trailer(cell: &Cell) -> bool {
    cell.flags & WIDE_TRAILER != 0
}

impl Grid {
    /// The absolute rows (counted as for [`Grid::line_text`]) where a shell
    /// prompt started (OSC 133 or 633 `A`), oldest first. Those that scrolled
    /// out of the retained history are gone.
    pub fn prompt_marks(&self) -> &[usize] {
        &self.prompt_marks
    }

    /// Every match of `query` in the history and on the screen, oldest
    /// first, case-insensitively, at most `max`. A logical line is the rows
    /// joined where the program wrapped, so a match may cross a wrap; it is
    /// reported as its part on the first row. An empty query finds nothing.
    pub fn find_text(&self, query: &str, max: usize) -> Vec<TextHit> {
        let q: Vec<char> = query.chars().map(fold).collect();
        let mut hits = Vec::new();
        if q.is_empty() || max == 0 {
            return hits;
        }
        let (mut text, mut at): (Vec<char>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
        let mut abs = 0;
        while let Some((cells, wrapped)) = self.line_cells(abs) {
            for (col, cell) in cells.iter().enumerate().filter(|(_, c)| !is_trailer(c)) {
                text.push(fold(cell.ch));
                at.push((abs, col));
            }
            abs += 1;
            if wrapped {
                continue;
            }
            scan(&text, &at, &q, &mut hits, max);
            text.clear();
            at.clear();
            if hits.len() >= max {
                return hits;
            }
        }
        scan(&text, &at, &q, &mut hits, max);
        hits
    }
}

/// The non-overlapping matches of `q` in one logical line, trailing blanks
/// ignored, added to `hits` up to `max` in all.
fn scan(text: &[char], at: &[(usize, usize)], q: &[char], hits: &mut Vec<TextHit>, max: usize) {
    let end = text.iter().rposition(|&c| c != ' ').map_or(0, |i| i + 1);
    let mut i = 0;
    while i + q.len() <= end && hits.len() < max {
        if text[i..i + q.len()] != *q {
            i += 1;
            continue;
        }
        let (abs, col) = at[i];
        let len = at[i..i + q.len()].iter().take_while(|p| p.0 == abs).count();
        hits.push(TextHit { abs, col, len });
        i += q.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnsiParser;

    fn grid(cols: usize, rows: usize, input: &str) -> Grid {
        let mut grid = Grid::new(cols, rows);
        AnsiParser::new().advance(&mut grid, input.as_bytes());
        grid
    }

    #[test]
    fn finds_in_history_and_on_screen_ignoring_case_up_to_max() {
        let g = grid(8, 2, "Foo one\r\nfoo two\r\nbar FOO\r\nzzz");
        let hits = g.find_text("foo", 10);
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0],
            TextHit {
                abs: 0,
                col: 0,
                len: 3
            }
        );
        assert_eq!(hits[2].col, 4);
        assert_eq!(g.find_text("foo", 2).len(), 2);
        assert!(g.find_text("", 10).is_empty());
        assert!(g.find_text("foo", 0).is_empty());
        assert!(g.find_text("nope", 10).is_empty());
    }

    #[test]
    fn a_match_across_a_wrap_is_reported_on_its_first_row() {
        let g = grid(4, 3, "abcdefgh");
        assert_eq!(
            g.find_text("cdef", 10),
            [TextHit {
                abs: 0,
                col: 2,
                len: 2
            }]
        );
    }

    #[test]
    fn prompt_marks_are_the_rows_where_a_prompt_started() {
        let g = grid(
            8,
            2,
            "\x1b]133;A\x07one\r\n\x1b]133;A\x07two\r\n\x1b]133;A\x07tri",
        );
        assert_eq!(g.prompt_marks(), [0, 1, 2]);
    }
}
