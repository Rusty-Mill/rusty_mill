//! Cases from review of #551: arithmetic on font-declared counts and offsets
//! must hold on 32-bit targets (run with `--target i686-unknown-linux-musl`),
//! and `sbix` `dupe` chains must resolve like `ttf-parser`'s bounded chains.

mod common;
use common::{B, cmap_with, font};

use rusty_ttf_parser::{Face, FaceParsingError, GlyphId};

/// A minimal valid 1x1 PNG (signature, IHDR, IDAT, IEND).
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x56, 0xC7, 0x2F, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn cmap_format12_with_an_absurd_group_count_is_unmapped_not_a_panic() {
    // nGroups = 715827880: its binary search first probes group 357913940,
    // whose byte offset is 16 + 357913940 * 12 = 2^32, overflowing a 32-bit usize.
    let table = B::new().u16(12).u16(0).u32(16).u32(0).u32(715_827_880).0;
    let f = font(1000, 10, &[(b"cmap", cmap_with(&[(3, 10, table)]))]);
    let face = Face::parse(&f, 0).unwrap();
    for c in ['A', '\u{FFFF}', '\u{1F600}', '\u{10FFFF}'] {
        assert_eq!(face.glyph_index(c), None);
    }
}

#[test]
fn cmap_format12_group_count_larger_than_the_table_is_clamped() {
    // Two real groups, but the header claims 0xFFFFFFFF: only groups that fit in
    // the table exist, so both map and nothing past them is read.
    let mut table = B::new()
        .u16(12)
        .u16(0)
        .u32(28)
        .u32(0)
        .u32(u32::MAX)
        .u32(0x41)
        .u32(0x41)
        .u32(7)
        .0;
    table.extend(B::new().u32(0x100).u32(0x200).u32(9).0);
    let f = font(1000, 20, &[(b"cmap", cmap_with(&[(3, 10, table)]))]);
    let face = Face::parse(&f, 0).unwrap();
    assert_eq!(face.glyph_index('A'), Some(GlyphId(7)));
    assert_eq!(face.glyph_index('\u{150}'), Some(GlyphId(9 + 0x50)));
    assert_eq!(
        face.glyph_index('\u{300}'),
        None,
        "past the last real group"
    );
}

#[test]
fn ttc_face_index_arithmetic_cannot_overflow() {
    // 'ttcf', version, numFonts = u32::MAX; a face index whose *4 overflows 32 bits.
    let ttc = B::new()
        .bytes(b"ttcf")
        .u32(0x0001_0000)
        .u32(u32::MAX)
        .zeros(16)
        .0;
    for index in [0x4000_0000, 0x8000_0000, u32::MAX - 1] {
        let err = Face::parse(&ttc, index).unwrap_err();
        assert!(
            matches!(
                err,
                FaceParsingError::MalformedFont | FaceParsingError::FaceIndexOutOfBounds
            ),
            "{err:?}"
        );
    }
}

#[test]
fn cblc_offsets_near_the_top_of_the_address_space_are_rejected() {
    // indexSubTableArrayOffset and a subtable offset of 0xFFFF_FFFx must not
    // overflow when small constants are added.
    for array in [0xFFFF_FFF0u32, 0xFFFF_FFFC, 0xFFFF_FFFE, u32::MAX] {
        let size = B::new()
            .u32(array)
            .u32(8)
            .u32(1)
            .u32(0)
            .zeros(24)
            .u16(0)
            .u16(0xFFFF)
            .u8(109)
            .u8(109)
            .u8(32)
            .u8(1);
        let cblc = B::new().u16(3).u16(0).u32(1).bytes(&size.0).zeros(32).0;
        let f = font(1000, 8, &[(b"CBLC", cblc), (b"CBDT", vec![0, 3, 0, 0])]);
        let face = Face::parse(&f, 0).unwrap();
        assert!(face.glyph_raster_image(GlyphId(3), 64).is_none());
    }
}

// ---------- sbix dupe chains ----------

/// An sbix with one strike (ppem 20). `kinds[g]` is `Some(target)` for a `dupe`
/// of glyph `target`, or `None` for a real PNG.
fn sbix_with(kinds: &[Option<u16>]) -> Vec<u8> {
    let header = 4 + 4 * (kinds.len() + 1);
    let mut records = Vec::new();
    let mut offsets = vec![header as u32];
    for kind in kinds {
        let record = match kind {
            None => B::new().i16(0).i16(0).bytes(b"png ").bytes(PNG).0,
            Some(target) => B::new().i16(0).i16(0).bytes(b"dupe").u16(*target).0,
        };
        records.extend(&record);
        offsets.push((header + records.len()) as u32);
    }
    let mut strike = B::new().u16(20).u16(72);
    for offset in offsets {
        strike = strike.u32(offset);
    }
    let strike = strike.bytes(&records).0;
    B::new().u16(1).u16(1).u32(1).u32(12).bytes(&strike).0
}

fn image(kinds: &[Option<u16>], glyph: u16) -> Option<Vec<u8>> {
    let f = font(1000, kinds.len() as u16, &[(b"sbix", sbix_with(kinds))]);
    let face = Face::parse(&f, 0).unwrap();
    face.glyph_raster_image(GlyphId(glyph), 20)
        .map(|i| i.data.to_vec())
}

#[test]
fn sbix_follows_a_two_hop_dupe_chain_to_a_real_png() {
    // glyph 2 -> dupe 1 -> dupe 0 -> PNG
    let kinds = [None, Some(0), Some(1)];
    assert_eq!(image(&kinds, 2).as_deref(), Some(PNG));
    assert_eq!(image(&kinds, 1).as_deref(), Some(PNG));
    assert_eq!(image(&kinds, 0).as_deref(), Some(PNG));
}

#[test]
fn sbix_dupe_chains_are_bounded_like_ttf_parser() {
    // ttf-parser bails at depth 10. A chain of 9 hops resolves; a longer one does not.
    let chain = |hops: usize| -> Vec<Option<u16>> {
        let mut kinds = vec![None];
        kinds.extend((0..hops).map(|i| Some(i as u16)));
        kinds
    };
    let nine = chain(9);
    assert_eq!(image(&nine, 9).as_deref(), Some(PNG));
    let twenty = chain(20);
    assert!(image(&twenty, 20).is_none());
}

#[test]
fn sbix_dupe_cycles_and_invalid_targets_yield_none() {
    assert!(image(&[Some(1), Some(0)], 0).is_none(), "two-glyph cycle");
    assert!(image(&[Some(0)], 0).is_none(), "self-cycle");
    assert!(
        image(&[None, Some(99)], 1).is_none(),
        "target past the strike"
    );
}
