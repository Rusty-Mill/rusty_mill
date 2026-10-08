//! Hand-built fonts covering formats the real fixture does not.

mod common;
use common::{
    B, PNG_A, PNG_B, cblc_cbdt, cmap_with, cmap4, cmap12, font, gsub_with_extension, sbix,
};

use rusty_ttf_parser::gsub::{SingleSubstitution, SubstitutionSubtable};
use rusty_ttf_parser::opentype_layout::{ClassDefinition, Coverage};
use rusty_ttf_parser::{Face, FromSlice, GlyphId, RasterImageFormat, Tag};

// ---------- cmap ----------

#[test]
fn cmap_format4_delta_array_and_gaps() {
    let f = font(1000, 30, &[(b"cmap", cmap_with(&[(3, 1, cmap4())]))]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!(face.glyph_index('A'), Some(GlyphId(10)));
    assert_eq!(face.glyph_index('C'), Some(GlyphId(12)));
    assert_eq!(face.glyph_index('a'), Some(GlyphId(20)));
    assert_eq!(
        face.glyph_index('b'),
        None,
        "glyph-array entry 0 is unmapped"
    );
    assert_eq!(face.glyph_index('D'), None, "between segments");
    assert_eq!(face.glyph_index('\u{1F600}'), None, "outside the BMP");
}

#[test]
fn cmap_format12_reaches_beyond_the_bmp() {
    let f = font(1000, 200, &[(b"cmap", cmap_with(&[(3, 10, cmap12())]))]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!(face.glyph_index('A'), Some(GlyphId(7)));
    assert_eq!(face.glyph_index('\u{1F600}'), Some(GlyphId(100)));
    assert_eq!(face.glyph_index('\u{1F602}'), Some(GlyphId(102)));
    assert_eq!(face.glyph_index('\u{1F603}'), None);
}

#[test]
fn cmap_format12_does_not_read_past_the_declared_groups() {
    // Code points above the last group must be unmapped even when bytes that
    // look like a valid group follow the table (found by differential testing
    // against wqy-zenhei.ttc, where the next table's bytes were read as a group).
    let mut table = cmap12();
    table.extend(B::new().u32(0x2F000).u32(0x2FFFF).u32(500).0);
    let f = font(1000, 600, &[(b"cmap", cmap_with(&[(3, 10, table)]))]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!(face.glyph_index('\u{2F800}'), None);
    assert_eq!(
        face.glyph_index('\u{1F602}'),
        Some(GlyphId(102)),
        "declared groups still work"
    );
}

#[test]
fn cmap_prefers_format12_over_format4() {
    let both = cmap_with(&[(3, 1, cmap4()), (3, 10, cmap12())]);
    let f = font(1000, 200, &[(b"cmap", both)]);
    let face = Face::parse(&f, 0).unwrap();
    // 'A' is 7 in the format-12 table and 10 in the format-4 table.
    assert_eq!(face.glyph_index('A'), Some(GlyphId(7)));
}

#[test]
fn cmap_format6_dense_range() {
    let six = B::new()
        .u16(6)
        .u16(0)
        .u16(0)
        .u16(0x30)
        .u16(3)
        .u16(5)
        .u16(6)
        .u16(0)
        .0;
    let f = font(1000, 10, &[(b"cmap", cmap_with(&[(0, 3, six)]))]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!(face.glyph_index('0'), Some(GlyphId(5)));
    assert_eq!(face.glyph_index('1'), Some(GlyphId(6)));
    assert_eq!(face.glyph_index('2'), None, "glyph 0 means unmapped");
    assert_eq!(face.glyph_index('3'), None);
}

#[test]
fn face_without_cmap_maps_nothing() {
    let f = font(2048, 3, &[]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!((face.units_per_em(), face.number_of_glyphs()), (2048, 3));
    assert_eq!(face.glyph_index('A'), None);
    assert!(face.tables().gsub.is_none());
}

// ---------- coverage and classes ----------

#[test]
fn coverage_formats_1_and_2() {
    let f1 = B::new().u16(1).u16(3).u16(4).u16(9).u16(20).0;
    let c1 = Coverage::parse(&f1).unwrap();
    assert_eq!(c1.get(GlyphId(4)), Some(0));
    assert_eq!(c1.get(GlyphId(20)), Some(2));
    assert_eq!(c1.get(GlyphId(5)), None);
    assert!(c1.contains(GlyphId(9)));

    // 10..=12 start at index 0; 30..=31 start at index 3.
    let f2 = B::new()
        .u16(2)
        .u16(2)
        .u16(10)
        .u16(12)
        .u16(0)
        .u16(30)
        .u16(31)
        .u16(3)
        .0;
    let c2 = Coverage::parse(&f2).unwrap();
    assert_eq!(c2.get(GlyphId(11)), Some(1));
    assert_eq!(c2.get(GlyphId(31)), Some(4));
    assert_eq!(c2.get(GlyphId(13)), None);
    assert_eq!(c2.get(GlyphId(9)), None);
}

#[test]
fn class_definition_formats_1_and_2_default_to_class_zero() {
    let f1 = B::new().u16(1).u16(10).u16(3).u16(1).u16(2).u16(1).0;
    let d1 = ClassDefinition::parse(&f1).unwrap();
    assert_eq!(
        [9, 10, 11, 12, 13].map(|g| d1.get(GlyphId(g))),
        [0, 1, 2, 1, 0]
    );

    let f2 = B::new()
        .u16(2)
        .u16(2)
        .u16(5)
        .u16(7)
        .u16(4)
        .u16(20)
        .u16(20)
        .u16(9)
        .0;
    let d2 = ClassDefinition::parse(&f2).unwrap();
    assert_eq!(
        [4, 5, 7, 8, 20, 21].map(|g| d2.get(GlyphId(g))),
        [0, 4, 4, 0, 9, 0]
    );
}

#[test]
fn unknown_formats_are_rejected_not_guessed() {
    assert!(Coverage::parse(&B::new().u16(3).u16(0).0).is_none());
    assert!(ClassDefinition::parse(&B::new().u16(0).0).is_none());
}

// ---------- GSUB, including an extension lookup ----------

#[test]
fn extension_lookups_resolve_to_their_real_subtable() {
    let f = font(1000, 50, &[(b"GSUB", gsub_with_extension())]);
    let gsub = Face::parse(&f, 0).unwrap().tables().gsub.expect("GSUB");
    let feature = gsub.features.into_iter().next().unwrap();
    assert_eq!(feature.tag, Tag::from_bytes(b"liga"));

    let lookup = gsub
        .lookups
        .get(feature.lookup_indices.get(0).unwrap())
        .unwrap();
    let mut subtables = lookup.subtables();
    let Some(SubstitutionSubtable::Single(single)) = subtables.next() else {
        panic!("extension should resolve to a single substitution");
    };
    assert!(subtables.next().is_none());
    let SingleSubstitution::Format2 { substitutes, .. } = single else {
        panic!("format 2")
    };
    assert_eq!(single.coverage().get(GlyphId(3)), Some(1));
    assert_eq!(substitutes.get(1), Some(GlyphId(41)));
}

#[test]
fn unsupported_lookup_types_are_reported_not_dropped() {
    let mut gsub = gsub_with_extension();
    // Lookup type lives right after the feature list; patch 7 -> 2 (multiple).
    let lookup_at = 10 + 2 + 6 + 6 + 4; // header + feature list + lookup list header
    assert_eq!(&gsub[lookup_at..lookup_at + 2], &[0, 7]);
    gsub[lookup_at + 1] = 2;
    let f = font(1000, 50, &[(b"GSUB", gsub)]);
    let gsub = Face::parse(&f, 0).unwrap().tables().gsub.unwrap();
    let first = gsub.lookups.get(0).unwrap().subtables().next().unwrap();
    assert!(matches!(first, SubstitutionSubtable::Unsupported(2)));
}

// ---------- colour bitmaps ----------

#[test]
fn sbix_picks_the_nearest_larger_strike_and_follows_dupe() {
    let f = font(1000, 3, &[(b"sbix", sbix())]);
    let face = Face::parse(&f, 0).unwrap();
    let at = |gid: u16, ppem: u16| face.glyph_raster_image(GlyphId(gid), ppem);

    let img = at(1, 30).expect("30 ppem -> the 40 strike");
    assert_eq!(
        (img.pixels_per_em, img.data, img.format),
        (40, PNG_B, RasterImageFormat::PNG)
    );
    assert_eq!((img.x, img.y), (1, 2));
    assert_eq!(at(1, 20).unwrap().data, PNG_A, "exact match");
    assert_eq!(
        at(1, 200).unwrap().data,
        PNG_B,
        "no larger strike: use the largest"
    );
    assert_eq!(
        at(1, 5).unwrap().data,
        PNG_A,
        "smaller request: use the smallest"
    );
    assert_eq!(at(2, 20).unwrap().data, PNG_A, "dupe resolves to glyph 1");
    assert!(at(0, 20).is_none(), "empty glyph");
    assert!(at(3, 20).is_none(), "glyph past the strike");
}

#[test]
fn cbdt_png_strike_with_small_metrics() {
    let (cblc, cbdt) = cblc_cbdt();
    let f = font(1000, 8, &[(b"CBLC", cblc), (b"CBDT", cbdt)]);
    let face = Face::parse(&f, 0).unwrap();

    let img = face.glyph_raster_image(GlyphId(3), 64).expect("glyph 3");
    assert_eq!(img.data, b"\x89PNG-CBDT");
    assert_eq!(img.format, RasterImageFormat::PNG);
    assert_eq!((img.width, img.height, img.pixels_per_em), (128, 136, 109));
    assert_eq!((img.x, img.y), (-2, 101 - 136));
    assert!(
        face.glyph_raster_image(GlyphId(4), 64).is_none(),
        "no bitmap for glyph 4"
    );
    assert!(
        face.glyph_raster_image(GlyphId(5), 64).is_none(),
        "outside the strike"
    );
}
