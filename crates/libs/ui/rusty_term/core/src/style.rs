//! The per-grid style table behind the 8-byte [`PackedCell`].
//!
//! A cell stores its character and a [`StyleId`]; everything else a [`Cell`]
//! carries (colours, SGR attributes, link, cluster) lives once per distinct
//! value in a [`StyleTable`]. Within one table equal styles always share one
//! id, so comparing ids compares styles.
//!
//! Reclamation: nothing is counted per cell. A sweep marks the ids every
//! retained cell uses (screens and history) and frees the rest. It runs
//! after the table doubles, or once history has turned over since the last
//! sweep, so its linear scan is amortized over at least as many interns or
//! evicted cells as it reads.
//!
//! Pauses: a sweep and a compaction only scan arrays. A freed id keeps its
//! index entry until its slot is reused, so freeing costs no hash work; a
//! lookup that finds a freed id revives it, since no cell holds it.

use std::collections::{HashMap, TryReserveError};
use std::hash::{Hash, Hasher};

use crate::cell::{
    ATTR_BG_DEFAULT, ATTR_FG_DEFAULT, ATTR_PROTECTED, Cell, DEFAULT_BG, DEFAULT_FG, PackedCell,
    Pen, Source, WIDE_TRAILER,
};

/// Everything a [`Cell`] holds except its character and the two layout bits
/// ([`WIDE_TRAILER`], [`ATTR_PROTECTED`]), which live in [`PackedCell`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellStyle {
    pub fg: u32,
    pub bg: u32,
    pub underline_color: u32,
    /// `Cell::flags` without the two layout bits.
    pub attrs: u16,
    pub link: u16,
    pub cluster: u16,
    /// [`crate::cell::Source::tag`] of `fg`: `1 + n` while it is palette
    /// entry `n`, so OSC 4 recolours it ([`StyleTable::repaint`]).
    pub fg_index: u16,
    /// [`crate::cell::Source::tag`] of `bg`.
    pub bg_index: u16,
}

/// Every field, packed into three words: one hasher write each instead of
/// one per field, on the intern path every uncached write takes.
impl Hash for CellStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(u64::from(self.fg) | u64::from(self.bg) << 32);
        state.write_u64(
            u64::from(self.underline_color)
                | u64::from(self.attrs) << 32
                | u64::from(self.link) << 48,
        );
        state.write_u64(
            u64::from(self.cluster)
                | u64::from(self.fg_index) << 16
                | u64::from(self.bg_index) << 32,
        );
    }
}

impl CellStyle {
    /// The style of `cell`, layout bits dropped, in canonical form.
    pub(crate) fn of(cell: &Cell) -> Self {
        CellStyle {
            fg: cell.fg,
            bg: cell.bg,
            underline_color: cell.underline_color,
            attrs: cell.flags & !(WIDE_TRAILER | ATTR_PROTECTED),
            link: cell.link,
            cluster: cell.cluster,
            fg_index: 0,
            bg_index: 0,
        }
        .canonical()
    }

    /// The style of a glyph written in `pen` under `link`. The pen's DECSCA
    /// bit is a layout bit and does not reach the style.
    #[inline]
    pub(crate) fn of_pen(pen: &Pen, link: u16) -> Self {
        CellStyle {
            fg: pen.fg,
            bg: pen.bg,
            underline_color: pen.underline_color,
            attrs: pen.attrs & !ATTR_PROTECTED,
            link,
            cluster: 0,
            fg_index: pen.fg_index,
            bg_index: pen.bg_index,
        }
    }

    /// A colour marked default ([`ATTR_FG_DEFAULT`] / [`ATTR_BG_DEFAULT`])
    /// stored as `0`: the table resolves it on read, so styles that differ
    /// only in the default in force when they were written are one entry.
    #[inline]
    pub(crate) fn canonical(mut self) -> Self {
        if self.attrs & ATTR_FG_DEFAULT != 0 {
            self.fg = 0;
        }
        if self.attrs & ATTR_BG_DEFAULT != 0 {
            self.bg = 0;
        }
        self
    }

    /// This style with its default colours filled in from `defaults`.
    #[inline]
    fn resolved(mut self, defaults: Defaults) -> Self {
        if self.attrs & ATTR_FG_DEFAULT != 0 {
            self.fg = defaults.fg;
        }
        if self.attrs & ATTR_BG_DEFAULT != 0 {
            self.bg = defaults.bg;
        }
        self
    }
}

/// The default foreground and background (OSC 10 / 11) that styles marked
/// default resolve to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Defaults {
    pub fg: u32,
    pub bg: u32,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        }
    }
}

/// An index into a [`StyleTable`]. Meaningful only within the grid that
/// interned it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct StyleId(pub u32);

impl StyleId {
    /// The default blank style, interned when the table is built and never
    /// freed. Its content may change under a retheme.
    pub(crate) const DEFAULT: StyleId = StyleId(0);

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// Old id to new id, from a compaction or a retheme merge.
pub(crate) struct Remap(Vec<StyleId>);

impl Remap {
    /// The new id of `id`.
    pub(crate) fn get(&self, id: StyleId) -> StyleId {
        self.0[id.index()]
    }

    /// Rewrite every cell of `cells` to its new id.
    pub(crate) fn apply(&self, cells: &mut [PackedCell]) {
        for cell in cells {
            *cell = cell.with_style(self.get(cell.style()));
        }
    }
}

/// Fewest live styles before either sweep trigger can fire.
pub(crate) const SWEEP_MIN: usize = 4096;
/// Fewest entries before a compaction is worth its full rewrite.
const COMPACT_MIN: usize = 65_536;
/// Bytes a decoder charges per reserved grid style: 24 B entry, 1 B free
/// flag, and two 29 B index buckets (hashbrown gives a power-of-two capacity
/// `c` exactly `2c` buckets), rounded up from 83.
pub(crate) const GRID_STYLE_BYTES: usize = 86;

/// A direct-mapped cache in front of the hash index. Output repeats a few
/// hundred pens at most (every write interns its pen), and a hit skips
/// SipHash. Misses fall through to the index, so hostile output gains
/// nothing by colliding here.
#[derive(Debug)]
struct Recent(Box<[(CellStyle, StyleId)]>);

impl Default for Recent {
    fn default() -> Self {
        let empty = (CellStyle::of(&Cell::default_blank()), Recent::EMPTY);
        Recent(vec![empty; Recent::SLOTS].into_boxed_slice())
    }
}

impl Recent {
    const SLOTS: usize = 1024;
    /// Marks an unused slot; no table issues it.
    const EMPTY: StyleId = StyleId(u32::MAX);

    fn slot(style: &CellStyle) -> usize {
        let rest = u32::from(style.attrs) << 16
            | u32::from(
                style.link ^ style.cluster ^ style.fg_index ^ style.bg_index.rotate_left(8),
            );
        let mix =
            (style.fg ^ style.bg.rotate_left(8) ^ style.underline_color.rotate_left(16) ^ rest)
                .wrapping_mul(0x9e37_79b9);
        (mix >> 22) as usize
    }

    fn get(&self, style: &CellStyle) -> Option<StyleId> {
        let (s, id) = self.0[Self::slot(style)];
        (id != Self::EMPTY && s == *style).then_some(id)
    }

    fn put(&mut self, style: CellStyle, id: StyleId) {
        self.0[Self::slot(&style)] = (style, id);
    }

    fn clear(&mut self) {
        self.0
            .fill((CellStyle::of(&Cell::default_blank()), Self::EMPTY));
    }
}

/// The distinct styles of one grid. See the module docs.
#[derive(Debug)]
pub(crate) struct StyleTable {
    /// Id to style. Freed slots keep stale contents until reused.
    entries: Vec<CellStyle>,
    /// Style to id, for every live entry and for freed entries whose slot
    /// has not been reused.
    index: HashMap<CellStyle, StyleId>,
    /// Id to whether it is freed.
    is_free: Vec<bool>,
    /// Freed ids, to reuse; an id revived since it was freed is skipped.
    free: Vec<StyleId>,
    /// Ids with `is_free` set.
    free_count: usize,
    /// Recently interned live styles; cleared whenever an id may change.
    recent: Recent,
    /// The last style interned, checked before `recent`: writes come in
    /// runs of one pen.
    last: (CellStyle, StyleId),
    /// The same for [`StyleTable::intern_pen`].
    last_pen: (CellStyle, StyleId),
    /// [`StyleTable::live`] after the last sweep.
    live_after_sweep: usize,
    /// Cells the last sweep read.
    marked: usize,
    /// Cells evicted from history since the last sweep.
    turnover: usize,
    /// What colours marked default resolve to on [`StyleTable::get`].
    defaults: Defaults,
}

impl Default for StyleTable {
    fn default() -> Self {
        let blank = CellStyle::of(&Cell::default_blank());
        StyleTable {
            entries: vec![blank],
            index: HashMap::from([(blank, StyleId::DEFAULT)]),
            is_free: vec![false],
            free: Vec::new(),
            free_count: 0,
            recent: Recent::default(),
            last: (blank, Recent::EMPTY),
            last_pen: (blank, Recent::EMPTY),
            live_after_sweep: 1,
            marked: 0,
            turnover: 0,
            defaults: Defaults::default(),
        }
    }
}

impl StyleTable {
    /// The id of `style`, adding it if new (into a freed slot when one is
    /// available). A freed id still indexed under `style` is revived.
    #[inline]
    pub(crate) fn intern(&mut self, style: CellStyle) -> StyleId {
        let style = style.canonical();
        if self.last.1 != Recent::EMPTY && self.last.0 == style {
            return self.last.1;
        }
        self.intern_cached(style)
    }

    /// [`StyleTable::intern`] for a writer's pen, with a last-style check of
    /// its own so other interns between two writes do not evict the pen.
    #[inline]
    pub(crate) fn intern_pen(&mut self, style: CellStyle) -> StyleId {
        let style = style.canonical();
        if self.last_pen.1 != Recent::EMPTY && self.last_pen.0 == style {
            return self.last_pen.1;
        }
        let id = self.intern_cached(style);
        self.last_pen = (style, id);
        id
    }

    /// [`StyleTable::intern`] past the last-style check.
    #[inline(never)]
    fn intern_cached(&mut self, style: CellStyle) -> StyleId {
        let id = match self.recent.get(&style) {
            Some(id) => id,
            None => {
                let id = self.lookup_or_add(style);
                self.recent.put(style, id);
                id
            }
        };
        self.last = (style, id);
        id
    }

    /// [`StyleTable::intern`] past the cache.
    fn lookup_or_add(&mut self, style: CellStyle) -> StyleId {
        if let Some(&id) = self.index.get(&style) {
            if self.is_free[id.index()] {
                self.is_free[id.index()] = false;
                self.free_count -= 1;
            }
            return id;
        }
        let id = match self.take_free() {
            Some(id) => {
                let old = self.entries[id.index()];
                if self.index.get(&old) == Some(&id) {
                    self.index.remove(&old);
                }
                self.entries[id.index()] = style;
                id
            }
            None => {
                self.entries.push(style);
                self.is_free.push(false);
                StyleId(u32::try_from(self.entries.len() - 1).unwrap_or(u32::MAX))
            }
        };
        self.index.insert(style, id);
        id
    }

    /// A freed id to reuse, skipping revived ones.
    fn take_free(&mut self) -> Option<StyleId> {
        while let Some(id) = self.free.pop() {
            if self.is_free[id.index()] {
                self.is_free[id.index()] = false;
                self.free_count -= 1;
                return Some(id);
            }
        }
        None
    }

    /// The style behind `id`, its default colours resolved to the current
    /// [`Defaults`]. An id this table never issued reads as the default style
    /// (a debug build asserts instead).
    pub(crate) fn get(&self, id: StyleId) -> CellStyle {
        debug_assert!(
            id.index() < self.entries.len(),
            "style id {id:?} out of range"
        );
        self.entries
            .get(id.index())
            .copied()
            .unwrap_or(self.entries[0])
            .resolved(self.defaults)
    }

    /// The colours that styles marked default resolve to.
    pub(crate) fn defaults(&self) -> Defaults {
        self.defaults
    }

    /// Change what styles marked default resolve to (OSC 10 / 11 and their
    /// resets). No entry changes: every cell in a default colour follows.
    pub(crate) fn set_defaults(&mut self, defaults: Defaults) {
        self.defaults = defaults;
    }

    /// Entries ever allocated, freed slots included: ids are below this.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reserve room for `additional` more entries, as a decoder does in step
    /// with its charged budget. Fails instead of aborting on allocation
    /// failure.
    pub(crate) fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        self.entries.try_reserve_exact(additional)?;
        self.is_free.try_reserve_exact(additional)?;
        self.index.try_reserve(additional)
    }

    /// Entries not freed, dead-but-unswept ones included.
    pub(crate) fn live(&self) -> usize {
        self.entries.len() - self.free_count
    }

    /// Bytes the table holds, by capacity.
    pub(crate) fn bytes(&self) -> usize {
        self.entries.capacity() * size_of::<CellStyle>()
            + self.is_free.capacity()
            + self.free.capacity() * size_of::<StyleId>()
            + self.index.capacity() * (size_of::<(CellStyle, StyleId)>() + 1)
    }

    /// Id to whether it is freed (validation: no cell may hold one).
    pub(crate) fn free_mask(&self) -> &[bool] {
        &self.is_free
    }

    /// Note `cells` cells leaving history.
    pub(crate) fn note_evicted(&mut self, cells: usize) {
        self.turnover = self.turnover.saturating_add(cells);
    }

    /// Make the next [`StyleTable::sweep_due`] true once the table is past
    /// [`SWEEP_MIN`] (history was cleared or rebuilt).
    pub(crate) fn request_sweep(&mut self) {
        self.turnover = usize::MAX;
    }

    /// Whether enough has changed since the last sweep to pay for one: the
    /// table doubled, or as many cells left history as the last sweep read.
    pub(crate) fn sweep_due(&self) -> bool {
        let live = self.live();
        let growth = live > SWEEP_MIN.max(2 * self.live_after_sweep);
        let eviction = live > SWEEP_MIN && self.turnover > self.marked;
        growth || eviction
    }

    /// Free every id no cell of `lines` uses: `lines` must be every retained
    /// cell. The caller has already dropped its id caches.
    pub(crate) fn sweep<'a>(&mut self, lines: impl Iterator<Item = &'a [PackedCell]>) {
        let mut live = vec![false; self.entries.len()];
        live[StyleId::DEFAULT.index()] = true;
        let mut marked = 0;
        for cells in lines {
            marked += cells.len();
            for cell in cells {
                if let Some(slot) = live.get_mut(cell.style().index()) {
                    *slot = true;
                }
            }
        }
        self.free.retain(|id| self.is_free[id.index()]);
        for (i, &keep) in live.iter().enumerate() {
            if !keep && !self.is_free[i] {
                self.is_free[i] = true;
                self.free.push(StyleId(i as u32));
            }
        }
        self.free_count = self.free.len();
        self.live_after_sweep = self.live();
        (self.marked, self.turnover) = (marked, 0);
        self.recent.clear();
        self.last.1 = Recent::EMPTY;
        self.last_pen.1 = Recent::EMPTY;
    }

    /// After a sweep: renumber live styles densely, in id order (id 0 stays
    /// 0), when a large table has at most [`COMPACT_MIN`] live entries and
    /// fewer than half. The caller applies the returned remap to every cell
    /// it holds. Waiting for few live styles keeps the index rebuild small;
    /// until then freed slots are reused, so the table does not grow.
    pub(crate) fn compact(&mut self) -> Option<Remap> {
        let live = self.live();
        if self.entries.len() <= COMPACT_MIN || live > COMPACT_MIN || live * 2 >= self.entries.len()
        {
            return None;
        }
        let is_free = self.is_free.clone();
        Some(self.rebuild(|i| !is_free[i], |i| i))
    }

    /// Map every entry's colours through `map` (a theme change). Entries that
    /// become equal merge into the lowest id; the returned remap, when some
    /// merged, must be applied to every cell.
    pub(crate) fn retheme(&mut self, map: impl Fn(u32) -> u32) -> Option<Remap> {
        // A default colour is stored as 0 and resolved on read: only the
        // colours the child chose are remapped.
        self.restyle(|style| {
            if style.attrs & ATTR_FG_DEFAULT == 0 {
                style.fg = map(style.fg);
            }
            if style.attrs & ATTR_BG_DEFAULT == 0 {
                style.bg = map(style.bg);
            }
        })
    }

    /// Recolour every entry in a palette colour from `palette` (OSC 4 and
    /// its resets): a colour tagged `1 + n` becomes `palette(n)`. Merges as
    /// [`StyleTable::retheme`] does.
    pub(crate) fn repaint(&mut self, palette: impl Fn(u8) -> u32) -> Option<Remap> {
        self.restyle(|style| {
            if let Some(n) = Source::entry(style.fg_index) {
                style.fg = palette(n);
            }
            if let Some(n) = Source::entry(style.bg_index) {
                style.bg = palette(n);
            }
        })
    }

    /// Apply `edit` to every live entry, merging entries that become equal
    /// into the lowest id; the returned remap, when some merged, must be
    /// applied to every cell.
    fn restyle(&mut self, edit: impl Fn(&mut CellStyle)) -> Option<Remap> {
        self.recent.clear();
        self.last.1 = Recent::EMPTY;
        self.last_pen.1 = Recent::EMPTY;
        let is_free = self.is_free.clone();
        for (i, style) in self.entries.iter_mut().enumerate() {
            if !is_free[i] {
                edit(style);
            }
        }
        let mut first: HashMap<CellStyle, usize> = HashMap::new();
        let mut target: Vec<usize> = (0..self.entries.len()).collect();
        for (i, style) in self.entries.iter().enumerate() {
            if !is_free[i] {
                target[i] = *first.entry(*style).or_insert(i);
            }
        }
        if target.iter().enumerate().all(|(i, &t)| t == i) {
            self.index = first
                .into_iter()
                .map(|(style, i)| (style, StyleId(i as u32)))
                .collect();
            // Freed ids are no longer indexed, so none can be revived.
            self.free.retain(|id| is_free[id.index()]);
            return None;
        }
        Some(self.rebuild(|i| !is_free[i] && target[i] == i, |i| target[i]))
    }

    /// Keep the entries `keep` selects, renumbered densely in order; every
    /// other live entry `i` folds into the kept entry `fold(i)`.
    fn rebuild(&mut self, keep: impl Fn(usize) -> bool, fold: impl Fn(usize) -> usize) -> Remap {
        let n = self.entries.len();
        let mut new_id = vec![StyleId::DEFAULT; n];
        let mut entries = Vec::new();
        for (i, &style) in self.entries.iter().enumerate() {
            if keep(i) {
                new_id[i] = StyleId(entries.len() as u32);
                entries.push(style);
            }
        }
        let is_free = std::mem::take(&mut self.is_free);
        for i in 0..n {
            if !keep(i) && !is_free[i] {
                new_id[i] = new_id[fold(i)];
            }
        }
        self.index = (entries.iter().enumerate())
            .map(|(i, &style)| (style, StyleId(i as u32)))
            .collect();
        self.is_free = vec![false; entries.len()];
        self.entries = entries;
        self.free = Vec::new();
        self.free_count = 0;
        self.live_after_sweep = self.entries.len();
        self.recent.clear();
        self.last.1 = Recent::EMPTY;
        self.last_pen.1 = Recent::EMPTY;
        Remap(new_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(n: u32) -> CellStyle {
        CellStyle {
            fg: n,
            ..CellStyle::of(&Cell::blank())
        }
    }

    fn cells(ids: &[StyleId]) -> Vec<PackedCell> {
        ids.iter()
            .map(|&id| PackedCell::glyph('a', false, id))
            .collect()
    }

    #[test]
    fn equal_styles_share_one_id_and_id_0_is_blank() {
        let mut t = StyleTable::default();
        assert_eq!(
            t.intern(CellStyle::of(&Cell::default_blank())),
            StyleId::DEFAULT
        );
        let a = t.intern(fg(1));
        assert_eq!(t.intern(fg(1)), a);
        assert_ne!(t.intern(fg(2)), a);
        assert_eq!(t.get(a), fg(1));
        assert_eq!((t.len(), t.live()), (3, 3));
    }

    #[test]
    fn sweep_keeps_marked_and_blank_ids_and_reuses_the_rest() {
        let mut t = StyleTable::default();
        let [screen, hist, dead] = [1, 2, 3].map(|n| t.intern(fg(n)));
        let (on_screen, in_history) = (cells(&[screen]), cells(&[hist]));
        t.sweep([on_screen.as_slice(), &in_history].into_iter());
        assert_eq!(t.free_mask(), [false, false, false, true]);
        assert_eq!(t.live(), 3);
        // A freed slot is reused, and its old style no longer resolves to it.
        let fresh = t.intern(fg(4));
        assert_eq!(fresh, dead);
        assert_ne!(t.intern(fg(3)), dead);
        // A freed id that is interned again before reuse is revived.
        t.sweep([cells(&[screen, fresh]).as_slice()].into_iter());
        assert!(t.free_mask()[hist.0 as usize]);
        assert_eq!(t.intern(fg(2)), hist);
        assert!(!t.free_mask()[hist.0 as usize]);
    }

    #[test]
    fn sweep_is_due_on_growth_or_turnover() {
        let mut t = StyleTable::default();
        let mut ids: Vec<_> = (1..SWEEP_MIN as u32).map(|n| t.intern(fg(n))).collect();
        assert!(!t.sweep_due(), "live is exactly the minimum");
        ids.push(t.intern(fg(0)));
        assert!(t.sweep_due(), "growth");
        let all = cells(&ids);
        t.sweep([all.as_slice()].into_iter());
        assert!(!t.sweep_due());
        t.note_evicted(SWEEP_MIN);
        assert!(!t.sweep_due(), "turnover equal to the cells last read");
        t.note_evicted(1);
        assert!(t.sweep_due(), "turnover");
        t.sweep([all.as_slice()].into_iter());
        t.request_sweep();
        assert!(t.sweep_due(), "requested");
    }

    #[test]
    fn compaction_renumbers_densely_and_keeps_styles() {
        let mut t = StyleTable::default();
        let n = COMPACT_MIN as u32 + 2;
        let ids: Vec<_> = (1..=n).map(|i| t.intern(fg(i))).collect();
        let keep = [ids[5], ids[COMPACT_MIN]];
        let mut screen = cells(&keep);
        let mut history = cells(&[ids[7]]);
        t.sweep([screen.as_slice(), &history].into_iter());
        let remap = t.compact().expect("mostly dead");
        remap.apply(&mut screen);
        remap.apply(&mut history);
        assert_eq!(t.len(), 4);
        assert_eq!(remap.get(StyleId::DEFAULT), StyleId::DEFAULT);
        assert_eq!(t.get(screen[0].style()), fg(6));
        assert_eq!(t.get(screen[1].style()), fg(COMPACT_MIN as u32 + 1));
        assert_eq!(t.get(history[0].style()), fg(8));
        assert_eq!(t.intern(fg(6)), screen[0].style(), "index rebuilt");
        assert!(t.compact().is_none(), "small and dense");
    }

    #[test]
    fn retheme_merges_styles_that_become_equal() {
        let mut t = StyleTable::default();
        let [a, b, c] = [1, 2, 3].map(|n| t.intern(fg(n)));
        // 2 becomes 1: b merges into a; 3 is untouched.
        let remap = t.retheme(|v| if v == 2 { 1 } else { v }).expect("a merge");
        assert_eq!(remap.get(b), remap.get(a));
        assert_eq!(t.get(remap.get(c)), fg(3));
        assert_eq!(t.len(), 3);
        assert_eq!(t.intern(fg(1)), remap.get(a), "one id for the merged style");
        // A retheme that merges nothing keeps every id.
        assert!(t.retheme(|v| v + 10).is_none());
        let moved = CellStyle {
            bg: fg(3).bg + 10,
            ..fg(13)
        };
        assert_eq!(t.get(remap.get(c)), moved);
        assert_eq!(t.intern(moved), remap.get(c), "index follows the new keys");
    }

    #[test]
    fn default_colours_resolve_at_read_and_survive_retheme() {
        let mut t = StyleTable::default();
        let blank = t.intern(CellStyle::of(&Cell::default_blank()));
        // An explicit colour equal to the default is a different style.
        let explicit = t.intern(fg(DEFAULT_FG));
        assert_ne!(blank, explicit);
        t.set_defaults(Defaults { fg: 7, bg: 9 });
        assert_eq!((t.get(blank).fg, t.get(blank).bg), (7, 9));
        assert_eq!(
            (t.get(explicit).fg, t.get(explicit).bg),
            (DEFAULT_FG, DEFAULT_BG)
        );
        // A retheme maps explicit colours only; the defaults are set apart.
        assert!(t.retheme(|v| v + 1).is_none());
        assert_eq!(t.get(blank).fg, 7);
        assert_eq!(t.get(explicit).fg, DEFAULT_FG + 1);
    }
}
