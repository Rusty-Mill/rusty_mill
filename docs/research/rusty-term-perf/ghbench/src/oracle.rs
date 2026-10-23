//! `ghbench grid <file.vt>`: replay a fidelity recording through libghostty-vt
//! and print the final 80x24 grid in the exact format of rusty_term_core's
//! `tests/fidelity.rs` dump, so the two can be diffed. Palette indices are
//! resolved with the standard xterm palette, the one rusty_term uses, so a
//! theme difference never shows up as a fidelity difference.
use libghostty_vt::Terminal;
use libghostty_vt::screen::{CellContentTag, CellWide, Screen};
use libghostty_vt::style::{StyleColor, Underline};
use libghostty_vt::terminal::{Point, PointCoordinate};
use std::fmt::Write as _;

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn xterm(index: u8) -> u32 {
    const BASE: [u32; 16] = [
        0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
        0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    match index {
        0..=15 => BASE[index as usize],
        16..=231 => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * u32::from(v) };
            let i = index - 16;
            (level(i / 36) << 16) | (level(i / 6 % 6) << 8) | level(i % 6)
        }
        _ => {
            let g = 8 + 10 * u32::from(index - 232);
            (g << 16) | (g << 8) | g
        }
    }
}

fn color(c: StyleColor) -> String {
    match c {
        StyleColor::None => "-".to_owned(),
        StyleColor::Palette(p) => format!("#{:06x}", xterm(p.0)),
        StyleColor::Rgb(rgb) => format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b),
    }
}

pub fn dump(path: &str) -> String {
    let data = std::fs::read(path).expect("read recording");
    let mut t = Terminal::new(COLS, ROWS).expect("terminal");
    for chunk in data.chunks(4096) {
        t.vt_write(chunk);
    }
    let mut out = format!(
        "size {COLS}x{ROWS} cursor {},{} visible={} alt={}\n",
        t.cursor_x().expect("cursor x"),
        t.cursor_y().expect("cursor y"),
        t.is_cursor_visible().expect("cursor visible"),
        t.active_screen().expect("screen") == Screen::Alternate,
    );
    let mut styles = String::new();
    let mut trailers = String::new();
    let mut buf = ['\0'; 32];
    for y in 0..ROWS {
        let mut tails = Vec::new();
        let mut text = String::new();
        let mut run: Option<(u16, u16, String)> = None;
        for x in 0..COLS {
            let at = t
                .grid_ref(Point::Active(PointCoordinate { x, y: u32::from(y) }))
                .expect("grid ref");
            let cell = at.cell().expect("cell");
            if cell.wide().expect("wide") == CellWide::SpacerTail {
                tails.push(x.to_string());
                continue;
            }
            let n = at.graphemes(&mut buf).unwrap_or(0);
            if n == 0 {
                text.push(' ');
            } else {
                text.extend(&buf[..n]);
            }
            let s = at.style().expect("style");
            let bg = match cell.content_tag().expect("tag") {
                CellContentTag::BgColorPalette => format!("#{:06x}", xterm(cell.bg_color_palette().expect("bg").0)),
                CellContentTag::BgColorRgb => {
                    let c = cell.bg_color_rgb().expect("bg");
                    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
                }
                _ => color(s.bg_color),
            };
            let mut attrs: Vec<String> = [
                (s.bold, "bold"),
                (s.faint, "dim"),
                (s.italic, "italic"),
                (s.blink, "blink"),
                (s.inverse, "reverse"),
                (s.invisible, "hidden"),
                (s.strikethrough, "strike"),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| (*name).to_owned())
            .collect();
            if s.underline != Underline::None {
                attrs.push(format!("underline:{:?}", s.underline).to_lowercase().replace("single", "straight"));
            }
            let mut style = format!("fg={} bg={bg} {}", color(s.fg_color), attrs.join(","));
            if s.underline != Underline::None && s.underline_color != StyleColor::None {
                let _ = write!(style, " ul={}", color(s.underline_color));
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
        if !tails.is_empty() {
            let _ = writeln!(trailers, "{y}:{}", tails.join(","));
        }
    }
    out.push_str("styles:\n");
    out.push_str(&styles);
    out.push_str("trailers:\n");
    out.push_str(&trailers);
    out
}
