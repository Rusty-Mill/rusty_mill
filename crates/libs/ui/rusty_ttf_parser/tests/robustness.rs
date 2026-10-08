//! Malformed input must never panic, loop forever, or read out of bounds.
//!
//! Every byte of the real fixture and of several synthetic fonts is truncated
//! and corrupted, then the whole public surface is walked.

mod common;
use common::{B, cblc_cbdt, cmap_with, cmap4, cmap12, font, gsub_with_extension, sbix};

use rusty_ttf_parser::gsub::SubstitutionSubtable as S;
use rusty_ttf_parser::opentype_layout::{ChainedContextLookup, ContextLookup};
use rusty_ttf_parser::{Face, GlyphId};

/// Touch everything reachable from a face. Returns a checksum so the work
/// cannot be optimised away.
fn walk(data: &[u8]) -> u64 {
    let mut sum = 0u64;
    let Ok(face) = Face::parse(data, 0) else {
        return sum;
    };
    sum += u64::from(face.units_per_em()) + u64::from(face.number_of_glyphs());
    for c in ['A', 'f', '=', '\u{FFFF}', '\u{1F600}', '\u{10FFFF}'] {
        sum += face.glyph_index(c).map_or(0, |g| u64::from(g.0));
    }
    for g in [0u16, 1, 5, 40, 0xFFFF] {
        for ppem in [1u16, 20, 109, 0xFFFF] {
            sum += face
                .glyph_raster_image(GlyphId(g), ppem)
                .map_or(0, |i| i.data.len() as u64);
        }
    }
    let Some(gsub) = face.tables().gsub else {
        return sum;
    };
    for feature in gsub.features {
        sum += feature.lookup_indices.iter().map(u64::from).sum::<u64>();
    }
    for li in 0..gsub.lookups.len() {
        let Some(lookup) = gsub.lookups.get(li) else {
            continue;
        };
        for subtable in lookup.subtables() {
            sum += match subtable {
                S::Single(s) => u64::from(s.coverage().get(GlyphId(2)).unwrap_or(0)),
                S::Ligature(l) => {
                    let mut n = 0;
                    for i in 0..l.ligature_sets.len() {
                        for j in 0..l.ligature_sets.get(i).map_or(0, |s| s.len()) {
                            n += l
                                .ligature_sets
                                .get(i)
                                .and_then(|s| s.get(j))
                                .map_or(0, |l| {
                                    l.components.iter().map(|g| u64::from(g.0)).sum::<u64>()
                                });
                        }
                    }
                    n
                }
                S::Context(ContextLookup::Format1 { sets, .. })
                | S::Context(ContextLookup::Format2 { sets, .. }) => (0..sets.len())
                    .filter_map(|i| sets.get(i))
                    .map(|s| u64::from(s.len()))
                    .sum(),
                S::Context(ContextLookup::Format3 { coverages, .. }) => u64::from(coverages.len()),
                S::ChainContext(ChainedContextLookup::Format1 { sets, .. })
                | S::ChainContext(ChainedContextLookup::Format2 { sets, .. }) => (0..sets.len())
                    .filter_map(|i| sets.get(i))
                    .map(|s| u64::from(s.len()))
                    .sum(),
                S::ChainContext(ChainedContextLookup::Format3 {
                    input_coverages, ..
                }) => u64::from(input_coverages.len()),
                S::Unsupported(k) => u64::from(k),
                _ => 0,
            };
        }
    }
    sum
}

/// All corpora worth mutating.
fn corpora() -> Vec<Vec<u8>> {
    let (cblc, cbdt) = cblc_cbdt();
    vec![
        include_bytes!("data/ligtest.ttf").to_vec(),
        font(
            1000,
            200,
            &[(b"cmap", cmap_with(&[(3, 1, cmap4()), (3, 10, cmap12())]))],
        ),
        font(1000, 50, &[(b"GSUB", gsub_with_extension())]),
        font(1000, 3, &[(b"sbix", sbix())]),
        font(1000, 8, &[(b"CBLC", cblc), (b"CBDT", cbdt)]),
    ]
}

#[test]
fn every_truncation_is_handled() {
    for font in corpora() {
        let baseline = walk(&font);
        assert!(baseline > 0, "the unmodified font should yield something");
        for len in 0..font.len() {
            walk(&font[..len]);
        }
    }
}

#[test]
fn every_single_byte_corruption_is_handled() {
    for font in corpora() {
        let mut bytes = font.clone();
        for i in 0..bytes.len() {
            let original = bytes[i];
            for replacement in [0x00, 0x01, 0x7F, 0x80, 0xFF, original ^ 0x55] {
                bytes[i] = replacement;
                walk(&bytes);
            }
            bytes[i] = original;
        }
    }
}

#[test]
fn pseudo_random_multi_byte_corruption_is_handled() {
    // xorshift: deterministic, no dependency.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for font in corpora() {
        for _ in 0..3000 {
            let mut bytes = font.clone();
            for _ in 0..(1 + next() % 8) {
                let at = (next() % bytes.len() as u64) as usize;
                bytes[at] = next() as u8;
            }
            walk(&bytes);
        }
    }
}

#[test]
fn absurd_counts_do_not_allocate_or_hang() {
    // A GSUB claiming 65535 features/lookups in a few bytes of data.
    let gsub = B::new()
        .u16(1)
        .u16(0)
        .u16(0)
        .u16(10)
        .u16(14)
        .u16(0xFFFF)
        .u16(0xFFFF)
        .0;
    let f = font(1000, 10, &[(b"GSUB", gsub)]);
    walk(&f);
}

#[test]
fn billions_of_declared_strikes_do_not_spin() {
    // Each of these claims 0xFFFFFFFF records in a table a few bytes long; an
    // unclamped loop would run for billions of iterations.
    let sbix = B::new().u16(1).u16(0).u32(u32::MAX).u32(12).u32(0).0;
    walk(&font(1000, 4, &[(b"sbix", sbix)]));

    let cblc_sizes = B::new().u16(3).u16(0).u32(u32::MAX).zeros(64).0;
    walk(&font(
        1000,
        4,
        &[(b"CBLC", cblc_sizes), (b"CBDT", vec![0, 3, 0, 0])],
    ));

    // One real-looking strike covering all glyphs, then an absurd subtable count.
    let size = B::new()
        .u32(56)
        .u32(8)
        .u32(u32::MAX)
        .u32(0)
        .zeros(24)
        .u16(0)
        .u16(0xFFFF)
        .u8(109)
        .u8(109)
        .u8(32)
        .u8(1);
    let cblc_tables = B::new().u16(3).u16(0).u32(1).bytes(&size.0).zeros(64).0;
    walk(&font(
        1000,
        4,
        &[(b"CBLC", cblc_tables), (b"CBDT", vec![0, 3, 0, 0])],
    ));
}
