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
    checkpoint,
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
    replay_with_parser(data, chunk).0
}

/// [`replay`], keeping the parser, which a checkpoint also encodes.
fn replay_with_parser(data: &[u8], chunk: usize) -> (Grid, AnsiParser) {
    let mut grid = Grid::new(COLS, ROWS);
    let mut parser = AnsiParser::new();
    for piece in data.chunks(chunk) {
        parser.advance(&mut grid, piece);
        parser.take_responses();
        grid.take_host_out();
    }
    (grid, parser)
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
    let blank = Grid::new(1, 1).row(0)[0];
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
    for y in 0..grid.rows {
        let row = grid.row(y);
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

/// The corpus recordings (`*.vt`), sorted.
fn recordings() -> Vec<PathBuf> {
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
    entries
}

#[test]
fn corpus_matches_goldens() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let entries = recordings();

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

/// Every recorded screen survives a snapshot round trip unchanged.
#[test]
fn corpus_round_trips_through_a_snapshot() {
    for vt in recordings() {
        let grid = replay(&fs::read(&vt).expect("read recording"), CHUNK);
        let back = Grid::decode_snapshot(&grid.encode_snapshot()).expect("decode snapshot");
        assert_eq!(dump(&back), dump(&grid), "{}", vt.display());
    }
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
    // [日, trailer, A, ·] -> [日, A, trailer, ·], written left to right so
    // `set_cell`'s wide-glyph repair never fires.
    let row = good.row(0);
    let mut moved = Grid::new(4, 1);
    moved.cursor = good.cursor;
    for (x, from) in [0, 2, 1].into_iter().enumerate() {
        moved.set_cell(x, 0, row[from]);
    }
    assert_ne!(
        good.row(0)[1].flags & WIDE_TRAILER,
        0,
        "fixture: cell 1 is the trailer"
    );
    assert_ne!(dump(&good), dump(&moved));
}

/// Inputs the recordings miss but the wire formats carry: OSC 8 links,
/// DECSCA-protected cells with DECSED, deep scrollback, and one grapheme
/// cluster drawn in two pens.
const WIRE_EXTRAS: [(&str, &[u8]); 4] = [
    (
        "x_osc8_links",
        b"\x1b]8;;https://a.example\x1b\\one\x1b]8;;\x1b\\ \x1b]8;id=2;https://b.example\x1b\\two\x1b]8;;\x1b\\\r\n",
    ),
    (
        "x_decsca_decsed",
        b"keep \x1b[1\"qguard\x1b[0\"q gone\r\n\x1b[?2J",
    ),
    (
        "x_scrollback",
        b"\x1b[31m01\r\n02\r\n03\r\n04\r\n05\r\n06\r\n07\r\n08\r\n09\r\n10\r\n\x1b[42m11\r\n12\r\n13\r\n14\r\n15\r\n16\r\n17\r\n18\r\n19\r\n20\r\n\x1b[0;1m21\r\n22\r\n23\r\n24\r\n25\r\n26\r\n27\r\n28\r\n29\r\n30",
    ),
    (
        "x_cluster_two_pens",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} \x1b[35m\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\x1b[0m".as_bytes(),
    ),
];

/// FNV-1a, 64-bit: a stable, dependency-free fingerprint of encoded bytes.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// `name rtcp=<hash> rtsn=<hash>` for one replayed input. Encoding twice a
/// few milliseconds apart must give the same bytes, so no timer age (sync
/// output, animation, command start) can leak into the pinned hash.
fn wire_line(name: &str, data: &[u8]) -> String {
    let (grid, parser) = replay_with_parser(data, CHUNK);
    let rtcp = checkpoint::encode(&grid, &parser).expect("encode checkpoint");
    std::thread::sleep(std::time::Duration::from_millis(2));
    let again = checkpoint::encode(&grid, &parser).expect("encode checkpoint");
    assert_eq!(rtcp, again, "{name}: checkpoint depends on the clock");
    let rtsn = grid.encode_snapshot();
    format!(
        "{name} rtcp={:016x} rtsn={:016x}",
        fnv1a64(&rtcp),
        fnv1a64(&rtsn)
    )
}

/// RTCP and RTSN bytes for every recording and extra input are pinned, so a
/// storage change behind `Grid` cannot change either wire format.
/// Regenerate with `UPDATE_GOLDEN=1`, like the grid goldens.
#[test]
fn wire_bytes_match_pinned_hashes() {
    let mut lines: Vec<String> = recordings()
        .iter()
        .map(|vt| {
            let name = vt.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            wire_line(name, &fs::read(vt).expect("read recording"))
        })
        .collect();
    lines.extend(WIRE_EXTRAS.iter().map(|(name, data)| wire_line(name, data)));
    let actual = lines.join("\n") + "\n";
    let golden = corpus_dir().join("wire.hashes");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, &actual).expect("write wire hashes");
        return;
    }
    let expected = fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("no {} ({e}); run with UPDATE_GOLDEN=1", golden.display()));
    assert!(
        expected == actual,
        "wire bytes changed:\n{}",
        line_diff(&expected, &actual)
    );
}
