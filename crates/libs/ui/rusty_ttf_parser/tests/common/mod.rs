//! Byte builders for synthetic fonts. Test-only.
#![allow(dead_code)]

/// A growable big-endian byte buffer.
#[derive(Default, Clone)]
pub struct B(pub Vec<u8>);

impl B {
    pub fn new() -> Self {
        B(Vec::new())
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    pub fn i16(self, v: i16) -> Self {
        self.u16(v as u16)
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    pub fn bytes(mut self, v: &[u8]) -> Self {
        self.0.extend(v);
        self
    }
    pub fn zeros(mut self, n: usize) -> Self {
        self.0.extend(std::iter::repeat_n(0, n));
        self
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// An sfnt with `head`, `maxp` and the given extra tables.
pub fn font(units_per_em: u16, glyphs: u16, extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let head = B::new().zeros(18).u16(units_per_em).zeros(34).0;
    let maxp = B::new().u32(0x0000_5000).u16(glyphs).0;
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"head", head), (b"maxp", maxp)];
    tables.extend(extra.iter().map(|(t, d)| (*t, d.clone())));
    tables.sort_by_key(|(tag, _)| **tag);

    let dir = 12 + 16 * tables.len();
    let mut out = B::new().u32(0x0001_0000).u16(tables.len() as u16).zeros(6);
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out = out
            .bytes(*tag)
            .u32(0)
            .u32((dir + body.len()) as u32)
            .u32(data.len() as u32);
        body.extend(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.bytes(&body).0
}

// ---------- table builders shared by the test files ----------

/// Format 4: 'A'..='C' -> glyph = code + delta; 'a'..='b' via the glyph array.
pub fn cmap4() -> Vec<u8> {
    // 3 segments: A..C (delta), a..b (glyph array), 0xFFFF terminator.
    let segs: u16 = 3;
    let header = B::new()
        .u16(4)
        .u16(0)
        .u16(0)
        .u16(segs * 2)
        .u16(0)
        .u16(0)
        .u16(0);
    let ends = B::new().u16(0x43).u16(0x62).u16(0xFFFF);
    let starts = B::new().u16(0x41).u16(0x61).u16(0xFFFF);
    // 'A'(0x41) -> 10 => delta = 10 - 0x41.
    let deltas = B::new().u16(10u16.wrapping_sub(0x41)).u16(0).u16(1);
    // Segment 1 reads the glyph array: offset from its own idRangeOffset slot.
    // Slots: [0, off1, 0]; the array follows the three slots, so from slot 1
    // it is (3 - 1) * 2 = 4 bytes ahead.
    let range_offsets = B::new().u16(0).u16(4).u16(0);
    let glyphs = B::new().u16(20).u16(0); // 'a' -> 20, 'b' -> unmapped
    let body = header
        .bytes(&ends.0)
        .u16(0)
        .bytes(&starts.0)
        .bytes(&deltas.0)
        .bytes(&range_offsets.0)
        .bytes(&glyphs.0);
    let len = body.len() as u16;
    let mut out = body.0;
    out[2..4].copy_from_slice(&len.to_be_bytes());
    out
}

pub fn cmap_with(records: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = B::new().u16(0).u16(records.len() as u16);
    let mut offset = 4 + 8 * records.len();
    for (platform, encoding, table) in records {
        out = out.u16(*platform).u16(*encoding).u32(offset as u32);
        offset += table.len();
    }
    for (_, _, table) in records {
        out = out.bytes(table);
    }
    out.0
}

pub fn cmap12() -> Vec<u8> {
    // U+1F600..=U+1F602 -> glyphs 100.., and 'A' -> 7.
    B::new()
        .u16(12)
        .u16(0)
        .u32(16 + 24)
        .u32(0)
        .u32(2)
        .u32(0x41)
        .u32(0x41)
        .u32(7)
        .u32(0x1F600)
        .u32(0x1F602)
        .u32(100)
        .0
}

/// One feature `liga` -> lookup 0, an extension (type 7) wrapping a format-2
/// single substitution {2 -> 40, 3 -> 41}.
pub fn gsub_with_extension() -> Vec<u8> {
    let single = B::new()
        .u16(2) // format
        .u16(10) // coverage offset
        .u16(2) // glyph count
        .u16(40)
        .u16(41)
        .u16(1) // coverage format 1
        .u16(2)
        .u16(2)
        .u16(3);
    let extension = B::new().u16(1).u16(1).u32(8).bytes(&single.0);
    let lookup = B::new().u16(7).u16(0).u16(1).u16(8).bytes(&extension.0);
    let lookup_list = B::new().u16(1).u16(4).bytes(&lookup.0);
    let feature = B::new().u16(0).u16(1).u16(0);
    let feature_list = B::new().u16(1).bytes(b"liga").u16(8).bytes(&feature.0);

    let header = 10;
    let features_at = header;
    let lookups_at = features_at + feature_list.len();
    B::new()
        .u16(1)
        .u16(0)
        .u16(0)
        .u16(features_at as u16)
        .u16(lookups_at as u16)
        .bytes(&feature_list.0)
        .bytes(&lookup_list.0)
        .0
}

pub const PNG_A: &[u8] = b"\x89PNG-A";
pub const PNG_B: &[u8] = b"\x89PNG-B";

/// sbix with strikes at 20 and 40 ppem; glyph 1 has a PNG, glyph 2 is `dupe`
/// of glyph 1, glyph 0 is empty. The 40 ppem strike's glyph 1 differs (PNG_B).
pub fn sbix() -> Vec<u8> {
    fn strike(ppem: u16, png: &[u8]) -> Vec<u8> {
        // 3 glyphs -> 4 offsets, relative to the strike start.
        let header = 4 + 4 * 4;
        let g1 = B::new().i16(1).i16(2).bytes(b"png ").bytes(png);
        let g2 = B::new().i16(0).i16(0).bytes(b"dupe").u16(1);
        let o0 = header;
        let o1 = o0; // glyph 0: empty
        let o2 = o1 + g1.len();
        let o3 = o2 + g2.len();
        B::new()
            .u16(ppem)
            .u16(72)
            .u32(o0 as u32)
            .u32(o1 as u32)
            .u32(o2 as u32)
            .u32(o3 as u32)
            .bytes(&g1.0)
            .bytes(&g2.0)
            .0
    }
    let (s20, s40) = (strike(20, PNG_A), strike(40, PNG_B));
    let head = 8 + 8;
    B::new()
        .u16(1)
        .u16(1)
        .u32(2)
        .u32(head as u32)
        .u32((head + s20.len()) as u32)
        .bytes(&s20)
        .bytes(&s40)
        .0
}

/// CBLC/CBDT: one 109 ppem strike covering glyphs 3..=4, index format 1,
/// image format 17. Glyph 4 has no bitmap (equal offsets).
pub fn cblc_cbdt() -> (Vec<u8>, Vec<u8>) {
    let png = b"\x89PNG-CBDT";
    // CBDT record: height, width, bearingX, bearingY, advance, dataLen, data.
    let record = B::new()
        .u8(136)
        .u8(128)
        .u8(0xFE)
        .u8(101)
        .u8(136)
        .u32(png.len() as u32)
        .bytes(png);
    let cbdt = B::new().u16(3).u16(0).bytes(&record.0).0;

    let index_sub = B::new()
        .u16(1)
        .u16(17)
        .u32(4)
        .u32(0)
        .u32(record.len() as u32)
        .u32(record.len() as u32);
    let array = B::new().u16(3).u16(4).u32(8); // one entry; subtable follows it
    let sizes_end = 8 + 48;
    let size = B::new()
        .u32(sizes_end as u32) // indexSubTableArrayOffset
        .u32((array.len() + index_sub.len()) as u32)
        .u32(1) // numberOfIndexSubTables
        .u32(0) // colorRef
        .zeros(24) // hori + vert line metrics
        .u16(3)
        .u16(4)
        .u8(109)
        .u8(109)
        .u8(32)
        .u8(1);
    assert_eq!(size.len(), 48);
    let cblc = B::new()
        .u16(3)
        .u16(0)
        .u32(1)
        .bytes(&size.0)
        .bytes(&array.0)
        .bytes(&index_sub.0)
        .0;
    (cblc, cbdt)
}
