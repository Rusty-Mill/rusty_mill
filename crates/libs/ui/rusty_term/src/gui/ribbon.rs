//! An attached window's ribbon (gui-design F1, F2; D49): a band along the
//! bottom saying where the window is connected, how many blocks it shows,
//! the focused block's shell and the encoding, and on the right the prefix
//! keys. While the prefix is armed the hints light up and list every key.
//!
//! Pure: [`paint`] draws a [`Ribbon`] into the bottom of a pixmap. Sizes
//! are the pack's logical px times the DPI scale, never the terminal zoom.

use super::chrome::Chrome;
use super::paint::{Pixmap, Rect};
use super::text::{UiText, Weight};

/// Ribbon height, its top rule included.
const HEIGHT: f32 = 34.0;
const RULE: f32 = 1.0;
/// Padding (left, right), and the space between items of each group.
const PAD: (f32, f32) = (24.0, 20.0);
const LEFT_GAP: f32 = 22.0;
const RIGHT_GAP: f32 = 18.0;
/// The connection dot and the space after it.
const DOT: f32 = 7.0;
const TEXT_PX: f32 = 12.0;
/// Keycaps: height, least width, side padding, radius, text size, and the
/// space between keycaps and before a hint's label.
const KEY_H: f32 = 18.0;
const KEY_MIN: f32 = 18.0;
const KEY_PAD: f32 = 5.0;
const KEY_R: f32 = 4.0;
const KEY_PX: f32 = 11.0;
const KEY_GAP: f32 = 6.0;
const BORDER: f32 = 1.0;

/// What the ribbon says (D49; data handoff D105).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ribbon {
    /// What the window is connected to: "Connected to {target}".
    pub target: String,
    /// Blocks in the window's layout, zoomed or not.
    pub blocks: usize,
    /// The focused block's shell.
    pub shell: Option<String>,
    /// The prefix is armed (F2).
    pub armed: bool,
}

/// One hint: its keys and what they do.
struct Hint {
    keys: &'static [&'static str],
    label: &'static str,
    /// Lit while armed; `Esc` stays plain.
    lit: bool,
}

const fn hint(keys: &'static [&'static str], label: &'static str) -> Hint {
    Hint {
        keys,
        label,
        lit: true,
    }
}

/// The hints at rest (F1), and while the prefix is armed (F2).
const REST: [Hint; 5] = [
    hint(&["Ctrl-]"], "then"),
    hint(&["|"], "Split right"),
    hint(&["-"], "Split down"),
    hint(&["z"], "Zoom"),
    hint(&["d"], "Detach"),
];
const ARMED: [Hint; 8] = [
    hint(&["Ctrl-]"], "then"),
    hint(&["h", "j", "k", "l"], "Move focus"),
    hint(&["1-9"], "Jump"),
    hint(&["|"], "Split right"),
    hint(&["-"], "Split down"),
    hint(&["z"], "Zoom"),
    hint(&["d"], "Detach"),
    Hint {
        keys: &["Esc"],
        label: "to cancel",
        lit: false,
    },
];

/// The ribbon's height in px at `scale`.
pub(crate) fn height(scale: f32) -> usize {
    (HEIGHT * scale).round() as usize
}

/// How a keycap looks: lit (an armed prefix key) or plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    Plain,
    Lit,
}

/// A keycap's width in px for `label`.
pub(crate) fn keycap_width(label: &str, s: f32, font: &UiText) -> f32 {
    (font.width(label, Weight::Regular, KEY_PX * s) + 2.0 * KEY_PAD * s).max(KEY_MIN * s)
}

/// Draw a keycap for `label` at `x`, centred on `mid`; returns its width.
pub(crate) fn keycap(
    out: &mut Pixmap,
    font: &mut UiText,
    label: &str,
    (x, mid): (f32, f32),
    cap: Cap,
    (c, s): (&Chrome, f32),
) -> f32 {
    let w = keycap_width(label, s, font);
    let r = Rect {
        x,
        y: mid - KEY_H * s / 2.0,
        w,
        h: KEY_H * s,
    };
    let (edge, ink) = match cap {
        Cap::Plain => (c.line, c.muted),
        Cap::Lit => (c.accent, c.text),
    };
    out.border(r, KEY_R * s, BORDER * s, edge);
    let tw = font.width(label, Weight::Regular, KEY_PX * s);
    let base = mid + font.centre_drop(KEY_PX * s);
    out.text(
        font,
        label,
        Weight::Regular,
        KEY_PX * s,
        x + (w - tw) / 2.0,
        base,
        ink,
    );
    w
}

/// The left group's texts, dropped from the end as room runs out: the
/// encoding, then the shell (D49).
fn left_texts(ribbon: &Ribbon) -> Vec<String> {
    let count = match ribbon.blocks {
        1 => "1 block".to_owned(),
        n => format!("{n} blocks"),
    };
    let mut texts = vec![format!("Connected to {}", ribbon.target), count];
    texts.extend(ribbon.shell.clone());
    texts.push("UTF-8".to_owned());
    texts
}

fn hint_width(h: &Hint, s: f32, font: &UiText) -> f32 {
    let keys: f32 = h.keys.iter().map(|k| keycap_width(k, s, font)).sum();
    keys + h.keys.len() as f32 * KEY_GAP * s + font.width(h.label, Weight::Regular, TEXT_PX * s)
}

fn row_width(widths: impl Iterator<Item = f32>, gap: f32) -> f32 {
    let (sum, n) = widths.fold((0.0, 0usize), |(sum, n), w| (sum + w, n + 1));
    sum + n.saturating_sub(1) as f32 * gap
}

/// Draw `ribbon` across `out`'s full width, its top at `y`.
pub(crate) fn paint(
    out: &mut Pixmap,
    y: f32,
    ribbon: &Ribbon,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let width = out.width as f32;
    let band = Rect {
        x: 0.0,
        y,
        w: width,
        h: HEIGHT * s,
    };
    out.fill(band, 0.0, c.sunk);
    out.fill(
        Rect {
            h: RULE * s,
            ..band
        },
        0.0,
        c.line,
    );
    let mid = y + RULE * s + (HEIGHT - RULE) * s / 2.0;
    let base = mid + font.centre_drop(TEXT_PX * s);
    let hints: &[Hint] = if ribbon.armed { &ARMED } else { &REST };
    let room = width - (PAD.0 + PAD.1) * s;
    let lead = (DOT + DOT) * s;
    let mut left = left_texts(ribbon);
    let left_w = |texts: &[String], font: &UiText| {
        lead + row_width(
            texts
                .iter()
                .map(|t| font.width(t, Weight::Regular, TEXT_PX * s)),
            LEFT_GAP * s,
        )
    };
    let mut shown = hints.len();
    let right_w = |n: usize, font: &UiText| {
        row_width(
            hints[..n].iter().map(|h| hint_width(h, s, font)),
            RIGHT_GAP * s,
        )
    };
    let between = LEFT_GAP * s;
    while shown > 1 && left_w(&left, font) + between + right_w(shown, font) > room {
        shown -= 1;
    }
    while left.len() > 2 && left_w(&left, font) + between + right_w(shown, font) > room {
        left.pop();
    }
    // The target is cut last, to whatever room is left.
    let rest = left_w(&left[1..], font) + LEFT_GAP * s * f32::from(left.len() > 1);
    let target_room = room - right_w(shown, font) - between - rest - lead;
    left[0] = font.fit(&left[0], Weight::Regular, TEXT_PX * s, target_room.max(0.0));

    let mut x = PAD.0 * s;
    let dot = Rect {
        x,
        y: mid - DOT * s / 2.0,
        w: DOT * s,
        h: DOT * s,
    };
    out.fill(dot, DOT * s / 2.0, c.success);
    x += lead;
    for t in &left {
        x += out.text(font, t, Weight::Regular, TEXT_PX * s, x, base, c.muted) + LEFT_GAP * s;
    }

    let mut x = width - PAD.1 * s - right_w(shown, font);
    for h in &hints[..shown] {
        let lit = ribbon.armed && h.lit;
        let cap = if lit { Cap::Lit } else { Cap::Plain };
        for k in h.keys {
            x += keycap(out, font, k, (x, mid), cap, (c, s)) + KEY_GAP * s;
        }
        let ink = if lit { c.text } else { c.muted };
        x += out.text(font, h.label, Weight::Regular, TEXT_PX * s, x, base, ink) + RIGHT_GAP * s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> Chrome {
        Chrome::from_theme(&crate::core::Theme::default())
    }

    fn ribbon() -> Ribbon {
        Ribbon {
            target: "baileyai".into(),
            blocks: 3,
            shell: Some("zsh".into()),
            armed: false,
        }
    }

    #[test]
    fn the_left_group_counts_blocks_and_names_the_shell() {
        assert_eq!(
            left_texts(&ribbon()),
            ["Connected to baileyai", "3 blocks", "zsh", "UTF-8"]
        );
        let one = Ribbon {
            blocks: 1,
            shell: None,
            ..ribbon()
        };
        assert_eq!(
            left_texts(&one),
            ["Connected to baileyai", "1 block", "UTF-8"]
        );
    }

    #[test]
    fn the_band_is_sunk_under_a_rule() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let mut p = Pixmap::new(1200, 100, c.frame);
        paint(&mut p, 66.0, &ribbon(), (&c, 1.0), &mut font);
        let at = |x: usize, y: usize| p.pixels()[y * p.width + x];
        assert_eq!(at(600, 65), c.frame, "above the ribbon");
        assert_eq!(at(600, 66), c.line, "the rule");
        assert_eq!(at(600, 99), c.sunk, "the band");
        assert_eq!(at(27, 83), c.success, "the connection dot");
    }

    #[test]
    fn arming_lights_the_hints() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let mut rest = Pixmap::new(1600, 34, c.frame);
        paint(&mut rest, 0.0, &ribbon(), (&c, 1.0), &mut font);
        let mut armed = Pixmap::new(1600, 34, c.frame);
        let on = Ribbon {
            armed: true,
            ..ribbon()
        };
        paint(&mut armed, 0.0, &on, (&c, 1.0), &mut font);
        // Near the accent: a 1 px border lands across pixel edges.
        let near = |px: u32| {
            (0..3).all(|i| {
                let (a, b) = ((px >> (8 * i)) & 0xFF, (c.accent >> (8 * i)) & 0xFF);
                a.abs_diff(b) < 48
            })
        };
        let accent = |p: &Pixmap| p.pixels().iter().filter(|&&px| near(px)).count();
        assert_eq!(accent(&rest), 0);
        assert!(accent(&armed) > 0, "lit keycaps have accent borders");
    }

    #[test]
    fn a_narrow_ribbon_drops_hints_then_details_and_never_overflows() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        for width in [200, 420, 700] {
            let mut p = Pixmap::new(width, 34, c.frame);
            paint(&mut p, 0.0, &ribbon(), (&c, 1.0), &mut font);
            let right_pad = PAD.1 as usize;
            let spill = (0..34).any(|y| {
                (width - right_pad + 1..width).any(|x| {
                    let px = p.pixels()[y * width + x];
                    px != c.sunk && px != c.line
                })
            });
            assert!(!spill, "nothing drawn in the right padding at {width}");
        }
    }
}
