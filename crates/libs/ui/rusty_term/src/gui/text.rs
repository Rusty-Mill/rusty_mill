//! Proportional interface text: the chrome's Instrument Sans (gui-design D6,
//! D59), drawn at pixel positions rather than on the cell grid.
//!
//! The faces are static instances of the OFL variable font at weights 400
//! and 500 (gui-design D46), embedded so the chrome looks the same on every
//! machine; see `assets/fonts/README.md`. Glyphs are placed on whole pixels
//! and not kerned: at 13 px the difference is under a pixel per pair.

use std::collections::HashMap;
use std::rc::Rc;

use rusty_font::Font;

use super::font::{Glyph, rasterize_id};

const REGULAR: &[u8] = include_bytes!("../../assets/fonts/InstrumentSans-Regular.ttf");
const MEDIUM: &[u8] = include_bytes!("../../assets/fonts/InstrumentSans-Medium.ttf");

/// The interface font's two weights (gui-design D46).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Weight {
    /// 400.
    Regular,
    /// 500.
    Medium,
}

/// A glyph placed on a line: `x` is its pen position from the line's start.
pub(crate) struct Placed {
    pub x: i32,
    pub glyph: Rc<Glyph>,
}

/// The interface font, with a cache of the glyphs drawn so far.
pub(crate) struct UiText {
    regular: Font,
    medium: Font,
    /// Keyed by weight, glyph id and size in 1/64 px.
    cache: HashMap<(Weight, u16, u32), Rc<Glyph>>,
}

impl UiText {
    /// The embedded faces; `None` only if the build embedded a broken file.
    pub(crate) fn embedded() -> Option<Self> {
        Some(Self {
            regular: Font::parse(REGULAR).ok()?,
            medium: Font::parse(MEDIUM).ok()?,
            cache: HashMap::new(),
        })
    }

    fn face(&self, weight: Weight) -> &Font {
        match weight {
            Weight::Regular => &self.regular,
            Weight::Medium => &self.medium,
        }
    }

    /// Pixels per font unit at `px`.
    fn ratio(&self, weight: Weight, px: f32) -> f32 {
        px / f32::from(self.face(weight).units_per_em().max(1))
    }

    /// Glyph ids and advances (px) of `s`; a character the font lacks shows
    /// as its missing-glyph box.
    fn run<'a>(
        &'a self,
        s: &'a str,
        weight: Weight,
        px: f32,
    ) -> impl Iterator<Item = (u16, f32)> + 'a {
        let face = self.face(weight);
        let ratio = self.ratio(weight, px);
        s.chars().map(move |c| {
            let id = face.glyph_index(c).unwrap_or(0);
            (id, f32::from(face.advance_width(id)) * ratio)
        })
    }

    /// Width of `s` in px.
    pub(crate) fn width(&self, s: &str, weight: Weight, px: f32) -> f32 {
        self.run(s, weight, px).map(|(_, a)| a).sum()
    }

    /// Px from a line's middle down to its baseline, so the capitals sit
    /// centred on it: half the cap height.
    pub(crate) fn centre_drop(&self, px: f32) -> f32 {
        let face = &self.regular;
        let cap = face
            .glyph_index('H')
            .and_then(|id| face.glyph_outline(id))
            .map_or(0.7 * f32::from(face.units_per_em()), |o| o.max_y as f32);
        cap * self.ratio(Weight::Regular, px) / 2.0
    }

    /// `s` cut to fit `max` px, ending in "…" when it was cut.
    pub(crate) fn fit(&self, s: &str, weight: Weight, px: f32, max: f32) -> String {
        if self.width(s, weight, px) <= max {
            return s.to_owned();
        }
        let room = max - self.width("…", weight, px);
        let mut used = 0.0;
        let mut out = String::new();
        for (c, (_, advance)) in s.chars().zip(self.run(s, weight, px)) {
            if used + advance > room {
                break;
            }
            used += advance;
            out.push(c);
        }
        out.push('…');
        out
    }

    /// `s`'s glyphs, each placed at its pen position from 0.
    pub(crate) fn layout(&mut self, s: &str, weight: Weight, px: f32) -> Vec<Placed> {
        let ratio = self.ratio(weight, px);
        let size = (px * 64.0).round() as u32;
        let run: Vec<(u16, f32)> = self.run(s, weight, px).collect();
        let mut pen = 0.0f32;
        let mut out = Vec::with_capacity(run.len());
        for (id, advance) in run {
            let face = match weight {
                Weight::Regular => &self.regular,
                Weight::Medium => &self.medium,
            };
            let glyph = self
                .cache
                .entry((weight, id, size))
                .or_insert_with(|| Rc::new(rasterize_id(face, ratio, id)))
                .clone();
            out.push(Placed {
                x: pen.round() as i32,
                glyph,
            });
            pen += advance;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{UiText, Weight};

    fn text() -> UiText {
        UiText::embedded().expect("the embedded faces parse")
    }

    #[test]
    fn the_embedded_faces_are_proportional() {
        let t = text();
        let (i, m) = (
            t.width("i", Weight::Regular, 13.0),
            t.width("m", Weight::Regular, 13.0),
        );
        assert!(m > 2.0 * i, "m {m} is much wider than i {i}");
    }

    #[test]
    fn medium_is_wider_than_regular() {
        let t = text();
        let s = "rusty_mill";
        assert!(t.width(s, Weight::Medium, 13.0) > t.width(s, Weight::Regular, 13.0));
    }

    #[test]
    fn width_scales_with_size() {
        let t = text();
        let (a, b) = (
            t.width("build", Weight::Regular, 13.0),
            t.width("build", Weight::Regular, 26.0),
        );
        assert!((b - 2.0 * a).abs() < 0.01);
    }

    #[test]
    fn fit_keeps_what_fits_and_cuts_the_rest() {
        let t = text();
        assert_eq!(t.fit("build", Weight::Regular, 13.0, 500.0), "build");
        let cut = t.fit("a rather long window name", Weight::Regular, 13.0, 60.0);
        assert!(cut.ends_with('…'), "{cut}");
        assert!(t.width(&cut, Weight::Regular, 13.0) <= 60.0);
    }

    #[test]
    fn layout_places_glyphs_left_to_right_with_ink() {
        let mut t = text();
        let placed = t.layout("Ab", Weight::Medium, 13.0);
        assert_eq!(placed.len(), 2);
        assert!(placed[1].x > placed[0].x);
        assert!(
            placed
                .iter()
                .all(|p| p.glyph.coverage.iter().any(|&c| c > 0))
        );
    }

    #[test]
    fn capitals_centre_about_half_their_height_below_the_middle() {
        let drop = text().centre_drop(13.0);
        assert!((4.0..5.5).contains(&drop), "{drop}");
    }
}
