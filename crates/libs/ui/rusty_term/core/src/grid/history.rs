//! Newest-first history (mux C03): a grid's history split into batches that
//! [`crate::checkpoint::encode_split`] sends after a small checkpoint, and
//! [`Grid::prepend_history`], which adds each batch above the history a
//! grid already holds.

use super::{Grid, Line, LineAttr, PROMPT_MARKS_MAX, reflow_history, trim_padding};
use crate::checkpoint::{self, Batch, BatchWriter, Error, Result};
use crate::color::{self, Theme};
use crate::style::{StyleId, rethemed};
use std::collections::VecDeque;

impl Grid {
    /// The first scrollback row of the logical line on the top screen row.
    /// The rows before it can leave a checkpoint for batches; the rows from
    /// it on wrap into the screen, so they stay with it.
    pub(crate) fn history_split(&self) -> usize {
        let mut split = self.scrollback.len();
        while split > 0 && self.scrollback[split - 1].wrapped {
            split -= 1;
        }
        split
    }

    /// Scrollback rows `..split` as batches, newest first. Each holds whole
    /// logical lines and is closed once it reaches `batch_bytes`.
    pub(crate) fn history_batches(&self, split: usize, batch_bytes: usize) -> Result<Vec<Vec<u8>>> {
        let mut marks = self
            .prompt_marks
            .iter()
            .rev()
            .copied()
            .filter(|&l| l < split)
            .peekable();
        let mut batches = Vec::new();
        let mut open: Option<BatchWriter<'_>> = None;
        for row in (0..split.min(self.scrollback.len())).rev() {
            let batch = match &mut open {
                Some(batch) => batch,
                None => open.insert(BatchWriter::new(&self.styles, self.cols)?),
            };
            let mark = marks.next_if_eq(&row).is_some();
            batch.row(&self.scrollback[row], mark)?;
            let line_starts = row == 0 || !self.scrollback[row - 1].wrapped;
            if line_starts && batch.len() >= batch_bytes {
                batches.extend(open.take().map(BatchWriter::finish));
            }
        }
        batches.extend(open.map(BatchWriter::finish));
        Ok(batches)
    }

    /// Add one batch from [`checkpoint::encode_split`] above this grid's
    /// history, rewrapped from the width it was captured at to this grid's,
    /// as a resize would have rewrapped it. Only rows that fit under the
    /// scrollback cap are kept, newest first, so a full history ignores the
    /// rest. What is on screen does not move: the scroll offset counts from
    /// the bottom, and every history index (prompt marks, search matches,
    /// folds) shifts by the rows added. Returns how many were added.
    ///
    /// `theme`, `(from, to)`, recolours the batch as [`Grid::retheme`]
    /// would: pass the pair this grid's checkpoint was rethemed with.
    ///
    /// The batch is untrusted: one naming a cluster or hyperlink this grid
    /// does not hold is refused, and the grid is unchanged.
    pub fn prepend_history(
        &mut self,
        batch: &[u8],
        theme: Option<(&Theme, &Theme)>,
    ) -> Result<usize> {
        let room = self.scrollback_max.saturating_sub(self.scrollback.len());
        if room == 0 {
            return Ok(0);
        }
        let mut decoded = checkpoint::decode_batch(batch, self.clusters.len(), self.links.len())?;
        check_batch(&decoded)?;
        self.adopt_styles(&mut decoded, theme)?;
        let (mut rows, mut marks) = self.rewrap(decoded, room)?;
        let drop = rows.len().saturating_sub(room);
        rows.drain(..drop);
        marks.retain(|&l| l >= drop);
        marks.iter_mut().for_each(|l| *l -= drop);
        let added = rows.len();
        for mut line in rows.into_iter().rev() {
            line.attr = LineAttr::Single;
            trim_padding(&mut line, &self.styles);
            self.scrollback.push_front(line);
        }
        self.shift_history(added, marks);
        self.touch_hist();
        Ok(added)
    }

    /// Moves `batch`'s cells onto this grid's style table, recoloured
    /// through `theme`. Each batch style is interned once.
    fn adopt_styles(&mut self, batch: &mut Batch, theme: Option<(&Theme, &Theme)>) -> Result<()> {
        let mut ids: Vec<Option<StyleId>> = vec![None; batch.styles.len()];
        for (line, _) in &mut batch.rows {
            for cell in line.cells_restyle() {
                let from = cell.style();
                let slot = ids.get_mut(from.0 as usize).ok_or(Error("style index"))?;
                let id = *slot.get_or_insert_with(|| {
                    let style = batch.styles.get(from);
                    let style = match theme {
                        Some((old, new)) => {
                            rethemed(style, &|c| color::remap(c, old, new), &|n, c| {
                                color::remap_entry(n, c, old, new)
                            })
                        }
                        None => style,
                    };
                    self.styles.intern(style)
                });
                *cell = cell.with_style(id);
            }
        }
        Ok(())
    }

    /// A batch's rows at this grid's width, oldest first, and the indexes
    /// of the rows with a prompt mark. Keeps at most `room` of the newest.
    fn rewrap(&mut self, batch: Batch, room: usize) -> Result<(VecDeque<Line>, Vec<usize>)> {
        let marks = batch
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, (_, mark))| mark.then_some(i))
            .collect::<Vec<_>>();
        let rows: VecDeque<Line> = batch.rows.into_iter().map(|(line, _)| line).collect();
        if batch.cols == self.cols {
            return Ok((rows, marks));
        }
        // Rewrap above one blank screen row holding the cursor, so every
        // batch row stays history.
        let blank = self.default_packed();
        let out = reflow_history(
            rows,
            vec![Line::blank(batch.cols, blank)],
            batch.cols,
            (0, 0),
            &marks,
            None,
            None,
            &[],
            self.cols,
            1,
            blank,
            room,
            &self.styles,
        );
        Ok((out.scrollback, out.prompt_marks))
    }

    /// `added` rows went in front of the history: move every history index
    /// past them, and put the batch's own `marks` first.
    fn shift_history(&mut self, added: usize, mut marks: Vec<usize>) {
        marks.extend(self.prompt_marks.iter().map(|&l| l + added));
        let over = marks.len().saturating_sub(PROMPT_MARKS_MAX);
        marks.drain(..over);
        self.prompt_marks = marks;
        if let Some(sel) = &mut self.selection {
            sel.anchor.1 += added;
            sel.head.1 += added;
        }
        #[cfg(feature = "l13")]
        if let Some(s) = &mut self.command_start {
            *s += added;
        }
        #[cfg(any(test, feature = "gui"))]
        {
            if let Some(s) = &mut self.fold_pending_start {
                *s += added;
            }
            for b in &mut self.fold_blocks {
                b.start += added;
                b.end += added;
            }
            if let Some(search) = &mut self.search {
                search.shift(added);
            }
        }
    }
}

/// Refuses a batch with a row wider than its width or a cell naming a
/// style it does not carry, before anything is added to the grid.
fn check_batch(batch: &Batch) -> Result<()> {
    let styles = batch.styles.len();
    for (line, _) in &batch.rows {
        if line.width() > batch.cols {
            return Err(Error("history batch row width"));
        }
        if line.cells().iter().any(|c| c.style().0 as usize >= styles) {
            return Err(Error("style index"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::checkpoint::{decode, encode, encode_split};
    use crate::{AnsiParser, Grid};

    /// A grid `cols` wide whose history holds `lines` numbered lines, every
    /// fifth long enough to wrap, a prompt mark on every seventh, and a
    /// coloured, hyperlinked and combining-mark line.
    fn sample(cols: usize, lines: usize) -> (Grid, AnsiParser) {
        let mut grid = Grid::new(cols, 3);
        let mut parser = AnsiParser::new();
        for i in 0..lines {
            if i % 7 == 0 {
                parser.advance(&mut grid, b"\x1b]133;A\x07");
            }
            let line = if i % 5 == 0 {
                format!("{i} {}\r\n", "long ".repeat(9))
            } else {
                format!("\x1b[3{}m{i}\x1b[0m line\r\n", i % 8)
            };
            parser.advance(&mut grid, line.as_bytes());
        }
        parser.advance(
            &mut grid,
            b"\x1b]8;;https://x.test\x1b\\link\x1b]8;;\x1b\\ e\xcc\x81\r\n",
        );
        parser.advance(&mut grid, b"top ");
        parser.advance(&mut grid, "wrapped ".repeat(9).as_bytes());
        (grid, parser)
    }

    /// History row `i` of `g` as text, trailing blanks dropped.
    fn row(g: &Grid, i: usize) -> String {
        let text: String = g.scrollback[i].cells().iter().map(|c| c.ch()).collect();
        text.trim_end().to_string()
    }

    /// The grid a client builds: the checkpoint, then every batch in order.
    fn rebuilt(grid: &Grid, parser: &AnsiParser, batch_bytes: usize) -> (Grid, AnsiParser, usize) {
        let (head, batches) = encode_split(grid, parser, batch_bytes).unwrap();
        let (mut back, parser) = decode(&head).unwrap();
        for batch in &batches {
            back.prepend_history(batch, None).unwrap();
        }
        (back, parser, batches.len())
    }

    #[test]
    fn checkpoint_then_batches_rebuild_the_whole_grid() {
        let (grid, parser) = sample(20, 120);
        for batch_bytes in [1, 200, 1 << 16] {
            let (back, parser_back, batches) = rebuilt(&grid, &parser, batch_bytes);
            assert!(batches >= 1);
            assert_eq!(back.scrollback_len(), grid.scrollback_len());
            assert_eq!(back.prompt_marks, grid.prompt_marks, "{batch_bytes}");
            assert_eq!(
                encode(&back, &parser_back).unwrap(),
                encode(&grid, &parser).unwrap(),
                "{batch_bytes}"
            );
        }
    }

    #[test]
    fn the_checkpoint_keeps_the_top_line_whole_and_batches_end_on_lines() {
        let (grid, parser) = sample(20, 30);
        let split = grid.history_split();
        assert!(split < grid.scrollback_len(), "top line wraps from history");
        assert!(!grid.scrollback[split - 1].wrapped);
        let (head, batches) = encode_split(&grid, &parser, 1).unwrap();
        let (back, _) = decode(&head).unwrap();
        assert_eq!(back.scrollback_len(), grid.scrollback_len() - split);
        // One batch per logical line at a one-byte budget.
        let starts = (0..split)
            .filter(|&i| i == 0 || !grid.scrollback[i - 1].wrapped)
            .count();
        assert_eq!(batches.len(), starts);
    }

    #[test]
    fn a_batch_rewraps_like_a_resize() {
        let (grid, parser) = sample(20, 60);
        let (head, batches) = encode_split(&grid, &parser, 300).unwrap();
        let mut whole = decode(&encode(&grid, &parser).unwrap()).unwrap().0;
        let mut back = decode(&head).unwrap().0;
        // A resize before any batch, and one between batches.
        whole.resize(13, 3);
        back.resize(13, 3);
        let (first, rest) = batches.split_first().unwrap();
        back.prepend_history(first, None).unwrap();
        whole.resize(31, 3);
        back.resize(31, 3);
        for batch in rest {
            back.prepend_history(batch, None).unwrap();
        }
        let text =
            |g: &Grid| -> Vec<String> { (0..g.scrollback_len()).map(|i| row(g, i)).collect() };
        assert_eq!(text(&back), text(&whole));
        assert_eq!(back.prompt_marks, whole.prompt_marks);
    }

    #[test]
    fn a_full_history_keeps_the_newest_rows_and_the_view_stays_put() {
        let (grid, parser) = sample(20, 80);
        let (head, batches) = encode_split(&grid, &parser, 1 << 16).unwrap();
        let mut back = decode(&head).unwrap().0;
        let kept = back.scrollback_len();
        back.set_scrollback_max(kept + 5);
        back.view_offset = 1;
        let top = row(&back, kept - 1);
        assert_eq!(back.prepend_history(&batches[0], None).unwrap(), 5);
        assert_eq!(back.prepend_history(&batches[0], None).unwrap(), 0);
        assert_eq!(back.view_offset, 1);
        assert_eq!(row(&back, back.scrollback_len() - 1), top);
        let newest = grid.history_split() - 1;
        assert_eq!(row(&back, 4), row(&grid, newest));
    }

    #[test]
    fn a_bad_batch_is_refused_and_changes_nothing() {
        let (grid, parser) = sample(20, 40);
        let (head, batches) = encode_split(&grid, &parser, 1 << 16).unwrap();
        // A grid without the checkpoint's hyperlink and cluster tables.
        let mut bare = Grid::new(20, 4);
        assert!(bare.prepend_history(&batches[0], None).is_err());
        assert_eq!(bare.scrollback_len(), 0);
        let mut back = decode(&head).unwrap().0;
        let before = back.scrollback_len();
        let styles = back.styles.len();
        let batch = &batches[0];
        // Declared one column wide, so every row is too wide for it.
        let mut narrow = batch.clone();
        assert_eq!(narrow[5], 20, "the width byte");
        narrow[5] = 1;
        for bad in [
            &b"RTHB\x02"[..],
            &batch[..batch.len() - 1],
            b"RTHB\x01\x00",
            &narrow,
        ] {
            assert!(back.prepend_history(bad, None).is_err(), "{bad:?}");
        }
        assert_eq!(back.scrollback_len(), before);
        assert_eq!(back.styles.len(), styles, "no style was added");
        assert!(back.prepend_history(&batches[0], None).is_ok());
    }

    #[test]
    fn a_selection_stays_on_its_text() {
        let (grid, parser) = sample(20, 40);
        let (head, batches) = encode_split(&grid, &parser, 1 << 16).unwrap();
        let mut back = decode(&head).unwrap().0;
        let row = back.scrollback_len();
        back.selection = Some(crate::grid::Selection {
            anchor: (0, row),
            head: (5, row),
        });
        let before = back.selected_text();
        let added = back.prepend_history(&batches[0], None).unwrap();
        assert!(added > 0);
        assert_eq!(back.selected_text(), before);
    }

    #[test]
    fn a_batch_takes_the_theme_its_checkpoint_was_given() {
        let (grid, parser) = sample(20, 40);
        let ours = crate::Theme {
            fg: 0x101010,
            bg: 0xf0f0f0,
            cursor: 0x101010,
            palette16: [0x123456; 16],
        };
        let rethemed = |mut g: Grid, mut p: AnsiParser| {
            let theirs = p.retheme(ours);
            g.retheme(&theirs, &ours);
            (g, p, theirs)
        };
        let (whole, whole_parser) = decode(&encode(&grid, &parser).unwrap()).unwrap();
        let (whole, whole_parser, _) = rethemed(whole, whole_parser);
        let (head, batches) = encode_split(&grid, &parser, 200).unwrap();
        let (back, back_parser) = decode(&head).unwrap();
        let (mut back, back_parser, theirs) = rethemed(back, back_parser);
        for batch in &batches {
            back.prepend_history(batch, Some((&theirs, &ours))).unwrap();
        }
        assert_eq!(
            encode(&back, &back_parser).unwrap(),
            encode(&whole, &whole_parser).unwrap()
        );
    }
}
