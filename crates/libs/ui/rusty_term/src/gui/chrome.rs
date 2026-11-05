//! Chrome colour tokens: the design pack's fixed surfaces and text, the
//! terminal theme's accent, and fixed status colours.
//!
//! Pack rows (`docs/projects/term/gui-design/DECISIONS.md`): surfaces and text
//! are the D5 (dark) or D25 (light) tokens (D34); the accent is the theme's
//! cursor colour unless it fails the legibility floor, when the pack verdigris
//! stands in (D35); success and failure are always verdigris and oxide (D36).
//! Program output is not chrome: the grid keeps the theme's own colours.

use crate::core::{Theme, contrast_ratio};

/// Contrast the accent must keep against the surface it marks (D27 floor).
const ACCENT_MIN: f32 = 3.0;
/// Contrast text needs against its surface, including text on an accent fill.
const TEXT_MIN: f32 = 4.5;

/// One mode's fixed pack tokens (D5 dark, D25 light).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pack {
    frame: u32,
    base: u32,
    raised: u32,
    sunk: u32,
    line: u32,
    bone: u32,
    slate: u32,
    dim: u32,
    verdigris: u32,
    oxide: u32,
}

/// D5: the dark palette.
const DARK: Pack = Pack {
    frame: 0x171C1F,
    base: 0x1C2226,
    raised: 0x262F34,
    sunk: 0x13171A,
    line: 0x2F393E,
    bone: 0xDCE2E1,
    slate: 0x8A979C,
    dim: 0x6E7B80,
    verdigris: 0x5FB8A5,
    oxide: 0xD9773A,
};

/// D25: the light palette (tinted ground, darker accents).
const LIGHT: Pack = Pack {
    frame: 0xD3DBDA,
    base: 0xEEF2F1,
    raised: 0xF8FAF9,
    sunk: 0xE1E8E7,
    line: 0xB9C4C3,
    bone: 0x182024,
    slate: 0x4A585D,
    dim: 0x76848A,
    verdigris: 0x176B5A,
    oxide: 0xA8460F,
};

/// Every colour the chrome draws with, resolved once per theme.
///
/// A surface and the text on it are used as a pair; see the field docs for
/// which pairs hold [`TEXT_MIN`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chrome {
    /// Tab strip band.
    pub frame: u32,
    /// Active tab, overlay pages.
    pub base: u32,
    /// Hover, menus, search bar, selected rows: one step up from `base`.
    pub raised: u32,
    /// Status ribbon and command dock.
    pub sunk: u32,
    /// Dividers and hairlines.
    pub line: u32,
    /// Primary text, on any surface.
    pub text: u32,
    /// Secondary text, on any surface.
    pub muted: u32,
    /// Non-text icons only (D17): fails the text floor on purpose.
    pub dim: u32,
    /// Focus, active tab, selection bar, running mark (D34, D35).
    pub accent: u32,
    /// Text drawn on an `accent` fill.
    pub on_accent: u32,
    /// Success (D36).
    pub success: u32,
    /// Failure (D36). Never as text on `raised` (D17).
    pub failure: u32,
}

impl Chrome {
    /// Resolve the chrome colours for `theme`: the light pack for a light
    /// background, the dark pack otherwise.
    pub(crate) fn from_theme(theme: &Theme) -> Self {
        let pack = if is_light(theme.bg) { LIGHT } else { DARK };
        let (accent, on_accent) = accent_for(theme.cursor, &pack);
        Self {
            frame: pack.frame,
            base: pack.base,
            raised: pack.raised,
            sunk: pack.sunk,
            line: pack.line,
            text: pack.bone,
            muted: pack.slate,
            dim: pack.dim,
            accent,
            on_accent,
            success: pack.verdigris,
            failure: pack.oxide,
        }
    }
}

/// Whether `bg` reads as a light background: dark text stands out more on it
/// than white does.
fn is_light(bg: u32) -> bool {
    contrast_ratio(bg, 0x000000) > contrast_ratio(bg, 0xFFFFFF)
}

/// The accent and the text on it (D35): the theme's colour when it holds
/// [`ACCENT_MIN`] on every chrome surface and one of the pack's text tokens
/// holds [`TEXT_MIN`] on it; otherwise the pack verdigris.
fn accent_for(themed: u32, pack: &Pack) -> (u32, u32) {
    let surfaces = [pack.frame, pack.base, pack.raised, pack.sunk];
    let visible = surfaces
        .iter()
        .all(|&s| contrast_ratio(themed, s) >= ACCENT_MIN);
    match on_fill(themed, pack) {
        Some(text) if visible => (themed, text),
        _ => (pack.verdigris, pack.sunk),
    }
}

/// The pack text token with the most contrast on `fill`, if it reaches
/// [`TEXT_MIN`]. Candidates are the darkest and lightest tokens.
fn on_fill(fill: u32, pack: &Pack) -> Option<u32> {
    let pick = [pack.sunk, pack.bone]
        .into_iter()
        .max_by(|&a, &b| contrast_ratio(a, fill).total_cmp(&contrast_ratio(b, fill)))?;
    (contrast_ratio(pick, fill) >= TEXT_MIN).then_some(pick)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces(c: &Chrome) -> [u32; 4] {
        [c.frame, c.base, c.raised, c.sunk]
    }

    /// Every chrome surface/text pair, accent and on-accent text hold their
    /// floors for `theme`.
    fn assert_legible(theme: &Theme, name: &str) {
        let c = Chrome::from_theme(theme);
        for s in surfaces(&c) {
            for (label, fg) in [("text", c.text), ("muted", c.muted)] {
                let r = contrast_ratio(fg, s);
                assert!(
                    r >= TEXT_MIN,
                    "{name}: {label} {fg:06x} on {s:06x} = {r:.2}"
                );
            }
            let r = contrast_ratio(c.accent, s);
            assert!(
                r >= ACCENT_MIN,
                "{name}: accent {:06x} on {s:06x} = {r:.2}",
                c.accent
            );
        }
        let r = contrast_ratio(c.on_accent, c.accent);
        assert!(r >= TEXT_MIN, "{name}: on-accent = {r:.2}");
        // Status colours: legible on every surface except failure on raised
        // (D17, D25 keep oxide text off the raised and light frame surfaces).
        for s in [c.base, c.sunk] {
            for fg in [c.success, c.failure] {
                let r = contrast_ratio(fg, s);
                assert!(r >= TEXT_MIN, "{name}: status {fg:06x} on {s:06x} = {r:.2}");
            }
        }
    }

    #[test]
    fn chrome_is_legible_on_every_preset() {
        for name in crate::config::PRESETS {
            let theme = crate::config::preset(name).expect("a preset");
            assert_legible(&theme, name);
        }
    }

    #[test]
    fn chrome_is_legible_for_a_sweep_of_custom_themes() {
        for step in 0..=16u32 {
            let v = (step * 255 / 16) & 0xFF;
            let grey = v << 16 | v << 8 | v;
            for cursor in [0x000000, 0xFFFFFF, 0xFFFF00, 0x0000FF, 0xFF00FF, grey] {
                let theme = Theme {
                    bg: grey,
                    cursor,
                    ..Theme::default()
                };
                assert_legible(&theme, &format!("bg {grey:06x} cursor {cursor:06x}"));
            }
        }
    }

    #[test]
    fn dark_and_light_backgrounds_pick_their_pack() {
        let dark = Chrome::from_theme(&Theme {
            bg: 0x101010,
            ..Theme::default()
        });
        let light = Chrome::from_theme(&Theme {
            bg: 0xF0F0F0,
            ..Theme::default()
        });
        assert_eq!(dark.frame, DARK.frame);
        assert_eq!(light.frame, LIGHT.frame);
    }

    #[test]
    fn a_legible_theme_accent_is_kept() {
        let theme = Theme {
            bg: 0x101010,
            cursor: 0xE0C060,
            ..Theme::default()
        };
        assert_eq!(Chrome::from_theme(&theme).accent, 0xE0C060);
    }

    #[test]
    fn an_illegible_theme_accent_falls_back_to_verdigris() {
        // Dark grey cursor on the dark pack: under 3:1 on every surface.
        let dark = Theme {
            bg: 0x101010,
            cursor: 0x303030,
            ..Theme::default()
        };
        assert_eq!(Chrome::from_theme(&dark).accent, DARK.verdigris);
        let light = Theme {
            bg: 0xF0F0F0,
            cursor: 0xF8F8F8,
            ..Theme::default()
        };
        assert_eq!(Chrome::from_theme(&light).accent, LIGHT.verdigris);
    }

    #[test]
    fn status_colours_ignore_the_theme_palette() {
        let mut theme = Theme {
            bg: 0x101010,
            ..Theme::default()
        };
        theme.palette16[1] = 0x00FF00;
        theme.palette16[2] = 0xFF0000;
        let c = Chrome::from_theme(&theme);
        assert_eq!((c.success, c.failure), (DARK.verdigris, DARK.oxide));
    }
}
