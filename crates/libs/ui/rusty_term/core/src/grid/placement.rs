//! Headless Kitty image placements.
//!
//! The windowed front-end overlays pixel images ([`super::GridImage`]) on the
//! cells they cover, but only with the `gui` feature, and the overlay is not
//! part of a checkpoint. A mux server's grid has neither. A [`Placement`] is
//! the small always-on record of the same fact for images that live in the
//! Kitty store: stored image `image` covers `cols` x `rows` cells whose top
//! row is anchored at `serial` (see [`super::Grid`]'s `total_scrolled`), so it
//! scrolls with its text. Pixels stay in the store; a placement holds none.
//!
//! Sixel, iTerm2 and store-less Kitty images keep only their half-block
//! cells here, exactly as before.

use super::Grid;
use crate::checkpoint::{Error, Result, Writer, fields};

/// The most placements a grid keeps; the oldest is dropped first.
pub const MAX_PLACEMENTS: usize = 64;

/// A stored Kitty image placed on the grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Id of the image in the Kitty store.
    pub image: u32,
    /// Stable scroll position of the top cell row.
    pub serial: usize,
    /// Leftmost cell column.
    pub col: usize,
    /// Footprint in cells.
    pub cols: usize,
    pub rows: usize,
}

/// A placement as checkpointed: `top` is the top row relative to the live
/// screen's first row.
struct WirePlacement {
    image: u32,
    top: i32,
    col: usize,
    cols: usize,
    rows: usize,
}
fields!(WirePlacement, [image, top, col, cols, rows]);

/// Where `render_image_sized` put an image: the cells it covers.
pub(crate) struct Footprint {
    pub serial: usize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
}

impl Grid {
    /// The images placed on the grid, oldest first.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// `(grid id, stamp)` of the last change to a stored image or placement:
    /// equal across two looks only if neither changed between them, so a
    /// mux delta can skip images that did not move.
    pub fn image_stamp(&self) -> (u64, u64) {
        (self.id, self.image_stamp)
    }

    /// The viewport row of `p`'s top cell: negative when scrolled partly
    /// above the view, as [`Grid::image_top_row`].
    pub fn placement_top_row(&self, p: &Placement) -> isize {
        let off = self.view_offset.min(self.scrollback.len());
        p.serial as isize - self.total_scrolled as isize + off as isize
    }

    pub(super) fn touch_images(&mut self) {
        self.stamp += 1;
        self.image_stamp = self.stamp;
    }

    /// Record that stored image `image` now covers `cols` x `rows` cells from
    /// `(col, serial)`. Ignored if the image is not in the store.
    pub(crate) fn place_image(
        &mut self,
        image: u32,
        serial: usize,
        col: usize,
        cols: usize,
        rows: usize,
    ) {
        if !self.kitty_images.iter().any(|i| i.id == image) || cols == 0 || rows == 0 {
            return;
        }
        if self.placements.len() >= MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        self.placements.push(Placement {
            image,
            serial,
            col,
            cols,
            rows,
        });
        self.touch_images();
    }

    /// Drop placements whose image left the store, or whose rows left the
    /// retained scrollback; stamp if any went.
    pub(super) fn prune_placements(&mut self) {
        let oldest = self.total_scrolled.saturating_sub(self.scrollback.len());
        let before = self.placements.len();
        let store = &self.kitty_images;
        self.placements
            .retain(|p| p.serial + p.rows > oldest && store.iter().any(|i| i.id == p.image));
        if self.placements.len() != before {
            self.touch_images();
        }
    }

    /// Drop every placement (clear, resize, alternate screen, reset).
    pub(super) fn clear_placements(&mut self) {
        if !self.placements.is_empty() {
            self.placements.clear();
            self.touch_images();
        }
    }

    /// Drop placements whose rows overlap screen rows `top..=bottom`, for a
    /// scroll that moves cells without advancing the scroll count (a scroll
    /// region, the alternate screen, insert/delete lines). Their footprint
    /// can no longer be tracked, so they go rather than go stale.
    pub(super) fn drop_placements_over(&mut self, top: usize, bottom: usize) {
        if self.placements.is_empty() {
            return;
        }
        let base = self.total_scrolled as isize;
        let before = self.placements.len();
        self.placements.retain(|p| {
            let first = p.serial as isize - base;
            let last = first + p.rows as isize - 1;
            last < top as isize || first > bottom as isize
        });
        if self.placements.len() != before {
            self.touch_images();
        }
    }

    /// The optional trailing checkpoint section: written only when there are
    /// placements, so a checkpoint without any is byte-identical to one from
    /// before placements existed and still decodes. Rows are written relative
    /// to the live screen's top (negative = in scrollback), because the
    /// absolute `total_scrolled` count is not carried.
    pub(crate) fn write_placements(&self, w: &mut Writer) -> Result<()> {
        if self.placements.is_empty() {
            return Ok(());
        }
        let wire = self
            .placements
            .iter()
            .map(|p| {
                Ok(WirePlacement {
                    image: p.image,
                    top: i32::try_from(p.serial as i64 - self.total_scrolled as i64)
                        .map_err(|_| Error("placement row"))?,
                    col: p.col,
                    cols: p.cols,
                    rows: p.rows,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        crate::checkpoint::Wire::write(&wire, w)
    }

    /// Read the trailing section into a freshly decoded grid. Anchors the
    /// grid's scroll count at its retained history, or deeper if a placement
    /// reaches above it. Validation then drops nothing that still shows.
    pub(crate) fn read_placements(&mut self, r: &mut crate::checkpoint::Reader<'_>) -> Result<()> {
        // The count is checked before anything is allocated for the list.
        let n: usize = crate::checkpoint::Wire::read(r)?;
        if n > MAX_PLACEMENTS {
            return Err(Error("placement count"));
        }
        let mut wire = Vec::with_capacity(n);
        for _ in 0..n {
            wire.push(<WirePlacement as crate::checkpoint::Wire>::read(r)?);
        }
        // Anchor so every serial is non-negative: a placement whose top rows
        // were evicted (scrollback capped below its height) has a top above
        // the retained history yet still shows on screen.
        let deepest = wire.iter().map(|p| -i64::from(p.top)).max().unwrap_or(0);
        self.total_scrolled = self
            .scrollback
            .len()
            .max(usize::try_from(deepest).unwrap_or(0));
        let base = self.total_scrolled as i64;
        self.placements = wire
            .into_iter()
            .filter_map(|p| {
                Some(Placement {
                    image: p.image,
                    serial: usize::try_from(base + i64::from(p.top)).ok()?,
                    col: p.col,
                    cols: p.cols,
                    rows: p.rows,
                })
            })
            .collect();
        self.touch_images();
        Ok(())
    }

    /// Placements must name a stored image, stay within the cap, and have a
    /// sane footprint. Height is not capped by the current history: a tall
    /// placement made under a larger cap may be partly retained after it
    /// shrinks, and its bottom is already bounded by the screen.
    pub(crate) fn validate_placements(&self) -> Result<()> {
        let bad = self.placements.len() > MAX_PLACEMENTS
            || self.placements.iter().any(|p| {
                p.cols == 0
                    || p.rows == 0
                    || p.col >= self.cols
                    || p.col.saturating_add(p.cols) > self.cols
                    || p.serial.saturating_add(p.rows)
                        > self.total_scrolled.saturating_add(self.rows)
                    || !self.kitty_images.iter().any(|i| i.id == p.image)
            });
        if bad {
            return Err(Error("placement bounds"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{AnsiParser, Grid, checkpoint};

    /// `a=T` of a 1x2 image with id 7 at the cursor, 4 cells wide, then `X`.
    const PLACE: &[u8] = b"\x1b_Ga=T,f=32,s=1,v=2,i=7,c=4,r=2;/wAA//8AAP8=\x1b\\X";

    fn fed(cols: usize, rows: usize, data: &[u8]) -> (Grid, AnsiParser) {
        let (mut g, mut p) = (Grid::new(cols, rows), AnsiParser::new());
        p.advance(&mut g, data);
        (g, p)
    }

    #[test]
    fn a_stored_kitty_image_leaves_a_placement() {
        let (g, _) = fed(20, 10, PLACE);
        let [p] = g.placements() else {
            panic!("one placement, got {:?}", g.placements())
        };
        assert_eq!((p.image, p.col, p.serial), (7, 0, 0));
        assert!((1..=4).contains(&p.cols) && p.rows >= 1);
    }

    #[test]
    fn images_without_an_id_leave_only_cells() {
        let (g, _) = fed(20, 10, b"\x1b_Ga=T,f=32,s=1,v=1;/wAA/w==\x1b\\");
        assert!(g.placements().is_empty());
    }

    #[test]
    fn the_stamp_moves_only_when_images_change() {
        let (mut g, mut p) = fed(20, 10, b"");
        let before = g.image_stamp();
        p.advance(&mut g, b"plain text\r\n");
        assert_eq!(g.image_stamp(), before);
        p.advance(&mut g, PLACE);
        assert_ne!(g.image_stamp(), before);
        let placed = g.image_stamp();
        p.advance(&mut g, b"more text");
        assert_eq!(g.image_stamp(), placed);
    }

    #[test]
    fn a_placement_scrolls_with_its_text_and_drops_with_its_rows() {
        let (mut g, mut p) = fed(20, 5, PLACE);
        g.set_scrollback_max(8);
        let top = |g: &Grid| g.placement_top_row(&g.placements()[0]);
        assert_eq!(top(&g), 0);
        p.advance(&mut g, b"\r\n\r\n\r\n\r\n\r\n");
        assert!(top(&g) < 0, "scrolled into history");
        p.advance(&mut g, "\r\n".repeat(40).as_bytes());
        assert!(g.placements().is_empty(), "rows left the scrollback");
    }

    #[test]
    fn deleting_or_evicting_the_image_drops_its_placement() {
        let (mut g, mut p) = fed(20, 10, PLACE);
        p.advance(&mut g, b"\x1b_Ga=d,d=I,i=7\x1b\\");
        assert!(g.placements().is_empty());
        let (mut g, mut p) = fed(20, 10, PLACE);
        p.advance(&mut g, b"\x1b[2J");
        assert!(g.placements().is_empty());
    }

    #[test]
    fn placements_are_bounded() {
        let (mut g, mut p) = fed(20, 10, b"");
        for _ in 0..(super::MAX_PLACEMENTS + 5) {
            p.advance(&mut g, PLACE);
        }
        assert_eq!(g.placements().len(), super::MAX_PLACEMENTS);
    }

    #[test]
    fn a_full_checkpoint_keeps_a_placement_whose_top_row_was_evicted() {
        let (mut g, mut p) = fed(20, 5, b"");
        g.set_scrollback_max(0);
        p.advance(&mut g, PLACE);
        let mut cut = false;
        for _ in 0..8 {
            p.advance(&mut g, b"\n");
            cut = g
                .placements()
                .first()
                .is_some_and(|a| g.placement_top_row(a) < 0);
            if cut || g.placements().is_empty() {
                break;
            }
        }
        assert!(cut, "a half-evicted placement stays tracked");
        let bytes = checkpoint::encode_with_images(&g, &p).unwrap();
        let (back, _) = checkpoint::decode(&bytes).unwrap();
        assert_eq!(back.placements().len(), 1);
        let (a, b) = (&g.placements()[0], &back.placements()[0]);
        assert_eq!(g.placement_top_row(a), back.placement_top_row(b));
    }

    #[test]
    fn a_full_checkpoint_keeps_a_tall_placement_after_the_history_cap_shrinks() {
        let (mut g, p) = fed(
            20,
            5,
            b"\x1b_Ga=T,f=32,s=1,v=2,i=7,c=6,r=6;/wAA//8AAP8=\x1b\\",
        );
        assert_eq!(g.placements().len(), 1);
        g.set_scrollback_max(0);
        let bytes = checkpoint::encode_with_images(&g, &p).unwrap();
        let (back, _) = checkpoint::decode(&bytes).unwrap();
        assert_eq!(back.placements().len(), g.placements().len());
    }

    #[test]
    fn a_checkpoint_with_images_carries_placements() {
        let (g, p) = fed(20, 5, PLACE);
        let bytes = checkpoint::encode_with_images(&g, &p).unwrap();
        let (back, _) = checkpoint::decode(&bytes).unwrap();
        assert_eq!(back.placements(), g.placements());
        let (mut g, mut p) = fed(20, 5, PLACE);
        p.advance(&mut g, b"\r\n\r\n\r\n\r\n\r\n\r\n");
        let bytes = checkpoint::encode_with_images(&g, &p).unwrap();
        let (back, _) = checkpoint::decode(&bytes).unwrap();
        let (a, b) = (&g.placements()[0], &back.placements()[0]);
        assert_eq!(g.placement_top_row(a), back.placement_top_row(b));
        let (g, p) = fed(20, 5, b"hi");
        let plain = checkpoint::encode_with_images(&g, &p).unwrap();
        assert_eq!(plain, checkpoint::encode(&g, &p).unwrap());
    }

    /// A v6 reader without the trailing section rejects bytes after the
    /// parser, so the default encoders must write none: the checkpoint of a
    /// grid with a placement is byte-identical to one of the same grid
    /// without it, and the half-block cells survive.
    #[test]
    fn the_default_encoders_stay_readable_by_older_v6_readers() {
        let (mut g, p) = fed(20, 5, PLACE);
        assert!(!g.placements().is_empty());
        let legacy = checkpoint::encode(&g, &p).unwrap();
        let split = checkpoint::encode_split(&g, &p, 1 << 20).unwrap().0;
        assert_ne!(legacy, checkpoint::encode_with_images(&g, &p).unwrap());
        g.placements.clear();
        assert_eq!(legacy, checkpoint::encode(&g, &p).unwrap());
        assert_eq!(split, checkpoint::encode_split(&g, &p, 1 << 20).unwrap().0);
        let (back, _) = checkpoint::decode(&legacy).unwrap();
        assert!(back.placements().is_empty());
    }

    #[test]
    fn a_hostile_trailing_section_is_rejected() {
        let (g, p) = fed(20, 5, PLACE);
        let mut bytes = checkpoint::encode_with_images(&g, &p).unwrap();
        // The last field written is the final placement's `rows`: blow it up.
        let n = bytes.len();
        bytes[n - 8..].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(checkpoint::decode(&bytes).is_err());
        // A placement whose image is not in the store.
        let mut bad = Grid::new(20, 5);
        bad.placements = g.placements().to_vec();
        assert!(bad.validate_placements().is_err());
    }

    #[test]
    fn placements_leave_when_history_is_removed_or_cells_move_untracked() {
        // History removed under a placement that already scrolled into it.
        let (mut g, mut p) = fed(20, 5, PLACE);
        p.advance(&mut g, b"\r\n\r\n\r\n\r\n\r\n\r\n\r\n");
        assert_eq!(g.placements().len(), 1);
        g.set_scrollback_max(0);
        assert!(g.placements().is_empty(), "history trimmed away");
        let (mut g, mut p) = fed(20, 5, PLACE);
        p.advance(&mut g, b"\r\n\r\n\r\n\r\n\r\n\r\n\r\n");
        p.advance(&mut g, b"\x1b[3J");
        assert!(g.placements().is_empty(), "ED 3");
        // A scroll region moves cells without history: the placement goes.
        let (mut g, mut p) = fed(20, 8, b"\x1b[1;4r");
        p.advance(&mut g, PLACE);
        assert_eq!(g.placements().len(), 1);
        p.advance(&mut g, b"\x1b[4;1H\n");
        assert!(g.placements().is_empty(), "DECSTBM scroll");
        // Insert line under it.
        let (mut g, mut p) = fed(20, 8, PLACE);
        p.advance(&mut g, b"\x1b[1;1H\x1b[L");
        assert!(g.placements().is_empty(), "IL");
        // Rows below the scrolled region are untouched.
        let (mut g, mut p) = fed(20, 8, b"\x1b[6;8r\x1b[1;1H");
        p.advance(&mut g, PLACE);
        p.advance(&mut g, b"\x1b[8;1H\n");
        assert_eq!(g.placements().len(), 1);
    }

    #[test]
    fn leaving_the_alternate_screen_drops_its_placements() {
        let (mut g, mut p) = fed(20, 5, b"\x1b[?1049h");
        p.advance(&mut g, PLACE);
        assert_eq!(g.placements().len(), 1);
        p.advance(&mut g, b"\x1b[?1049l");
        assert!(g.placements().is_empty());
    }

    #[test]
    fn every_store_mutation_moves_the_stamp() {
        let (mut g, mut p) = fed(20, 5, b"");
        let mut last = g.image_stamp();
        let mut step = |g: &Grid, what: &str| {
            let now = g.image_stamp();
            assert_ne!(now, last, "{what}");
            last = now;
        };
        p.advance(&mut g, b"\x1b_Ga=t,f=32,s=1,v=1,i=9;/wAA/w==\x1b\\");
        step(&g, "a=t");
        p.advance(&mut g, b"\x1b_Ga=f,f=32,s=1,v=1,i=9;/wAA/w==\x1b\\");
        step(&g, "a=f frame");
        p.advance(&mut g, b"\x1b_Ga=a,i=9,s=3\x1b\\");
        step(&g, "a=a");
        g.advance_animations(std::time::Instant::now());
        g.advance_animations(std::time::Instant::now() + std::time::Duration::from_secs(1));
        step(&g, "frame advance");
        p.advance(&mut g, b"\x1bc");
        step(&g, "RIS empties a transmit-only store");
    }

    #[test]
    fn an_image_that_scrolls_without_history_leaves_no_placement() {
        // Alternate screen: drawn at the bottom, the screen scrolls under it.
        let (mut g, mut p) = fed(20, 5, b"\x1b[?1049h\x1b[5;1H");
        p.advance(&mut g, PLACE);
        assert!(g.placements().is_empty(), "alt screen");
        // A scroll region: the same.
        let (mut g, mut p) = fed(20, 8, b"\x1b[1;4r\x1b[4;1H");
        p.advance(&mut g, PLACE);
        assert!(g.placements().is_empty(), "region");
        // Primary screen at the bottom scrolls into history: kept, anchored.
        let (mut g, mut p) = fed(20, 5, b"\x1b[5;1H");
        p.advance(&mut g, PLACE);
        assert_eq!(g.placements().len(), 1);
        assert!(g.placement_top_row(&g.placements()[0]) < 5);
    }

    #[test]
    fn hostile_counts_and_rects_are_rejected_on_decode() {
        let (g, p) = fed(20, 5, PLACE);
        let ok = checkpoint::encode_with_images(&g, &p).unwrap();
        // The count is the first u64 of the section; the section is the tail.
        let section = 8 + 4 + 4 + 8 * 4; // count + image + top + col, cols, rows (u64 each)
        let at = ok.len() - section;
        let mut huge = ok.clone();
        huge[at..at + 8].copy_from_slice(&(u64::MAX >> 1).to_le_bytes());
        assert!(checkpoint::decode(&huge).is_err(), "count");
        // `top` one row past the screen.
        let mut low = ok.clone();
        low[at + 12..at + 16].copy_from_slice(&5i32.to_le_bytes());
        assert!(checkpoint::decode(&low).is_err(), "top past the screen");
        // A rect hanging below the screen.
        let mut tall = ok.clone();
        tall[ok.len() - 8..].copy_from_slice(&3u64.to_le_bytes());
        tall[at + 12..at + 16].copy_from_slice(&3i32.to_le_bytes());
        assert!(checkpoint::decode(&tall).is_err(), "hangs below");
    }
}
