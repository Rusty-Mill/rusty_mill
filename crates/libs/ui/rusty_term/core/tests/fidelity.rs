//! Fidelity gate: replay each recorded session in `tests/fidelity/corpus/`
//! and compare the final grid (text, styles, cursor) with its committed golden
//! `<name>.grid`. Any cell change fails, so engine refactors (ring-buffer
//! scrolling, batched printing) must keep every screen identical.
//!
//! Each stream is also replayed one byte at a time; the result must match the
//! 4 KiB-chunk replay, since escape sequences may straddle PTY reads.
//!
//! Regenerate goldens after an intended change, and review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p rusty_term_core --test fidelity`.
//! Record new sessions with `tests/fidelity/record.py`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use rusty_term_core::{
    ATTR_BLINK, ATTR_BOLD, ATTR_DIM, ATTR_HIDDEN, ATTR_ITALIC, ATTR_REVERSE, ATTR_STRIKE,
    ATTR_UNDERLINE, ATTR_UNDERLINE_COLOR, AnsiParser, Grid, UnderlineStyle, WIDE_TRAILER,
};

/// Every recording is made on an 80x24 PTY (see `record.py`).
const COLS: usize = 80;
const ROWS: usize = 24;
/// A typical PTY read size.
const CHUNK: usize = 4096;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fidelity/corpus")
}

fn replay(data: &[u8], chunk: usize) -> Grid {
    let mut grid = Grid::new(COLS, ROWS);
    let mut parser = AnsiParser::new();
    for piece in data.chunks(chunk) {
        parser.advance(&mut grid, piece);
        parser.take_responses();
        grid.take_host_out();
    }
    grid
}

/// `-` for the grid's default color, else `#rrggbb`.
fn color(value: u32, default: u32) -> String {
    if value == default {
        "-".to_owned()
    } else {
        format!("#{value:06x}")
    }
}

fn attrs(flags: u16) -> String {
    const NAMES: [(u16, &str); 7] = [
        (ATTR_BOLD, "bold"),
        (ATTR_DIM, "dim"),
        (ATTR_ITALIC, "italic"),
        (ATTR_BLINK, "blink"),
        (ATTR_REVERSE, "reverse"),
        (ATTR_HIDDEN, "hidden"),
        (ATTR_STRIKE, "strike"),
    ];
    let mut out: Vec<String> = NAMES
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect();
    if flags & ATTR_UNDERLINE != 0 {
        out.push(format!("underline:{:?}", UnderlineStyle::from_attrs(flags)).to_lowercase());
    }
    out.join(",")
}

/// The comparable state of a grid: one text line per row (trailing blanks
/// trimmed, wide-glyph trailers skipped, grapheme clusters included), one line
/// per run of identically styled non-default cells, then the column of every
/// wide-glyph trailer, which pins each glyph to its column.
fn dump(grid: &Grid) -> String {
    let blank = Grid::new(1, 1).cells[0];
    let (default_fg, default_bg) = (blank.fg, blank.bg);
    let mut out = format!(
        "size {}x{} cursor {},{} visible={} alt={}\n",
        grid.cols,
        grid.rows,
        grid.cursor.0,
        grid.cursor.1,
        grid.cursor_visible,
        grid.in_alt_screen()
    );
    let mut styles = String::new();
    let mut trailers = String::new();
    for (y, row) in grid.cells.chunks(grid.cols).enumerate() {
        let cols: Vec<String> = (0..row.len())
            .filter(|&x| row[x].flags & WIDE_TRAILER != 0)
            .map(|x| x.to_string())
            .collect();
        if !cols.is_empty() {
            let _ = writeln!(trailers, "{y}:{}", cols.join(","));
        }
        let mut text = String::new();
        let mut run: Option<(usize, usize, String)> = None;
        for (x, cell) in row.iter().enumerate() {
            if cell.flags & WIDE_TRAILER != 0 {
                continue;
            }
            text.push(cell.ch);
            if cell.cluster != 0 {
                text.push_str(&grid.clusters[usize::from(cell.cluster) - 1]);
            }
            let mut style = format!(
                "fg={} bg={} {}",
                color(cell.fg, default_fg),
                color(cell.bg, default_bg),
                attrs(cell.flags)
            );
            if cell.flags & ATTR_UNDERLINE_COLOR != 0 {
                let _ = write!(style, " ul=#{:06x}", cell.underline_color);
            }
            let plain = style == "fg=- bg=- ";
            match &mut run {
                Some((_, end, current)) if *current == style => *end = x,
                _ => {
                    if let Some((start, end, current)) = run.take() {
                        let _ = writeln!(styles, "{y}:{start}-{end} {}", current.trim_end());
                    }
                    run = (!plain).then_some((x, x, style));
                }
            }
        }
        if let Some((start, end, current)) = run {
            let _ = writeln!(styles, "{y}:{start}-{end} {}", current.trim_end());
        }
        let _ = writeln!(out, "|{}", text.trim_end());
    }
    out.push_str("styles:\n");
    out.push_str(&styles);
    out.push_str("trailers:\n");
    out.push_str(&trailers);
    out
}

#[test]
fn corpus_matches_goldens() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut entries: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("corpus directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "vt"))
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "no recordings in {}",
        corpus_dir().display()
    );

    let mut failures = Vec::new();
    for vt in &entries {
        let name = vt
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_owned();
        let data = fs::read(vt).expect("read recording");
        let actual = dump(&replay(&data, CHUNK));
        let bytewise = dump(&replay(&data, 1));
        if bytewise != actual {
            failures.push(format!(
                "{name}: byte-at-a-time replay differs from {CHUNK}-byte chunks"
            ));
        }
        let golden = vt.with_extension("grid");
        if update {
            fs::write(&golden, &actual).expect("write golden");
            continue;
        }
        match fs::read_to_string(&golden) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => failures.push(format!(
                "{name}: grid differs\n{}",
                line_diff(&expected, &actual)
            )),
            Err(e) => failures.push(format!("{name}: no golden ({e}); run with UPDATE_GOLDEN=1")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The first few differing lines, enough to see what moved.
fn line_diff(expected: &str, actual: &str) -> String {
    let (e, a): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    (0..e.len().max(a.len()))
        .filter(|&i| e.get(i) != a.get(i))
        .take(8)
        .map(|i| {
            format!(
                "  line {i}:\n    - {}\n    + {}",
                e.get(i).unwrap_or(&""),
                a.get(i).unwrap_or(&"")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A wide glyph's trailer moved one column right must change the dump: the
/// text line alone (trailers skipped) cannot tell these rows apart.
#[test]
fn dump_pins_wide_trailer_columns() {
    let mut parser = AnsiParser::new();
    let mut good = Grid::new(4, 1);
    parser.advance(&mut good, "日A".as_bytes());
    let mut moved = Grid::new(4, 1);
    parser.advance(&mut moved, "日A".as_bytes());
    // [日, trailer, A, ·] -> [日, A, trailer, ·]
    moved.cells.swap(1, 2);
    assert_ne!(
        good.cells[1].flags & WIDE_TRAILER,
        0,
        "fixture: cell 1 is the trailer"
    );
    assert_ne!(dump(&good), dump(&moved));
}
