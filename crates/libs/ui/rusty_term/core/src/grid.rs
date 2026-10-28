//! The screen buffer (L06 state): the authoritative grid of cells, the cursor,
//! scrolling region, alternate screen, and scrollback history.
//!
//! The [`Grid`] exposes a semantic API (`put_char`, `set_cursor`, `scroll_up`,
//! …) that the [`AnsiParser`](super::parser::AnsiParser) drives, and produces a
//! [`DirtyFrame`] snapshot for the renderer.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};

use super::cell::{ATTR_DEFAULT_COLORS, Cell, PackedCell, Pen, WIDE_TRAILER, char_width};
use super::style::{CellStyle, Defaults, StyleId, StyleTable};
use unicode_segmentation::UnicodeSegmentation;

use super::sixel::SixelImage;

mod line;
mod snapshot;
pub(crate) use line::Line;
#[cfg(test)]
mod stamp_tests;
#[cfg(test)]
mod trim_tests;
pub use snapshot::SnapshotError;
#[cfg(any(test, feature = "gui"))]
mod frame;
#[cfg(any(test, feature = "gui"))]
mod view;
#[cfg(any(test, feature = "gui"))]
pub use frame::{FrameImage, FrameView, RowKey};
#[cfg(any(test, feature = "gui"))]
pub use view::View;

/// Default maximum number of lines retained in the scrollback history. Older
/// lines are dropped as new ones arrive. Overridable per-grid via
/// [`Grid::set_scrollback_max`] (the `scrollback` config key).
pub const SCROLLBACK_MAX: usize = 10_000;

/// How long a synchronized-output window (DEC `?2026`) may stay open before
/// [`Grid::sync_output_active`] gives up on it and lets the render loop
/// resume painting anyway. Generous enough that a real frame update finishes
/// well within it; short enough that a misbehaving or crashed client can't
/// freeze the display for long.
const SYNC_OUTPUT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

/// Maximum number of prompt marks (OSC 133;A) retained for navigation. Older
/// marks are dropped once exceeded — far more than any real session needs.
const PROMPT_MARKS_MAX: usize = 1024;

/// Maximum number of foldable command-output blocks (OSC 133/633's C..D
/// range) retained. Older blocks are dropped once exceeded, the same
/// generous-but-bounded pattern as [`PROMPT_MARKS_MAX`].
///
/// Not itself feature-gated (unlike the `Grid` fields it bounds) because
/// [`reflow_history`] — which always compiles, TUI included — needs to name
/// [`CommandBlock`] to thread fold state through a resize the same way it
/// does prompt marks; the fields that actually populate it stay behind
/// `test`/`gui`, so this is dead weight only in a TUI-only build, not a
/// capability leak.
const FOLD_BLOCKS_MAX: usize = 1024;

/// One finished command's output range in absolute logical lines (`start` at
/// its OSC C, `end` at its matching D — a half-open `[start, end)`), and
/// whether the windowed front-end's scrollback view currently collapses it to
/// one summary line. The range only tracks *state*; painting the collapsed
/// summary and expanding it back on click/keybind is renderer work that
/// consumes this, not modeled here. See [`FOLD_BLOCKS_MAX`] for why this type
/// itself isn't feature-gated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandBlock {
    pub start: usize,
    pub end: usize,
    pub folded: bool,
    /// Exit code its OSC 133;D reported (`None` when the shell omitted one).
    /// Colors the windowed front-end's command gutter marks.
    pub exit: Option<i32>,
    /// How long the command ran (C to D), when the timer saw its C. Shown by
    /// the windowed front-end's command dock.
    pub runtime: Option<std::time::Duration>,
}

/// What a command gutter mark says about the viewport row it sits beside:
/// part of a command that succeeded (exit 0), failed (non-zero), or is still
/// running (OSC 133;C seen, no D yet). Rows outside any command block (or
/// whose block reported no exit code) carry no mark.
#[cfg(any(test, feature = "gui"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockMark {
    Success,
    Error,
    Running,
}

/// One line of the fold-aware history view: a real scrollback line, or the
/// one-line summary standing in for a folded [`CommandBlock`] (by index).
#[cfg(any(test, feature = "gui"))]
enum HistLine {
    Abs(usize),
    Summary(usize),
}

/// Maximum depth of the XTPUSHTITLE stack. Further pushes are dropped rather
/// than growing without bound — no real app nests this deep.
const TITLE_STACK_MAX: usize = 64;

/// Maximum depth of the Kitty keyboard protocol's enhancement-flag stack
/// (`CSI > flags u` pushes). Real apps push at most a handful of levels
/// (Neovim pushes/pops once around its own event loop); bounded against a
/// runaway script the same way [`TITLE_STACK_MAX`] is.
const KITTY_FLAGS_STACK_MAX: usize = 16;

/// Maximum number of distinct hyperlink URIs interned from OSC 8. Past this,
/// further links are dropped (rendered as plain text) rather than growing the
/// table without bound.
const LINK_MAX: usize = 4096;

/// Maximum number of distinct grapheme-continuation strings interned from
/// multi-scalar glyphs. Past this, further continuations are dropped (the base
/// glyph still renders) rather than growing the table without bound.
const CLUSTER_MAX: usize = 8192;

/// A text selection in **absolute** cell coordinates (`(col, abs_row)`,
/// where `abs_row` indexes scrollback + live screen), set by the windowed
/// front-end (mouse drag, multi-click, copy mode) and read by the renderer
/// (to invert the highlighted cells) and by [`Grid::selected_text`]
/// (clipboard copy). Absolute rows keep a selection anchored to its text
/// while the viewport scrolls; long-lived selections can still drift when
/// scrollback evicts or a resize reflows (both clear/invalidate it in
/// practice). `anchor` is where the drag began, `head` where it is now; the
/// pair is normalized into row-major order on read, so either drag
/// direction works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Cell where the drag started.
    pub anchor: (usize, usize),
    /// Cell under the pointer now.
    pub head: (usize, usize),
}

/// In-window scrollback search results (windowed front-end). Matches span the
/// scrollback + live screen joined into logical lines, so a query can cross a
/// soft wrap; the renderer highlights them and next/prev steps through.
#[cfg(any(test, feature = "gui"))]
#[derive(Default)]
pub(crate) struct Search {
    /// Absolute row → highlighted col spans `(start, end_exclusive, match_index)`.
    rows: HashMap<usize, Vec<(usize, usize, usize)>>,
    /// Per match, the cell to scroll into view: `(absolute_row, col)`.
    anchors: Vec<(usize, usize)>,
    /// The active match — highlighted distinctly and the target of next/prev.
    current: usize,
}

/// Cap on retained search matches, bounding work and highlight storage.
#[cfg(any(test, feature = "gui"))]
const SEARCH_MAX: usize = 2000;

/// Detect plain-text URLs in one logical line, as `(start, end_exclusive,
/// url)` char spans. Recognized: `http(s)://`, `ftp://`, `file://`,
/// `mailto:`, and bare `www.` (returned with `http://` prepended). A URL
/// runs over RFC 3986 characters and is trimmed of trailing punctuation
/// that's overwhelmingly sentence context (`.` `,` `;` `:` `!` `?` and any
/// closer without its matching opener in the URL), matching what kitty and
/// WezTerm do.
#[cfg(any(test, feature = "gui"))]
fn detect_urls(text: &[char]) -> Vec<(usize, usize, String)> {
    const SCHEMES: [&str; 6] = [
        "https://", "http://", "ftp://", "file://", "mailto:", "www.",
    ];
    let is_url_char = |c: char| c.is_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c);
    let lower: Vec<char> = text.iter().map(|c| c.to_ascii_lowercase()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let Some(scheme) = SCHEMES
            .iter()
            .find(|s| lower[i..].starts_with(&s.chars().collect::<Vec<_>>()[..]))
        else {
            i += 1;
            continue;
        };
        // A scheme mid-word (e.g. "xhttp://") isn't a URL start.
        if i > 0 && (text[i - 1].is_alphanumeric() || text[i - 1] == '.') {
            i += 1;
            continue;
        }
        let mut end = i + scheme.chars().count();
        while end < text.len() && is_url_char(text[end]) {
            end += 1;
        }
        // Trim trailing sentence punctuation and unbalanced closers.
        while end > i {
            let c = text[end - 1];
            let trim = match c {
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
                ')' => {
                    text[i..end].iter().filter(|&&x| x == '(').count()
                        < text[i..end].iter().filter(|&&x| x == ')').count()
                }
                ']' => {
                    text[i..end].iter().filter(|&&x| x == '[').count()
                        < text[i..end].iter().filter(|&&x| x == ']').count()
                }
                _ => false,
            };
            if !trim {
                break;
            }
            end -= 1;
        }
        // Require something after the scheme, and skip bare "www." itself.
        if end > i + scheme.chars().count() {
            let mut url: String = text[i..end].iter().collect();
            if *scheme == "www." {
                url.insert_str(0, "http://");
            }
            out.push((i, end, url));
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Record the match `text[i..i + mlen]` into `st` as an anchor plus per-row
/// highlight spans (a match can cross a soft wrap, hence per-row).
#[cfg(any(test, feature = "gui"))]
fn record_match(text: &[char], at: &[(usize, usize)], i: usize, mlen: usize, st: &mut Search) {
    let mi = st.anchors.len();
    st.anchors.push(at[i]);
    let mut j = i;
    while j < i + mlen {
        let row = at[j].0;
        let start = at[j].1;
        let mut end = at[j].1 + char_width(text[j]).max(1);
        let mut k = j + 1;
        while k < i + mlen && at[k].0 == row {
            end = at[k].1 + char_width(text[k]).max(1);
            k += 1;
        }
        st.rows.entry(row).or_default().push((start, end, mi));
        j = k;
    }
}

/// The searchable length of one logical line: its `text` minus trailing spaces.
#[cfg(any(test, feature = "gui"))]
fn line_len(text: &[char]) -> usize {
    let mut len = text.len();
    while len > 0 && text[len - 1] == ' ' {
        len -= 1;
    }
    len
}

/// Find every (non-overlapping) occurrence of `q` in one logical line's `text`
/// (with parallel per-char `at` cell positions), recording each as an anchor
/// plus per-row highlight spans in `st`. Trailing spaces are ignored.
#[cfg(any(test, feature = "gui"))]
fn find_matches(text: &[char], at: &[(usize, usize)], q: &[char], st: &mut Search) {
    let len = line_len(text);
    if q.is_empty() || q.len() > len {
        return;
    }
    let mut i = 0;
    while i + q.len() <= len {
        if text[i..i + q.len()] != *q {
            i += 1;
            continue;
        }
        record_match(text, at, i, q.len(), st);
        i += q.len();
        if st.anchors.len() >= SEARCH_MAX {
            return;
        }
    }
}

/// Simple one-char Unicode case fold (the first char of `to_lowercase()`).
/// Full folding (ß → ss) changes lengths and would break the char↔column
/// mapping, so it's deliberately out. Both the search haystack and the query
/// (or regex pattern) fold through this, making every search mode
/// case-insensitive beyond plain ASCII.
#[cfg(any(test, feature = "gui"))]
pub(crate) fn fold_char(c: char) -> char {
    // Canonical-base fold first (bidi phase 5): precomposed and decomposed
    // spellings — and accents generally — land on the same base char, so
    // "e" finds "é" whether the app wrote U+00E9 or e + combining acute
    // (whose combining mark rides in a cluster suffix the search never
    // sees). Then the case fold as before.
    let cp = c as u32;
    let c = match super::canon_tables::CANON_BASE.binary_search_by_key(&cp, |&(a, _)| a) {
        Ok(i) => char::from_u32(super::canon_tables::CANON_BASE[i].1).unwrap_or(c),
        Err(_) => c,
    };
    c.to_lowercase().next().unwrap_or(c)
}

/// The regex flavor of [`find_matches`]: every non-overlapping match of the
/// compiled pattern (`rusty_regx`, POSIX-longest semantics) in one logical
/// line. The engine matches `&str` with byte offsets, so the folded char
/// line is materialized once with a byte→char index table. Empty-width
/// matches (e.g. `x*` where there is no `x`) are skipped — there is nothing
/// to highlight — advancing one char so the scan always terminates. A
/// `^`-anchored pattern matches at most once per line: find-all scans
/// successive suffixes, where `^` would falsely re-anchor.
#[cfg(any(test, feature = "gui"))]
fn find_matches_rx(
    text: &[char],
    at: &[(usize, usize)],
    re: &rusty_regx::Regex,
    anchored: bool,
    st: &mut Search,
) {
    let len = line_len(text);
    let line: String = text[..len].iter().collect();
    // Byte offset of each char (plus the end sentinel), for offset mapping.
    let mut byte_of = Vec::with_capacity(len + 1);
    let mut b = 0;
    for &c in &text[..len] {
        byte_of.push(b);
        b += c.len_utf8();
    }
    byte_of.push(b);
    let mut from = 0usize; // byte offset the next scan starts at
    while from <= line.len() {
        let suffix = &line[from..];
        let Some(caps) = re.captures(suffix) else {
            return;
        };
        let Some(m) = caps.get(0) else { return };
        // The engine has no positional API, but `m` borrows from `suffix`,
        // so its byte offset is plain (safe) pointer arithmetic on the same
        // allocation. (A textual `find(&m)` would be wrong for `$`-anchored
        // patterns, whose matched text can also occur earlier in the line.)
        let rel = m.as_ptr() as usize - suffix.as_ptr() as usize;
        let (sb, eb) = (from + rel, from + rel + m.len());
        let s_idx = byte_of.partition_point(|&x| x < sb);
        let e_idx = byte_of.partition_point(|&x| x < eb);
        if e_idx > s_idx {
            record_match(text, at, s_idx, e_idx - s_idx, st);
            if st.anchors.len() >= SEARCH_MAX {
                return;
            }
            from = eb;
        } else {
            // Empty-width match: nothing to highlight; step one char.
            from = *byte_of.get(s_idx + 1).unwrap_or(&(line.len() + 1));
        }
        if anchored {
            return; // `^…` can only ever match at the true line start
        }
    }
}

/// One stored Kitty image: its root pixels plus any animation frames.
pub struct KittyImage {
    pub id: u32,
    pub w: usize,
    pub h: usize,
    /// Full-size frames (frame 0 is the root; `a=f` composites partial
    /// payloads onto the previous frame, so every entry is directly drawable).
    pub frames: Vec<KittyFrame>,
    pub current: usize,
    pub playing: bool,
    /// When the current frame was shown, for gap pacing.
    pub last_advance: Option<std::time::Instant>,
}

impl KittyImage {
    /// The store-budget cost: every frame is full-size.
    fn pixel_budget(&self) -> usize {
        self.w * self.h * self.frames.len()
    }
}

pub struct KittyFrame {
    /// Shared so a frame snapshot can hold it without copying the pixels.
    pub pixels: Arc<[Option<u32>]>,
    /// Display time in milliseconds (floored to 40 — the protocol allows 0,
    /// but a zero-gap loop would spin the repaint timer).
    pub gap_ms: u32,
}

/// The most program-status signals [`Grid::program_status`] holds between
/// drains.
pub const PROGRAM_STATUS_MAX: usize = 256;

/// An input to a program-status (OSC 7501) record store, in the order the
/// program produced it. The grid only collects them; the host interprets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusSignal {
    /// An OSC 7501 body, the bytes after `7501;`, unvalidated.
    Report(String),
    /// A shell prompt started (OSC 133 or 633 `A`), which ends the records
    /// of the command before it.
    Prompt,
    /// A full reset (RIS), which removes every record.
    Reset,
}

fn is_boundary(signal: &StatusSignal) -> bool {
    !matches!(signal, StatusSignal::Report(_))
}

/// Removes the oldest report and, if that leaves two boundaries adjacent,
/// the later one, which then changes nothing.
fn evict_oldest_report(queue: &mut Vec<StatusSignal>) {
    let Some(i) = queue.iter().position(|s| !is_boundary(s)) else {
        return;
    };
    queue.remove(i);
    let merged = i > 0 && queue.get(i - 1).is_some_and(is_boundary);
    if merged && queue.get(i).is_some_and(is_boundary) {
        queue.remove(i);
    }
}

/// The authoritative screen buffer: a grid of [`Cell`]s held as one [`Line`]
/// per row, plus a per-row damage (`dirty`) tracker and a cursor position.
pub struct Grid {
    /// Number of columns.
    pub cols: usize,
    /// Number of rows.
    pub rows: usize,
    /// The live screen, top row first; `len() == rows`, and every line holds
    /// exactly `cols` cells. A scroll rotates rows instead of moving cells, and
    /// a line leaving the top moves whole into [`Grid::scrollback`]. Read it
    /// through [`Grid::row`].
    screen: Vec<Line>,
    /// Per-row damage flags; `len() == rows`. One consumer clears them
    /// ([`Grid::clear_dirty`]); [`Grid::row_stamp`] serves any number.
    pub dirty: Vec<bool>,
    /// Per-row change stamps (`len() == rows`): the [`Grid::stamp`] taken
    /// when the row last changed. Never cleared, so any number of observers
    /// can each ask "changed since I looked". Set wherever `dirty` is.
    row_stamp: Vec<u64>,
    /// Stamp of the last scrollback change (push, eviction, clear, reflow,
    /// retheme).
    hist_stamp: u64,
    /// The latest change stamp; strictly increases.
    stamp: u64,
    /// Process-unique id, so stamps from different grids never compare equal.
    id: u64,
    /// Cursor position as `(col, row)`, both zero-based.
    pub cursor: (usize, usize),
    /// Cursor position stashed by a save (`DECSC` / `CSI s`) and restored by
    /// `DECRC` / `CSI u`.
    pub saved_cursor: (usize, usize),
    /// Top row of the scrolling region (inclusive, 0-based).
    pub scroll_top: usize,
    /// Bottom row of the scrolling region (inclusive, 0-based).
    pub scroll_bottom: usize,
    /// When `Some`, the alternate screen is active and this holds the primary
    /// screen to restore on exit. Kept the same dimensions as the live buffer.
    primary: Option<SavedScreen>,
    /// The alternate screen's rows, kept after leaving it and blanked for
    /// reuse on the next entry, so an app that toggles the alternate screen
    /// does not allocate a row per switch. Empty until first use.
    spare_screen: Vec<Line>,
    /// One row of blank cells that [`Line::clear`] copies from, so blanking a
    /// row is a single `memcpy` instead of a per-cell store loop (the bulk of
    /// a line feed's cost). Uniform by construction; refreshed by
    /// [`Grid::prime_blank_row`] when the width or the fill cell changes, and
    /// emptied by every style sweep (it holds a style id).
    blank_row: Vec<PackedCell>,
    /// The distinct styles behind every [`PackedCell`] of this grid.
    styles: StyleTable,
    /// Monotonic counter bumped on every parsed batch; lets the renderer
    /// reason about frame freshness.
    pub epoch: u64,
    /// Window title last set by the child via OSC 0/2. The renderer forwards
    /// changes to the host terminal's title bar; empty until the child sets one.
    pub title: String,
    /// Titles saved by `CSI 22 t` (XTPUSHTITLE), most-recently-pushed last;
    /// `CSI 23 t` (XTPOPTITLE) restores from here. Lets a full-screen app
    /// (vim, tmux) set a working title and hand the caller's back on exit.
    /// Bounded by [`TITLE_STACK_MAX`] against a runaway script.
    title_stack: Vec<String>,
    /// Kitty keyboard protocol enhancement-flag stack (`CSI > flags u` push,
    /// `CSI < N u` pop, `CSI = flags ; mode u` set). The top entry (`0` when
    /// empty — legacy encoding) is what the windowed front-end's own key
    /// encoder (`gui/input.rs`) consults; TUI mode has a real host to relay
    /// these sequences to instead and never reads this. Bounded by
    /// [`KITTY_FLAGS_STACK_MAX`] against a runaway script.
    kitty_flags_stack: Vec<u8>,
    /// Working directory last reported by the child via OSC 7 (typically a
    /// `file://host/path` URI). Captured for future use (e.g. "open new tab in
    /// the same directory"); empty until reported.
    pub cwd: String,
    /// Lines that have scrolled off the top of the primary screen, oldest at the
    /// front. Bounded by [`SCROLLBACK_MAX`]. Each [`Line`] carries the cells plus
    /// its soft-wrap bit, so a resize can rejoin wrapped runs and re-wrap them to
    /// the new width. Rewrapped to the live width on every resize.
    pub(crate) scrollback: VecDeque<Line>,
    /// How many of the newest [`Grid::scrollback`] lines
    /// [`Grid::compact_history`] has not looked at yet. Clamped to the
    /// history length on use, so an eviction or clear needs no update.
    untrimmed: usize,
    /// The live scrollback cap: [`SCROLLBACK_MAX`] unless overridden by the
    /// `scrollback` config key via [`Grid::set_scrollback_max`].
    pub(crate) scrollback_max: usize,
    /// How many lines the viewport is scrolled up into [`Grid::scrollback`].
    /// `0` is the live view (bottom); the renderer composites history above the
    /// live grid when this is non-zero.
    pub view_offset: usize,
    /// Active text selection (windowed front-end only), in viewport cell
    /// coordinates; `None` when nothing is selected. Pure view state, like
    /// [`Grid::view_offset`] — the TUI renderer ignores it (the host owns
    /// selection there).
    pub selection: Option<Selection>,
    /// Active scrollback search (windowed front-end); `None` when not searching.
    /// Pure view state like [`Grid::selection`]; the TUI ignores it.
    #[cfg(any(test, feature = "gui"))]
    search: Option<Search>,
    /// Bytes destined for the *host* terminal (not the grid): OSC 52 clipboard
    /// requests forwarded verbatim. The renderer drains these via
    /// [`Grid::take_host_out`] each frame and writes them to its stdout.
    pub(crate) host_out: Vec<u8>,
    /// Interned hyperlink URIs from OSC 8; a [`Cell::link`] of `n` refers to
    /// `links[n - 1]` (`0` means no link). Append-only and bounded by
    /// [`LINK_MAX`], so ids stay stable for cells held in scrollback.
    pub(crate) links: Vec<String>,
    /// An `Arc` snapshot of `links`, rebuilt only in `intern_link` when a new
    /// URI is actually appended — every rendered frame (`snapshot_dirty`/
    /// `snapshot_viewport`) previously deep-cloned the whole `Vec<String>`
    /// (each frame, forever, even though the table almost never changes
    /// between frames); cloning this `Arc` instead is an `O(1)` refcount bump.
    pub(crate) links_arc: Arc<[String]>,
    /// Interned grapheme-continuation strings; a [`Cell::cluster`] of `n` refers
    /// to `clusters[n - 1]` (`0` means a lone `ch`). Append-only and bounded by
    /// [`CLUSTER_MAX`], so ids stay stable for cells held in scrollback.
    pub clusters: Vec<String>,
    /// `Arc` snapshot of `clusters`, mirroring `links_arc` but built on the
    /// first frame after the table grows, not on every insert: a burst of new
    /// clusters between two frames costs one copy of the table, not one each.
    clusters_arc: OnceLock<Arc<[String]>>,
    /// Id of each `clusters` entry, so interning a combining mark is one hash
    /// lookup instead of a scan of the table.
    cluster_ids: HashMap<String, u16>,
    /// Reused buffer for the grown continuation in `append_to_glyph`, so a
    /// mark whose continuation is already interned allocates nothing.
    cluster_scratch: String,
    /// The hyperlink id stamped onto cells written while an OSC 8 link is open
    /// (`0` when none). Set by the parser via [`Grid::set_link`].
    current_link: u16,
    /// Columns at which a horizontal tab stops; `len() == cols`. Defaults to
    /// every 8th column and is modified by `HTS` / `TBC`. A resize preserves
    /// stops within the surviving width and defaults the new columns.
    tab_stops: Vec<bool>,
    /// Whether the text cursor is visible (DECTCEM `?25`, default on). The
    /// renderer shows/hides the host cursor accordingly — independent of the
    /// separate hide it applies while browsing scrollback.
    pub cursor_visible: bool,
    /// Whether the child has an active synchronized-output window open (DEC
    /// `?2026`, `CSI ?2026h` .. `CSI ?2026l`): while true, render-loop wakeups
    /// are suppressed so a multi-write frame update never paints half-drawn.
    /// See [`Grid::sync_output_active`] for the read side (which also expires
    /// a window that's been open too long).
    sync_output: bool,
    /// When `sync_output` was last set, for the timeout in
    /// [`Grid::sync_output_active`]. `None` when `sync_output` is false.
    pub(crate) sync_output_since: Option<std::time::Instant>,
    /// Whether bracketed paste (DEC `?2004`) is enabled by the child. In TUI
    /// mode it is also relayed to the host; the windowed front-end reads it to
    /// decide whether to wrap pasted text in `ESC[200~` / `ESC[201~`.
    pub bracketed_paste: bool,
    /// Whether DECCKM (application cursor keys, DEC `?1`) is enabled by the
    /// child. In TUI mode the mode is also relayed to the host, which does
    /// its own arrow-key encoding; the windowed front-end has no host to
    /// relay to, so its native key encoder reads this directly to choose
    /// `CSI` (normal) vs `SS3` (application) arrow sequences.
    pub app_cursor_keys: bool,
    /// Whether alternate scroll mode (DEC `?1007`) is enabled. When on *and*
    /// the alternate screen is active, the windowed front-end translates
    /// mouse-wheel scrolling to Up/Down (or Page Up/Down) key presses instead
    /// of browsing rusty_term's own scrollback — lets the wheel drive `less`/
    /// `man`/other pagers that never registered native mouse support.
    pub alt_scroll: bool,
    /// Pending BEL (C0 `0x07`) ring, set by the parser and drained by the
    /// windowed front-end (which raises a window-attention request and a tab
    /// badge); the TUI relays the byte to the host instead. A boolean, so a
    /// burst of bells coalesces into one alert.
    pub bell: bool,
    /// Stored Kitty images (`a=t`/`a=T` with an id), bounded; each carries
    /// its animation frames and playback state.
    pub kitty_images: Vec<KittyImage>,
    /// Counter behind the synthesized `0xFFFF_xxxx` ids that inline animated
    /// GIFs store their frames under (see `render_animated_image`).
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    next_anim_id: u32,
    /// Virtual Kitty placements (`U=1`): `(image id, cols, rows)` — the
    /// geometry `U+10EEEE` placeholder cells render against.
    pub(crate) kitty_virtual: Vec<(u32, usize, usize)>,
    /// ConEmu OSC `9;4` progress state as `(state, percent)`: state `1`
    /// normal, `2` error, `3` indeterminate, `4` warning/paused; `None` when
    /// cleared (state `0` or unknown). The windowed front-end surfaces it in
    /// the tab strip; the TUI relays the sequence to the host.
    pub progress: Option<(u8, u8)>,
    /// Whether the child asked for unsolicited color-scheme reports
    /// (DEC mode 2031, Contour's dark/light extension): when the terminal's
    /// appearance flips, it sends `CSI ? 997 ; 1|2 n`.
    pub report_color_scheme: bool,
    /// Minimum WCAG contrast ratio enforced between every cell's fg and bg
    /// at render time (`minimum_contrast` config); `1.0` (default) disables.
    /// Lives on the grid so both renderers read one knob without new
    /// plumbing, like the cursor-style defaults.
    pub min_contrast: f32,
    /// Implicit bidi (`bidi = "auto"`): render-time UAX #9 reordering of
    /// RTL-containing rows. Off by default; the storage model stays logical
    /// order regardless (see `src/core/bidi.rs`).
    #[cfg(any(test, feature = "gui"))]
    pub bidi: bool,
    /// BDSM (ECMA-48 mode 8): `Some(true)` = the app asked for implicit
    /// bidi, `Some(false)` = explicit (app does its own layout), `None` =
    /// never set (defaults apply: implicit on the main screen, explicit on
    /// the alternate screen, per the Terminal-WG recommendation).
    pub(crate) bdsm: Option<bool>,
    /// Paragraph-direction autodetection (Terminal-WG private mode 2501;
    /// default on = per-line first-strong scan).
    pub(crate) bidi_autodetect: bool,
    /// Fixed paragraph direction while autodetect is off (ECMA-48 SCP):
    /// `Some(true)` RTL, `Some(false)`/`None` LTR.
    pub(crate) bidi_para_rtl: Option<bool>,
    /// In-flight multi-part OSC 99 notifications, keyed by the client's `i=`
    /// identifier: `(id, title, body)` accumulating until a `d=1` (done)
    /// part finalizes into [`Grid::notifications`]. Bounded.
    notif99_pending: Vec<(String, String, String)>,
    /// When the running command's output began (OSC 133/633 `C`), for
    /// measuring how long a command ran (windowed front-end "command
    /// finished" notifications).
    #[cfg(any(test, feature = "gui"))]
    command_began: Option<std::time::Instant>,
    /// Commands that finished (OSC 133/633 `D`) since last drained:
    /// `(exit code, runtime)`. The windowed front-end drains these to notify
    /// about long commands finishing in an unfocused window; bounded so an
    /// undrained grid (TUI mode) never grows it.
    #[cfg(any(test, feature = "gui"))]
    pub finished_commands: Vec<(Option<i32>, std::time::Duration)>,
    /// Whether focus reporting (DEC `?1004`) is enabled by the child. In TUI
    /// mode the mode is also relayed to the host, which generates the actual
    /// `CSI I`/`CSI O` reports; the windowed front-end has no host, so it
    /// reads this directly and reports its own `WindowEvent::Focused`
    /// transitions to the child.
    pub focus_reporting: bool,
    /// Whether win32-input-mode (DEC `?9001`, microsoft/terminal) is enabled
    /// by the child. The windowed front-end's key encoder reads this to send
    /// full key records (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`, press and release)
    /// instead of VT sequences; TUI mode relays the mode to the host.
    pub win32_input: bool,
    /// Whether application keypad mode (DECKPAM `ESC =` / DECNKM DEC `?66`)
    /// is enabled. The windowed front-end's key encoder reads this to encode
    /// numpad keys as `SS3 p`–`SS3 y` (and friends) instead of their plain
    /// characters; TUI mode relays the sequence to the host, which does its
    /// own keypad encoding.
    pub app_keypad: bool,
    /// Whether ECMA-48 Line/New Line mode (LNM, ANSI mode `20`) is enabled.
    /// Reset (the default, matching xterm) means a bare LF only moves the
    /// cursor down a line; set means it also returns to column 0, as if it
    /// were a CR+LF. Most programs send an explicit `\r\n` and never depend
    /// on this, but some legacy line-oriented tools assume the reset
    /// behavior and manage the column themselves.
    pub(crate) line_feed_new_line: bool,
    /// Mouse reporting enabled by the child (DECSET `?1000`/`?1002`/`?1003`),
    /// plus extended-format bits (`?1006`/`?1015`/`?1016`). The window backend
    /// uses this to route clicks/drags/scrolls back to the child as encoded
    /// input bytes instead of handling them locally.
    pub mouse_modes: MouseModes,
    /// One cell's size in physical pixels `(width, height)`, set by the
    /// windowed front-end from its font metrics; `None` in TUI mode (no real
    /// pixels are ours to report) or before the first frame. Answers XTWINOPS
    /// `14t`/`16t` pixel-size queries — `18t` (text-area size in *cells*)
    /// needs only `cols`/`rows`, always known.
    pub cell_px: Option<(u16, u16)>,
    /// Mouse pointer shape last requested by the child via OSC 22, as a CSS
    /// `cursor` keyword (`"text"`, `"pointer"`, …); `None` means the default
    /// arrow. Read (not drained — it's persistent state, not a one-shot
    /// event) by the windowed front-end while the pointer is over pane
    /// content. TUI mode has no pointer of its own to change.
    pub cursor_icon: Option<String>,
    /// Text the child asked to place on the system clipboard via OSC 52 set,
    /// pending pickup by the window backend (which owns the clipboard); `None`
    /// when nothing is pending. The TUI relays OSC 52 to the host and ignores it.
    pub clipboard_set: Option<String>,
    /// Like [`Grid::clipboard_set`] but targeting the PRIMARY selection
    /// (OSC 52 with a `p` selection argument). Serviced on X11/Wayland;
    /// elsewhere it falls back to the regular clipboard.
    pub clipboard_set_primary: Option<String>,
    /// Set when the child queried the clipboard (`OSC 52 ; … ; ?`); the window
    /// backend answers from the system clipboard and clears it.
    pub clipboard_query: bool,
    /// Like [`Grid::clipboard_query`] but for the PRIMARY selection.
    pub clipboard_query_primary: bool,
    /// Desktop notifications (OSC 9 / OSC 777) the child requested, drained by
    /// the windowed front-end which raises them via the OS; the TUI relays the
    /// OSC to the host. Each entry is `(title, body)`; an empty title means the
    /// front-end picks a default. Bounded by [`Grid::push_notification`].
    pub notifications: Vec<(String, String)>,
    /// Whether to queue [`StatusSignal`]s in [`Grid::program_status`]. Off by
    /// default, so a host that never drains them stores nothing. When on,
    /// the parser answers the OSC 7501 `?` query for the host, claiming all
    /// of draft 0.4's states and kinds, so set it only in a host that
    /// handles them all.
    pub collect_program_status: bool,
    /// Program-status signals (OSC 7501 reports and the events that end
    /// records' lifetimes), in arrival order, for a host such as a
    /// multiplexer to drain. Bounded by [`Grid::push_program_status`].
    pub program_status: Vec<StatusSignal>,
    /// The active IME composition (preedit) text, shown reverse-video at the
    /// cursor by the windowed front-end; empty when not composing. Set by the
    /// window backend on `WindowEvent::Ime`, not by the parser.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub ime_preedit: String,
    /// Total lines ever scrolled into history — a stable serial anchoring
    /// [`GridImage`]s so they track scroll/scrollback (windowed front-end only).
    #[cfg(any(test, feature = "gui"))]
    pub(crate) total_scrolled: usize,
    /// Pixel images (Sixel/Kitty) drawn over their reserved half-block cells by
    /// the windowed CPU renderer; the TUI and GPU use the half-block cells.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) images: Vec<GridImage>,
    /// Whether autowrap (DECAWM `?7`, default on) is enabled. When off, a glyph
    /// printed at the right margin overwrites the last column instead of
    /// wrapping to the next line.
    pub(crate) autowrap: bool,
    /// DEC private mode 2027 (grapheme clustering). Off by default: widths
    /// follow wcwidth one code point at a time, as the programs laying out the
    /// screen assume. When on, a whole UAX #29 grapheme shares one cell.
    pub(crate) grapheme_clustering: bool,
    /// DEC private mode 40: allow DECCOLM (80/132 column switching). Off by
    /// default; while off, DECCOLM is ignored.
    pub(crate) allow_deccolm: bool,
    /// DEC private mode 3 (DECCOLM), as last set while mode 40 allowed it.
    /// Reported by DECRQM only: the grid is never resized for it (see
    /// [`Grid::deccolm`]).
    pub(crate) column_mode_132: bool,
    /// Whether origin mode (DECOM `?6`, default off) is enabled. When on,
    /// absolute cursor positioning (`CUP`/`HVP`/`VPA`) is relative to the scroll
    /// region top and the cursor is confined to the region.
    pub(crate) origin_mode: bool,
    /// DECLRMM (DEC private mode 69): whether left/right margins are in
    /// effect (and whether `CSI Pl;Pr s` means DECSLRM instead of SCP).
    pub(crate) lr_margin_mode: bool,
    /// DECSLRM left margin, 0-based inclusive. Only meaningful while
    /// [`Grid::lr_margin_mode`] is set.
    pub(crate) left_margin: usize,
    /// DECSLRM right margin, 0-based inclusive.
    pub(crate) right_margin: usize,
    /// Whether insert mode (IRM, ANSI mode `4`, default off) is enabled. When
    /// on, a printed glyph shifts the rest of the row right instead of
    /// overwriting.
    pub(crate) insert_mode: bool,
    /// Background color erases fill with (BCE): the parser's SGR background,
    /// or `None` while that is the default, so the fill tracks OSC 11. Kept in
    /// sync by the parser via [`Grid::set_erase_bg`].
    erase_bg: Option<u32>,
    /// Cursor color (OSC 12 / the `cursor` config key), mirrored from the
    /// parser's palette like the defaults above. The windowed renderers paint
    /// the block cursor in it; the TUI host owns its own cursor.
    pub cursor_color: u32,
    /// The cursor's rendered shape (DECSCUSR `CSI Ps SP q` or the `cursor_style`
    /// config key). The windowed renderers draw it; the TUI relays DECSCUSR to
    /// the host, which owns its own cursor.
    pub cursor_shape: CursorShape,
    /// Whether the cursor blinks (DECSCUSR odd params / the `cursor_blink` config
    /// key). The windowed event loop animates it; the TUI relays it to the host.
    pub cursor_blink: bool,
    /// The power-on cursor shape/blink (from config), restored by RIS/DECSTR.
    default_cursor_shape: CursorShape,
    default_cursor_blink: bool,
    /// Logical line indices (counting from the oldest retained scrollback line)
    /// of shell prompt starts reported via OSC 133;A, kept sorted. Powers
    /// prompt-to-prompt scrollback navigation. Bounded by [`PROMPT_MARKS_MAX`].
    prompt_marks: Vec<usize>,
    /// Absolute logical line (`scrollback.len() + cursor row`) where the
    /// running command's output began (OSC 133/633;C), pending the matching
    /// `D` to close it into a [`CommandBlock`] — independent of the `l13`
    /// feature's own (separately tracked) capture-for-MCP anchor.
    #[cfg(any(test, feature = "gui"))]
    fold_pending_start: Option<usize>,
    /// Finished commands' output ranges (OSC 133/633;C..D), foldable to one
    /// summary line in the windowed front-end's scrollback view. Bounded by
    /// [`FOLD_BLOCKS_MAX`]; remapped across a resize like a prompt mark.
    #[cfg(any(test, feature = "gui"))]
    fold_blocks: Vec<CommandBlock>,
    /// Structured side-channel (L13) session state: the resource subscriptions a
    /// connected client has registered for change notifications. Carried here
    /// because it must outlive any single channel OSC and ride alongside the grid
    /// state whose changes it reports.
    #[cfg(feature = "l13")]
    pub(crate) channel: rusty_term_l13::ChannelState,
    /// Optional status-line overlay set via the L13 `render` protocol; composited
    /// over the bottom row by every renderer. `None` (no allocation) when unset.
    status_line: Option<StatusLine>,
    /// Exit code of the last finished command, reported via OSC 133;D. Surfaced
    /// to a structured client as the `terminal://exit` resource and shown by
    /// the windowed front-end's status ribbon; `None` until a command
    /// finishes (or when the shell omits the code). Written by every build
    /// (the OSC 133;D path), read only by the `gui`/`l13` fronts.
    #[cfg_attr(not(any(feature = "gui", feature = "l13")), allow(dead_code))]
    last_exit: Option<i32>,
    /// Absolute logical line (`scrollback.len() + cursor row`) where the running
    /// command's output began (OSC 133;C), or `None` outside a command. Decays
    /// with scrollback eviction and is remapped across a resize (it rides the
    /// reflow like a prompt mark), so a mid-command resize keeps the capture.
    #[cfg(feature = "l13")]
    command_start: Option<usize>,
    /// Text of the last finished command's output, captured at OSC 133;D and
    /// surfaced as the `terminal://command` resource.
    #[cfg(feature = "l13")]
    last_command_output: Option<String>,
}

/// Build the default tab-stop table for a `cols`-wide grid: a stop at every
/// 8th column (0, 8, 16, …), matching the classic 8-column default.
fn default_tab_stops(cols: usize) -> Vec<bool> {
    (0..cols).map(|i| i % 8 == 0).collect()
}

/// Per-row size attribute set by `ESC # 3/4/5/6` (DECDHL/DECDWL/DECSWL). A
/// double-width or double-height line renders at twice the cell width, so only
/// the left half of the columns is displayed; the renderer relays the attribute
/// to the host terminal, which does the scaling.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineAttr {
    /// Normal single-width, single-height (`ESC # 5`, the default).
    #[default]
    Single,
    /// Double-width, single-height (`ESC # 6`).
    DoubleWidth,
    /// Top half of a double-width, double-height line (`ESC # 3`).
    DoubleTop,
    /// Bottom half of a double-width, double-height line (`ESC # 4`).
    DoubleBottom,
}

/// Which DEC private mode selected the alternate screen, which determines the
/// cursor save/restore behaviour on exit (per xterm: only `1049` does it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AltMode {
    /// `?47` — bare buffer swap, no cursor save/restore.
    Dec47,
    /// `?1047` — buffer swap, no cursor save/restore.
    Dec1047,
    /// `?1049` — save cursor on entry (DECSC-style), restore on exit.
    Dec1049,
}

/// Active mouse-reporting mode envelope kept on the grid. The window backend
/// consults this to decide whether a click/drag/scroll is forwarded to the
/// child as an encoded input sequence or handled locally (selection, scroll).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MouseModes {
    /// Base mode: click (`1000`), drag (`1002`), any-event (`1003`).
    pub base: usize,
    /// Extended encoding flags combined from `1005`/`1006`/`1015`/`1016`.
    pub extended: u8,
}

impl MouseModes {
    /// Whether any mouse reporting is active. Consumed by the windowed
    /// front-end's mouse path (and tests); the TUI relays mouse modes to the
    /// host instead of acting on them.
    #[cfg(any(test, feature = "gui"))]
    pub fn active(self) -> bool {
        self.base != 0
    }
}

/// The cursor's rendered shape, set by DECSCUSR (`CSI Ps SP q`) or the
/// `cursor_style` config key.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    /// Filled block covering the whole cell (the default).
    #[default]
    Block,
    /// A horizontal bar along the bottom of the cell.
    Underline,
    /// A vertical bar (I-beam) at the left of the cell.
    Bar,
}

/// Map a DEC private parameter to its alternate-screen mode, if any.
pub(crate) fn alt_mode(param: usize) -> Option<AltMode> {
    match param {
        47 => Some(AltMode::Dec47),
        1047 => Some(AltMode::Dec1047),
        1049 => Some(AltMode::Dec1049),
        _ => None,
    }
}

/// A stashed screen buffer plus its cursor, used to swap the primary screen out
/// while the alternate screen is active.
struct SavedScreen {
    screen: Vec<Line>,
    cursor: (usize, usize),
    /// The DECSC/`CSI s` register at the time of the switch, kept separate from
    /// the alternate screen's so the two buffers don't share one save slot.
    saved_cursor: (usize, usize),
    /// The mode that activated the alternate screen, governing exit behaviour.
    mode: AltMode,
}

/// A process-unique [`Grid`] id.
fn next_grid_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A `rows`-high screen of [`Line::blank`] rows.
fn blank_screen(cols: usize, rows: usize, fill: PackedCell) -> Vec<Line> {
    (0..rows).map(|_| Line::blank(cols, fill)).collect()
}

/// Clip/extend the screen `old` into a fresh `cols`×`rows` screen, preserving
/// the top-left overlap and blank-filling any new area. Used for the
/// *alternate* screen, whose full-screen apps repaint on resize — there is no
/// logical-line history there to rejoin, so a plain clip is correct.
fn reflow_clip(old: &[Line], cols: usize, rows: usize, blank: PackedCell) -> Vec<Line> {
    let mut new = blank_screen(cols, rows, blank);
    for (dst, src) in new.iter_mut().zip(old) {
        let n = cols.min(src.cells().len());
        dst.cells_mut()[..n].copy_from_slice(&src.cells()[..n]);
    }
    new
}

/// A cell that contributes nothing to a logical line's content — a plain space
/// in default rendition with no link or grapheme tail. Trailing runs of these
/// are trimmed when rejoining wrapped rows so re-wrapping doesn't preserve dead
/// padding. `flags == 0` excludes both [`WIDE_TRAILER`] and any SGR attribute,
/// so a colored/attributed blank is *kept* (its background is content).
fn is_padding(c: &Cell) -> bool {
    c.ch == ' ' && c.cluster == 0 && c.link == 0 && c.flags & !ATTR_DEFAULT_COLORS == 0
}

/// [`is_padding`] for a stored cell, through its style.
fn is_padding_packed(c: PackedCell, styles: &StyleTable) -> bool {
    c.ch() == ' ' && is_padding(&c.resolve(&styles.get(c.style())))
}

/// Store a history line's trailing run of one padding cell as a single
/// cell and a repeat count.
/// Only a hard-broken line: the padding of a soft-wrapped one is part of
/// the logical line that continues below it. A reflow drops this padding
/// anyway, so it never sees the difference.
fn trim_padding(line: &mut Line, styles: &StyleTable) {
    let cells = line.cells();
    let Some(&last) = cells.last() else {
        return;
    };
    if line.wrapped || !is_padding_packed(last, styles) {
        return;
    }
    let keep = cells.iter().rposition(|c| *c != last).map_or(0, |i| i + 1);
    line.trim_tail(keep);
}

/// A pixel image (Sixel/Kitty) placed on the grid: source pixels plus the cell
/// footprint reserved for it, anchored by `serial` (its top cell row's stable
/// scroll position) so it tracks scrollback. Used by the CPU renderer only.
#[cfg(any(test, feature = "gui"))]
pub struct GridImage {
    pub serial: usize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub pw: usize,
    pub ph: usize,
    /// Source pixels, row-major `pw × ph`; `None` = transparent (skipped).
    /// Shared so a frame snapshot can hold them without copying.
    pub pixels: Arc<[Option<u32>]>,
    /// For animated images (inline GIF): the backing [`KittyImage`] id whose
    /// *current frame* renderers draw instead of `pixels` (same `pw × ph`).
    /// `pixels` stays as the first frame, the fallback if the store evicts it.
    pub anim: Option<u32>,
}

/// One viewport row's bidi view (see [`Grid::bidi_row`]): the visual→logical
/// and logical→visual cell permutations plus which logical cells sit in an
/// RTL (odd-level) run — those draw their glyph's mirrored form (rule L4).
#[cfg(any(test, feature = "gui"))]
#[derive(Clone, PartialEq)]
pub struct BidiRow {
    pub vis2log: Vec<u16>,
    pub log2vis: Vec<u16>,
    pub rtl: Vec<bool>,
    /// Arabic contextual forms (phase 3), per **logical** cell: `Some(ch)`
    /// substitutes the presentation-form glyph for that cell's base char.
    /// `None` (whole field) when the row has nothing to shape.
    pub shaped: Option<Vec<Option<char>>>,
}

/// The product of a wrap-aware reflow: the rebuilt history, the new screen,
/// the relocated cursor, and the remapped prompt marks.
struct Reflowed {
    scrollback: VecDeque<Line>,
    screen: Vec<Line>,
    cursor: (usize, usize),
    prompt_marks: Vec<usize>,
    /// Remapped command-capture anchor; only consumed under the `l13` feature.
    #[cfg_attr(not(feature = "l13"), allow(dead_code))]
    command_start: Option<usize>,
    /// Remapped fold-block state; only consumed under `test`/`gui`.
    #[cfg_attr(not(any(test, feature = "gui")), allow(dead_code))]
    fold_pending_start: Option<usize>,
    #[cfg_attr(not(any(test, feature = "gui")), allow(dead_code))]
    fold_blocks: Vec<CommandBlock>,
}

/// Re-wrap the combined history `scrollback ++ screen` into `new_cols`×`new_rows`.
///
/// The buffer is a sequence of physical rows; consecutive rows joined by the
/// soft-wrap bit form one *logical line*. We flatten those runs back into
/// logical lines (dropping the one-column filler a wide glyph leaves when it
/// wraps, and trimming trailing padding), re-wrap each to the new width without
/// splitting a double-width glyph across the margin, then re-window the result so
/// the cursor's line stays on screen. The cursor and the OSC-133 prompt marks
/// (physical-row indices into the old `scrollback ++ screen`) are carried through
/// by tracking, respectively, the cursor's offset within its logical line and a
/// physical-row → logical-line → new-physical-row mapping.
#[allow(clippy::too_many_arguments)]
fn reflow_history(
    scrollback: VecDeque<Line>,
    screen: Vec<Line>,
    old_cols: usize,
    cursor: (usize, usize),
    prompt_marks: &[usize],
    command_start: Option<usize>,
    fold_pending_start: Option<usize>,
    fold_blocks: &[CommandBlock],
    new_cols: usize,
    new_rows: usize,
    blank: PackedCell,
    scrollback_max: usize,
    styles: &StyleTable,
) -> Reflowed {
    // Whether a cell pads is resolved once per run of equal cells, not per
    // cell: trimming walks every trailing blank of every line.
    let last = std::cell::Cell::new(None::<(PackedCell, bool)>);
    let is_padding = |c: &PackedCell| {
        if let Some((cell, pads)) = last.get()
            && cell == *c
        {
            return pads;
        }
        let pads = is_padding_packed(*c, styles);
        last.set(Some((*c, pads)));
        pads
    };
    let hist = scrollback.len();
    let old_total = hist + screen.len();
    let cursor_phys = hist + cursor.1.min(screen.len().saturating_sub(1));
    // Borrow physical row `i` as (cells, wrapped, line attr). Scrollback lines
    // have no size attribute (a double-width line is meaningless once scrolled
    // off) and may be narrower than `old_cols`; both are handled by the joiner.
    let phys = |i: usize| -> (&[PackedCell], bool, LineAttr) {
        if i < hist {
            (
                scrollback[i].cells(),
                scrollback[i].wrapped,
                LineAttr::Single,
            )
        } else {
            let line = &screen[i - hist];
            (line.cells(), line.wrapped, line.attr)
        }
    };

    // --- Phase 1: flatten physical rows into logical lines. ---
    // Every logical line's cells go into one buffer, `flat`; `logical` holds
    // each line's range in it.
    let cells_total = (0..old_total).map(|i| phys(i).0.len()).sum();
    let mut flat: Vec<PackedCell> = Vec::with_capacity(cells_total);
    let mut logical: Vec<std::ops::Range<usize>> = Vec::new();
    let mut logical_attr: Vec<LineAttr> = Vec::new();
    let mut phys_to_logical: Vec<usize> = Vec::with_capacity(old_total);
    let mut cur_start = 0usize;
    let mut cur_attr = LineAttr::Single;
    let mut cur_started = false;
    let mut cursor_logical = 0usize;
    let mut cursor_off = 0usize;
    // Close the open logical line, trimming trailing padding but never past
    // the cursor's column on the cursor's own line.
    let close = |flat: &mut Vec<PackedCell>, start: usize, min_keep: usize| {
        let mut end = flat.len();
        while end > start + min_keep && is_padding(&flat[end - 1]) {
            end -= 1;
        }
        flat.truncate(end);
        start..end
    };
    for i in 0..old_total {
        let (cells, wrapped, attr) = phys(i);
        let li = logical.len(); // index this physical row's logical line will get
        phys_to_logical.push(li);
        if !cur_started {
            cur_attr = attr;
            cur_start = flat.len();
            cur_started = true;
        }
        if i == cursor_phys {
            cursor_logical = li;
            cursor_off = flat.len() - cur_start + cursor.0.min(old_cols);
        }
        // Append this row's content. A wrapped row that ends in one padding cell
        // because a wide glyph didn't fit drops that filler — detected precisely
        // by the next row beginning with a width-2 glyph.
        let mut take = cells.len();
        if wrapped && take > 0 && is_padding(&cells[take - 1]) {
            let next_wide = i + 1 < old_total && {
                let (nc, _, _) = phys(i + 1);
                !nc.is_empty() && char_width(nc[0].ch()) == 2
            };
            if next_wide {
                take -= 1;
            }
        }
        flat.extend_from_slice(&cells[..take]);
        if wrapped {
            continue; // logical line continues on the next physical row
        }
        let min_keep = if li == cursor_logical { cursor_off } else { 0 };
        logical.push(close(&mut flat, cur_start, min_keep));
        logical_attr.push(cur_attr);
        cur_started = false;
    }
    if cur_started {
        // A trailing wrapped run with no closing hard break (shouldn't normally
        // happen, since a wrap always creates the next row): keep it anyway.
        let min_keep = if logical.len() == cursor_logical {
            cursor_off
        } else {
            0
        };
        logical.push(close(&mut flat, cur_start, min_keep));
        logical_attr.push(cur_attr);
    }
    // The old rows' cell buffers, reused for the new rows below.
    let mut spare: Vec<Vec<PackedCell>> = scrollback
        .into_iter()
        .chain(screen)
        .map(Line::into_cells)
        .collect();

    // Drop blank logical lines trailing *below* the cursor: those are the screen's
    // empty tail (padding under the last output), not history. Lines at or above
    // the cursor — including intentional mid-content blank lines — are kept, so a
    // resize doesn't sweep real content into scrollback just to fill the window.
    while logical.len() > cursor_logical + 1 && logical.last().is_some_and(|l| l.is_empty()) {
        logical.pop();
        logical_attr.pop();
    }

    // --- Phase 2: split each logical line into rows of the new width. ---
    // Only the split is computed here: a row is its cell range in `flat`
    // (empty for a blank row). Phase 3 builds just the rows it keeps.
    let mut out_cells: Vec<std::ops::Range<usize>> = Vec::new();
    let mut out_wrapped: Vec<bool> = Vec::new();
    let mut out_attr: Vec<LineAttr> = Vec::new();
    let mut logical_first_row: Vec<usize> = vec![0; logical.len()];
    let mut ncur = (0usize, 0usize);
    let mut cursor_set = false;
    for (li, span) in logical.iter().enumerate() {
        let line = &flat[span.clone()];
        logical_first_row[li] = out_cells.len();
        let attr = logical_attr[li];
        if line.is_empty() {
            if li == cursor_logical && !cursor_set {
                ncur = (out_cells.len(), 0);
                cursor_set = true;
            }
            out_cells.push(span.start..span.start);
            out_wrapped.push(false);
            out_attr.push(attr);
            continue;
        }
        let mut i = 0;
        let mut seg_start = 0;
        while i < line.len() {
            let mut take = new_cols.min(line.len() - i);
            // Don't cut a double-width glyph from its trailer: if the cell just
            // past the segment is a trailer, push its head to the next row.
            if i + take < line.len() && line[i + take].is_wide_trailer() && take > 1 {
                take -= 1;
            }
            if take == 0 {
                take = 1; // degenerate new_cols == 1 vs a wide glyph: force progress
            }
            let n = take.min(new_cols);
            seg_start = i;
            if li == cursor_logical && !cursor_set && cursor_off >= i && cursor_off < i + take {
                ncur = (out_cells.len(), (cursor_off - i).min(new_cols - 1));
                cursor_set = true;
            }
            let more = i + take < line.len();
            out_cells.push(span.start + i..span.start + i + n);
            out_wrapped.push(more);
            out_attr.push(attr);
            i += take;
        }
        if li == cursor_logical && !cursor_set {
            // Cursor sat at or past the last cell of its (trimmed) line.
            let col = (cursor_off - seg_start).min(new_cols - 1);
            ncur = (out_cells.len() - 1, col);
            cursor_set = true;
        }
    }
    let total = out_cells.len();
    if !cursor_set {
        ncur = (total.saturating_sub(1), 0);
    }

    // --- Phase 3: re-window into screen + scrollback, keeping the cursor visible. ---
    let mut screen_top = total.saturating_sub(new_rows);
    if ncur.0 < screen_top {
        screen_top = ncur.0; // cursor would be above the window: pull the window up
    }
    let dropped = screen_top.saturating_sub(scrollback_max);
    // Rows past the scrollback cap and below the cursor's window are never built.
    let mut row = |r: usize| -> Vec<PackedCell> {
        let mut cells = spare.pop().unwrap_or_default();
        cells.clear();
        cells.extend_from_slice(&flat[out_cells[r].clone()]);
        cells.resize(new_cols, blank);
        cells
    };
    let new_scrollback: VecDeque<Line> = (dropped..screen_top)
        .map(|r| Line::new(row(r), out_wrapped[r], LineAttr::Single))
        .collect();
    let mut new_screen: Vec<Line> = Vec::with_capacity(new_rows);
    new_screen.extend(
        (screen_top..total.min(screen_top + new_rows))
            .map(|r| Line::new(row(r), out_wrapped[r], out_attr[r])),
    );
    new_screen.resize_with(new_rows, || Line::blank(new_cols, blank));
    let cy = ncur.0.saturating_sub(screen_top).min(new_rows - 1);
    let cx = ncur.1.min(new_cols - 1);

    // --- Phase 4: remap line indices (old physical row → logical → new row). ---
    // A physical row maps to the first new row of its logical line, minus the
    // scrollback dropped off the front; a row past the kept logical lines (a
    // dropped trailing blank) or below the cap has no image.
    let remap = |m: usize| -> Option<usize> {
        if m < phys_to_logical.len() && phys_to_logical[m] < logical_first_row.len() {
            let outrow = logical_first_row[phys_to_logical[m]];
            (outrow >= dropped).then(|| outrow - dropped)
        } else {
            None
        }
    };
    let mut new_marks: Vec<usize> = prompt_marks.iter().filter_map(|&m| remap(m)).collect();
    new_marks.sort_unstable();
    new_marks.dedup();
    if new_marks.len() > PROMPT_MARKS_MAX {
        new_marks.drain(0..new_marks.len() - PROMPT_MARKS_MAX);
    }
    let new_command_start = command_start.and_then(remap);
    let new_fold_pending_start = fold_pending_start.and_then(remap);
    // `end` is exclusive; remap the block's last physical row and step one
    // past it, so a block ending exactly at a dropped/merged boundary still
    // remaps instead of vanishing. A block that no longer spans any rows
    // (start == end after remapping) is dropped — nothing left to fold.
    let mut new_fold_blocks: Vec<CommandBlock> = fold_blocks
        .iter()
        .filter_map(|b| {
            let start = remap(b.start)?;
            let end = remap(b.end.saturating_sub(1))? + 1;
            (start < end).then_some(CommandBlock {
                start,
                end,
                folded: b.folded,
                exit: b.exit,
                runtime: b.runtime,
            })
        })
        .collect();
    if new_fold_blocks.len() > FOLD_BLOCKS_MAX {
        new_fold_blocks.drain(0..new_fold_blocks.len() - FOLD_BLOCKS_MAX);
    }

    Reflowed {
        scrollback: new_scrollback,
        screen: new_screen,
        cursor: (cx, cy),
        prompt_marks: new_marks,
        command_start: new_command_start,
        fold_pending_start: new_fold_pending_start,
        fold_blocks: new_fold_blocks,
    }
}

/// A terminal-owned status-line overlay (L13 `render` protocol): one row of
/// pre-rendered cells the renderer composites over the bottom of the live
/// screen, independent of the child's output stream. Kept pre-rendered so the
/// hot render path just borrows the slice; re-laid from `text`/`fg`/`bg` on a
/// resize.
pub(crate) struct StatusLine {
    text: String,
    fg: u32,
    bg: u32,
    cells: Vec<Cell>,
}

impl StatusLine {
    /// Lay `text` out into exactly `cols` cells in `fg`/`bg`: advance wide glyphs
    /// by two columns with a flagged trailer, drop zero-width scalars, stop at the
    /// margin, and pad the tail with blanks in `bg`.
    fn lay_out(text: &str, fg: u32, bg: u32, cols: usize) -> Vec<Cell> {
        let blank = Cell {
            ch: ' ',
            cluster: 0,
            fg,
            bg,
            flags: 0,
            link: 0,
            underline_color: fg,
        };
        let mut cells = vec![blank; cols];
        let mut x = 0;
        for ch in text.chars() {
            let w = char_width(ch);
            if w == 0 {
                continue;
            }
            if x + w > cols {
                break;
            }
            cells[x] = Cell {
                ch,
                cluster: 0,
                fg,
                bg,
                flags: 0,
                link: 0,
                underline_color: fg,
            };
            if w == 2 {
                cells[x + 1] = Cell {
                    ch: ' ',
                    cluster: 0,
                    fg,
                    bg,
                    flags: WIDE_TRAILER,
                    link: 0,
                    underline_color: fg,
                };
            }
            x += w;
        }
        cells
    }

    #[cfg(feature = "l13")]
    fn new(text: String, fg: u32, bg: u32, cols: usize) -> Self {
        let cells = Self::lay_out(&text, fg, bg, cols);
        StatusLine {
            text,
            fg,
            bg,
            cells,
        }
    }

    /// Re-lay the existing text/colors at a new width (after a resize).
    fn relayout(&mut self, cols: usize) {
        self.cells = Self::lay_out(&self.text, self.fg, self.bg, cols);
    }
}

/// What [`Grid::style_stats`] reports.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct StyleStats {
    /// Bytes per stored cell.
    pub cell_bytes: usize,
    /// Styles not freed, dead-but-unswept ones included.
    pub live: usize,
    /// Style slots allocated, freed ones included.
    pub entries: usize,
    /// Bytes the style table holds, by capacity.
    pub bytes: usize,
}

impl Grid {
    /// Create a `cols`×`rows` grid filled with blank cells.
    pub fn new(cols: usize, rows: usize) -> Self {
        let screen = blank_screen(cols, rows, PackedCell::blank());
        Self::with_screen(cols, rows, screen, StyleTable::default())
    }

    /// A `cols`×`rows` grid in its initial state around an already-built
    /// `screen` (`rows` lines of `cols` cells) whose ids are in `styles`, so a
    /// decoder can build the rows first and allocate no blank screen to
    /// throw away.
    pub(crate) fn with_screen(
        cols: usize,
        rows: usize,
        screen: Vec<Line>,
        styles: StyleTable,
    ) -> Self {
        Self {
            cols,
            rows,
            screen,
            dirty: vec![false; rows],
            row_stamp: vec![0; rows],
            hist_stamp: 0,
            stamp: 0,
            id: next_grid_id(),
            cursor: (0, 0),
            saved_cursor: (0, 0),
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            primary: None,
            spare_screen: Vec::new(),
            blank_row: Vec::new(),
            styles,
            epoch: 0,
            title: String::new(),
            title_stack: Vec::new(),
            kitty_flags_stack: Vec::new(),
            cwd: String::new(),
            scrollback: VecDeque::new(),
            untrimmed: 0,
            scrollback_max: SCROLLBACK_MAX,
            view_offset: 0,
            selection: None,
            #[cfg(any(test, feature = "gui"))]
            search: None,
            #[cfg(any(test, feature = "gui"))]
            total_scrolled: 0,
            #[cfg(any(test, feature = "gui"))]
            images: Vec::new(),
            host_out: Vec::new(),
            links: Vec::new(),
            links_arc: Arc::from(Vec::<String>::new()),
            clusters: Vec::new(),
            clusters_arc: OnceLock::new(),
            cluster_ids: HashMap::new(),
            cluster_scratch: String::new(),
            current_link: 0,
            tab_stops: default_tab_stops(cols),
            cursor_visible: true,
            sync_output: false,
            sync_output_since: None,
            bracketed_paste: false,
            app_cursor_keys: false,
            alt_scroll: false,
            bell: false,
            kitty_images: Vec::new(),
            next_anim_id: 0,
            kitty_virtual: Vec::new(),
            progress: None,
            report_color_scheme: false,
            min_contrast: 1.0,
            #[cfg(any(test, feature = "gui"))]
            bidi: false,
            bdsm: None,
            bidi_autodetect: true,
            bidi_para_rtl: None,
            notif99_pending: Vec::new(),
            #[cfg(any(test, feature = "gui"))]
            command_began: None,
            #[cfg(any(test, feature = "gui"))]
            finished_commands: Vec::new(),
            focus_reporting: false,
            win32_input: false,
            app_keypad: false,
            line_feed_new_line: false,
            clipboard_set: None,
            clipboard_set_primary: None,
            clipboard_query: false,
            clipboard_query_primary: false,
            notifications: Vec::new(),
            collect_program_status: false,
            program_status: Vec::new(),
            ime_preedit: String::new(),
            mouse_modes: MouseModes::default(),
            cell_px: None,
            cursor_icon: None,
            autowrap: true,
            grapheme_clustering: false,
            allow_deccolm: false,
            column_mode_132: false,
            origin_mode: false,
            lr_margin_mode: false,
            left_margin: 0,
            right_margin: cols.saturating_sub(1),
            insert_mode: false,
            erase_bg: None,
            cursor_color: crate::cell::DEFAULT_FG,
            cursor_shape: CursorShape::default(),
            cursor_blink: false,
            default_cursor_shape: CursorShape::default(),
            default_cursor_blink: false,
            prompt_marks: Vec::new(),
            #[cfg(any(test, feature = "gui"))]
            fold_pending_start: None,
            #[cfg(any(test, feature = "gui"))]
            fold_blocks: Vec::new(),
            #[cfg(feature = "l13")]
            channel: rusty_term_l13::ChannelState::default(),
            status_line: None,
            last_exit: None,
            #[cfg(feature = "l13")]
            command_start: None,
            #[cfg(feature = "l13")]
            last_command_output: None,
        }
    }

    /// Write `cell` at `(x, y)`, marking the row dirty. Out-of-bounds writes
    /// are silently ignored (the caller is responsible for clamping).
    ///
    /// If the write lands on one half of an existing double-width glyph, the
    /// orphaned partner cell is blanked so no stale head/trailer is left behind.
    pub fn set_cell(&mut self, x: usize, y: usize, cell: Cell) {
        if x >= self.cols || y >= self.rows {
            return;
        }
        let cell = self.pack(cell);
        let blank = self.default_packed();
        let row = self.screen[y].cells_mut();
        if row[x].is_wide_trailer() {
            // Overwriting the trailing half: blank the head to its left.
            if x >= 1 {
                row[x - 1] = blank;
            }
        } else if x + 1 < row.len() && row[x + 1].is_wide_trailer() {
            // Overwriting the leading half: blank the orphaned trailer to its right.
            row[x + 1] = blank;
        }
        row[x] = cell;
        self.touch(y);
    }

    /// The cells of live screen row `y` (`cols` of them), collected; empty
    /// past the last row. Per-frame readers use [`Grid::row_cells`].
    pub fn row(&self, y: usize) -> Vec<Cell> {
        self.row_cells(y).collect()
    }

    /// The cells of live screen row `y`, by value and without allocating;
    /// empty past the last row. Always the live screen, never the scrolled
    /// or folded viewport.
    pub fn row_cells(&self, y: usize) -> impl Iterator<Item = Cell> + '_ {
        self.screen
            .get(y)
            .into_iter()
            .flat_map(move |line| (0..line.cells().len()).map(move |x| self.line_cell(line, x)))
    }

    /// The cell at `col` of `line`; a blank past the line's width. Every
    /// by-value cell read goes through here, so the cell storage can change
    /// behind it.
    pub(crate) fn line_cell(&self, line: &Line, col: usize) -> Cell {
        line.cell(col)
            .map_or_else(|| self.default_cell(), |cell| self.resolve(cell))
    }

    /// The full cell behind a stored one.
    pub(crate) fn resolve(&self, cell: PackedCell) -> Cell {
        cell.resolve(&self.styles.get(cell.style()))
    }

    /// The first `cols` cells of `line` resolved and appended to `out`,
    /// blanks past its width. A run of one style looks the table up once.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn push_resolved(&self, line: &Line, cols: usize, out: &mut Vec<Cell>) {
        let start = out.len();
        let cells = &line.cells()[..line.cells().len().min(cols)];
        let mut memo: Option<(StyleId, CellStyle)> = None;
        for &cell in cells {
            let style = match memo {
                Some((id, style)) if id == cell.style() => style,
                _ => {
                    let style = self.styles.get(cell.style());
                    memo = Some((cell.style(), style));
                    style
                }
            };
            out.push(cell.resolve(&style));
        }
        let width = start + line.width().min(cols);
        if let Some(&fill) = line.cells().last()
            && out.len() < width
        {
            out.resize(width, self.resolve(fill));
        }
        out.resize(start + cols, self.default_cell());
    }

    /// `cells` resolved, for readers that want a whole row by value.
    pub(crate) fn resolved(&self, cells: &[PackedCell]) -> Vec<Cell> {
        cells.iter().map(|&cell| self.resolve(cell)).collect()
    }

    /// The text of `line` (see [`row_text`]).
    fn text_of(&self, line: &Line) -> String {
        row_text(
            line.cells().iter().map(|&c| self.resolve(c)),
            &self.clusters,
        )
    }

    /// Store `cell`, interning its style.
    pub(crate) fn pack(&mut self, cell: Cell) -> PackedCell {
        PackedCell::new(&cell, self.styles.intern(CellStyle::of(&cell)))
    }

    /// The grid's style table.
    pub(crate) fn styles(&self) -> &StyleTable {
        &self.styles
    }

    /// Cell and style table sizes, for memory tests.
    #[doc(hidden)]
    pub fn style_stats(&self) -> StyleStats {
        StyleStats {
            cell_bytes: size_of::<PackedCell>(),
            live: self.styles.live(),
            entries: self.styles.len(),
            bytes: self.styles.bytes(),
        }
    }

    /// The style id of a glyph written in `pen` under `link`. `pen`'s DECSCA
    /// bit is a layout bit and does not reach the style.
    fn pen_id(&mut self, pen: Pen, link: u16) -> StyleId {
        self.styles.intern_pen(CellStyle {
            fg: pen.fg,
            bg: pen.bg,
            underline_color: pen.underline_color,
            attrs: pen.attrs & !super::cell::ATTR_PROTECTED,
            link,
            cluster: 0,
        })
    }

    /// Run a style sweep if one is due, and a compaction if the sweep left
    /// the table mostly dead. Called between parser batches and at the end
    /// of operations that rebuild the screen, when no caller holds a style
    /// id outside the grid's cells.
    pub(crate) fn sweep_styles(&mut self) {
        if self.styles.sweep_due() {
            self.collect_styles();
        }
    }

    /// [`Grid::sweep_styles`] without the trigger.
    fn collect_styles(&mut self) {
        self.blank_row.clear();
        let stashed = self.primary.iter().flat_map(|p| p.screen.iter());
        let lines = self
            .screen
            .iter()
            .chain(&self.spare_screen)
            .chain(stashed)
            .chain(&self.scrollback)
            .map(Line::cells);
        self.styles.sweep(lines);
        if let Some(remap) = self.styles.compact() {
            self.apply_remap(&remap);
        }
    }

    /// Rewrite every stored cell through `remap`. Row stamps do not change:
    /// every cache keyed on them holds resolved cells, which are the same.
    fn apply_remap(&mut self, remap: &super::style::Remap) {
        let stashed = self.primary.iter_mut().flat_map(|p| p.screen.iter_mut());
        for line in self
            .screen
            .iter_mut()
            .chain(&mut self.spare_screen)
            .chain(&mut self.scrollback)
            .chain(stashed)
        {
            remap.apply(line.cells_restyle());
        }
    }

    /// Whether live screen row `y` soft-wraps into the next one.
    #[cfg(test)]
    pub(crate) fn row_wrapped(&self, y: usize) -> bool {
        self.screen.get(y).is_some_and(|line| line.wrapped)
    }

    /// Move the cursor to `(x, y)`, clamping into the grid so a malformed or
    /// oversized positioning sequence can never park the cursor off-screen.
    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor = (
            x.min(self.cols.saturating_sub(1)),
            y.min(self.rows.saturating_sub(1)),
        );
    }

    /// Absolute cursor positioning that honors origin mode (DECOM). `col`/`row`
    /// are 0-based. With origin mode on, `row` is relative to the scroll region
    /// top and the cursor is confined to the region; otherwise it is screen-
    /// absolute. Used by `CUP`/`HVP` and `VPA`.
    pub(crate) fn set_cursor_abs(&mut self, col: usize, row: usize) {
        let y = if self.origin_mode {
            (self.scroll_top + row).min(self.scroll_bottom)
        } else {
            row.min(self.rows.saturating_sub(1))
        };
        let x = if self.origin_mode && self.side_margins_active() {
            (self.left_margin + col).min(self.right_margin)
        } else {
            col.min(self.cols.saturating_sub(1))
        };
        self.cursor = (x, y);
    }

    /// Move the cursor up `n` rows (`CUU`). The scroll region's top margin is a
    /// floor when the cursor starts at or below it, so cursor-up can't escape
    /// the region; above the region it floors at row 0.
    pub(crate) fn cursor_up(&mut self, n: usize) {
        let floor = if self.cursor.1 >= self.scroll_top {
            self.scroll_top
        } else {
            0
        };
        self.cursor.1 = self.cursor.1.saturating_sub(n).max(floor);
    }

    /// Move the cursor down `n` rows (`CUD`). The scroll region's bottom margin
    /// is a ceiling when the cursor starts at or above it; below the region it
    /// ceilings at the last row.
    pub(crate) fn cursor_down(&mut self, n: usize) {
        let ceil = if self.cursor.1 <= self.scroll_bottom {
            self.scroll_bottom
        } else {
            self.rows.saturating_sub(1)
        };
        self.cursor.1 = self.cursor.1.saturating_add(n).min(ceil);
    }

    /// The cursor home position: the top-left of the screen, or of the scroll
    /// region when origin mode is on.
    fn home_position(&self) -> (usize, usize) {
        let x = if self.origin_mode && self.side_margins_active() {
            self.left_margin
        } else {
            0
        };
        (x, if self.origin_mode { self.scroll_top } else { 0 })
    }

    /// Enable or disable origin mode (DECOM), moving the cursor to the (now
    /// possibly origin-relative) home position as the spec requires.
    pub(crate) fn set_origin_mode(&mut self, on: bool) {
        self.origin_mode = on;
        self.cursor = self.home_position();
    }

    /// Push the current window title onto a stack (XTPUSHTITLE, `CSI 22 t`),
    /// restorable via [`Grid::pop_title`]. Silently drops the push once
    /// [`TITLE_STACK_MAX`] is reached rather than growing without bound.
    pub(crate) fn push_title(&mut self) {
        if self.title_stack.len() < TITLE_STACK_MAX {
            self.title_stack.push(self.title.clone());
        }
    }

    /// Pop and restore the most recently pushed title (XTPOPTITLE, `CSI 23
    /// t`). A no-op if the stack is empty.
    pub(crate) fn pop_title(&mut self) {
        if let Some(t) = self.title_stack.pop() {
            self.title = t;
        }
    }

    /// The Kitty keyboard protocol enhancement flags currently in effect —
    /// the top of [`Self::kitty_flags_stack`], or `0` (legacy encoding) when
    /// nothing has pushed an entry. Consulted by the windowed front-end's own
    /// key encoder.
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.kitty_flags_stack.last().copied().unwrap_or(0)
    }

    /// Push a new enhancement-flag entry (`CSI > flags u`), inheriting
    /// nothing from the previous top — a client that wants to extend it reads
    /// the current flags via `CSI ? u` first, as the spec expects. Silently
    /// drops the push once [`KITTY_FLAGS_STACK_MAX`] is reached.
    pub(crate) fn push_kitty_flags(&mut self, flags: u8) {
        if self.kitty_flags_stack.len() < KITTY_FLAGS_STACK_MAX {
            self.kitty_flags_stack.push(flags);
        }
    }

    /// Pop `n` enhancement-flag entries (`CSI < n u`, default `n=1`). Popping
    /// more than the stack holds just empties it (back to legacy encoding).
    pub(crate) fn pop_kitty_flags(&mut self, n: usize) {
        for _ in 0..n.max(1) {
            if self.kitty_flags_stack.pop().is_none() {
                break;
            }
        }
    }

    /// Set the current enhancement flags (`CSI = flags ; mode u`): mode `1`
    /// (default) replaces them, `2` ORs `flags` in, `3` clears `flags`' bits.
    /// Pushes a fresh (`0`) entry first if the stack is empty, since there's
    /// no existing top to modify.
    pub(crate) fn set_kitty_flags(&mut self, flags: u8, mode: u8) {
        if self.kitty_flags_stack.is_empty() {
            self.kitty_flags_stack.push(0);
        }
        let top = self
            .kitty_flags_stack
            .last_mut()
            .expect("just ensured non-empty");
        *top = match mode {
            2 => *top | flags,
            3 => *top & !flags,
            _ => flags,
        };
    }

    /// Enter/leave a synchronized-output window (DEC `?2026`), set by the
    /// parser on `CSI ?2026h` / `CSI ?2026l`.
    pub(crate) fn set_sync_output(&mut self, on: bool) {
        self.sync_output = on;
        self.sync_output_since = on.then(std::time::Instant::now);
    }

    /// Whether the render loop should suppress its wakeup right now because a
    /// synchronized-output window is open. Auto-expires after
    /// [`SYNC_OUTPUT_TIMEOUT`] so a misbehaving or crashed client that opens a
    /// window and never closes it can't freeze the display indefinitely.
    pub fn sync_output_active(&mut self) -> bool {
        if !self.sync_output {
            return false;
        }
        if self
            .sync_output_since
            .is_some_and(|t| t.elapsed() >= SYNC_OUTPUT_TIMEOUT)
        {
            self.sync_output = false;
            self.sync_output_since = None;
            return false;
        }
        true
    }

    /// When the open synchronized-output window times out, for a render loop
    /// to wake and repaint then even if no more output arrives. `None` when
    /// no window is open; one already past its timeout is closed here, as by
    /// [`Grid::sync_output_active`].
    pub fn sync_output_until(&mut self) -> Option<std::time::Instant> {
        if !self.sync_output_active() {
            return None;
        }
        self.sync_output_since.map(|t| t + SYNC_OUTPUT_TIMEOUT)
    }

    /// Move the cursor to column 0 of the current row.
    /// Whether DECSLRM margins narrower than the full width are in effect —
    /// the guard on every margin-aware branch, so the common case pays one
    /// boolean test.
    pub(crate) fn side_margins_active(&self) -> bool {
        self.lr_margin_mode && (self.left_margin > 0 || self.right_margin + 1 < self.cols)
    }

    /// Set the DECSLRM left/right margins (0-based, inclusive). An invalid
    /// pair resets to the full width; the cursor homes, per VT420.
    pub(crate) fn set_lr_margins(&mut self, left: usize, right: usize) {
        let right = right.min(self.cols.saturating_sub(1));
        if left < right {
            self.left_margin = left;
            self.right_margin = right;
        } else {
            self.left_margin = 0;
            self.right_margin = self.cols.saturating_sub(1);
        }
        self.cursor = self.home_position();
    }

    /// Scroll the margin band (scroll region rows × left/right margin
    /// columns) up one row. Partial-width scrolls never form scrollback.
    fn scroll_band_up(&mut self) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom.min(self.rows - 1));
        let (l, r) = (self.left_margin, self.right_margin.min(self.cols - 1));
        let blank = self.erase_packed();
        for y in top..bottom {
            self.copy_band(y + 1, y, l, r);
        }
        self.screen[bottom].cells_mut()[l..=r].fill(blank);
        self.touch_rows(top..=bottom);
    }

    /// Copy columns `l..=r` of row `src` over the same columns of row `dst`.
    fn copy_band(&mut self, src: usize, dst: usize, l: usize, r: usize) {
        if src == dst {
            return;
        }
        let (lo, hi) = self.screen.split_at_mut(src.max(dst));
        let (from, to) = if src < dst {
            (&lo[src], &mut hi[0])
        } else {
            (&hi[0], &mut lo[dst])
        };
        to.cells_mut()[l..=r].copy_from_slice(&from.cells()[l..=r]);
    }

    /// Scroll the margin band down one row (the [`Grid::scroll_band_up`]
    /// mirror, for RI / SD).
    fn scroll_band_down(&mut self) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom.min(self.rows - 1));
        let (l, r) = (self.left_margin, self.right_margin.min(self.cols - 1));
        let blank = self.erase_packed();
        for y in (top..bottom).rev() {
            self.copy_band(y, y + 1, l, r);
        }
        self.screen[top].cells_mut()[l..=r].fill(blank);
        self.touch_rows(top..=bottom);
    }

    pub fn carriage_return(&mut self) {
        // With DECSLRM margins, CR returns to the left margin when the
        // cursor is at or right of it (a cursor left of the margin still
        // goes to column 0, per VT420/xterm).
        if self.side_margins_active() && self.cursor.0 >= self.left_margin {
            self.cursor.0 = self.left_margin;
            return;
        }
        self.cursor.0 = 0;
    }

    /// Advance the cursor one row. At the bottom of the scrolling region this
    /// scrolls the region up instead of moving the cursor past it.
    pub fn newline(&mut self) {
        if self.cursor.1 == self.scroll_bottom {
            if self.side_margins_active() {
                self.scroll_band_up();
            } else {
                self.scroll_up();
            }
        } else if self.cursor.1 + 1 < self.rows {
            self.cursor.1 += 1;
        }
    }

    /// Render an RGB(A) image at the cursor as truecolor half-block glyphs
    /// (`▀`/`▄`) — one cell per pixel column, two pixel rows per cell. `pixels`
    /// is row-major `width × height`, `None` meaning transparent. The image is
    /// shrunk to fit the columns remaining from the cursor (aspect-preserving,
    /// never enlarged), placed top-left at the cursor, and scrolled like printed
    /// lines if it runs past the bottom; the cursor ends at column 0 of the row
    /// below it (xterm "sixel scrolling"). Shared by the Sixel and Kitty paths,
    /// which have no explicit-size concept of their own.
    pub fn render_image(&mut self, width: usize, height: usize, pixels: &[Option<u32>]) {
        self.render_image_sized(width, height, pixels, None, None, true);
    }

    /// [`Self::render_image`], but honoring iTerm2's `File=` geometry hints:
    /// `target_cols`/`target_rows` (cell counts, `None` meaning "use the
    /// image's own size for this axis") and `preserve_aspect` (iTerm2's
    /// `preserveAspectRatio`, default-on `true`). With both axes given and
    /// aspect preserved, the image is "contain"-fit within the requested
    /// footprint (never cropped or stretched off-axis). Either axis is still
    /// clamped to the columns available from the cursor — an explicit size
    /// can shrink or grow the image within that limit, never exceed it.
    pub(crate) fn render_image_sized(
        &mut self,
        width: usize,
        height: usize,
        pixels: &[Option<u32>],
        target_cols: Option<usize>,
        target_rows: Option<usize>,
        preserve_aspect: bool,
    ) {
        if width == 0 || height == 0 || pixels.len() < width * height {
            return;
        }
        // Nearest-neighbor source sample for target pixel `(tx, ty)`.
        let sample = |tx: usize, ty: usize, tw: usize, th: usize| -> Option<u32> {
            if ty >= th {
                return None;
            }
            pixels[(ty * height / th) * width + (tx * width / tw)]
        };
        // Combine the two pixels of one cell into a half-block glyph; an unset
        // half takes the default background, both unset leaves the cell alone.
        let half_block = |top: Option<u32>, bottom: Option<u32>, def_bg: u32| {
            let mk = |ch, fg, bg| Cell {
                ch,
                cluster: 0,
                fg,
                bg,
                flags: 0,
                link: 0,
                underline_color: fg,
            };
            match (top, bottom) {
                (None, None) => None,
                (Some(t), Some(b)) => Some(mk('\u{2580}', t, b)),
                (Some(t), None) => Some(mk('\u{2580}', t, def_bg)),
                (None, Some(b)) => Some(mk('\u{2584}', b, def_bg)),
            }
        };

        let origin = self.cursor.0;
        let avail = self.cols.saturating_sub(origin).max(1);
        // `tw` is both the cell-column count and the pixel-column count (one
        // pixel column per cell column, this function's whole convention);
        // `th` is a *pixel*-row count (two pixel rows pack into one cell row
        // via the half-block glyphs below) — so a `target_rows` cell count
        // becomes `target_rows * 2` in this local `th` domain.
        let (tw, th) = match (target_cols, target_rows) {
            (None, None) => {
                // No hint: fit to the available width (shrink only), aspect
                // always preserved — the pre-C12 behavior, unchanged.
                let tw = width.min(avail);
                (tw, (height * tw / width).max(1))
            }
            (Some(cols), None) => {
                let tw = cols.min(avail).max(1);
                let th = if preserve_aspect {
                    (height * tw / width).max(1)
                } else {
                    height
                };
                (tw, th)
            }
            (None, Some(rows)) => {
                let th = rows.saturating_mul(2).max(1);
                let tw = if preserve_aspect {
                    (width * th / height).max(1).min(avail)
                } else {
                    width.min(avail)
                };
                (tw, th)
            }
            (Some(cols), Some(rows)) => {
                let tw_req = cols.min(avail).max(1);
                let th_req = rows.saturating_mul(2).max(1);
                if preserve_aspect {
                    // "Contain"-fit: try filling the width, and if that would
                    // overshoot the requested height, fill the height instead.
                    let th_from_w = (height * tw_req / width).max(1);
                    if th_from_w <= th_req {
                        (tw_req, th_from_w)
                    } else {
                        ((width * th_req / height).max(1).min(avail), th_req)
                    }
                } else {
                    (tw_req, th_req)
                }
            }
        };
        // `th`/`rows` come straight from untrusted OSC 1337 `height=` or Kitty
        // `r=` hints and are otherwise unclamped (unlike `tw`, which is always
        // bounded by `avail`). Without a cap here, a single escape sequence
        // requesting an absurd height (e.g. `height=999999999`) would drive
        // this function's `newline()` loop below billions of times. Nothing
        // past what could ever be visible or retained in scrollback needs to
        // be drawn, so crop to that — same "clip, don't rescale" convention
        // `tw` already uses against `avail`.
        let max_cell_rows = self.rows.saturating_add(self.scrollback_max).max(1);
        let cell_rows = th.div_ceil(2).min(max_cell_rows);
        #[cfg(any(test, feature = "gui"))]
        self.store_image(width, height, pixels, origin, tw, cell_rows);

        for cr in 0..cell_rows {
            let y = self.cursor.1;
            for cc in 0..tw {
                let col = origin + cc;
                if col >= self.cols {
                    break;
                }
                let top = sample(cc, cr * 2, tw, th);
                let bottom = sample(cc, cr * 2 + 1, tw, th);
                if let Some(cell) = half_block(top, bottom, self.default_bg()) {
                    self.set_cell(col, y, cell);
                }
            }
            self.newline();
        }
        self.carriage_return();
    }

    /// Render a decoded Sixel image (delegates to [`Grid::render_image`]).
    pub(crate) fn render_sixel(&mut self, img: &SixelImage) {
        self.render_image(img.width, img.height, &img.pixels);
    }

    /// Store a placed pixel image for the CPU renderer to overlay over its
    /// reserved half-block cells, anchored by serial (top cell row) so it
    /// scrolls with text. Bounded — the oldest image is dropped past the cap.
    #[cfg(any(test, feature = "gui"))]
    fn store_image(
        &mut self,
        pw: usize,
        ph: usize,
        pixels: &[Option<u32>],
        col: usize,
        cols: usize,
        rows: usize,
    ) {
        const MAX_IMAGES: usize = 8;
        let serial = self.total_scrolled + self.cursor.1;
        self.images.push(GridImage {
            serial,
            col,
            cols,
            rows,
            pw,
            ph,
            pixels: pixels[..pw * ph].into(),
            anim: None,
        });
        if self.images.len() > MAX_IMAGES {
            self.images.remove(0);
        }
    }

    /// Render a decoded multi-frame image (an inline animated GIF): the first
    /// frame draws exactly like [`Self::render_image_sized`] (half-block
    /// cells plus the placed overlay), and the full frame set is stored as a
    /// playing [`KittyImage`] under a synthesized id that the overlay
    /// substitutes its current frame from, driven by the ordinary animation
    /// timer. TUI passthrough (no `gui` overlay) shows the first frame.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn render_animated_image(
        &mut self,
        width: usize,
        height: usize,
        frames: Vec<(Vec<Option<u32>>, u32)>,
        target_cols: Option<usize>,
        target_rows: Option<usize>,
        preserve_aspect: bool,
    ) {
        /// Same total-pixel budget the Kitty store enforces.
        const MAX_STORE_PIXELS: usize = 16 * 1024 * 1024;
        let Some((first, _)) = frames.first() else {
            return;
        };
        let over_budget =
            width.saturating_mul(height).saturating_mul(frames.len()) > MAX_STORE_PIXELS;
        if frames.len() == 1
            || over_budget
            || width == 0
            || height == 0
            || first.len() < width * height
        {
            if let Some((px, _)) = frames.into_iter().next() {
                self.render_image_sized(
                    width,
                    height,
                    &px,
                    target_cols,
                    target_rows,
                    preserve_aspect,
                );
            }
            return;
        }
        let first = first.clone();
        self.render_image_sized(
            width,
            height,
            &first,
            target_cols,
            target_rows,
            preserve_aspect,
        );
        // `render_image_sized` bails (no image stored) on degenerate input;
        // only animate an overlay that actually exists.
        let Some(im) = self.images.last_mut() else {
            return;
        };
        // Synthesized ids live at the top of the id space so a Kitty client's
        // own ids (typically small integers) don't collide with them.
        let id = 0xFFFF_0000u32 | (self.next_anim_id & 0xFFFF);
        self.next_anim_id = self.next_anim_id.wrapping_add(1);
        im.anim = Some(id);
        self.kitty_images.retain(|img| img.id != id);
        self.kitty_images.push(KittyImage {
            id,
            w: width,
            h: height,
            frames: frames
                .into_iter()
                .map(|(pixels, gap_ms)| KittyFrame {
                    pixels: pixels.into(),
                    gap_ms: gap_ms.max(40),
                })
                .collect(),
            current: 0,
            playing: true,
            last_advance: None,
        });
    }

    /// Drop images that have scrolled out of the retained scrollback.
    #[cfg(any(test, feature = "gui"))]
    fn evict_scrolled_images(&mut self) {
        let oldest = self.total_scrolled.saturating_sub(self.scrollback.len());
        self.images.retain(|im| im.serial + im.rows > oldest);
    }

    /// Placed pixel images for the CPU renderer to composite over the cells.
    #[cfg(any(test, feature = "gui"))]
    pub fn images(&self) -> &[GridImage] {
        &self.images
    }

    /// The viewport row of image `im`'s top cell (negative when scrolled partly
    /// above the view).
    #[cfg(any(test, feature = "gui"))]
    pub fn image_top_row(&self, im: &GridImage) -> isize {
        let off = self.view_offset.min(self.scrollback.len());
        im.serial as isize - self.total_scrolled as isize + off as isize
    }

    /// Scroll the current scrolling region up by one row, blanking the freed
    /// bottom row of the region. Only the region's rows are marked dirty.
    ///
    /// Rows rotate; no cells move. On a full-screen scroll of the primary
    /// buffer the line leaving the top moves into scrollback, and the line
    /// that eviction drops from the front of a full scrollback is recycled as
    /// the new bottom row.
    pub fn scroll_up(&mut self) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        if bottom <= top || bottom >= self.rows {
            return;
        }
        let blank = self.erase_packed();
        self.screen[top..=bottom].rotate_left(1);
        // The line that left the top now sits at `bottom`.
        // Capture it into scrollback, but only for a full-screen scroll on the
        // primary buffer: partial-region scrolls (TUI apps using DECSTBM) and
        // the alternate screen don't form history.
        if top == 0 && bottom == self.rows - 1 && self.primary.is_none() {
            let mut line = std::mem::take(&mut self.screen[bottom]);
            line.attr = LineAttr::Single;
            self.scrollback.push_back(line);
            self.untrimmed += 1;
            self.touch_hist();
            if self.scrollback.len() > self.scrollback_max {
                if let Some(evicted) = self.scrollback.pop_front() {
                    self.styles.note_evicted(evicted.cells().len());
                    self.screen[bottom] = evicted;
                }
                self.evict_prompt_mark();
                #[cfg(feature = "l13")]
                if let Some(s) = &mut self.command_start {
                    *s = s.saturating_sub(1);
                }
                #[cfg(any(test, feature = "gui"))]
                {
                    if let Some(s) = &mut self.fold_pending_start {
                        *s = s.saturating_sub(1);
                    }
                    self.evict_fold_blocks();
                }
            }
            #[cfg(any(test, feature = "gui"))]
            {
                self.total_scrolled += 1;
                self.evict_scrolled_images();
            }
            // If the user is browsing history, advance the offset in step with
            // the incoming line so the viewed region stays put under new output.
            if self.view_offset > 0 {
                self.view_offset = (self.view_offset + 1).min(self.display_history_len());
                self.touch_all();
            }
        }
        self.prime_blank_row(blank);
        self.screen[bottom].clear(&self.blank_row);
        self.touch_rows(top..=bottom);
    }

    /// Set the scrolling region to rows `top..=bottom` (0-based, inclusive) and
    /// home the cursor. An invalid range resets the region to the full screen.
    pub(crate) fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows.saturating_sub(1));
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        } else {
            self.scroll_top = 0;
            self.scroll_bottom = self.rows.saturating_sub(1);
        }
        self.cursor = self.home_position();
    }

    /// Reset the scrolling region to span the full screen.
    fn reset_scroll_region(&mut self) {
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
    }

    /// Write `ch` at the cursor with the given [`Pen`], wrapping to the next
    /// line if it would not fit, then advancing the cursor by the glyph's
    /// display width. A double-width glyph also writes a flagged trailing cell.
    pub fn put_char(&mut self, ch: char, pen: Pen) {
        // A non-ASCII scalar may belong to the glyph on its left. ASCII is
        // always its own glyph, so the common path skips the check entirely.
        let w = char_width(ch);
        if !ch.is_ascii() {
            if self.grapheme_clustering {
                // Mode 2027: join anything that continues the UAX #29
                // grapheme (combining mark, ZWJ join, skin tone, flag pair).
                if let Some((bx, by)) = self.left_base()
                    && self.continues_grapheme(bx, by, ch)
                {
                    self.append_to_glyph(bx, by, ch);
                    return;
                }
            } else if w == 0
                && let Some((bx, by)) = self.left_base()
            {
                // Default (wcwidth) widths: only a zero-width scalar (combining
                // mark, ZWJ, variation selector) joins the glyph on its left;
                // every other scalar takes its own columns below.
                self.append_to_glyph(bx, by, ch);
                return;
            }
            // Not joined: a zero-width scalar has no cell of its own.
            if w == 0 {
                return;
            }
        }
        // With DECSLRM margins, a cursor inside them wraps (or pins) at the
        // right margin instead of the screen edge.
        let limit = if self.side_margins_active() && self.cursor.0 >= self.left_margin {
            (self.right_margin + 1).min(self.cols)
        } else {
            self.cols
        };
        if self.cursor.0 + w > limit {
            if self.autowrap {
                // Mark this row as soft-wrapped before leaving it, so the reflow
                // and scrollback know it and its successor are one logical line.
                let y = self.cursor.1;
                if let Some(line) = self.screen.get_mut(y)
                    && !line.wrapped
                {
                    line.wrapped = true;
                    self.touch(y);
                }
                self.carriage_return();
                self.newline();
            } else {
                // Autowrap off: keep the glyph in the last cell(s) of the row
                // (or of the margin band).
                self.cursor.0 = limit.saturating_sub(w);
            }
        }
        // Insert mode (IRM): make room by shifting the rest of the row right.
        if self.insert_mode {
            self.insert_chars(w);
        }
        let (x, y) = self.cursor;
        let link = self.current_link;
        self.set_cell(
            x,
            y,
            Cell {
                ch,
                cluster: 0,
                fg: pen.fg,
                bg: pen.bg,
                flags: pen.attrs,
                link,
                underline_color: pen.underline_color,
            },
        );
        if w == 2 && x + 1 < self.cols {
            // Trailing half: a flagged placeholder the renderer skips. It keeps
            // the pen's colors but only the WIDE_TRAILER layout bit.
            self.set_cell(
                x + 1,
                y,
                Cell {
                    ch: ' ',
                    cluster: 0,
                    fg: pen.fg,
                    bg: pen.bg,
                    flags: WIDE_TRAILER | (pen.attrs & ATTR_DEFAULT_COLORS),
                    link,
                    underline_color: pen.underline_color,
                },
            );
        }
        // `cols` (one past the last column) is the pending-wrap position; a
        // glyph wider than the grid itself (w=2, cols=1) must not push past it,
        // or later column arithmetic indexes outside the row.
        self.cursor.0 = (self.cursor.0 + w).min(self.cols);
    }

    /// Write a run of printable ASCII at the cursor, as [`Grid::put_char`]
    /// would one byte at a time. The part that fits before the right edge is
    /// filled straight into the row; wrapping, insert mode and DECSLRM margins
    /// fall back to [`Grid::put_char`] for one byte.
    pub(crate) fn put_ascii(&mut self, run: &[u8], pen: Pen) {
        let mut rest = run;
        while let Some((&first, tail)) = rest.split_first() {
            let (x, y) = self.cursor;
            let room = self.cols.saturating_sub(x);
            if room == 0 || self.insert_mode || self.side_margins_active() {
                self.put_char(char::from(first), pen);
                rest = tail;
                continue;
            }
            let n = room.min(rest.len());
            self.fill_ascii(x, y, &rest[..n], pen);
            self.cursor.0 = x + n;
            rest = &rest[n..];
        }
    }

    /// Write a run of non-ASCII text at the cursor, as [`Grid::put_char`]
    /// would one scalar at a time. Glyphs that fit before the right edge are
    /// filled straight into the row; a wrap, a zero-width scalar, insert mode,
    /// DECSLRM margins and mode 2027 fall back to [`Grid::put_char`] for one
    /// scalar. `text` holds no C0 control (the parser only passes bytes at or
    /// above 0x80).
    pub(crate) fn put_text(&mut self, text: &str, pen: Pen) {
        let mut rest = text;
        while let Some(ch) = rest.chars().next() {
            let filled = if self.can_fill_text() {
                self.fill_text(rest, pen)
            } else {
                0
            };
            let used = if filled == 0 {
                self.put_char(ch, pen);
                ch.len_utf8()
            } else {
                filled
            };
            rest = &rest[used..];
        }
    }

    /// Whether [`Grid::fill_text`] may write at the cursor: no mode that
    /// changes where a glyph lands or what it joins.
    fn can_fill_text(&self) -> bool {
        !self.insert_mode
            && !self.side_margins_active()
            && !self.grapheme_clustering
            && self.cursor.1 < self.rows
    }

    /// Fill the leading glyphs of `text` into the cursor row while each has a
    /// width and fits before the right edge, advancing the cursor. Returns
    /// the bytes consumed (`0` when the first glyph does not qualify). A wide
    /// glyph cut by either end of the run loses its other half, as
    /// [`Grid::set_cell`] does.
    fn fill_text(&mut self, text: &str, pen: Pen) -> usize {
        let (x0, y) = self.cursor;
        let (cols, link) = (self.cols, self.current_link);
        if y >= self.screen.len() {
            return 0;
        }
        let head_id = self.pen_id(pen, link);
        // A trailer carries the pen's colours and their provenance but no
        // rendition. It and the blank are interned only when a wide glyph
        // needs them.
        let trailer = CellStyle {
            fg: pen.fg,
            bg: pen.bg,
            underline_color: pen.underline_color,
            attrs: pen.attrs & ATTR_DEFAULT_COLORS,
            link,
            cluster: 0,
        };
        let protected = pen.attrs & super::cell::ATTR_PROTECTED != 0;
        let styles = &mut self.styles;
        let blank = |styles: &mut StyleTable| {
            let cell = Cell::default_blank();
            PackedCell::new(&cell, styles.intern(CellStyle::of(&cell)))
        };
        // With no attributes the trailer's style is the head's.
        let plain = pen.attrs & !(super::cell::ATTR_PROTECTED | ATTR_DEFAULT_COLORS) == 0;
        let mut trailer_id = plain.then_some(head_id);
        let row = self.screen[y].cells_mut();
        let (mut x, mut used) = (x0, 0);
        for ch in text.chars() {
            let w = char_width(ch);
            if w == 0 || x + w > cols {
                break;
            }
            if x == x0 && x >= 1 && row[x].is_wide_trailer() {
                row[x - 1] = blank(styles);
            }
            row[x] = PackedCell::glyph(ch, protected, head_id);
            if w == 2 {
                let id = *trailer_id.get_or_insert_with(|| styles.intern(trailer));
                row[x + 1] = PackedCell::trailer(id);
            }
            x += w;
            used += ch.len_utf8();
        }
        if used == 0 {
            return 0;
        }
        if x < cols && row[x].is_wide_trailer() {
            row[x] = blank(styles);
        }
        self.touch(y);
        self.cursor.0 = x;
        used
    }

    /// Overwrite `bytes.len()` cells of row `y` from column `x` (the caller
    /// keeps them on the row) with ASCII glyphs. A wide glyph cut by either
    /// end of the run loses its other half, as [`Grid::set_cell`] does.
    fn fill_ascii(&mut self, x: usize, y: usize, bytes: &[u8], pen: Pen) {
        if y >= self.screen.len() {
            return;
        }
        let id = self.pen_id(pen, self.current_link);
        let protected = pen.attrs & super::cell::ATTR_PROTECTED != 0;
        let row = self.screen[y].cells_mut();
        let end = x + bytes.len();
        let blank_needed =
            (x >= 1 && row[x].is_wide_trailer()) || (end < row.len() && row[end].is_wide_trailer());
        if blank_needed {
            let blank = self.default_packed();
            let row = self.screen[y].cells_mut();
            if x >= 1 && row[x].is_wide_trailer() {
                row[x - 1] = blank;
            }
            if end < row.len() && row[end].is_wide_trailer() {
                row[end] = blank;
            }
        }
        let row = self.screen[y].cells_mut();
        for (cell, &b) in row[x..end].iter_mut().zip(bytes) {
            *cell = PackedCell::glyph(char::from(b), protected, id);
        }
        self.touch(y);
    }

    /// The base cell of the grapheme immediately left of the cursor, stepping
    /// back over a wide glyph's trailer to its head. `None` at column 0.
    fn left_base(&self) -> Option<(usize, usize)> {
        let (cx, cy) = self.cursor;
        if cy >= self.rows || cx == 0 || cx > self.cols {
            return None;
        }
        let left = cx - 1;
        if self.screen[cy].cells()[left].is_wide_trailer() && left >= 1 {
            Some((left - 1, cy)) // land on the wide glyph's head, not its trailer
        } else {
            Some((left, cy))
        }
    }

    /// Whether appending `next` to the glyph at `(x, y)` keeps it a single
    /// grapheme cluster (UAX #29) — i.e. `next` continues that glyph rather than
    /// starting a new one.
    fn continues_grapheme(&self, x: usize, y: usize, next: char) -> bool {
        let mut s = self.glyph_text(x, y);
        s.push(next);
        s.graphemes(true).count() == 1
    }

    /// The full glyph text at `(x, y)` of the live grid: the base scalar plus
    /// any interned grapheme continuation.
    fn glyph_text(&self, x: usize, y: usize) -> String {
        self.cell_text(self.resolve(self.screen[y].cells()[x]))
    }

    /// The full glyph text of `cell` (live or scrollback — the cluster table is
    /// shared): the base scalar plus any interned grapheme continuation.
    fn cell_text(&self, cell: Cell) -> String {
        let mut s = String::new();
        s.push(cell.ch);
        if cell.cluster != 0
            && let Some(suffix) = self.clusters.get((cell.cluster - 1) as usize)
        {
            s.push_str(suffix);
        }
        s
    }

    /// Normalize the active selection into an inclusive `(start, end)` pair of
    /// `(col, row)` cells in row-major order, clamped to the grid. `None` when
    /// there is no selection.
    #[cfg(any(test, feature = "gui"))]
    fn selection_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let sel = self.selection?;
        let total = self.scrollback.len() + self.rows;
        let clamp = |(c, r): (usize, usize)| {
            (
                c.min(self.cols.saturating_sub(1)),
                r.min(total.saturating_sub(1)),
            )
        };
        let a = clamp(sel.anchor);
        let b = clamp(sel.head);
        // Row-major linear order, so a backward drag still yields start <= end.
        if (a.1, a.0) <= (b.1, b.0) {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    /// Whether the cell at `(col, row)` lies within the active selection
    /// (stream/linear order — full intermediate rows are included). Read by the
    /// windowed renderer to invert highlighted cells.
    #[cfg(any(test, feature = "gui"))]
    pub fn is_selected(&self, col: usize, row: usize) -> bool {
        let Some((start, end)) = self.selection_bounds() else {
            return false;
        };
        let abs = self.abs_of_view_row(row);
        let lin = |c: usize, r: usize| r * self.cols + c;
        (lin(start.0, start.1)..=lin(end.0, end.1)).contains(&lin(col, abs))
    }

    /// The absolute row (scrollback + live) a viewport row currently shows.
    /// A fold-summary row maps to its block's first (heading) line, so
    /// selection/copy anchors land somewhere sensible.
    #[cfg(any(test, feature = "gui"))]
    pub fn abs_of_view_row(&self, vr: usize) -> usize {
        self.abs_of_line(self.view_line(vr))
    }

    /// Folded command blocks that actually collapse in the view: fully
    /// inside scrollback (the live screen never folds) and at least two
    /// lines long. Yields `(fold_blocks index, block)` in start order —
    /// blocks are recorded in stream order, so they're sorted and disjoint.
    #[cfg(any(test, feature = "gui"))]
    fn visible_folds(&self) -> impl Iterator<Item = (usize, &CommandBlock)> {
        let h = self.scrollback.len();
        self.fold_blocks
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.folded && b.end <= h && b.end > b.start + 1)
    }

    /// History length in *display* lines: each visibly-folded block's rows
    /// collapse to one summary line. Equal to `scrollback.len()` when
    /// nothing is folded (and always, in a TUI-only build).
    pub(crate) fn display_history_len(&self) -> usize {
        #[cfg(any(test, feature = "gui"))]
        {
            let hidden: usize = self.visible_folds().map(|(_, b)| b.end - b.start - 1).sum();
            self.scrollback.len() - hidden
        }
        #[cfg(not(any(test, feature = "gui")))]
        {
            self.scrollback.len()
        }
    }

    /// What history *display* index `d` (0 = oldest visible line) shows: an
    /// absolute scrollback line, or a folded block's one-line summary.
    #[cfg(any(test, feature = "gui"))]
    fn history_line(&self, d: usize) -> HistLine {
        let mut abs = 0usize; // absolute line the walk has reached
        let mut rem = d; // display lines still to cover
        for (i, b) in self.visible_folds() {
            let plain = b.start - abs; // unfolded lines before this block
            if rem < plain {
                return HistLine::Abs(abs + rem);
            }
            rem -= plain;
            if rem == 0 {
                return HistLine::Summary(i);
            }
            rem -= 1; // past the summary line
            abs = b.end;
        }
        HistLine::Abs(abs + rem)
    }

    /// The history display index that shows absolute line `abs` — the
    /// summary line's index when `abs` is hidden inside a folded block.
    /// Identity in a TUI-only build (nothing folds there).
    fn display_index_of_abs(&self, abs: usize) -> usize {
        #[cfg(any(test, feature = "gui"))]
        {
            let mut hidden = 0usize;
            for (_, b) in self.visible_folds() {
                if abs < b.start {
                    break;
                }
                if abs < b.end {
                    // Inside the block: everything past its first line is
                    // hidden behind the summary at the block-start's display
                    // position.
                    hidden += abs - b.start;
                    break;
                }
                hidden += b.end - b.start - 1;
            }
            abs - hidden
        }
        #[cfg(not(any(test, feature = "gui")))]
        {
            abs
        }
    }

    /// The absolute line shown at the top of the viewport for a display
    /// offset `off` (identity `history - off` when nothing is folded).
    fn abs_top_for_offset(&self, off: usize) -> usize {
        #[cfg(any(test, feature = "gui"))]
        {
            let dhl = self.display_history_len();
            if off == 0 || off > dhl {
                return self.scrollback.len();
            }
            match self.history_line(dhl - off) {
                HistLine::Abs(l) => l,
                HistLine::Summary(i) => self.fold_blocks[i].start,
            }
        }
        #[cfg(not(any(test, feature = "gui")))]
        {
            self.scrollback.len() - off.min(self.scrollback.len())
        }
    }

    /// The fold block whose one-line summary is shown at viewport row `vr`,
    /// as an index into `fold_blocks`.
    #[cfg(any(test, feature = "gui"))]
    fn summary_block_at(&self, vr: usize) -> Option<usize> {
        match self.view_line(vr) {
            view::ViewLine::Summary(i) => Some(i),
            _ => None,
        }
    }

    /// Expand the folded block whose summary line is at viewport row `vr`
    /// (a click on the summary). Returns whether one was expanded.
    #[cfg(any(test, feature = "gui"))]
    pub fn unfold_summary_at(&mut self, vr: usize) -> bool {
        let Some(i) = self.summary_block_at(vr) else {
            return false;
        };
        self.fold_blocks[i].folded = false;
        self.view_offset = self.view_offset.min(self.display_history_len());
        self.touch_all();
        true
    }

    /// Toggle the most recent command block that has fully scrolled into
    /// history (the fold keybinding's target: "collapse that last wall of
    /// output"). Returns whether a block was toggled.
    #[cfg(any(test, feature = "gui"))]
    pub fn toggle_last_fold(&mut self) -> bool {
        let h = self.scrollback.len();
        let Some(b) = self
            .fold_blocks
            .iter_mut()
            .rev()
            .find(|b| b.end <= h && b.end > b.start + 1)
        else {
            return false;
        };
        b.folded = !b.folded;
        self.view_offset = self.view_offset.min(self.display_history_len());
        self.touch_all();
        true
    }

    /// A fold block's summary line: an "N lines hidden" notice.
    #[cfg(any(test, feature = "gui"))]
    fn summary_text(&self, block: usize) -> String {
        let b = &self.fold_blocks[block];
        format!(
            "\u{25B7} {} lines hidden \u{2014} click to expand",
            b.end - b.start
        )
    }

    /// A summary-line cell showing `ch`: dim italic on default colors.
    #[cfg(any(test, feature = "gui"))]
    fn summary_cell(&self, ch: char) -> Cell {
        Cell {
            ch,
            cluster: 0,
            flags: crate::cell::ATTR_DIM | crate::cell::ATTR_ITALIC | ATTR_DEFAULT_COLORS,
            ..self.default_cell()
        }
    }

    /// Select the "word" under `(col, row)` — the maximal run of word
    /// characters around it on that screen row (double-click). Word
    /// characters are everything except blanks and common separators, kept
    /// deliberately URL/path-friendly (`/`, `.`, `-`, `_`, `:`, `~`, … stay
    /// part of a word) so a double-click grabs a whole path or URL, matching
    /// kitty/foot defaults more than xterm's alnum-only class. Clicking a
    /// separator or blank selects just that cell. Operates on the live
    /// screen, like the rest of the selection model.
    #[cfg(any(test, feature = "gui"))]
    pub fn select_word_at(&mut self, col: usize, row: usize) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        let (col, row) = (col.min(self.cols - 1), row.min(self.rows - 1));
        let abs = self.abs_of_view_row(row);
        let (start, end) = self.word_bounds_at(abs, col);
        self.selection = Some(Selection {
            anchor: (start, abs),
            head: (end, abs),
        });
    }

    /// Word-character bounds `(start_col, end_col)` inclusive, containing
    /// `col` on absolute row `abs` — `(col, col)` if `col` itself isn't a
    /// word character. Pulled out of `select_word_at` so drag-extension
    /// (`extend_word_selection`) can reuse the same word-boundary logic
    /// without re-selecting just the single word at the drag point.
    #[cfg(any(test, feature = "gui"))]
    fn word_bounds_at(&self, abs: usize, col: usize) -> (usize, usize) {
        if self.cols == 0 {
            return (col, col);
        }
        let col = col.min(self.cols - 1);
        let (cells, _) = self.phys_row(abs);
        let is_word = |c: usize| {
            let Some(cell) = cells.get(c) else {
                return false;
            };
            if cell.flags & WIDE_TRAILER != 0 {
                return true; // ride with its wide lead cell
            }
            let ch = cell.ch;
            !(ch == ' ' || ch == '\0' || "\t\"'`()[]{}<>|;,".contains(ch))
        };
        if !is_word(col) {
            return (col, col);
        }
        let mut start = col;
        while start > 0 && is_word(start - 1) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < self.cols && is_word(end + 1) {
            end += 1;
        }
        (start, end)
    }

    /// Extend a double-click word selection as the drag moves: the
    /// initially-clicked word (at `anchor`) and the word under the current
    /// drag point (`head`) both stay whole, with everything between them
    /// included — reading-order aware, so dragging back past the click
    /// point flips which end is anchor vs. head instead of ever shrinking
    /// the selection below the single originally-clicked word. `anchor` and
    /// `head` are `(col, abs_row)`, matching `Selection`'s fields.
    #[cfg(any(test, feature = "gui"))]
    pub fn extend_word_selection(&mut self, anchor: (usize, usize), head: (usize, usize)) {
        let (a_col, a_row) = anchor;
        let (h_col, h_row) = head;
        let (a_start, a_end) = self.word_bounds_at(a_row, a_col);
        let (h_start, h_end) = self.word_bounds_at(h_row, h_col);
        // Reading-order (row, then col) comparison: whichever side of the
        // anchor word the drag point falls on decides which end of each
        // word bounds the union.
        let dragging_back = (h_row, h_col) < (a_row, a_col);
        let (anchor_out, head_out) = if dragging_back {
            ((a_end, a_row), (h_start, h_row))
        } else {
            ((a_start, a_row), (h_end, h_row))
        };
        self.selection = Some(Selection {
            anchor: anchor_out,
            head: head_out,
        });
    }

    /// Select the whole logical line through screen `row` (triple-click):
    /// the row plus any soft-wrapped continuation rows above/below it, per
    /// the per-row wrap bits the resize reflow also uses. Only the live
    /// screen's wrap bits are consulted, so while scrolled into history the
    /// selection is the single visual row.
    #[cfg(any(test, feature = "gui"))]
    pub fn select_line_at(&mut self, row: usize) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        let abs = self.abs_of_view_row(row.min(self.rows - 1));
        let (start, end) = self.line_bounds_at(abs);
        self.selection = Some(Selection {
            anchor: (0, start),
            head: (self.cols - 1, end),
        });
    }

    /// The absolute-row span `(start_abs, end_abs)` inclusive of the
    /// soft-wrapped logical line containing absolute row `abs`. Pulled out
    /// of `select_line_at` so drag-extension (`extend_line_selection`) can
    /// reuse the same wrap-following logic.
    #[cfg(any(test, feature = "gui"))]
    fn line_bounds_at(&self, abs: usize) -> (usize, usize) {
        let total = self.scrollback.len() + self.rows;
        let (mut start, mut end) = (abs, abs);
        // Follow the soft-wrap bits across scrollback + screen (bounded like
        // logical_line_of, so a degenerate fully-wrapped history stays cheap).
        while start > 0 && abs - start < 128 && self.phys_line(start - 1).wrapped {
            start -= 1;
        }
        while end + 1 < total && end - abs < 128 && self.phys_line(end).wrapped {
            end += 1;
        }
        (start, end)
    }

    /// Extend a triple-click line selection as the drag moves, analogous to
    /// `extend_word_selection` but at whole-(soft-wrapped)-line granularity.
    /// `anchor_row`/`head_row` are absolute rows.
    #[cfg(any(test, feature = "gui"))]
    pub fn extend_line_selection(&mut self, anchor_row: usize, head_row: usize) {
        let (a_start, a_end) = self.line_bounds_at(anchor_row);
        let (h_start, h_end) = self.line_bounds_at(head_row);
        let dragging_back = head_row < anchor_row;
        let (start_row, end_row) = if dragging_back {
            (h_start, a_end)
        } else {
            (a_start, h_end)
        };
        self.selection = Some(Selection {
            anchor: (0, start_row),
            head: (self.cols.saturating_sub(1), end_row),
        });
    }

    /// The selected text, or `None` when nothing is selected. Lines join with
    /// `\n`; per-line trailing blanks are trimmed, wide-glyph trailers skipped,
    /// and grapheme continuations preserved.
    #[cfg(any(test, feature = "gui"))]
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_bounds()?;
        let mut out = String::new();
        for row in start.1..=end.1 {
            let (cells, _) = self.phys_row(row);
            let c0 = if row == start.1 { start.0 } else { 0 };
            let c1 = if row == end.1 { end.0 } else { self.cols - 1 };
            let mut line = String::new();
            let hi = c1.min(cells.len().saturating_sub(1));
            for cell in cells.get(c0..=hi).unwrap_or(&[]) {
                if cell.flags & WIDE_TRAILER != 0 {
                    continue;
                }
                line.push(cell.ch);
                if cell.cluster != 0
                    && let Some(suffix) = self.clusters.get((cell.cluster - 1) as usize)
                {
                    line.push_str(suffix);
                }
            }
            if row != start.1 {
                out.push('\n');
            }
            out.push_str(line.trim_end());
        }
        Some(out)
    }

    /// The current selection as styled HTML (a `<pre>` of per-run `<span>`s
    /// carrying color/bold/italic/underline/strike/dim), for rich clipboard
    /// copy (G29). `None` when nothing is selected. The alt/plain flavor is
    /// [`Self::selected_text`]; both walk the same cells.
    #[cfg(any(test, feature = "gui"))]
    pub fn selected_html(&self) -> Option<String> {
        let (start, end) = self.selection_bounds()?;
        let esc = |c: char, out: &mut String| match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        };
        let hex = |c: u32| format!("#{:06x}", c & 0xFF_FF_FF);
        let style_of = |cell: &Cell| -> (u32, u32, u16) {
            // Reverse video swaps the resolved pair; the run key is what the
            // reader sees, not the raw attribute.
            let (fg, bg) = if cell.flags & super::cell::ATTR_REVERSE != 0 {
                (cell.bg, cell.fg)
            } else {
                (cell.fg, cell.bg)
            };
            const STYLED: u16 = super::cell::ATTR_BOLD
                | super::cell::ATTR_ITALIC
                | super::cell::ATTR_UNDERLINE
                | super::cell::ATTR_STRIKE
                | super::cell::ATTR_DIM;
            (fg, bg, cell.flags & STYLED)
        };
        let mut out = format!(
            "<pre style=\"font-family:monospace;background:{};color:{}\">",
            hex(self.default_bg()),
            hex(self.default_fg())
        );
        for row in start.1..=end.1 {
            let (cells, _) = self.phys_row(row);
            let c0 = if row == start.1 { start.0 } else { 0 };
            let c1 = if row == end.1 { end.0 } else { self.cols - 1 };
            if row != start.1 {
                out.push('\n');
            }
            // Collect the styled chars, then trim trailing blanks like the
            // plain flavor does.
            let hi = c1.min(cells.len().saturating_sub(1));
            // One selected char: its cluster suffix and (fg, bg, flags) run key.
            type StyledChar<'a> = (char, Option<&'a str>, (u32, u32, u16));
            let mut line: Vec<StyledChar> = Vec::new();
            for cell in cells.get(c0..=hi).unwrap_or(&[]) {
                if cell.flags & WIDE_TRAILER != 0 {
                    continue;
                }
                let suffix = (cell.cluster != 0)
                    .then(|| self.clusters.get((cell.cluster - 1) as usize))
                    .flatten()
                    .map(String::as_str);
                line.push((cell.ch, suffix, style_of(cell)));
            }
            while line
                .last()
                .is_some_and(|(c, s, _)| *c == ' ' && s.is_none())
            {
                line.pop();
            }
            let mut open: Option<(u32, u32, u16)> = None;
            for (ch, suffix, style) in line {
                if open != Some(style) {
                    if open.is_some() {
                        out.push_str("</span>");
                    }
                    let (fg, bg, flags) = style;
                    let mut css = format!("color:{}", hex(fg));
                    if bg != self.default_bg() {
                        css.push_str(&format!(";background:{}", hex(bg)));
                    }
                    if flags & super::cell::ATTR_BOLD != 0 {
                        css.push_str(";font-weight:bold");
                    }
                    if flags & super::cell::ATTR_ITALIC != 0 {
                        css.push_str(";font-style:italic");
                    }
                    let deco: Vec<&str> = [
                        (flags & super::cell::ATTR_UNDERLINE != 0, "underline"),
                        (flags & super::cell::ATTR_STRIKE != 0, "line-through"),
                    ]
                    .iter()
                    .filter_map(|&(on, name)| on.then_some(name))
                    .collect();
                    if !deco.is_empty() {
                        css.push_str(&format!(";text-decoration:{}", deco.join(" ")));
                    }
                    if flags & super::cell::ATTR_DIM != 0 {
                        css.push_str(";opacity:.6");
                    }
                    out.push_str(&format!("<span style=\"{css}\">"));
                    open = Some(style);
                }
                esc(ch, &mut out);
                if let Some(s) = suffix {
                    for c in s.chars() {
                        esc(c, &mut out);
                    }
                }
            }
            if open.is_some() {
                out.push_str("</span>");
            }
        }
        out.push_str("</pre>");
        Some(out)
    }

    /// Append `ch` to the grapheme continuation of the cell at `(x, y)`,
    /// re-interning the grown suffix and marking the row dirty.
    fn append_to_glyph(&mut self, x: usize, y: usize, ch: char) {
        let mut suffix = std::mem::take(&mut self.cluster_scratch);
        suffix.clear();
        let cell = self.screen[y].cells()[x];
        let mut style = self.styles.get(cell.style());
        if let Some(prev) = style
            .cluster
            .checked_sub(1)
            .and_then(|i| self.clusters.get(usize::from(i)))
        {
            suffix.push_str(prev);
        }
        suffix.push(ch);
        style.cluster = self.intern_cluster(&suffix);
        self.cluster_scratch = suffix;
        self.screen[y].cells_mut()[x] = cell.with_style(self.styles.intern(style));
        self.touch(y);
    }

    /// Return the id for grapheme continuation `suffix`, interning it on first
    /// use. Ids are `index + 1` so `0` means "no continuation". Returns `0` once
    /// the table is full (the continuation is dropped; the base glyph remains).
    fn intern_cluster(&mut self, suffix: &str) -> u16 {
        if let Some(&id) = self.cluster_ids.get(suffix) {
            return id;
        }
        if self.clusters.len() >= CLUSTER_MAX {
            return 0;
        }
        self.clusters.push(suffix.to_owned());
        let id = self.clusters.len() as u16;
        self.cluster_ids.insert(suffix.to_owned(), id);
        self.clusters_arc = OnceLock::new();
        id
    }

    /// The grapheme table as handed to a frame: shared until the next insert.
    pub(crate) fn clusters_arc(&self) -> Arc<[String]> {
        Arc::clone(
            self.clusters_arc
                .get_or_init(|| self.clusters.clone().into()),
        )
    }

    /// Rebuild the caches derived from `clusters` after a restore replaced it.
    /// Every entry is indexed: an RTSN snapshot may hold up to `u16::MAX`
    /// (every id a cell can name), more than [`CLUSTER_MAX`], and a longer
    /// table indexes only the ids a cell can name instead of overflowing.
    pub(crate) fn rebuild_cluster_caches(&mut self) {
        self.cluster_ids = self
            .clusters
            .iter()
            .zip(1..=u16::MAX)
            .map(|(c, id)| (c.clone(), id))
            .collect();
        self.clusters_arc = OnceLock::new();
    }

    /// Heap a checkpoint decode spends rebuilding the tables derived from
    /// `links` and `clusters` (their `Arc` copies and the cluster id index,
    /// with the map's spare capacity). Encode charges the same amount, so a
    /// state that encodes within a budget also decodes within it.
    fn table_cache_cost(&self) -> usize {
        let bytes = |t: &[String]| t.iter().map(String::len).sum::<usize>();
        let index_entry = 2 * (size_of::<(String, u16)>() + 1);
        (self.links.len() + self.clusters.len())
            .saturating_mul(2 * size_of::<String>())
            .saturating_add(self.clusters.len().saturating_mul(index_entry))
            .saturating_add(4 * size_of::<usize>())
            .saturating_add(bytes(&self.links))
            .saturating_add(bytes(&self.clusters).saturating_mul(2))
    }

    /// Blank columns `[from, to)` of row `y`, marking it dirty.
    /// DECSEL: erase the line like EL `mode` (0 to end, 1 to start, 2 all),
    /// skipping cells protected by DECSCA.
    pub(crate) fn selective_erase_line(&mut self, mode: usize) {
        let (from, to) = match mode {
            1 => (0, self.cursor.0.min(self.cols.saturating_sub(1))),
            2 => (0, self.cols.saturating_sub(1)),
            _ => (
                self.cursor.0.min(self.cols.saturating_sub(1)),
                self.cols.saturating_sub(1),
            ),
        };
        self.selective_clear(self.cursor.1, self.cursor.1, Some((from, to)));
    }

    /// DECSED: erase the display like ED `mode`, skipping DECSCA-protected
    /// cells. Unlike ED 2, protected content survives, so this never takes
    /// the clear-whole-rows fast path.
    pub(crate) fn selective_erase_display(&mut self, mode: usize) {
        let last = self.rows.saturating_sub(1);
        match mode {
            1 => {
                if self.cursor.1 > 0 {
                    self.selective_clear(0, self.cursor.1 - 1, None);
                }
                self.selective_erase_line(1);
            }
            2 => self.selective_clear(0, last, None),
            _ => {
                self.selective_erase_line(0);
                if self.cursor.1 < last {
                    self.selective_clear(self.cursor.1 + 1, last, None);
                }
            }
        }
    }

    /// Erase every unprotected cell in rows `y0..=y1` (columns limited to
    /// `cols` when given, inclusive).
    fn selective_clear(&mut self, y0: usize, y1: usize, cols: Option<(usize, usize)>) {
        let erase = self.erase_packed();
        let (c0, c1) = cols.unwrap_or((0, self.cols.saturating_sub(1)));
        for y in y0..=y1.min(self.rows.saturating_sub(1)) {
            for x in c0..=c1.min(self.cols.saturating_sub(1)) {
                let cell = &mut self.screen[y].cells_mut()[x];
                if !cell.is_protected() {
                    *cell = erase;
                }
            }
            self.touch(y);
        }
    }

    pub(crate) fn clear_row_range(&mut self, y: usize, from: usize, to: usize) {
        if y >= self.rows {
            return;
        }
        let to = to.min(self.cols);
        if from >= to {
            return;
        }
        let blank = self.erase_packed();
        let line = &mut self.screen[y];
        line.cells_mut()[from..to].fill(blank);
        // Erasing through the right margin ends the line here: it no longer
        // wraps into the row below (EL, the tail of ED, ECH at the margin).
        if to >= self.cols {
            line.wrapped = false;
        }
        self.touch(y);
    }

    /// DECERA — blank the rectangle `[top, bottom] × [left, right]` (0-based,
    /// inclusive) in the default colors, the same erase convention
    /// [`Self::clear_row_range`] uses. Out-of-range or inverted bounds
    /// (`top > bottom`, `left > right`) are a no-op rather than panicking —
    /// a malformed sequence from the child shouldn't crash the terminal.
    pub(crate) fn erase_rect(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        let bottom = bottom.min(self.rows.saturating_sub(1));
        let right = right.min(self.cols.saturating_sub(1));
        if top > bottom || left > right || top >= self.rows || left >= self.cols {
            return;
        }
        let blank = self.erase_packed();
        for y in top..=bottom {
            self.screen[y].cells_mut()[left..=right].fill(blank);
            self.touch(y);
        }
    }

    /// DECFRA — fill the rectangle `[top, bottom] × [left, right]` (0-based,
    /// inclusive) with `ch` in `pen`'s current colors/attributes. Same
    /// bounds handling as [`Self::erase_rect`].
    pub(crate) fn fill_rect(
        &mut self,
        ch: char,
        pen: Pen,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        let bottom = bottom.min(self.rows.saturating_sub(1));
        let right = right.min(self.cols.saturating_sub(1));
        if top > bottom || left > right || top >= self.rows || left >= self.cols {
            return;
        }
        let cell = Cell {
            ch,
            cluster: 0,
            fg: pen.fg,
            bg: pen.bg,
            flags: pen.attrs,
            link: 0,
            underline_color: pen.underline_color,
        };
        let cell = self.pack(cell);
        for y in top..=bottom {
            self.screen[y].cells_mut()[left..=right].fill(cell);
            self.touch(y);
        }
    }

    /// DECCRA — copy the rectangle `[top, bottom] × [left, right]` (0-based,
    /// inclusive) to a destination whose top-left corner is `(dst_top,
    /// dst_left)`; both footprints are independently clamped to the grid
    /// (a destination near an edge copies only the portion that fits).
    /// Snapshots the source before writing, so an overlapping destination
    /// (shifting a rectangle down-and-right onto itself, say) can't corrupt
    /// still-unread source rows.
    pub(crate) fn copy_rect(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        dst_top: usize,
        dst_left: usize,
    ) {
        let bottom = bottom.min(self.rows.saturating_sub(1));
        let right = right.min(self.cols.saturating_sub(1));
        if top > bottom || left > right || top >= self.rows || left >= self.cols {
            return;
        }
        if dst_top >= self.rows || dst_left >= self.cols {
            return;
        }
        let h = (bottom - top + 1).min(self.rows - dst_top);
        let w = (right - left + 1).min(self.cols - dst_left);
        let mut buf = Vec::with_capacity(h * w);
        for y in 0..h {
            buf.extend_from_slice(&self.screen[top + y].cells()[left..left + w]);
        }
        for (y, src) in buf.chunks(w).enumerate() {
            self.screen[dst_top + y].cells_mut()[dst_left..dst_left + w].copy_from_slice(src);
            self.touch(dst_top + y);
        }
    }

    /// DECCOLM (`CSI ? 3 h/l`) while mode 40 allows it: the side effects
    /// only. The screen is cleared, every margin reset and the cursor homed,
    /// but the grid keeps its size. In a mux the server owns the PTY size and
    /// each client its own viewport, so a program must not force 80 or 132
    /// columns (Ghostty resizes the grid here; this deliberately does not).
    pub(crate) fn deccolm(&mut self, wide: bool) {
        self.column_mode_132 = wide;
        self.reset_scroll_region();
        self.left_margin = 0;
        self.right_margin = self.cols.saturating_sub(1);
        self.clear_all(); // also homes the cursor
    }

    /// Blank every cell and home the cursor (used by `CSI 2 J`).
    pub(crate) fn clear_all(&mut self) {
        let blank = self.erase_packed();
        for line in &mut self.screen {
            line.fill(blank);
        }
        #[cfg(any(test, feature = "gui"))]
        self.images.clear();
        self.touch_all();
        self.cursor = (0, 0);
    }

    /// Erase saved lines (xterm `ED 3`): drop the scrollback history, the
    /// prompt marks that index into it, and any scrolled view of it. The
    /// visible screen is untouched — `ED 3` callers pair this with
    /// [`Grid::clear_all`].
    pub(crate) fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.styles.request_sweep();
        self.touch_hist();
        self.prompt_marks.clear();
        self.view_offset = 0;
        self.touch_all();
    }

    /// The fill used by every erase / scroll-clear path: a blank in the
    /// default foreground on the current SGR background (BCE, as xterm does),
    /// falling back to the default background, so an OSC 11 theme also colors
    /// cleared regions.
    fn erase_cell(&self) -> Cell {
        let mut c = self.default_cell();
        if let Some(bg) = self.erase_bg {
            c.bg = bg;
            c.flags &= !crate::cell::ATTR_BG_DEFAULT;
        }
        c
    }

    /// [`Grid::erase_cell`], stored. Every scroll and erase takes one, so it
    /// goes through the pen cache rather than the table's hash index.
    fn erase_packed(&mut self) -> PackedCell {
        let cell = self.erase_cell();
        let pen = Pen {
            fg: cell.fg,
            bg: cell.bg,
            attrs: cell.flags,
            underline_color: cell.underline_color,
        };
        PackedCell::glyph(cell.ch, false, self.pen_id(pen, cell.link))
    }

    /// [`Grid::default_cell`], stored.
    fn default_packed(&mut self) -> PackedCell {
        let cell = self.default_cell();
        self.pack(cell)
    }

    /// Make [`Grid::blank_row`] `cols` copies of `fill`, rebuilding it only
    /// when the width or the fill differs. The row is uniform, so its first
    /// cell stands for all of them.
    fn prime_blank_row(&mut self, fill: PackedCell) {
        let stale =
            self.blank_row.len() != self.cols || self.blank_row.first().is_some_and(|c| *c != fill);
        if stale {
            self.blank_row.clear();
            self.blank_row.resize(self.cols, fill);
        }
    }

    /// A blank cell in the default colors, for fills that are not erases
    /// (resize, theme application, full reset).
    fn default_cell(&self) -> Cell {
        let d = self.styles.defaults();
        Cell {
            fg: d.fg,
            bg: d.bg,
            underline_color: d.fg,
            ..Cell::default_blank()
        }
    }

    /// The default foreground (OSC 10), which cells marked default show.
    fn default_fg(&self) -> u32 {
        self.styles.defaults().fg
    }

    /// The default background (OSC 11), which cells marked default show.
    fn default_bg(&self) -> u32 {
        self.styles.defaults().bg
    }

    /// Set the background erases fill with (see [`Grid::erase_cell`]):
    /// `Some(rgb)` for an SGR background, `None` for the default.
    pub(crate) fn set_erase_bg(&mut self, bg: Option<u32>) {
        self.erase_bg = bg;
    }

    /// Update the default foreground/background/cursor colors (OSC 10/11/12
    /// and their resets). Mirrors the parser's palette so subsequent erases
    /// fill with the new background and the windowed cursor paints in the new
    /// cursor color. Every cell whose colour is marked default
    /// ([`crate::ATTR_FG_DEFAULT`] / [`crate::ATTR_BG_DEFAULT`]) follows, on
    /// screen and in history; colours the child chose do not.
    pub(crate) fn set_default_colors(&mut self, fg: u32, bg: u32, cursor: u32) {
        let defaults = Defaults { fg, bg };
        if self.styles.defaults() != defaults {
            self.styles.set_defaults(defaults);
            self.reset_damage();
            self.touch_hist();
            self.epoch += 1;
        }
        if self.cursor_color != cursor {
            self.cursor_color = cursor;
            // Repaint the cursor cell's row so a pure OSC 12 change shows.
            let row = self.cursor.1;
            if row < self.rows {
                self.touch(row);
            }
        }
    }

    /// Apply DECSCUSR (`CSI Ps SP q`): set the cursor shape and blink, repainting
    /// the cursor's row so the change shows.
    pub(crate) fn set_cursor_style(&mut self, shape: CursorShape, blink: bool) {
        self.cursor_shape = shape;
        self.cursor_blink = blink;
        if self.cursor.1 < self.rows {
            self.touch(self.cursor.1);
        }
    }

    /// Set the power-on default cursor (from config) and apply it now. RIS and
    /// DECSTR restore to this default.
    pub fn set_default_cursor(&mut self, shape: CursorShape, blink: bool) {
        self.default_cursor_shape = shape;
        self.default_cursor_blink = blink;
        self.set_cursor_style(shape, blink);
    }

    /// Number of lines currently held in scrollback (folds not applied).
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Release the memory of trailing padding in up to `budget` history lines
    /// not yet looked at, oldest first. Readers see the same cells, so call
    /// it whenever the terminal is idle; it is never needed for correctness.
    /// `true` while lines remain to look at.
    pub fn compact_history(&mut self, budget: usize) -> bool {
        let len = self.scrollback.len();
        let pending = self.untrimmed.min(len);
        let take = pending.min(budget);
        let first = len - pending;
        for line in self.scrollback.range_mut(first..first + take) {
            trim_padding(line, &self.styles);
        }
        self.untrimmed = pending - take;
        self.untrimmed > 0
    }

    /// Override the scrollback line cap (the `scrollback` config key). `0`
    /// disables history. An already-overfull buffer is trimmed immediately.
    pub fn set_scrollback_max(&mut self, max: usize) {
        self.scrollback_max = max;
        while self.scrollback.len() > max {
            if let Some(evicted) = self.scrollback.pop_front() {
                self.styles.note_evicted(evicted.cells().len());
            }
            self.touch_hist();
            self.evict_prompt_mark();
            #[cfg(feature = "l13")]
            if let Some(s) = &mut self.command_start {
                *s = s.saturating_sub(1);
            }
            #[cfg(any(test, feature = "gui"))]
            {
                if let Some(s) = &mut self.fold_pending_start {
                    *s = s.saturating_sub(1);
                }
                self.evict_fold_blocks();
            }
        }
        self.view_offset = self.view_offset.min(self.display_history_len());
    }

    /// Seed the grid's default colors from the configured startup theme and
    /// repaint the (still pristine) screen in them. Called once at startup,
    /// before any child output is parsed — the screen is all blank cells, so
    /// refilling is exact, not lossy.
    pub fn apply_theme(&mut self, theme: &super::Theme) {
        self.set_default_colors(theme.fg, theme.bg, theme.cursor);
        self.screen = blank_screen(self.cols, self.rows, self.default_packed());
        self.reset_damage();
    }

    /// Live theme switch (config reload): remap every color that resolves to
    /// an `old`-theme entry — across the live screen, scrollback, and the
    /// stashed primary screen — to its `new`-theme counterpart, and adopt the
    /// new defaults. Truecolor/256-cube colors pass through untouched (we
    /// can't know they meant "the theme's red"). Full repaint requested.
    pub fn retheme(&mut self, old: &super::Theme, new: &super::Theme) {
        let map = |c: u32| super::color::remap(c, old, new);
        // Cells hold style ids, so only the table changes; styles that become
        // equal merge, and only then are the cells rewritten. Cells in the
        // default colours follow the new defaults without a remap.
        self.blank_row.clear();
        if let Some(remap) = self.styles.retheme(map) {
            self.apply_remap(&remap);
        }
        let d = self.styles.defaults();
        self.styles.set_defaults(Defaults {
            fg: map(d.fg),
            bg: map(d.bg),
        });
        if let Some(bg) = &mut self.erase_bg {
            *bg = map(*bg);
        }
        self.cursor_color = super::color::remap(self.cursor_color, old, new);
        self.reset_damage();
        self.touch_hist();
        self.epoch += 1;
    }

    /// Resize the grid to `cols`×`rows`.
    ///
    /// On the primary screen this is a **wrap-aware reflow**: soft-wrapped runs
    /// across scrollback + the live screen are rejoined into logical lines and
    /// re-wrapped to the new width, so narrowing rewraps long lines (pushing the
    /// overflow into history) and widening pulls wrapped continuations — and
    /// lines back out of scrollback — instead of truncating. The cursor, prompt
    /// marks, and per-row line-size attributes ride through the reflow. The saved
    /// (DECSC) cursor is clamped. Every row is marked dirty for a full repaint.
    ///
    /// On the alternate screen the live buffer is merely clipped/extended (its
    /// full-screen app repaints on resize); the *stashed* primary is reflowed so
    /// it is correct when the app exits the alt screen.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 || (cols == self.cols && rows == self.rows) {
            return;
        }
        // DECSLRM margins are width-relative; a real resize resets them (the
        // simple, xterm-compatible choice — apps re-establish them).
        self.left_margin = 0;
        self.right_margin = cols.saturating_sub(1);
        #[cfg(any(test, feature = "gui"))]
        self.images.clear(); // serials/reflow change; drop placed images
        let old_cols = self.cols;
        let clamp = |(x, y): (usize, usize)| (x.min(cols - 1), y.min(rows - 1));
        let blank = self.default_packed(); // a resize is not an erase: no BCE
        // Preserve tab stops within the surviving width; default the new columns.
        let mut stops = default_tab_stops(cols);
        let keep = cols.min(old_cols);
        stops[..keep].copy_from_slice(&self.tab_stops[..keep]);
        self.tab_stops = stops;

        // The in-flight command-capture anchor (l13) rides the reflow like a
        // prompt mark, so a mid-command resize preserves the capture.
        #[cfg(feature = "l13")]
        let cmd_start = self.command_start;
        #[cfg(not(feature = "l13"))]
        let cmd_start: Option<usize> = None;
        #[cfg(any(test, feature = "gui"))]
        let (fold_start, fold_blocks) = (self.fold_pending_start, self.fold_blocks.clone());
        #[cfg(not(any(test, feature = "gui")))]
        let (fold_start, fold_blocks): (Option<usize>, Vec<CommandBlock>) = (None, Vec::new());
        if let Some(primary) = self.primary.take() {
            let r = reflow_history(
                std::mem::take(&mut self.scrollback),
                primary.screen,
                old_cols,
                primary.cursor,
                &self.prompt_marks,
                cmd_start,
                fold_start,
                &fold_blocks,
                cols,
                rows,
                blank,
                self.scrollback_max,
                &self.styles,
            );
            self.scrollback = r.scrollback;
            self.untrimmed = self.scrollback.len();
            self.touch_hist();
            self.prompt_marks = r.prompt_marks;
            #[cfg(feature = "l13")]
            {
                self.command_start = r.command_start;
            }
            #[cfg(any(test, feature = "gui"))]
            {
                self.fold_pending_start = r.fold_pending_start;
                self.fold_blocks = r.fold_blocks;
            }
            self.primary = Some(SavedScreen {
                screen: r.screen,
                cursor: r.cursor,
                saved_cursor: clamp(primary.saved_cursor),
                mode: primary.mode,
            });
            self.screen = reflow_clip(&self.screen, cols, rows, blank);
            self.cursor = clamp(self.cursor);
            self.saved_cursor = clamp(self.saved_cursor);
        } else {
            let r = reflow_history(
                std::mem::take(&mut self.scrollback),
                std::mem::take(&mut self.screen),
                old_cols,
                self.cursor,
                &self.prompt_marks,
                cmd_start,
                fold_start,
                &fold_blocks,
                cols,
                rows,
                blank,
                self.scrollback_max,
                &self.styles,
            );
            self.scrollback = r.scrollback;
            self.untrimmed = self.scrollback.len();
            self.touch_hist();
            self.screen = r.screen;
            self.cursor = r.cursor;
            self.prompt_marks = r.prompt_marks;
            #[cfg(feature = "l13")]
            {
                self.command_start = r.command_start;
            }
            #[cfg(any(test, feature = "gui"))]
            {
                self.fold_pending_start = r.fold_pending_start;
                self.fold_blocks = r.fold_blocks;
            }
            self.saved_cursor = clamp(self.saved_cursor);
        }

        self.cols = cols;
        self.rows = rows;
        self.reset_damage();
        self.view_offset = 0;
        if let Some(s) = &mut self.status_line {
            s.relayout(cols);
        }
        self.reset_scroll_region();
        self.styles.request_sweep();
        self.sweep_styles();
    }

    /// Switch to the alternate screen, stashing the primary buffer. Only
    /// `?1049` saves the cursor and homes it; `?47`/`?1047` swap the buffer and
    /// leave the cursor where it is. No-op if already on the alternate screen.
    pub(crate) fn enter_alt_screen(&mut self, mode: AltMode) {
        if self.primary.is_some() {
            return;
        }
        #[cfg(any(test, feature = "gui"))]
        self.images.clear();
        let fill = self.erase_packed();
        let alt = self.take_spare_screen(fill);
        self.primary = Some(SavedScreen {
            screen: std::mem::replace(&mut self.screen, alt),
            cursor: self.cursor,
            saved_cursor: self.saved_cursor,
            mode,
        });
        if mode == AltMode::Dec1049 {
            self.cursor = (0, 0);
        }
        // History isn't browsable under a full-screen app; snap to the live view.
        self.view_offset = 0;
        self.reset_scroll_region();
        self.touch_all();
    }

    /// The spare rows (see [`Grid::spare_screen`]) sized to the grid and filled
    /// with `fill`.
    fn take_spare_screen(&mut self, fill: PackedCell) -> Vec<Line> {
        let mut screen = std::mem::take(&mut self.spare_screen);
        screen.resize_with(self.rows, Line::default);
        self.prime_blank_row(fill);
        for line in &mut screen {
            line.clear(&self.blank_row);
        }
        screen
    }

    /// Switch back to the primary screen, restoring its buffer. For `?1049` the
    /// cursor (and DECSC register) saved on entry are restored; `?47`/`?1047`
    /// leave the cursor as the alternate session left it. No-op if not on the
    /// alternate screen.
    pub(crate) fn leave_alt_screen(&mut self) {
        if let Some(saved) = self.primary.take() {
            self.spare_screen = std::mem::replace(&mut self.screen, saved.screen);
            if saved.mode == AltMode::Dec1049 {
                self.cursor = (
                    saved.cursor.0.min(self.cols.saturating_sub(1)),
                    saved.cursor.1.min(self.rows.saturating_sub(1)),
                );
                self.saved_cursor = (
                    saved.saved_cursor.0.min(self.cols.saturating_sub(1)),
                    saved.saved_cursor.1.min(self.rows.saturating_sub(1)),
                );
            }
            self.reset_scroll_region();
            self.touch_all();
        }
    }

    /// Whether the alternate screen buffer (`?47`/`?1047`/`?1049`) is active.
    pub fn in_alt_screen(&self) -> bool {
        self.primary.is_some()
    }

    /// Save the current cursor position (`DECSC` / `CSI s`).
    pub(crate) fn save_cursor(&mut self) {
        self.saved_cursor = self.cursor;
    }

    /// Restore the saved cursor position (`DECRC` / `CSI u`), clamped.
    pub(crate) fn restore_cursor(&mut self) {
        let (x, y) = self.saved_cursor;
        self.set_cursor(x, y);
    }

    /// Delete `n` characters at the cursor, shifting the remainder of the row
    /// left and blanking the freed cells at the right (`DCH`).
    pub(crate) fn delete_chars(&mut self, n: usize) {
        let (x, y) = self.cursor;
        if y >= self.rows || x >= self.cols || n == 0 {
            return;
        }
        let n = n.min(self.cols - x);
        let blank = self.erase_packed();
        let row = self.screen[y].cells_mut();
        row[x..].rotate_left(n);
        let end = row.len();
        row[end - n..].fill(blank);
        self.touch(y);
    }

    /// Insert `n` blank characters at the cursor, shifting the rest of the row
    /// right and dropping cells that fall off the right margin (`ICH`).
    pub(crate) fn insert_chars(&mut self, n: usize) {
        let (x, y) = self.cursor;
        if y >= self.rows || x >= self.cols || n == 0 {
            return;
        }
        let n = n.min(self.cols - x);
        let blank = self.erase_packed();
        let row = self.screen[y].cells_mut();
        row[x..].rotate_right(n);
        row[x..x + n].fill(blank);
        self.touch(y);
    }

    /// Blank `n` characters starting at the cursor without shifting (`ECH`).
    pub(crate) fn erase_chars(&mut self, n: usize) {
        let (x, y) = self.cursor;
        let n = n.min(self.cols.saturating_sub(x));
        self.clear_row_range(y, x, x + n);
    }

    /// Insert `n` blank lines at the cursor row, pushing the rows below it down
    /// within the scrolling region; rows pushed past the region bottom are lost
    /// (`IL`). A no-op when the cursor is outside the scrolling region.
    pub(crate) fn insert_lines(&mut self, n: usize) {
        let cy = self.cursor.1;
        if cy < self.scroll_top || cy > self.scroll_bottom {
            return;
        }
        if self.side_margins_active() {
            // VT420: IL requires the cursor within the side margins, and
            // shifts only the margin band.
            if self.cursor.0 < self.left_margin || self.cursor.0 > self.right_margin {
                return;
            }
            let n = n.min(self.scroll_bottom + 1 - cy);
            let (l, r) = (self.left_margin, self.right_margin.min(self.cols - 1));
            let blank = self.erase_packed();
            for y in ((cy + n)..=self.scroll_bottom).rev() {
                self.copy_band(y - n, y, l, r);
            }
            for line in &mut self.screen[cy..(cy + n).min(self.scroll_bottom + 1)] {
                line.cells_mut()[l..=r].fill(blank);
            }
            self.touch_rows(cy..=self.scroll_bottom);
            return;
        }
        let n = n.min(self.scroll_bottom + 1 - cy);
        // Rotate rows [cy, scroll_bottom] down by n; the n rows that fall off
        // the region bottom come round to the cursor row and are blanked.
        let blank = self.erase_packed();
        self.screen[cy..=self.scroll_bottom].rotate_right(n);
        self.prime_blank_row(blank);
        for line in &mut self.screen[cy..cy + n] {
            line.clear(&self.blank_row);
        }
        self.touch_rows(cy..=self.scroll_bottom);
    }

    /// Delete `n` lines at the cursor row, pulling the rows below it up within
    /// the scrolling region and blanking the freed rows at the region bottom
    /// (`DL`). A no-op when the cursor is outside the scrolling region.
    pub(crate) fn delete_lines(&mut self, n: usize) {
        let cy = self.cursor.1;
        if cy < self.scroll_top || cy > self.scroll_bottom {
            return;
        }
        if self.side_margins_active() {
            if self.cursor.0 < self.left_margin || self.cursor.0 > self.right_margin {
                return;
            }
            let n = n.min(self.scroll_bottom + 1 - cy);
            let (l, r) = (self.left_margin, self.right_margin.min(self.cols - 1));
            let blank = self.erase_packed();
            // `n` can reach `scroll_bottom + 1 - cy` (deleting the whole
            // region), at which point `scroll_bottom - n` underflows when
            // `cy == 0` — e.g. `CSI 9999 M` with DECSLRM margins active and
            // the cursor parked on row 0. `checked_sub` skips the copy
            // instead of panicking; there's nothing to shift up in that case.
            if let Some(copy_end) = self.scroll_bottom.checked_sub(n) {
                for y in cy..=copy_end {
                    self.copy_band(y + n, y, l, r);
                }
            }
            for line in &mut self.screen[(self.scroll_bottom + 1 - n)..=self.scroll_bottom] {
                line.cells_mut()[l..=r].fill(blank);
            }
            self.touch_rows(cy..=self.scroll_bottom);
            return;
        }
        let n = n.min(self.scroll_bottom + 1 - cy);
        // Rotate rows [cy, scroll_bottom] up by n; the n deleted rows come
        // round to the region bottom and are blanked.
        let blank = self.erase_packed();
        self.screen[cy..=self.scroll_bottom].rotate_left(n);
        self.prime_blank_row(blank);
        for line in &mut self.screen[self.scroll_bottom + 1 - n..=self.scroll_bottom] {
            line.clear(&self.blank_row);
        }
        self.touch_rows(cy..=self.scroll_bottom);
    }

    /// Scroll the current scrolling region up by `n` rows (`SU`). Reuses the
    /// single-row [`Grid::scroll_up`] per line, so a full-screen scroll on the
    /// primary buffer captures the displaced lines into scrollback exactly as a
    /// line feed would. `n` is clamped to the region height: scrolling past it
    /// clears the region, and the cap bounds the per-row loop against a hostile
    /// count (e.g. `CSI 9999999999 S`) that would otherwise run for minutes.
    pub(crate) fn scroll_up_n(&mut self, n: usize) {
        if self.side_margins_active() {
            let h = self.scroll_bottom.saturating_sub(self.scroll_top) + 1;
            for _ in 0..n.min(h) {
                self.scroll_band_up();
            }
            return;
        }
        let n = n.min(self.scroll_bottom + 1 - self.scroll_top);
        for _ in 0..n {
            self.scroll_up();
        }
    }

    /// Scroll the current scrolling region down by `n` rows (`SD`): shift the
    /// region's rows down and blank the `n` freed rows at the top. Displaced
    /// bottom rows are lost (scrollback is never un-scrolled).
    pub(crate) fn scroll_down_n(&mut self, n: usize) {
        if self.side_margins_active() {
            let h = self.scroll_bottom.saturating_sub(self.scroll_top) + 1;
            for _ in 0..n.min(h) {
                self.scroll_band_down();
            }
            return;
        }
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        if bottom <= top || bottom >= self.rows {
            return;
        }
        let n = n.min(bottom + 1 - top);
        // Rotate the region down by n and blank the n rows freed at its top.
        let blank = self.erase_packed();
        self.screen[top..=bottom].rotate_right(n);
        self.prime_blank_row(blank);
        for line in &mut self.screen[top..top + n] {
            line.clear(&self.blank_row);
        }
        self.touch_rows(top..=bottom);
    }

    /// Move the cursor up one row, scrolling the region down when already at its
    /// top (`RI`, reverse index). The mirror of a line feed at the region bottom.
    pub(crate) fn reverse_index(&mut self) {
        if self.cursor.1 == self.scroll_top {
            if self.side_margins_active() {
                self.scroll_band_down();
            } else {
                self.scroll_down_n(1);
            }
        } else if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        }
    }

    /// Set the current cursor row's line size attribute (`ESC # 3/4/5/6`).
    pub(crate) fn set_line_attr(&mut self, attr: LineAttr) {
        let y = self.cursor.1;
        if let Some(line) = self.screen.get_mut(y) {
            line.attr = attr;
            self.touch(y);
        }
    }

    /// Move the cursor forward `n` tab stops (`HT` / `CHT`), without writing
    /// over the cells it passes. Stops at the right margin when no further tab
    /// stop exists.
    pub(crate) fn tab_forward(&mut self, n: usize) {
        let last = self.cols.saturating_sub(1);
        let mut x = self.cursor.0;
        for _ in 0..n {
            if x >= last {
                x = last;
                break;
            }
            let mut nx = x + 1;
            while nx < last && !self.tab_stops[nx] {
                nx += 1;
            }
            x = nx;
        }
        self.cursor.0 = x.min(last);
    }

    /// Move the cursor back `n` tab stops (`CBT`). Stops at column 0.
    pub(crate) fn tab_backward(&mut self, n: usize) {
        let mut x = self.cursor.0.min(self.cols.saturating_sub(1));
        for _ in 0..n {
            if x == 0 {
                break;
            }
            let mut nx = x - 1;
            while nx > 0 && !self.tab_stops[nx] {
                nx -= 1;
            }
            x = nx;
        }
        self.cursor.0 = x;
    }

    /// Set a tab stop at the current cursor column (`HTS`).
    pub(crate) fn set_tab_stop(&mut self) {
        let x = self.cursor.0;
        if x < self.tab_stops.len() {
            self.tab_stops[x] = true;
        }
    }

    /// Clear the tab stop at the current cursor column (`TBC 0`).
    pub(crate) fn clear_tab_stop(&mut self) {
        let x = self.cursor.0;
        if x < self.tab_stops.len() {
            self.tab_stops[x] = false;
        }
    }

    /// Clear every tab stop (`TBC 3`).
    pub(crate) fn clear_all_tab_stops(&mut self) {
        self.tab_stops.iter_mut().for_each(|s| *s = false);
    }

    /// Full reset (`RIS`): return the grid to its power-on state — blank primary
    /// screen, home cursor, full-screen scroll region, default tab stops,
    /// cleared scrollback, cursor visible, autowrap on. The window title and cwd
    /// are intentionally left alone (a hardware reset doesn't relabel the tab).
    /// The parser separately resets its pen and re-syncs the default colors
    /// (via [`Grid::set_default_colors`]) *before* calling this, so the blank
    /// fill below lands in the configured theme's colors.
    pub(crate) fn reset(&mut self) {
        self.bell = false;
        self.progress = None;
        self.kitty_images.clear();
        self.kitty_virtual.clear();
        self.report_color_scheme = false;
        self.notif99_pending.clear();
        self.primary = None; // leave the alternate screen if active
        self.erase_bg = None; // RIS resets the SGR pen too
        self.grapheme_clustering = false; // and every mode (DECSTR keeps 2027)
        self.allow_deccolm = false;
        self.column_mode_132 = false;
        self.screen = blank_screen(self.cols, self.rows, self.default_packed());
        #[cfg(any(test, feature = "gui"))]
        self.images.clear();
        self.reset_damage();
        self.cursor = (0, 0);
        self.saved_cursor = (0, 0);
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        self.scrollback.clear();
        self.styles.request_sweep();
        self.touch_hist();
        self.prompt_marks.clear();
        #[cfg(any(test, feature = "gui"))]
        {
            self.fold_pending_start = None;
            self.fold_blocks.clear();
        }
        self.view_offset = 0;
        self.tab_stops = default_tab_stops(self.cols);
        self.cursor_visible = true;
        self.bracketed_paste = false;
        self.app_cursor_keys = false;
        self.alt_scroll = false;
        self.focus_reporting = false;
        self.win32_input = false;
        self.app_keypad = false;
        self.line_feed_new_line = false;
        self.kitty_flags_stack.clear();
        self.mouse_modes = MouseModes::default();
        self.cursor_icon = None;
        self.sync_output = false;
        self.sync_output_since = None;
        self.selection = None;
        self.autowrap = true;
        self.origin_mode = false;
        self.lr_margin_mode = false;
        self.left_margin = 0;
        self.right_margin = self.cols.saturating_sub(1);
        self.insert_mode = false;
        self.cursor_shape = self.default_cursor_shape;
        self.cursor_blink = self.default_cursor_blink;
        // default_fg/bg are intentionally untouched: the parser re-syncs them
        // from its (theme-seeded) palette right after — see RIS handling.
        self.current_link = 0;
    }

    /// Soft reset (`DECSTR`): reset terminal modes without clearing the screen
    /// or moving the active cursor — full-screen scroll region, saved cursor to
    /// home, cursor visible, autowrap on. The parser separately resets its pen.
    pub(crate) fn soft_reset(&mut self) {
        self.erase_bg = None; // DECSTR resets the SGR pen too
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        self.saved_cursor = (0, 0);
        self.cursor_visible = true;
        self.bracketed_paste = false;
        self.app_cursor_keys = false;
        self.alt_scroll = false;
        self.focus_reporting = false;
        self.win32_input = false;
        self.app_keypad = false;
        self.line_feed_new_line = false;
        self.kitty_flags_stack.clear();
        self.mouse_modes = MouseModes::default();
        self.cursor_icon = None;
        self.sync_output = false;
        self.sync_output_since = None;
        self.selection = None;
        self.autowrap = true;
        self.origin_mode = false;
        self.lr_margin_mode = false;
        self.left_margin = 0;
        self.right_margin = self.cols.saturating_sub(1);
        self.insert_mode = false;
        self.cursor_shape = self.default_cursor_shape;
        self.cursor_blink = self.default_cursor_blink;
        // default_fg/bg are intentionally untouched: the parser re-syncs them
        // from its (theme-seeded) palette right after — see DECSTR handling.
        self.current_link = 0;
        for line in &mut self.screen {
            line.attr = LineAttr::Single;
        }
    }

    /// Screen-alignment test (`DECALN`, `ESC # 8`): fill every cell with `E` and
    /// home the cursor. Used to check character positioning.
    pub(crate) fn fill_alignment(&mut self) {
        let mut e = self.default_cell();
        e.ch = 'E';
        let e = self.pack(e);
        self.prime_blank_row(e);
        for line in &mut self.screen {
            line.clear(&self.blank_row);
        }
        self.touch_all();
        self.cursor = (0, 0);
    }

    /// Mark row `y` changed: dirty, with a fresh stamp.
    fn touch(&mut self, y: usize) {
        self.stamp += 1;
        self.dirty[y] = true;
        self.row_stamp[y] = self.stamp;
    }

    /// Mark rows `r` changed under one fresh stamp.
    fn touch_rows(&mut self, r: std::ops::RangeInclusive<usize>) {
        self.stamp += 1;
        self.dirty[r.clone()].fill(true);
        self.row_stamp[r].fill(self.stamp);
    }

    /// Mark every row changed.
    fn touch_all(&mut self) {
        self.touch_rows(0..=self.rows.saturating_sub(1));
    }

    /// Mark every row changed, resizing the trackers to `rows`.
    fn reset_damage(&mut self) {
        self.stamp += 1;
        self.dirty = vec![true; self.rows];
        self.row_stamp = vec![self.stamp; self.rows];
    }

    /// Mark the scrollback changed.
    fn touch_hist(&mut self) {
        self.stamp += 1;
        self.hist_stamp = self.stamp;
    }

    /// `(grid id, stamp)` of live row `y`'s last change: equal across two
    /// looks only if the row did not change between them.
    pub fn row_stamp(&self, y: usize) -> (u64, u64) {
        (self.id, self.row_stamp[y])
    }

    /// `(grid id, stamp)` of the last scrollback change, as
    /// [`Grid::row_stamp`] for history lines.
    pub fn hist_stamp(&self) -> (u64, u64) {
        (self.id, self.hist_stamp)
    }

    /// Clear all per-row dirty flags. Call after handing a frame to the renderer.
    pub fn clear_dirty(&mut self) {
        self.dirty.iter_mut().for_each(|d| *d = false);
    }

    /// Mark every row dirty without touching any cell. Used when something
    /// outside the grid's own model wrote to the host terminal directly (e.g.
    /// a config-reload warning printed mid-session, in the TUI's raw-mode
    /// host-passthrough path) — the dirty-row diffing in
    /// [`Self::snapshot_dirty`] has no way to know those rows now need
    /// repainting on their own, so the next frame must be forced to redraw
    /// everything rather than skip rows it thinks are unchanged.
    pub fn force_full_repaint(&mut self) {
        self.reset_damage();
    }

    /// Overlay a status line at the bottom of the screen (L13 `render/set_status`),
    /// laid out at the current width; `fg`/`bg` default to the grid's defaults.
    /// Marks the bottom row dirty so the next frame composites it.
    #[cfg(feature = "l13")]
    pub fn set_status_line(&mut self, text: String, fg: Option<u32>, bg: Option<u32>) {
        let fg = fg.unwrap_or(self.default_fg());
        let bg = bg.unwrap_or(self.default_bg());
        self.status_line = Some(StatusLine::new(text, fg, bg, self.cols));
        if let Some(y) = self.rows.checked_sub(1) {
            self.touch(y);
        }
    }

    /// Remove the status-line overlay (L13 `render/clear_status`), repainting the
    /// bottom row it covered.
    #[cfg(feature = "l13")]
    pub(crate) fn clear_status_line(&mut self) {
        if self.status_line.take().is_some()
            && let Some(y) = self.rows.checked_sub(1)
        {
            self.touch(y);
        }
    }

    /// The status-line overlay cells (length `cols`) to composite over the bottom
    /// row, or `None` when no overlay is set or the alternate screen is active
    /// (its full-screen app owns the whole grid).
    pub fn status_row(&self) -> Option<&[Cell]> {
        if self.primary.is_some() {
            return None;
        }
        self.status_line.as_ref().map(|s| s.cells.as_slice())
    }

    /// Begin capturing a command's output (OSC 133;C): anchor the output start at
    /// the current cursor line, captured at the matching `D`.
    #[cfg(feature = "l13")]
    pub(crate) fn command_output_begin(&mut self) {
        self.command_start = Some(self.scrollback.len() + self.cursor.1);
    }

    /// A command finished (OSC 133;D): record its exit status and, if its output
    /// start was marked (C), capture the text from there to the cursor as the
    /// `terminal://command` resource.
    #[cfg(feature = "l13")]
    pub(crate) fn command_finished(&mut self, exit: Option<i32>) {
        self.last_exit = exit;
        if let Some(start) = self.command_start.take() {
            self.last_command_output = Some(self.capture_command_output(start));
        }
    }

    /// Begin tracking a command's output for folding (OSC 133/633;C): anchor
    /// the output start at the current cursor line, closed into a
    /// [`CommandBlock`] at the matching `D`. Independent of the `l13`
    /// feature's own (separately anchored) output capture.
    /// Whether the terminal currently presents as dark (background
    /// luminance below mid), the answer to `DSR ?996n` and the payload of
    /// mode-2031 reports. Derived from the live default background, so it
    /// tracks theme changes with no extra state.
    pub fn appearance_is_dark(&self) -> bool {
        super::color::luminance(self.default_bg()) < 0.5
    }

    /// The Contour color-scheme report (`CSI ? 997 ; 1|2 n`): `1` dark,
    /// `2` light. Sent unsolicited on theme changes while mode 2031 is set,
    /// and as the reply to `DSR ?996n`.
    pub fn color_scheme_report(&self) -> Vec<u8> {
        let kind = if self.appearance_is_dark() { 1 } else { 2 };
        format!("\x1b[?997;{kind}n").into_bytes()
    }

    /// Accumulate one OSC 99 (kitty desktop notification) part. `id` is the
    /// client's `i=` identifier ("" when absent), `part` is the decoded
    /// payload for `p=title` / `p=body`, and `done` finalizes the
    /// notification into [`Grid::notifications`]. Multi-part state is
    /// bounded: a fifth concurrent id evicts the oldest.
    pub(crate) fn notif99_part(&mut self, id: &str, title: bool, part: &str, done: bool) {
        let idx = match self.notif99_pending.iter().position(|(i, _, _)| i == id) {
            Some(i) => i,
            None => {
                if self.notif99_pending.len() >= 4 {
                    self.notif99_pending.remove(0);
                }
                self.notif99_pending
                    .push((id.to_string(), String::new(), String::new()));
                self.notif99_pending.len() - 1
            }
        };
        let slot = &mut self.notif99_pending[idx];
        let field = if title { &mut slot.1 } else { &mut slot.2 };
        if field.len() + part.len() <= 4096 {
            field.push_str(part);
        }
        if done {
            let (_, title, body) = self.notif99_pending.remove(idx);
            if !title.is_empty() || !body.is_empty() {
                // kitty semantics: a lone payload with no explicit body part
                // is the title; surface it as the body when body is empty so
                // the OS notification always has text.
                if body.is_empty() {
                    self.push_notification(String::new(), title);
                } else {
                    self.push_notification(title, body);
                }
            }
        }
    }

    /// Click-to-move-cursor (G21): the arrow presses that would move the
    /// shell's readline cursor from where it is to viewport cell
    /// `(col, row)`, as `(dx, dy)` (positive = right/down), or `None` when
    /// the click shouldn't move anything: alternate screen, scrolled view,
    /// no shell-integration prompt mark, a command currently running (open
    /// 133 `C`…`D` capture), a click above the prompt's first row, or a
    /// click on the cursor cell itself.
    #[cfg(any(test, feature = "gui"))]
    pub fn prompt_cursor_moves(&self, col: usize, row: usize) -> Option<(isize, isize)> {
        if self.in_alt_screen() || self.view_offset != 0 || self.fold_pending_start.is_some() {
            return None;
        }
        let prompt = *self.prompt_marks.last()?;
        let abs = self.scrollback.len() + row.min(self.rows.saturating_sub(1));
        let cursor_abs = self.scrollback.len() + self.cursor.1;
        if prompt > cursor_abs || abs < prompt {
            return None; // mark is stale, or the click is above the prompt
        }
        let (col, row) = (col.min(self.cols.saturating_sub(1)), row.min(self.rows - 1));
        if (col, row) == self.cursor {
            return None;
        }
        Some((
            col as isize - self.cursor.0 as isize,
            row as isize - self.cursor.1 as isize,
        ))
    }

    /// The scrollbar overlay for the current view, or `None` at the live
    /// bottom (the bar auto-hides): `(first_row, rows, color)` — the thumb's
    /// viewport rows plus its color (a fg/bg mix). Cell-resolution, so both
    /// renderers can draw it without new plumbing.
    #[cfg(any(test, feature = "gui"))]
    pub fn scrollbar(&self) -> Option<(usize, usize, u32)> {
        let dhl = self.display_history_len();
        let off = self.view_offset.min(dhl);
        if off == 0 || self.rows == 0 {
            return None;
        }
        let total = dhl + self.rows;
        let len = ((self.rows * self.rows) / total).clamp(1, self.rows);
        // Top of the viewport in display rows, over the scrollable range.
        let top = dhl - off;
        let denom = (total - self.rows).max(1);
        let first = (top * (self.rows - len)) / denom;
        let mix = |a: u32, b: u32| {
            let ch = |sh: u32| (((a >> sh) & 0xFF) * 2 + ((b >> sh) & 0xFF)) / 3;
            (ch(16) << 16) | (ch(8) << 8) | ch(0)
        };
        Some((first, len, mix(self.default_fg(), self.default_bg())))
    }

    /// Store a Kitty image by id (`a=t`/`a=T`), bounded: a 17th image (or
    /// one that would push the store past the pixel budget) evicts the
    /// oldest. Restoring an existing id replaces it (frames reset).
    pub(crate) fn kitty_store(&mut self, id: u32, w: usize, h: usize, pixels: Vec<Option<u32>>) {
        const MAX_IMAGES: usize = 16;
        const MAX_STORE_PIXELS: usize = 16 * 1024 * 1024;
        if id == 0 || w == 0 || h == 0 {
            return;
        }
        self.kitty_images.retain(|img| img.id != id);
        let mut total: usize = self.kitty_images.iter().map(|i| i.pixel_budget()).sum();
        while (self.kitty_images.len() >= MAX_IMAGES || total + w * h > MAX_STORE_PIXELS)
            && !self.kitty_images.is_empty()
        {
            let dropped = self.kitty_images.remove(0);
            total -= dropped.pixel_budget();
        }
        if w * h > MAX_STORE_PIXELS {
            return;
        }
        self.kitty_images.push(KittyImage {
            id,
            w,
            h,
            frames: vec![KittyFrame {
                pixels: pixels.into(),
                gap_ms: 0,
            }],
            current: 0,
            playing: false,
            last_advance: None,
        });
    }

    /// A stored image's current frame, cloned for placement.
    pub(crate) fn kitty_get(&self, id: u32) -> Option<(usize, usize, Vec<Option<u32>>)> {
        let img = self.kitty_images.iter().find(|i| i.id == id)?;
        Some((img.w, img.h, img.frames[img.current].pixels.to_vec()))
    }

    /// Record a virtual placement (`U=1`): the image renders wherever
    /// `U+10EEEE` placeholder cells with this id appear, on a `cols × rows`
    /// grid (0 = derive from the pixel and cell sizes at draw time).
    pub(crate) fn kitty_virtual_place(&mut self, id: u32, cols: usize, rows: usize) {
        self.kitty_virtual.retain(|(i, _, _)| *i != id);
        if self.kitty_virtual.len() < 16 {
            self.kitty_virtual.push((id, cols, rows));
        }
    }

    /// The virtual placement for image `id`, as `(cols, rows)`. Only the
    /// renderers' placeholder path (gui/test builds) consults it.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn kitty_virtual_geometry(&self, id: u32) -> Option<(usize, usize)> {
        self.kitty_virtual
            .iter()
            .find(|(i, _, _)| *i == id)
            .map(|&(_, c, r)| (c, r))
    }

    /// Append an animation frame (`a=f`) to image `id`: the payload is
    /// composited onto a copy of the previous frame at `(x, y)`, so partial
    /// frames accumulate the way the protocol specifies. `gap_ms` is how
    /// long the *new* frame displays. Returns whether the image exists.
    #[allow(clippy::too_many_arguments)] // mirrors the protocol's key set
    pub(crate) fn kitty_add_frame(
        &mut self,
        id: u32,
        w: usize,
        h: usize,
        pixels: Vec<Option<u32>>,
        x: usize,
        y: usize,
        gap_ms: u32,
    ) -> bool {
        const MAX_FRAMES: usize = 64;
        let Some(img) = self.kitty_images.iter_mut().find(|i| i.id == id) else {
            return false;
        };
        if img.frames.len() >= MAX_FRAMES {
            return false;
        }
        let mut base = img
            .frames
            .last()
            .expect("images always have a root frame")
            .pixels
            .to_vec();
        for row in 0..h {
            let dy = y + row;
            if dy >= img.h {
                break;
            }
            for col in 0..w {
                let dx = x + col;
                if dx >= img.w {
                    break;
                }
                base[dy * img.w + dx] = pixels[row * w + col];
            }
        }
        img.frames.push(KittyFrame {
            pixels: base.into(),
            gap_ms: gap_ms.max(40),
        });
        true
    }

    /// Start or stop an image's animation (`a=a`). Returns whether the image
    /// exists and has more than one frame.
    pub(crate) fn kitty_animate(&mut self, id: u32, run: bool) -> bool {
        let Some(img) = self.kitty_images.iter_mut().find(|i| i.id == id) else {
            return false;
        };
        img.playing = run && img.frames.len() > 1;
        img.last_advance = None;
        img.frames.len() > 1
    }

    /// Advance every playing animation to `now`, returning whether any frame
    /// changed (the caller repaints). Cheap when nothing animates.
    pub fn advance_animations(&mut self, now: std::time::Instant) -> bool {
        let mut changed = false;
        for img in &mut self.kitty_images {
            if !img.playing || img.frames.len() < 2 {
                continue;
            }
            let last = *img.last_advance.get_or_insert(now);
            // A frame's gap is its own display time (kitty `z=`): show the
            // current frame for its gap, then step to the next.
            let gap = img.frames[img.current].gap_ms.max(40) as u128;
            if now.duration_since(last).as_millis() >= gap {
                img.current = (img.current + 1) % img.frames.len();
                img.last_advance = Some(now);
                changed = true;
            }
        }
        if changed {
            self.touch_all();
        }
        changed
    }

    /// The current frame of stored image `id`, borrowed for drawing.
    #[cfg(any(test, feature = "gui"))]
    pub fn kitty_frame(&self, id: u32) -> Option<(usize, usize, &[Option<u32>])> {
        let img = self.kitty_images.iter().find(|i| i.id == id)?;
        Some((img.w, img.h, &img.frames[img.current].pixels))
    }

    /// Every placeholder cell in the viewport, resolved: `(id, row, col)`
    /// per cell (`None` for ordinary cells), with omitted diacritics
    /// inferred from the left/top neighbor per the spec. Shared by both
    /// renderers so inference can't drift between them.
    #[cfg(any(test, feature = "gui"))]
    pub fn placeholder_map(&self) -> Option<Vec<Option<(u32, u32, u32)>>> {
        let view = self.view();
        let mut cells = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            view.push_row(row, &mut cells);
        }
        self.placeholders_of(&cells)
    }

    /// [`Grid::placeholder_map`] over the viewport's cells, row-major.
    #[cfg(any(test, feature = "gui"))]
    fn placeholders_of(&self, cells: &[Cell]) -> Option<Vec<Option<(u32, u32, u32)>>> {
        let mut ph: Vec<Option<(u32, u32, u32)>> = Vec::new();
        let mut any = false;
        for row in 0..self.rows {
            for col in 0..self.cols {
                let entry = self
                    .placeholder_of(cells[row * self.cols + col])
                    .map(|(id, r, c)| {
                        let left = (col > 0)
                            .then(|| ph.get(row * self.cols + col - 1).copied().flatten())
                            .flatten()
                            .filter(|&(lid, _, _)| lid == id);
                        let above = (row > 0)
                            .then(|| ph.get((row - 1) * self.cols + col).copied().flatten())
                            .flatten()
                            .filter(|&(aid, _, _)| aid == id);
                        let cc = c.or_else(|| left.map(|(_, _, lc)| lc + 1)).unwrap_or(0);
                        let rr = r
                            .or_else(|| left.map(|(_, lr, _)| lr))
                            .or_else(|| above.map(|(_, ar, _)| ar + 1))
                            .unwrap_or(0);
                        (id, rr, cc)
                    });
                any |= entry.is_some();
                ph.push(entry);
            }
        }
        any.then_some(ph)
    }

    /// The placement grid (cols, rows) a placeholder image renders on:
    /// explicit `c`/`r` from its virtual placement, else derived from the
    /// image and cell pixel sizes.
    #[cfg(any(test, feature = "gui"))]
    pub fn placeholder_grid(&self, id: u32, cw: usize, ch: usize) -> Option<(usize, usize)> {
        let (iw, ih, _) = self.kitty_frame(id)?;
        let (mut pcols, mut prows) = self.kitty_virtual_geometry(id).unwrap_or((0, 0));
        if pcols == 0 {
            pcols = iw.div_ceil(cw.max(1)).max(1);
        }
        if prows == 0 {
            prows = ih.div_ceil(ch.max(1)).max(1);
        }
        Some((pcols, prows))
    }

    /// Delete stored Kitty images: by id, or everything (`None`). Virtual
    /// placements of deleted images go with them.
    pub(crate) fn kitty_delete(&mut self, id: Option<u32>) {
        match id {
            Some(id) => {
                self.kitty_images.retain(|i| i.id != id);
                self.kitty_virtual.retain(|(i, _, _)| *i != id);
            }
            None => {
                self.kitty_images.clear();
                self.kitty_virtual.clear();
            }
        }
    }

    /// Decode a Unicode placeholder cell (`U+10EEEE`) at viewport
    /// `(col, row)`: the image id (24 bits from the foreground color, plus
    /// the third diacritic's high byte) and the encoded `(row, col)` indices
    /// from the first two diacritics (`None` = omitted, to be inferred from
    /// neighbors). `None` when the cell isn't a placeholder.
    #[cfg(any(test, feature = "gui"))]
    pub fn placeholder_at(
        &self,
        col: usize,
        row: usize,
    ) -> Option<(u32, Option<u32>, Option<u32>)> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.placeholder_of(self.viewport_cell(col, row))
    }

    /// [`Grid::placeholder_at`] for an already-resolved cell.
    #[cfg(any(test, feature = "gui"))]
    fn placeholder_of(&self, cell: Cell) -> Option<(u32, Option<u32>, Option<u32>)> {
        if cell.ch != '\u{10EEEE}' {
            return None;
        }
        let mut r_idx = None;
        let mut c_idx = None;
        let mut id = cell.fg & 0x00FF_FFFF;
        if cell.cluster != 0
            && let Some(suffix) = self.clusters.get((cell.cluster - 1) as usize)
        {
            let mut vals = suffix
                .chars()
                .filter_map(super::kitty_diacritics::diacritic_value);
            r_idx = vals.next();
            c_idx = vals.next();
            if let Some(hi) = vals.next() {
                id |= hi << 24;
            }
        }
        Some((id, r_idx, c_idx))
    }

    /// Apply a ConEmu OSC `9;4` progress report. State `0` (or anything
    /// unrecognized) clears; percent clamps to 100. Indeterminate (`3`)
    /// carries no meaningful percent and stores 0.
    pub(crate) fn set_progress(&mut self, state: u8, percent: u8) {
        self.progress = match state {
            1 | 2 | 4 => Some((state, percent.min(100))),
            3 => Some((3, 0)),
            _ => None,
        };
    }

    /// A command's output began (OSC 133/633 `C`): start the runtime clock
    /// for the windowed front-end's "command finished" notification.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn command_timer_begin(&mut self) {
        self.command_began = Some(std::time::Instant::now());
    }

    /// A command finished (OSC 133/633 `D`): record its exit code and
    /// runtime for the windowed front-end to drain (no-op without a matching
    /// `C`). Bounded so an undrained grid never grows it.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn command_timer_end(&mut self, exit: Option<i32>) {
        // The status ribbon's exit pill reads this even when the l13 capture
        // path (which also records it) is compiled out. `D` without a
        // matching `C` still updates it — the exit code needs no timer.
        self.last_exit = exit;
        let Some(began) = self.command_began.take() else {
            return;
        };
        if self.finished_commands.len() < 8 {
            self.finished_commands.push((exit, began.elapsed()));
        }
    }

    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn fold_output_begin(&mut self) {
        self.fold_pending_start = Some(self.scrollback.len() + self.cursor.1);
    }

    /// A command finished (OSC 133/633;D): close the pending fold block from
    /// its `C` anchor to the current cursor line, unfolded by default (a
    /// no-op if `C` was never seen — e.g. a shell that only ever sends `D`).
    /// Silently drops the block once [`FOLD_BLOCKS_MAX`] is reached.
    #[cfg(any(test, feature = "gui"))]
    pub(crate) fn fold_output_end(&mut self, exit: Option<i32>) {
        let Some(start) = self.fold_pending_start.take() else {
            return;
        };
        let end = self.scrollback.len() + self.cursor.1;
        if start < end && self.fold_blocks.len() < FOLD_BLOCKS_MAX {
            // `command_began` is still armed here — `command_timer_end`
            // (called right after us on the OSC 133;D path) takes it.
            let runtime = self.command_began.map(|t| t.elapsed());
            self.fold_blocks.push(CommandBlock {
                start,
                end,
                folded: false,
                exit,
                runtime,
            });
        }
    }

    /// Absolute logical line where the currently-running command's output
    /// began (OSC 133;C seen, no D yet), or `None` outside a command.
    #[cfg(any(test, feature = "gui"))]
    pub fn running_command(&self) -> Option<usize> {
        self.fold_pending_start
    }

    /// The command gutter mark beside each viewport row: which finished
    /// command block the row's line belongs to (colored by its exit code),
    /// or the still-running command. `None` outside any block, for blocks
    /// whose shell reported no exit code, and everywhere on the alt screen
    /// (full-screen apps own the whole surface; the marks are a scrollback
    /// affordance).
    #[cfg(any(test, feature = "gui"))]
    pub fn viewport_block_marks(&self) -> Vec<Option<BlockMark>> {
        self.block_marks_of(&self.view_lines())
    }

    /// [`Grid::viewport_block_marks`] for already-resolved viewport rows.
    #[cfg(any(test, feature = "gui"))]
    fn block_marks_of(&self, lines: &[view::ViewLine]) -> Vec<Option<BlockMark>> {
        if self.in_alt_screen() {
            return vec![None; self.rows];
        }
        let running = self
            .fold_pending_start
            .map(|s| (s, self.scrollback.len() + self.cursor.1));
        lines
            .iter()
            .map(|&line| {
                let abs = self.abs_of_line(line);
                if let Some((s, e)) = running
                    && abs >= s
                    && abs <= e
                {
                    return Some(BlockMark::Running);
                }
                // Blocks are recorded in stream order: sorted and disjoint,
                // so the first block whose end is past `abs` is the only
                // candidate. A folded block's summary row maps to `start`.
                let i = self.fold_blocks.partition_point(|b| b.end <= abs);
                match self.fold_blocks.get(i) {
                    Some(b) if b.start <= abs => match b.exit {
                        Some(0) => Some(BlockMark::Success),
                        Some(_) => Some(BlockMark::Error),
                        None => None,
                    },
                    _ => None,
                }
            })
            .collect()
    }

    /// Toggle the fold state of whichever command block contains absolute
    /// logical line `line`, returning whether one was found. The API a
    /// future click-on-summary-line or keybind-at-cursor interaction would
    /// call; not yet wired to one (see [`Self::fold_blocks`]).
    #[cfg(any(test, feature = "gui"))]
    #[allow(dead_code)] // exercised by tests; the viewport-rendering consumer is future work
    pub(crate) fn toggle_fold_at(&mut self, line: usize) -> bool {
        match self
            .fold_blocks
            .iter_mut()
            .find(|b| (b.start..b.end).contains(&line))
        {
            Some(b) => {
                b.folded = !b.folded;
                true
            }
            None => false,
        }
    }

    /// The command blocks tracked for folding, in the order their `C` marks
    /// arrived. The state a viewport compositor would read to actually
    /// collapse a folded block's rows to one summary line — that render-path
    /// integration (`viewport_cell`/`snapshot_viewport`'s row math, plus
    /// selection/search/click-hit-testing all keying off the same rows) is
    /// deferred; this method exists so the data model that will feed it is
    /// already tested and correct across scroll/resize/RIS.
    #[cfg(any(test, feature = "gui"))]
    #[allow(dead_code)] // exercised by tests; the viewport-rendering consumer is future work
    pub fn fold_blocks(&self) -> &[CommandBlock] {
        &self.fold_blocks
    }

    /// The trimmed text of absolute logical line `abs` — scrollback or the
    /// live screen; empty when out of range. The command dock uses the line
    /// just above a block's output start (the prompt line) as its label.
    #[cfg(any(test, feature = "gui"))]
    pub fn abs_line_text(&self, abs: usize) -> String {
        self.line_text(abs)
            .map(|text| text.trim_start().to_string())
            .unwrap_or_default()
    }

    /// The text of absolute logical line `abs`, counting from the oldest
    /// scrollback row through the live screen, with trailing blanks trimmed.
    /// `None` past the last screen row.
    pub fn line_text(&self, abs: usize) -> Option<String> {
        let history = self.scrollback.len();
        let line = match abs.checked_sub(history) {
            None => self.scrollback.get(abs)?,
            Some(row) => self.screen.get(row)?,
        };
        Some(self.text_of(line))
    }

    /// The cells of absolute row `abs`, counted as for [`Grid::line_text`],
    /// and whether the program's text wrapped from it onto the next row.
    /// Holds only the cells stored, so trailing blanks may be missing.
    /// `None` past the last screen row.
    pub fn line_cells(&self, abs: usize) -> Option<(Vec<Cell>, bool)> {
        let history = self.scrollback.len();
        let line = match abs.checked_sub(history) {
            None => self.scrollback.get(abs)?,
            Some(row) => self.screen.get(row)?,
        };
        Some((self.resolved(line.cells()), line.wrapped))
    }

    /// Join the cell rows in the absolute line range `[start, cursor line)` into
    /// text, one row per line, trailing blanks trimmed per row.
    #[cfg(feature = "l13")]
    fn capture_command_output(&self, start: usize) -> String {
        let history = self.scrollback.len();
        let end = history + self.cursor.1;
        let start = start.min(end);
        let mut out = String::new();
        for i in start..end {
            if i != start {
                out.push('\n');
            }
            let line = if i < history {
                &self.scrollback[i]
            } else {
                &self.screen[i - history]
            };
            out.push_str(&self.text_of(line));
        }
        out
    }

    /// The exit code of the last finished command, or `None` if none has
    /// finished (or the shell reported no code). The windowed front-end's
    /// status ribbon shows it as a success/error pill; L13 reads it for the
    /// structured side-channel.
    #[cfg(any(feature = "l13", feature = "gui"))]
    pub fn last_command_exit(&self) -> Option<i32> {
        self.last_exit
    }

    /// The output text of the last finished command (OSC 133;C..D), if captured.
    #[cfg(feature = "l13")]
    pub(crate) fn last_command_output(&self) -> Option<&str> {
        self.last_command_output.as_deref()
    }

    /// Build the `terminal://dimensions` change notification if a client is
    /// subscribed, for the runtime driver to write to the child after a resize
    /// (which happens outside the parser's `advance`, so it has no `responses` in
    /// hand). Returns `None` when nobody is subscribed — no wasted frame.
    #[cfg(feature = "l13")]
    pub fn resize_notification(&self) -> Option<Vec<u8>> {
        let mut buf = Vec::new();
        rusty_term_l13::notify_resource_changed(self, rusty_term_l13::RES_DIMENSIONS, &mut buf);
        (!buf.is_empty()).then_some(buf)
    }

    /// Snapshot only the rows currently marked dirty, cloning their cells into
    /// a [`DirtyFrame`]. This is the high-locality handoff the renderer consumes.
    pub fn snapshot_dirty(&self) -> DirtyFrame {
        let rows = self
            .dirty
            .iter()
            .enumerate()
            .filter(|&(_, d)| *d)
            .map(|(y, _)| {
                let cells = match (y + 1 == self.rows).then(|| self.status_row()).flatten() {
                    Some(s) => s.to_vec(),
                    None => self.resolved(self.screen[y].cells()),
                };
                (y, cells)
            })
            .collect();
        DirtyFrame {
            cursor: self.cursor,
            rows,
            links: self.links_arc.clone(),
            clusters: self.clusters_arc(),
            line_attrs: self.screen.iter().map(|l| l.attr).collect(),
        }
    }

    /// Move the viewport up into history by up to `n` lines, clamped to the
    /// available scrollback. No-op on the alternate screen (no history there).
    /// Returns `true` if the view actually moved.
    pub fn scroll_view_up(&mut self, n: usize) -> bool {
        if self.primary.is_some() {
            return false;
        }
        let target = (self.view_offset + n).min(self.display_history_len());
        self.set_view_offset(target)
    }

    /// Move the viewport back down toward the live bottom by up to `n` lines.
    /// Returns `true` if the view actually moved.
    pub fn scroll_view_down(&mut self, n: usize) -> bool {
        let target = self.view_offset.saturating_sub(n);
        self.set_view_offset(target)
    }

    /// Snap the viewport back to the live bottom. Returns `true` if it moved.
    pub fn reset_view(&mut self) -> bool {
        self.set_view_offset(0)
    }

    /// Physical row `abs` (0 = oldest scrollback line).
    #[cfg(any(test, feature = "gui"))]
    fn phys_line(&self, abs: usize) -> &Line {
        let h = self.scrollback.len();
        if abs < h {
            &self.scrollback[abs]
        } else {
            &self.screen[abs - h]
        }
    }

    /// Physical row `abs` as `(cells, wrapped)`, its cells resolved.
    #[cfg(any(test, feature = "gui"))]
    fn phys_row(&self, abs: usize) -> (Vec<Cell>, bool) {
        let line = self.phys_line(abs);
        (self.resolved(&line.padded()), line.wrapped)
    }

    /// Scroll the viewport so absolute row `abs` is visible, about a third
    /// down. A target hidden inside a folded command block unfolds it first
    /// (a search hit must be visible, not buried behind a summary line).
    #[cfg(any(test, feature = "gui"))]
    pub fn scroll_to_abs(&mut self, abs: usize) {
        let hit = self
            .visible_folds()
            .find(|(_, b)| abs > b.start && abs < b.end)
            .map(|(i, _)| i);
        if let Some(i) = hit {
            self.fold_blocks[i].folded = false;
        }
        let dhl = self.display_history_len();
        let d = self.display_index_of_abs(abs.min(self.scrollback.len()));
        let want = dhl as isize + (self.rows / 3) as isize - d as isize;
        let off = want.clamp(0, dhl as isize) as usize;
        self.set_view_offset(off);
    }

    /// Search the scrollback + live screen for `query`, case-insensitively
    /// (ASCII), joining soft-wrapped rows into logical lines so a match can cross
    /// a wrap. Stores matches for highlighting and next/prev, scrolls the first
    /// into view, and returns the count. An empty query clears the search.
    /// Search the scrollback + live screen for `query`. `regex = true` compiles the
    /// query with the from-scratch engine (`core::rx`) and matches per
    /// logical line (`^`/`$` anchor to the line); a malformed pattern finds
    /// nothing, which the find bar shows as "no matches". Case-insensitive
    /// (simple Unicode folding, not just ASCII) — a thin wrapper over
    /// `search_with_case` for every existing caller, which never needed a
    /// case-sensitive option.
    #[cfg(any(test, feature = "gui"))]
    pub fn search_with(&mut self, query: &str, regex: bool) -> usize {
        self.search_with_case(query, regex, false)
    }

    /// `search_with`, but `case_sensitive` skips the Unicode case folding on
    /// both the query/pattern and the haystack — the find bar's case-
    /// sensitivity toggle (Alt+C).
    #[cfg(any(test, feature = "gui"))]
    pub fn search_with_case(&mut self, query: &str, regex: bool, case_sensitive: bool) -> usize {
        let fold = |c: char| if case_sensitive { c } else { fold_char(c) };
        self.search = None;
        let re = if regex {
            // Fold the pattern like the haystack, so literals match
            // case-insensitively. Safe for POSIX ERE: folding never touches
            // metacharacters, and the engine has no `\D`-style classes whose
            // meaning a case change could invert.
            let folded: String = query.chars().map(fold).collect();
            match rusty_regx::Regex::new_posix(&folded) {
                Ok(re) => Some((re, folded.starts_with('^'))),
                Err(_) => {
                    // Malformed pattern: finds nothing ("no matches" in the bar).
                    self.touch_all();
                    return 0;
                }
            }
        } else {
            None
        };
        let q: Vec<char> = query.chars().map(fold).collect();
        if q.is_empty() {
            self.touch_all();
            return 0;
        }
        let total = self.scrollback.len() + self.rows;
        let mut st = Search::default();
        let mut text: Vec<char> = Vec::new();
        let mut at: Vec<(usize, usize)> = Vec::new();
        for abs in 0..total {
            let (cells, wrapped) = self.phys_row(abs);
            for (col, cell) in cells.iter().enumerate() {
                if cell.flags & WIDE_TRAILER != 0 {
                    continue;
                }
                text.push(fold(cell.ch));
                at.push((abs, col));
            }
            if wrapped {
                continue;
            }
            match &re {
                Some((re, anchored)) => find_matches_rx(&text, &at, re, *anchored, &mut st),
                None => find_matches(&text, &at, &q, &mut st),
            }
            text.clear();
            at.clear();
            if st.anchors.len() >= SEARCH_MAX {
                break;
            }
        }
        match &re {
            Some((re, anchored)) => find_matches_rx(&text, &at, re, *anchored, &mut st),
            None => find_matches(&text, &at, &q, &mut st),
        }
        let n = st.anchors.len();
        if n > 0 {
            let anchor = st.anchors[0].0;
            self.search = Some(st);
            self.scroll_to_abs(anchor);
        }
        self.touch_all();
        n
    }

    /// Step to the next (`forward`) or previous match, scrolling it into view.
    #[cfg(any(test, feature = "gui"))]
    pub fn search_jump(&mut self, forward: bool) -> bool {
        let abs = {
            let Some(s) = &mut self.search else {
                return false;
            };
            if s.anchors.is_empty() {
                return false;
            }
            let n = s.anchors.len();
            s.current = if forward {
                (s.current + 1) % n
            } else {
                (s.current + n - 1) % n
            };
            s.anchors[s.current].0
        };
        self.scroll_to_abs(abs);
        self.touch_all();
        true
    }

    /// Highlight state for viewport cell `(col, vr)`: `None` (no match),
    /// `Some(false)` (a match), `Some(true)` (the active match).
    #[cfg(any(test, feature = "gui"))]
    pub fn search_highlight(&self, col: usize, vr: usize) -> Option<bool> {
        self.search_highlight_abs(col, self.abs_of_view_row(vr))
    }

    /// [`Grid::search_highlight`] for absolute row `abs`.
    #[cfg(any(test, feature = "gui"))]
    fn search_highlight_abs(&self, col: usize, abs: usize) -> Option<bool> {
        let s = self.search.as_ref()?;
        for &(start, end, mi) in s.rows.get(&abs)? {
            if col >= start && col < end {
                return Some(mi == s.current);
            }
        }
        None
    }

    /// `(current_match_1based, total)` while searching, else `None`.
    #[cfg(any(test, feature = "gui"))]
    pub fn search_status(&self) -> Option<(usize, usize)> {
        let s = self.search.as_ref()?;
        Some((s.current + 1, s.anchors.len()))
    }

    /// Clear any active search (highlights + matches).
    #[cfg(any(test, feature = "gui"))]
    pub fn clear_search(&mut self) {
        if self.search.take().is_some() {
            self.touch_all();
        }
    }

    /// Set the scrollback view offset, marking every row dirty so the renderer
    /// repaints the whole viewport. Returns `true` if the offset changed.
    fn set_view_offset(&mut self, offset: usize) -> bool {
        if offset == self.view_offset {
            return false;
        }
        self.view_offset = offset;
        self.touch_all();
        true
    }

    /// Record a shell prompt start (OSC 133;A) at the current cursor row, for
    /// prompt-to-prompt scrollback navigation. No-op on the alternate screen,
    /// which has no history. Marks are logical line indices (0 = oldest retained
    /// scrollback line), kept sorted and deduplicated.
    pub(crate) fn mark_prompt(&mut self) {
        if self.primary.is_some() {
            return;
        }
        let line = self.scrollback.len() + self.cursor.1;
        if let Err(pos) = self.prompt_marks.binary_search(&line) {
            self.prompt_marks.insert(pos, line);
            if self.prompt_marks.len() > PROMPT_MARKS_MAX {
                self.prompt_marks.remove(0);
            }
        }
    }

    /// One scrollback line was evicted from the front: every logical index drops
    /// by one, and a mark on the evicted line (index 0) is discarded.
    fn evict_prompt_mark(&mut self) {
        self.prompt_marks.retain(|&l| l != 0);
        for l in &mut self.prompt_marks {
            *l -= 1;
        }
    }

    /// One scrollback line was evicted from the front: every fold block's
    /// range shifts down by one; a block entirely on the evicted line (`end
    /// == 1`, i.e. `[0, 1)`) is dropped outright rather than left empty.
    #[cfg(any(test, feature = "gui"))]
    fn evict_fold_blocks(&mut self) {
        self.fold_blocks.retain(|b| b.end > 1);
        for b in &mut self.fold_blocks {
            b.start = b.start.saturating_sub(1);
            b.end -= 1;
        }
    }

    /// Scroll the viewport up to the nearest prompt mark above the current top
    /// visible line (OSC 133 navigation). Returns `true` if it moved.
    pub fn scroll_to_prev_prompt(&mut self) -> bool {
        if self.primary.is_some() {
            return false;
        }
        let top_visible = self.abs_top_for_offset(self.view_offset.min(self.display_history_len()));
        match self
            .prompt_marks
            .iter()
            .copied()
            .filter(|&l| l < top_visible)
            .max()
        {
            Some(l) => {
                let off = self.display_history_len() - self.display_index_of_abs(l);
                self.set_view_offset(off)
            }
            None => false,
        }
    }

    /// Scroll the viewport down to the nearest prompt mark below the current top
    /// visible line, snapping to the live bottom when the next mark is on the
    /// live screen or there is none. Returns `true` if it moved.
    pub fn scroll_to_next_prompt(&mut self) -> bool {
        if self.primary.is_some() {
            return false;
        }
        let h = self.scrollback.len();
        let top_visible = self.abs_top_for_offset(self.view_offset.min(self.display_history_len()));
        match self
            .prompt_marks
            .iter()
            .copied()
            .filter(|&l| l > top_visible)
            .min()
        {
            Some(l) if l < h => {
                let off = self.display_history_len() - self.display_index_of_abs(l);
                self.set_view_offset(off)
            }
            _ => self.reset_view(),
        }
    }

    /// The cell visible at viewport `(col, row)`, compositing scrollback history
    /// above the live grid according to [`Grid::view_offset`]: the top `off`
    /// rows show the tail of history, the rest the live grid shifted down by
    /// `off`. A blank cell fills positions past a short history line's width.
    /// Used by the windowed renderers so a scrolled-up view paints history.
    #[cfg(any(test, feature = "gui"))]
    pub fn viewport_cell(&self, col: usize, row: usize) -> Cell {
        self.cell_of(self.view_line(row), col)
    }

    /// The bidi view of viewport row `row` (rule L2 applied to the row's
    /// cells): `None` for pure-LTR rows or when `bidi` is off. Wide glyphs
    /// (lead + trailer) reorder as one unit so the pair stays adjacent and
    /// in lead-trailer order. Fold summaries and every other synthesized row
    /// come out LTR and short-circuit to `None`, so the layers compose.
    #[cfg(any(test, feature = "gui"))]
    pub fn bidi_row(&self, row: usize) -> Option<BidiRow> {
        if !self.bidi_active() || row >= self.rows {
            return None;
        }
        let line = self.view_line(row);
        self.bidi_row_of(row, |col| self.cell_of(line, col))
    }

    /// Config is the master switch; within it BDSM (mode 8) decides, and an
    /// app that never touches it gets implicit on the main screen / explicit
    /// on the alt screen (full-screen apps lay out themselves).
    #[cfg(any(test, feature = "gui"))]
    fn bidi_active(&self) -> bool {
        self.bidi && self.bdsm.unwrap_or(!self.in_alt_screen())
    }

    /// [`Grid::bidi_row`] over the row's cells as `cell_at(col)` yields them.
    #[cfg(any(test, feature = "gui"))]
    fn bidi_row_of(&self, row: usize, cell_at: impl Fn(usize) -> Cell) -> Option<BidiRow> {
        if !self.bidi_active() || row >= self.rows {
            return None;
        }
        // Unit per cell run: a wide lead absorbs its trailer.
        let mut unit_chars: Vec<char> = Vec::with_capacity(self.cols);
        let mut unit_cells: Vec<(u16, u16)> = Vec::new(); // (first col, ncells)
        let mut col = 0usize;
        while col < self.cols {
            let cell = cell_at(col);
            let w = if cell.flags & WIDE_TRAILER != 0 {
                // Orphan trailer (shouldn't happen mid-row): own unit.
                1
            } else {
                let mut n = 1;
                while col + n < self.cols && cell_at(col + n).flags & WIDE_TRAILER != 0 {
                    n += 1;
                }
                n
            };
            unit_chars.push(cell.ch);
            unit_cells.push((col as u16, w as u16));
            col += w;
        }
        // Paragraph direction: per-line first-strong when autodetect (2501)
        // is on (the default), else the SCP-fixed direction.
        let base = if self.bidi_autodetect {
            None
        } else {
            Some(self.bidi_para_rtl.unwrap_or(false))
        };
        let unit_map = super::bidi::visual_map(&unit_chars, base)?;
        let levels_para = match base {
            Some(rtl) => rtl as u8,
            None => super::bidi::paragraph_level(&unit_chars),
        };
        let (levels, _) = super::bidi::reorder(&unit_chars, levels_para);
        // Expand units back to cells, visual order.
        let mut vis2log = Vec::with_capacity(self.cols);
        for &u in &unit_map {
            let (first, n) = unit_cells[u as usize];
            for k in 0..n {
                vis2log.push(first + k);
            }
        }
        let mut log2vis = vec![0u16; self.cols];
        for (v, &l) in vis2log.iter().enumerate() {
            log2vis[l as usize] = v as u16;
        }
        let mut rtl = vec![false; self.cols];
        for (ui, &(first, n)) in unit_cells.iter().enumerate() {
            if !levels[ui].is_multiple_of(2) {
                for k in 0..n {
                    rtl[(first + k) as usize] = true;
                }
            }
        }
        // Arabic contextual shaping (phase 3), computed on the logical
        // sequence (joining is defined pre-reorder).
        let shaped = super::arabic::shape_row(&unit_chars).map(|su| {
            let mut v: Vec<Option<char>> = vec![None; self.cols];
            for (ui, &(first, _)) in unit_cells.iter().enumerate() {
                if su[ui] != unit_chars[ui] {
                    v[first as usize] = Some(su[ui]);
                }
            }
            v
        });
        Some(BidiRow {
            vis2log,
            log2vis,
            rtl,
            shaped,
        })
    }

    /// Map a viewport cell the *user pointed at* (visual coordinates) to the
    /// logical cell the model stores there — identity without bidi. Every
    /// mouse path (selection, clicks, hyperlinks) converts through this.
    #[cfg(any(test, feature = "gui"))]
    pub fn logical_col(&self, col: usize, row: usize) -> usize {
        match self.bidi_row(row) {
            Some(b) => b.vis2log.get(col).map_or(col, |&l| l as usize),
            None => col,
        }
    }

    /// The plain-text URL under viewport `(col, row)`, detected by scanning
    /// the cell's whole logical line (following soft wraps) — the implicit
    /// counterpart of [`Grid::link_at`] for the overwhelming majority of
    /// programs that print URLs without OSC 8. A `www.`-prefixed match gets
    /// `http://` prepended so the result is directly openable.
    #[cfg(any(test, feature = "gui"))]
    pub fn url_at(&self, col: usize, row: usize) -> Option<String> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let abs = self.abs_of_view_row(row);
        let (text, at) = self.logical_line_of(abs);
        let idx = at.iter().position(|&(a, c)| a == abs && c == col)?;
        detect_urls(&text)
            .into_iter()
            .find(|&(s, e, _)| idx >= s && idx < e)
            .map(|(_, _, url)| url)
    }

    /// Every distinct link visible in the viewport, explicit (OSC 8) and
    /// detected (plain text), top-to-bottom, capped at 16 — feeds the
    /// window's "open a link" menu.
    #[cfg(any(test, feature = "gui"))]
    pub fn visible_links(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |u: String| {
            if !out.contains(&u) && out.len() < 16 {
                out.push(u);
            }
        };
        let mut row = 0;
        while row < self.rows {
            let abs = self.abs_of_view_row(row);
            // OSC 8 links on this visual row.
            for col in 0..self.cols {
                if let Some(u) = self.link_at(col, row) {
                    push(u.to_string());
                }
            }
            // Detected URLs on the logical line through this row; skip the
            // line's remaining visual rows so it isn't scanned once per row.
            let (text, at) = self.logical_line_of(abs);
            for (_, _, url) in detect_urls(&text) {
                push(url);
            }
            let line_rows = at.last().map_or(1, |&(last_abs, _)| last_abs - at[0].0 + 1);
            let skip = line_rows - (abs - at[0].0);
            row += skip.max(1);
        }
        out
    }

    /// The logical line containing absolute row `abs`: its chars (wide-glyph
    /// trailers skipped, like search) and per-char `(abs_row, col)` positions.
    /// Bounded to 64 physical rows each way so a degenerate fully-wrapped
    /// scrollback can't make one lookup scan everything.
    #[cfg(any(test, feature = "gui"))]
    fn logical_line_of(&self, abs: usize) -> (Vec<char>, Vec<(usize, usize)>) {
        let total = self.scrollback.len() + self.rows;
        let mut start = abs.min(total.saturating_sub(1));
        let mut back = 0;
        while start > 0 && back < 64 && self.phys_line(start - 1).wrapped {
            start -= 1;
            back += 1;
        }
        let mut text = Vec::new();
        let mut at = Vec::new();
        let mut r = start;
        loop {
            let (cells, wrapped) = self.phys_row(r);
            for (col, cell) in cells.iter().enumerate() {
                if cell.flags & WIDE_TRAILER != 0 {
                    continue;
                }
                text.push(cell.ch);
                at.push((r, col));
            }
            r += 1;
            if !wrapped || r >= total || r - start > 128 {
                break;
            }
        }
        (text, at)
    }

    /// The OSC 8 hyperlink URI covering viewport `(col, row)`, if any. Mirrors
    /// [`Grid::viewport_cell`] so links in scrolled-back history resolve too;
    /// powers Ctrl+click in the windowed front-end.
    #[cfg(any(test, feature = "gui"))]
    pub fn link_at(&self, col: usize, row: usize) -> Option<&str> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let id = self.viewport_cell(col, row).link as usize;
        self.links.get(id.checked_sub(1)?).map(String::as_str)
    }

    /// The column span `(start_col, end_col)` inclusive, plus URL, of the
    /// hyperlink (OSC 8 or detected plain-text) under viewport `(col, row)` —
    /// clamped to the visual row it's hovered on, unlike `link_at`/`url_at`
    /// which only test single-cell membership. Powers the hover
    /// underline/pointer-cursor affordance in the windowed front-end; a
    /// wrapped plain-text URL only underlines on the hovered row, which is a
    /// fine simplification since the full URL still opens correctly either
    /// way (this only feeds the *visual* hint).
    #[cfg(any(test, feature = "gui"))]
    pub fn hover_link_at(&self, col: usize, row: usize) -> Option<(usize, usize, String)> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let id = self.viewport_cell(col, row).link as usize;
        if id != 0 {
            let url = self.links.get(id - 1)?.clone();
            let mut start = col;
            while start > 0 && self.viewport_cell(start - 1, row).link as usize == id {
                start -= 1;
            }
            let mut end = col;
            while end + 1 < self.cols && self.viewport_cell(end + 1, row).link as usize == id {
                end += 1;
            }
            return Some((start, end, url));
        }
        let abs = self.abs_of_view_row(row);
        let (text, at) = self.logical_line_of(abs);
        let idx = at.iter().position(|&(a, c)| a == abs && c == col)?;
        let (s, e, url) = detect_urls(&text)
            .into_iter()
            .find(|&(s, e, _)| idx >= s && idx < e)?;
        let (mut start, mut end) = (col, col);
        for &(a, c) in &at[s..e] {
            if a == abs {
                start = start.min(c);
                end = end.max(c);
            }
        }
        Some((start, end, url))
    }

    /// Snapshot the entire visible viewport, compositing scrollback history
    /// above the live grid according to [`Grid::view_offset`]. Every row is
    /// included. History lines are padded/truncated to the current width.
    /// Used by the renderer whenever the view is scrolled up.
    pub fn snapshot_viewport(&self) -> DirtyFrame {
        let history = self.scrollback.len();
        let off = self.view_offset.min(history);
        let mut rows = Vec::with_capacity(self.rows);
        let mut attrs = Vec::with_capacity(self.rows);
        for y in 0..self.rows {
            let status = if y + 1 == self.rows {
                self.status_row()
            } else {
                None
            };
            let cells = if let Some(s) = status {
                s.to_vec()
            } else if y < off {
                // Top `off` viewport rows show the tail of history.
                let line = &self.scrollback[history - off + y];
                (0..self.cols).map(|x| self.line_cell(line, x)).collect()
            } else {
                // The rest shows the live grid, shifted down by `off`.
                self.resolved(self.screen[y - off].cells())
            };
            rows.push((y, cells));
            attrs.push(if status.is_some() || y < off {
                LineAttr::Single // history + status-overlay rows are single width
            } else {
                self.screen[y - off].attr
            });
        }
        DirtyFrame {
            cursor: self.cursor,
            rows,
            links: self.links_arc.clone(),
            clusters: self.clusters_arc(),
            line_attrs: attrs,
        }
    }

    /// Drain bytes queued for the host terminal (forwarded OSC 52 clipboard
    /// requests). Empty when there's nothing to send.
    pub fn take_host_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.host_out)
    }

    /// Set the active hyperlink (OSC 8). `None` (or an empty URI) closes the
    /// link; a URI is interned and its id stamped onto subsequently written
    /// cells.
    pub(crate) fn set_link(&mut self, uri: Option<&str>) {
        self.current_link = match uri {
            None | Some("") => 0,
            Some(u) => self.intern_link(u),
        };
    }

    /// Queue a desktop notification (OSC 9/777) for the windowed front-end to
    /// raise. Bounded so a flood can't grow the queue without limit between
    /// drains; the TUI relays the OSC to the host instead.
    pub(crate) fn push_notification(&mut self, title: String, body: String) {
        const MAX: usize = 32;
        if self.notifications.len() < MAX {
            self.notifications.push((title, body));
        }
    }

    /// Queue `signal` when [`Grid::collect_program_status`] is set, holding
    /// at most [`PROGRAM_STATUS_MAX`] without losing a lifecycle boundary or
    /// the order of what is kept. A reset supersedes everything queued
    /// before it, a prompt right after a prompt or reset adds nothing, and
    /// when full the oldest report makes room, merging the boundaries it
    /// separated. So no two non-reports are ever adjacent: a full queue is
    /// mostly reports, and a later report is always admitted.
    pub(crate) fn push_program_status(&mut self, signal: StatusSignal) {
        if !self.collect_program_status {
            return;
        }
        let queue = &mut self.program_status;
        match signal {
            StatusSignal::Reset => queue.clear(),
            StatusSignal::Prompt if queue.last().is_some_and(is_boundary) => return,
            StatusSignal::Prompt | StatusSignal::Report(_) => {}
        }
        if queue.len() >= PROGRAM_STATUS_MAX {
            evict_oldest_report(queue);
        }
        queue.push(signal);
    }

    /// Return the id for `uri`, interning it on first use. Ids are `index + 1`
    /// so `0` can mean "no link". Returns `0` once the table is full.
    fn intern_link(&mut self, uri: &str) -> u16 {
        if let Some(i) = self.links.iter().position(|l| l == uri) {
            return (i + 1) as u16;
        }
        if self.links.len() >= LINK_MAX {
            return 0;
        }
        self.links.push(uri.to_string());
        self.links_arc = Arc::from(self.links.clone());
        self.links.len() as u16
    }
}

/// The narrow view of `Grid` the `l13` crate's protocol handlers see — they
/// have no dependency on `Grid` itself, only on this trait, so the side-channel
/// stays independently buildable/testable (see `l13/src/lib.rs`). Every method
/// here delegates to an existing inherent method or field; this impl carries no
/// logic of its own.
#[cfg(feature = "l13")]
impl rusty_term_l13::TerminalState for Grid {
    fn screen_text(&self) -> String {
        let mut lines: Vec<String> = self.screen.iter().map(|line| self.text_of(line)).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    fn scrollback_text(&self, max: usize) -> String {
        let skip = self.scrollback.len().saturating_sub(max);
        self.scrollback
            .iter()
            .skip(skip)
            .map(|line| self.text_of(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cwd(&self) -> &str {
        &self.cwd
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn dimensions(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    fn cursor(&self) -> (usize, usize) {
        self.cursor
    }

    fn last_command_exit(&self) -> Option<i32> {
        Grid::last_command_exit(self)
    }

    fn last_command_output(&self) -> Option<&str> {
        Grid::last_command_output(self)
    }

    fn set_status_line(&mut self, text: String, fg: Option<u32>, bg: Option<u32>) {
        Grid::set_status_line(self, text, fg, bg)
    }

    fn clear_status_line(&mut self) {
        Grid::clear_status_line(self)
    }

    fn is_subscribed(&self, uri: &'static str) -> bool {
        self.channel.is_subscribed(uri)
    }

    fn subscribe(&mut self, uri: &'static str) {
        self.channel.subscribe(uri)
    }

    fn unsubscribe(&mut self, uri: &str) {
        self.channel.unsubscribe(uri)
    }
}

/// A snapshot of the dirty rows of a [`Grid`], plus the cursor position so the
/// renderer can place the hardware cursor.
pub struct DirtyFrame {
    /// Cursor position `(col, row)` at snapshot time.
    pub cursor: (usize, usize),
    /// Dirty rows as `(row_index, cells)` pairs.
    pub rows: Vec<(usize, Vec<Cell>)>,
    /// Interned hyperlink URIs (OSC 8); a [`Cell::link`] of `n` indexes
    /// `links[n - 1]`. An `Arc` clone (O(1)) of the grid's cached snapshot, so
    /// the renderer can resolve links without the lock and without the O(n)
    /// deep string clone this used to be on every single frame.
    pub links: Arc<[String]>,
    /// Interned grapheme continuations (see [`Grid::clusters`]); a
    /// [`Cell::cluster`] of `n` indexes `clusters[n - 1]`. An `Arc` clone
    /// (O(1)) of the grid's cached snapshot, mirroring [`Self::links`].
    pub clusters: Arc<[String]>,
    /// Per-row line size attributes (DECDWL/DECDHL), indexed by each row's `y`
    /// in [`rows`](Self::rows). Cloned so the renderer can relay them to the host.
    pub line_attrs: Vec<LineAttr>,
}

/// Reconstruct a row of cells as text: base glyph plus any interned grapheme
/// continuation, skipping wide-glyph trailers, with trailing blanks trimmed.
/// Shared by the MCP screen/scrollback/command readers and the windowed
/// front-end's command dock labels, and by [`Grid::line_text`].
pub(crate) fn row_text(cells: impl IntoIterator<Item = Cell>, clusters: &[String]) -> String {
    let mut s = String::new();
    for cell in cells {
        if cell.flags & WIDE_TRAILER != 0 {
            continue;
        }
        s.push(cell.ch);
        if cell.cluster != 0
            && let Some(suffix) = clusters.get((cell.cluster - 1) as usize)
        {
            s.push_str(suffix);
        }
    }
    s.trim_end().to_string()
}

// RTCP deliberately excludes view selection/search/folds, render caches,
// recycled spare rows, pending host actions and L13 channel state.
impl crate::checkpoint::Wire for Grid {
    fn write(&self, w: &mut crate::checkpoint::Writer) -> crate::checkpoint::Result<()> {
        crate::checkpoint::Wire::write(&self.cols, w)?;
        crate::checkpoint::Wire::write(&self.rows, w)?;
        crate::checkpoint::Wire::write(&self.screen, w)?;
        crate::checkpoint::Wire::write(&self.cursor, w)?;
        crate::checkpoint::Wire::write(&self.saved_cursor, w)?;
        crate::checkpoint::Wire::write(&self.scroll_top, w)?;
        crate::checkpoint::Wire::write(&self.scroll_bottom, w)?;
        crate::checkpoint::Wire::write(&self.primary, w)?;
        crate::checkpoint::Wire::write(&self.title, w)?;
        crate::checkpoint::Wire::write(&self.title_stack, w)?;
        crate::checkpoint::Wire::write(&self.kitty_flags_stack, w)?;
        crate::checkpoint::Wire::write(&self.cwd, w)?;
        crate::checkpoint::Wire::write(&self.scrollback, w)?;
        crate::checkpoint::Wire::write(&self.scrollback_max, w)?;
        crate::checkpoint::Wire::write(&self.current_link, w)?;
        crate::checkpoint::Wire::write(&self.tab_stops, w)?;
        crate::checkpoint::Wire::write(&self.cursor_visible, w)?;
        crate::checkpoint::Wire::write(&self.sync_output, w)?;
        crate::checkpoint::Wire::write(&self.sync_output_since, w)?;
        crate::checkpoint::Wire::write(&self.bracketed_paste, w)?;
        crate::checkpoint::Wire::write(&self.app_cursor_keys, w)?;
        crate::checkpoint::Wire::write(&self.alt_scroll, w)?;
        crate::checkpoint::Wire::write(&self.kitty_images, w)?;
        crate::checkpoint::Wire::write(&self.next_anim_id, w)?;
        crate::checkpoint::Wire::write(&self.kitty_virtual, w)?;
        crate::checkpoint::Wire::write(&self.progress, w)?;
        crate::checkpoint::Wire::write(&self.report_color_scheme, w)?;
        crate::checkpoint::Wire::write(&self.bdsm, w)?;
        crate::checkpoint::Wire::write(&self.bidi_autodetect, w)?;
        crate::checkpoint::Wire::write(&self.bidi_para_rtl, w)?;
        crate::checkpoint::Wire::write(&self.notif99_pending, w)?;
        crate::checkpoint::Wire::write(&self.focus_reporting, w)?;
        crate::checkpoint::Wire::write(&self.win32_input, w)?;
        crate::checkpoint::Wire::write(&self.app_keypad, w)?;
        crate::checkpoint::Wire::write(&self.line_feed_new_line, w)?;
        crate::checkpoint::Wire::write(&self.mouse_modes, w)?;
        crate::checkpoint::Wire::write(&self.cell_px, w)?;
        crate::checkpoint::Wire::write(&self.cursor_icon, w)?;
        crate::checkpoint::Wire::write(&self.autowrap, w)?;
        crate::checkpoint::Wire::write(&self.grapheme_clustering, w)?;
        crate::checkpoint::Wire::write(&self.allow_deccolm, w)?;
        crate::checkpoint::Wire::write(&self.column_mode_132, w)?;
        crate::checkpoint::Wire::write(&self.origin_mode, w)?;
        crate::checkpoint::Wire::write(&self.lr_margin_mode, w)?;
        crate::checkpoint::Wire::write(&self.left_margin, w)?;
        crate::checkpoint::Wire::write(&self.right_margin, w)?;
        crate::checkpoint::Wire::write(&self.insert_mode, w)?;
        crate::checkpoint::Wire::write(&self.default_fg(), w)?;
        crate::checkpoint::Wire::write(&self.default_bg(), w)?;
        crate::checkpoint::Wire::write(&self.erase_bg, w)?;
        crate::checkpoint::Wire::write(&self.cursor_color, w)?;
        crate::checkpoint::Wire::write(&self.cursor_shape, w)?;
        crate::checkpoint::Wire::write(&self.cursor_blink, w)?;
        crate::checkpoint::Wire::write(&self.default_cursor_shape, w)?;
        crate::checkpoint::Wire::write(&self.default_cursor_blink, w)?;
        crate::checkpoint::Wire::write(&self.prompt_marks, w)?;
        crate::checkpoint::Wire::write(&self.last_exit, w)?;
        crate::checkpoint::Wire::write(&self.status_line, w)?;
        crate::checkpoint::Wire::write(&self.clusters, w)?;
        crate::checkpoint::Wire::write(&self.links, w)?;
        w.charge(self.table_cache_cost().saturating_add(self.rows))?;
        Ok(())
    }
    fn read(r: &mut crate::checkpoint::Reader<'_>) -> crate::checkpoint::Result<Self> {
        let mut grid = Self::new(1, 1);
        grid.cols = crate::checkpoint::Wire::read(r)?;
        grid.rows = crate::checkpoint::Wire::read(r)?;
        grid.screen = crate::checkpoint::Wire::read(r)?;
        grid.cursor = crate::checkpoint::Wire::read(r)?;
        grid.saved_cursor = crate::checkpoint::Wire::read(r)?;
        grid.scroll_top = crate::checkpoint::Wire::read(r)?;
        grid.scroll_bottom = crate::checkpoint::Wire::read(r)?;
        grid.primary = crate::checkpoint::Wire::read(r)?;
        grid.title = crate::checkpoint::Wire::read(r)?;
        grid.title_stack = crate::checkpoint::Wire::read(r)?;
        grid.kitty_flags_stack = crate::checkpoint::Wire::read(r)?;
        grid.cwd = crate::checkpoint::Wire::read(r)?;
        grid.scrollback = crate::checkpoint::Wire::read(r)?;
        grid.untrimmed = grid.scrollback.len();
        grid.scrollback_max = crate::checkpoint::Wire::read(r)?;
        grid.current_link = crate::checkpoint::Wire::read(r)?;
        grid.tab_stops = crate::checkpoint::Wire::read(r)?;
        grid.cursor_visible = crate::checkpoint::Wire::read(r)?;
        grid.sync_output = crate::checkpoint::Wire::read(r)?;
        grid.sync_output_since = crate::checkpoint::Wire::read(r)?;
        grid.bracketed_paste = crate::checkpoint::Wire::read(r)?;
        grid.app_cursor_keys = crate::checkpoint::Wire::read(r)?;
        grid.alt_scroll = crate::checkpoint::Wire::read(r)?;
        grid.kitty_images = crate::checkpoint::Wire::read(r)?;
        grid.next_anim_id = crate::checkpoint::Wire::read(r)?;
        grid.kitty_virtual = crate::checkpoint::Wire::read(r)?;
        grid.progress = crate::checkpoint::Wire::read(r)?;
        grid.report_color_scheme = crate::checkpoint::Wire::read(r)?;
        grid.bdsm = crate::checkpoint::Wire::read(r)?;
        grid.bidi_autodetect = crate::checkpoint::Wire::read(r)?;
        grid.bidi_para_rtl = crate::checkpoint::Wire::read(r)?;
        grid.notif99_pending = crate::checkpoint::Wire::read(r)?;
        grid.focus_reporting = crate::checkpoint::Wire::read(r)?;
        grid.win32_input = crate::checkpoint::Wire::read(r)?;
        grid.app_keypad = crate::checkpoint::Wire::read(r)?;
        grid.line_feed_new_line = crate::checkpoint::Wire::read(r)?;
        grid.mouse_modes = crate::checkpoint::Wire::read(r)?;
        grid.cell_px = crate::checkpoint::Wire::read(r)?;
        grid.cursor_icon = crate::checkpoint::Wire::read(r)?;
        grid.autowrap = crate::checkpoint::Wire::read(r)?;
        grid.grapheme_clustering = crate::checkpoint::Wire::read(r)?;
        grid.allow_deccolm = crate::checkpoint::Wire::read(r)?;
        grid.column_mode_132 = crate::checkpoint::Wire::read(r)?;
        grid.origin_mode = crate::checkpoint::Wire::read(r)?;
        grid.lr_margin_mode = crate::checkpoint::Wire::read(r)?;
        grid.left_margin = crate::checkpoint::Wire::read(r)?;
        grid.right_margin = crate::checkpoint::Wire::read(r)?;
        grid.insert_mode = crate::checkpoint::Wire::read(r)?;
        let defaults = Defaults {
            fg: crate::checkpoint::Wire::read(r)?,
            bg: crate::checkpoint::Wire::read(r)?,
        };
        grid.erase_bg = crate::checkpoint::Wire::read(r)?;
        grid.cursor_color = crate::checkpoint::Wire::read(r)?;
        grid.cursor_shape = crate::checkpoint::Wire::read(r)?;
        grid.cursor_blink = crate::checkpoint::Wire::read(r)?;
        grid.default_cursor_shape = crate::checkpoint::Wire::read(r)?;
        grid.default_cursor_blink = crate::checkpoint::Wire::read(r)?;
        grid.prompt_marks = crate::checkpoint::Wire::read(r)?;
        grid.last_exit = crate::checkpoint::Wire::read(r)?;
        grid.status_line = crate::checkpoint::Wire::read(r)?;
        grid.clusters = crate::checkpoint::Wire::read(r)?;
        grid.links = crate::checkpoint::Wire::read(r)?;
        // Bound the tables before building caches from them; the full
        // validation runs after the parser is read.
        if grid.clusters.len() > CLUSTER_MAX || grid.links.len() > LINK_MAX {
            return Err(crate::checkpoint::Error("grid bounds"));
        }
        r.charge(grid.table_cache_cost())?;
        grid.styles = r.take_styles();
        grid.styles.set_defaults(defaults);
        grid.links_arc = grid.links.clone().into();
        grid.rebuild_cluster_caches();
        r.charge(grid.rows)?;
        if grid.rows > 4096 {
            return Err(crate::checkpoint::Error("grid dimensions"));
        }
        grid.reset_damage();
        Ok(grid)
    }
}
crate::checkpoint::fields!(SavedScreen, [screen, cursor, saved_cursor, mode]);
crate::checkpoint::fields!(MouseModes, [base, extended]);
crate::checkpoint::fields!(
    KittyImage,
    [id, w, h, frames, current, playing, last_advance]
);
crate::checkpoint::fields!(KittyFrame, [pixels, gap_ms]);
crate::checkpoint::fields!(StatusLine, [text, fg, bg, cells]);
crate::checkpoint::variants!(CursorShape, [Block, Underline, Bar]);
crate::checkpoint::variants!(LineAttr, [Single, DoubleWidth, DoubleTop, DoubleBottom]);
crate::checkpoint::variants!(AltMode, [Dec47, Dec1047, Dec1049]);

impl Grid {
    pub(crate) fn validate_checkpoint(&self) -> crate::checkpoint::Result<()> {
        use crate::checkpoint::Error;
        if self.cols == 0
            || self.rows == 0
            || self.cols > 4096
            || self.rows > 4096
            || self.screen.len() != self.rows
            || self.tab_stops.len() != self.cols
            || self.scroll_top > self.scroll_bottom
            || self.scroll_bottom >= self.rows
            || self.left_margin > self.right_margin
            || self.right_margin >= self.cols
            || self.scrollback_max > SCROLLBACK_MAX
            || self.scrollback.len() > self.scrollback_max
            || self.clusters.len() > CLUSTER_MAX
            || self.links.len() > LINK_MAX
            || self.title_stack.len() > TITLE_STACK_MAX
            || self.kitty_flags_stack.len() > KITTY_FLAGS_STACK_MAX
            || usize::from(self.current_link) > self.links.len()
            || self.kitty_virtual.len() > 16
            || self.prompt_marks.len() > PROMPT_MARKS_MAX
            || self.notif99_pending.len() > 4
        {
            return Err(Error("grid bounds"));
        }
        let point = |p: (usize, usize)| p.0 <= self.cols && p.1 < self.rows;
        if !point(self.cursor) || !point(self.saved_cursor) {
            return Err(Error("cursor bounds"));
        }
        if let Some(p) = &self.primary
            && (p.screen.len() != self.rows || !point(p.cursor) || !point(p.saved_cursor))
        {
            return Err(Error("primary screen bounds"));
        }
        // Styles are checked once each; cells only for naming a live one.
        let free = self.styles.free_mask();
        let style_ok = |i: usize| {
            let style = self.styles.get(StyleId(i as u32));
            usize::from(style.cluster) <= self.clusters.len()
                && usize::from(style.link) <= self.links.len()
        };
        if (0..free.len()).any(|i| !free[i] && !style_ok(i)) {
            return Err(Error("style bounds"));
        }
        if self.lines().any(|line| line.cells().len() > 4096) {
            return Err(Error("cell bounds"));
        }
        if self.screen.iter().any(|l| l.cells().len() != self.cols)
            || self
                .primary
                .iter()
                .any(|p| p.screen.iter().any(|l| l.cells().len() != self.cols))
        {
            return Err(Error("screen width"));
        }
        if let Some(s) = &self.status_line
            && (s.cells.len() != self.cols
                || s.cells.iter().any(|c| {
                    usize::from(c.cluster) > self.clusters.len()
                        || usize::from(c.link) > self.links.len()
                }))
        {
            return Err(Error("status bounds"));
        }
        let mut pixels = 0usize;
        for image in &self.kitty_images {
            let n = image
                .w
                .checked_mul(image.h)
                .ok_or(Error("image dimensions"))?;
            if n == 0 || image.frames.is_empty() || image.current >= image.frames.len() {
                return Err(Error("image bounds"));
            }
            for frame in &image.frames {
                pixels = pixels
                    .checked_add(frame.pixels.len())
                    .ok_or(Error("pixel overflow"))?;
                if frame.pixels.len() != n {
                    return Err(Error("image frame size"));
                }
            }
        }
        if pixels > crate::checkpoint::MAX_DECODED / size_of::<Option<u32>>() {
            return Err(Error("image store budget"));
        }
        Ok(())
    }

    /// Whether every cell names a live style. Encoding checks this per run
    /// as it writes; decoding checks the restored grid here.
    pub(crate) fn validate_cell_styles(&self) -> crate::checkpoint::Result<()> {
        let free = self.styles.free_mask();
        let live = |c: &PackedCell| free.get(c.style().0 as usize) == Some(&false);
        if self.lines().all(|line| line.cells().iter().all(live)) {
            return Ok(());
        }
        Err(crate::checkpoint::Error("cell bounds"))
    }

    /// Live and primary screen rows, then history.
    fn lines(&self) -> impl Iterator<Item = &Line> {
        let primary = self.primary.iter().flat_map(|p| p.screen.iter());
        self.screen.iter().chain(primary).chain(&self.scrollback)
    }
}
