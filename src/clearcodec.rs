//! ClearCodec bitmap codec (MS-RDPEGFX `CODECID_CLEARCODEC`), std-only.
//!
//! Decodes `CLEARCODEC_BITMAP_STREAM` — the wire format carried as opaque
//! `bitmap_data` in [`crate::gfx::WireToSurface1Pdu`] when
//! `codec_id == `[`crate::gfx::CODECID_CLEARCODEC`] — to a flat, top-down
//! RGBA8888 pixel buffer, matching [`crate::display::Framebuffer`]'s
//! convention.
//!
//! Unlike [`crate::rfx`] and [`crate::planar`], ClearCodec is **not
//! stateless**: a glyph cache and two ring-buffer "VBar" caches (one for
//! full vertical-bar runs, one for a shorter variant) persist across
//! messages within a session and are referenced by later messages via
//! cache index. [`ClearCodecDecoder`] owns that state — construct one per
//! session (not one per call) and call [`ClearCodecDecoder::decode`] for
//! every bitmap.
//!
//! A `CLEARCODEC_BITMAP_STREAM` composites up to three independent
//! payloads onto the destination rectangle, in order: `residualData` (a
//! full-canvas run-length background fill), `bandsData` (per-column
//! vertical-bar runs, the caching unit above), and `subcodecsData`
//! (independent sub-tiles, each raw, [MS-RDPEGFX 2.2.4.2] "RLEX"
//! run-length, or NSCodec-compressed). **NSCodec sub-tiles
//! (`subcodecId == 1`) are not implemented** — MS-RDPNSC is a whole
//! separate legacy codec and real-world ClearCodec traffic favors the raw
//! and RLEX sub-tiles; `decode` returns an error rather than silently
//! misinterpreting NSCodec payloads. Optionally the whole composited
//! result is also cached as a "glyph" for later exact-replay by index.
//!
//! Decode-only, like `rfx` and `planar`: encoding (the server-side
//! direction) is not implemented.

use crate::cursor::Reader;
use crate::error::{Error, Result};

const FLAG_GLYPH_INDEX: u8 = 0x01;
const FLAG_GLYPH_HIT: u8 = 0x02;
const FLAG_CACHE_RESET: u8 = 0x04;

const GLYPH_CACHE_SIZE: usize = 4000;
const VBAR_CACHE_SIZE: usize = 32768;
const SHORT_VBAR_CACHE_SIZE: usize = 16384;
const MAX_VBAR_HEIGHT: usize = 52;

#[derive(Debug, Clone, Default)]
struct VBarEntry {
    pixels: Vec<[u8; 3]>,
}

#[derive(Debug, Clone)]
struct GlyphEntry {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 3]>,
}

/// Persistent ClearCodec decode state: the glyph cache and the two VBar
/// ring-buffer caches that later messages reference by index. Reuse the
/// same decoder across every bitmap in a session.
pub struct ClearCodecDecoder {
    seq_number: u8,
    seq_number_set: bool,
    glyph_cache: Vec<Option<GlyphEntry>>,
    vbar_storage: Vec<Option<VBarEntry>>,
    vbar_cursor: usize,
    short_vbar_storage: Vec<Option<VBarEntry>>,
    short_vbar_cursor: usize,
}

impl Default for ClearCodecDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClearCodecDecoder {
    /// Create a fresh decoder with empty caches, as at the start of a
    /// session.
    pub fn new() -> Self {
        ClearCodecDecoder {
            seq_number: 0,
            seq_number_set: false,
            glyph_cache: vec![None; GLYPH_CACHE_SIZE],
            vbar_storage: vec![None; VBAR_CACHE_SIZE],
            vbar_cursor: 0,
            short_vbar_storage: vec![None; SHORT_VBAR_CACHE_SIZE],
            short_vbar_cursor: 0,
        }
    }

    /// Decode one `CLEARCODEC_BITMAP_STREAM` to a `width * height * 4`-byte
    /// top-down RGBA8888 pixel buffer, updating this decoder's glyph/VBar
    /// caches as a side effect.
    pub fn decode(&mut self, data: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
        let mut r = Reader::new(data);
        let flags = r.read_u8()?;
        let seq_number = r.read_u8()?;

        if !self.seq_number_set {
            self.seq_number = seq_number;
            self.seq_number_set = true;
        }
        if seq_number != self.seq_number {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC_BITMAP_STREAM seqNumber",
                value: format!("{seq_number} (expected {})", self.seq_number),
            });
        }
        self.seq_number = seq_number.wrapping_add(1);

        if flags & FLAG_CACHE_RESET != 0 {
            self.vbar_cursor = 0;
            self.short_vbar_cursor = 0;
        }

        if flags & FLAG_GLYPH_HIT != 0 && flags & FLAG_GLYPH_INDEX == 0 {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC_BITMAP_STREAM flags",
                value: format!("0x{flags:02X} (GLYPH_HIT without GLYPH_INDEX)"),
            });
        }

        let w = width as usize;
        let h = height as usize;

        let mut record_glyph_index = None;
        if flags & FLAG_GLYPH_INDEX != 0 {
            let idx = r.read_u16_le()? as usize;
            if idx >= GLYPH_CACHE_SIZE {
                return Err(Error::InvalidValue {
                    field: "CLEARCODEC glyphIndex",
                    value: idx.to_string(),
                });
            }
            if flags & FLAG_GLYPH_HIT != 0 {
                let entry = self.glyph_cache[idx].as_ref().ok_or(Error::InvalidValue {
                    field: "CLEARCODEC glyphIndex",
                    value: format!("{idx} (cache miss)"),
                })?;
                if entry.width != width as u32 || entry.height != height as u32 {
                    return Err(Error::InvalidValue {
                        field: "CLEARCODEC glyphIndex",
                        value: format!("{idx} (cached size does not match requested size)"),
                    });
                }
                return Ok(rgb_to_rgba(&entry.pixels));
            }
            record_glyph_index = Some(idx);
        }

        let residual_len = r.read_u32_le()? as usize;
        let bands_len = r.read_u32_le()? as usize;
        let subcodec_len = r.read_u32_le()? as usize;

        let mut canvas = vec![0u8; w * h * 4];
        for i in 0..w * h {
            canvas[i * 4 + 3] = 0xFF;
        }

        if residual_len > 0 {
            let chunk = r.read_bytes(residual_len)?;
            decode_residual(chunk, w, h, &mut canvas)?;
        }
        if bands_len > 0 {
            let chunk = r.read_bytes(bands_len)?;
            self.decode_bands(chunk, w, h, &mut canvas)?;
        }
        if subcodec_len > 0 {
            let chunk = r.read_bytes(subcodec_len)?;
            decode_subcodecs(chunk, w, h, &mut canvas)?;
        }

        if let Some(idx) = record_glyph_index {
            let mut pixels = Vec::with_capacity(w * h);
            for i in 0..w * h {
                pixels.push([canvas[i * 4], canvas[i * 4 + 1], canvas[i * 4 + 2]]);
            }
            self.glyph_cache[idx] = Some(GlyphEntry {
                width: width as u32,
                height: height as u32,
                pixels,
            });
        }

        Ok(canvas)
    }

    /// Decode `bandsData`: a list of column bands, each a run of VBar
    /// entries either read fresh or replayed from the VBar/short-VBar
    /// caches, blitted directly onto `canvas`.
    fn decode_bands(&mut self, data: &[u8], w: usize, h: usize, canvas: &mut [u8]) -> Result<()> {
        let mut r = Reader::new(data);
        while !r.is_empty() {
            let x_start = r.read_u16_le()?;
            let x_end = r.read_u16_le()?;
            let y_start = r.read_u16_le()?;
            let y_end = r.read_u16_le()?;
            let cb = r.read_u8()?;
            let cg = r.read_u8()?;
            let cr = r.read_u8()?;
            let color_bkg = [cr, cg, cb];

            if x_end < x_start || y_end < y_start {
                return Err(Error::InvalidValue {
                    field: "CLEARCODEC band bounds",
                    value: format!("[{x_start},{x_end}]x[{y_start},{y_end}]"),
                });
            }
            let vbar_count = (x_end - x_start) as usize + 1;
            let vbar_height = (y_end - y_start) as usize + 1;
            if vbar_height > MAX_VBAR_HEIGHT {
                return Err(Error::InvalidValue {
                    field: "CLEARCODEC vBarHeight",
                    value: vbar_height.to_string(),
                });
            }

            for i in 0..vbar_count {
                let vbar_header = r.read_u16_le()?;
                let pixels: Vec<[u8; 3]> = if vbar_header & 0xC000 == 0x4000 {
                    // SHORT_VBAR_CACHE_HIT.
                    let idx = (vbar_header & 0x3FFF) as usize;
                    let vbar_yon = r.read_u8()? as usize;
                    let short =
                        self.short_vbar_storage[idx]
                            .as_ref()
                            .ok_or(Error::InvalidValue {
                                field: "CLEARCODEC short VBar index",
                                value: format!("{idx} (cache miss)"),
                            })?;
                    build_vbar(color_bkg, vbar_yon, &short.pixels, vbar_height)
                } else if vbar_header & 0xC000 == 0x0000 {
                    // SHORT_VBAR_CACHE_MISS.
                    let vbar_yon = (vbar_header & 0xFF) as usize;
                    let vbar_yoff = ((vbar_header >> 8) & 0x3F) as usize;
                    if vbar_yoff < vbar_yon {
                        return Err(Error::InvalidValue {
                            field: "CLEARCODEC vBarYOff",
                            value: format!("{vbar_yoff} < vBarYOn {vbar_yon}"),
                        });
                    }
                    let short_count = vbar_yoff - vbar_yon;
                    if short_count > MAX_VBAR_HEIGHT {
                        return Err(Error::InvalidValue {
                            field: "CLEARCODEC vBarShortPixelCount",
                            value: short_count.to_string(),
                        });
                    }
                    let mut short_pixels = Vec::with_capacity(short_count);
                    for _ in 0..short_count {
                        let b = r.read_u8()?;
                        let g = r.read_u8()?;
                        let red = r.read_u8()?;
                        short_pixels.push([red, g, b]);
                    }
                    self.short_vbar_storage[self.short_vbar_cursor] = Some(VBarEntry {
                        pixels: short_pixels.clone(),
                    });
                    self.short_vbar_cursor = (self.short_vbar_cursor + 1) % SHORT_VBAR_CACHE_SIZE;
                    build_vbar(color_bkg, vbar_yon, &short_pixels, vbar_height)
                } else {
                    // VBAR_CACHE_HIT (top bit set, either 0x8000 or 0xC000).
                    let idx = (vbar_header & 0x7FFF) as usize;
                    match &self.vbar_storage[idx] {
                        Some(entry) if entry.pixels.len() == vbar_height => entry.pixels.clone(),
                        // Empty or size-mismatched slot (e.g. after a
                        // CACHE_RESET that only rewinds the cursor):
                        // degrade gracefully to an all-background bar
                        // rather than failing the whole decode.
                        _ => vec![color_bkg; vbar_height],
                    }
                };

                if pixels.len() != vbar_height {
                    return Err(Error::InvalidValue {
                        field: "CLEARCODEC VBar pixel count",
                        value: format!("{} != vBarHeight {vbar_height}", pixels.len()),
                    });
                }

                if vbar_header & 0x8000 == 0 {
                    // A freshly built bar (short hit or miss) is appended
                    // to the long-VBar cache; a direct long-cache hit
                    // reuses its existing slot untouched.
                    self.vbar_storage[self.vbar_cursor] = Some(VBarEntry {
                        pixels: pixels.clone(),
                    });
                    self.vbar_cursor = (self.vbar_cursor + 1) % VBAR_CACHE_SIZE;
                }

                let dst_x = x_start as usize + i;
                if dst_x >= w {
                    return Err(Error::InvalidValue {
                        field: "CLEARCODEC band column",
                        value: format!("{dst_x} >= width {w}"),
                    });
                }
                for (dy, px) in pixels.iter().enumerate() {
                    let dst_y = y_start as usize + dy;
                    if dst_y >= h {
                        return Err(Error::InvalidValue {
                            field: "CLEARCODEC band row",
                            value: format!("{dst_y} >= height {h}"),
                        });
                    }
                    write_pixel(canvas, w, dst_x, dst_y, *px);
                }
            }
        }
        Ok(())
    }
}

/// Build a full-height VBar (background / cached-short-run / background)
/// from a short-VBar's cached pixels.
fn build_vbar(bkg: [u8; 3], y_on: usize, short: &[[u8; 3]], height: usize) -> Vec<[u8; 3]> {
    let mut out = Vec::with_capacity(height);
    for y in 0..height {
        if y < y_on {
            out.push(bkg);
        } else if y - y_on < short.len() {
            out.push(short[y - y_on]);
        } else {
            out.push(bkg);
        }
    }
    out
}

fn write_pixel(canvas: &mut [u8], w: usize, x: usize, y: usize, rgb: [u8; 3]) {
    let i = (y * w + x) * 4;
    canvas[i] = rgb[0];
    canvas[i + 1] = rgb[1];
    canvas[i + 2] = rgb[2];
}

fn rgb_to_rgba(pixels: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        out.extend_from_slice(&[p[0], p[1], p[2], 0xFF]);
    }
    out
}

/// Read a run-length-encoded color run count (MS-RDPEGFX residual data /
/// RLEX): a `u8`, extended to `u16` if it read as `0xFF`, extended further
/// to `u32` if that read as `0xFFFF`.
fn read_run_length(r: &mut Reader<'_>, first: u32) -> Result<u32> {
    if first < 0xFF {
        return Ok(first);
    }
    let ext16 = r.read_u16_le()? as u32;
    if ext16 < 0xFFFF {
        return Ok(ext16);
    }
    r.read_u32_le()
}

/// Decode `residualData`: a flat run-length-encoded fill covering the
/// entire `w * h` canvas.
fn decode_residual(data: &[u8], w: usize, h: usize, canvas: &mut [u8]) -> Result<()> {
    let mut r = Reader::new(data);
    let pixel_count = w * h;
    let mut pixel_index = 0usize;
    while !r.is_empty() {
        let b = r.read_u8()?;
        let g = r.read_u8()?;
        let red = r.read_u8()?;
        let first = r.read_u8()? as u32;
        let run_length = read_run_length(&mut r, first)? as usize;

        if pixel_index + run_length > pixel_count {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC residual run",
                value: format!("{pixel_index} + {run_length} > {pixel_count}"),
            });
        }
        for _ in 0..run_length {
            let x = pixel_index % w;
            let y = pixel_index / w;
            write_pixel(canvas, w, x, y, [red, g, b]);
            pixel_index += 1;
        }
    }
    if pixel_index != pixel_count {
        return Err(Error::InvalidValue {
            field: "CLEARCODEC residual pixel count",
            value: format!("{pixel_index} != {pixel_count}"),
        });
    }
    Ok(())
}

fn floor_log2(x: u32) -> u32 {
    if x == 0 {
        0
    } else {
        31 - x.leading_zeros()
    }
}

/// Decode one `CLEARCODEC_SUBCODEC_RLEX` sub-tile: a small indexed palette
/// followed by runs, each run pairing a flat color repeat with a short
/// "suite" of sequential palette entries.
fn decode_subcode_rlex(
    data: &[u8],
    w: usize,
    h: usize,
    canvas: &mut [u8],
    x0: usize,
    y0: usize,
) -> Result<()> {
    let mut r = Reader::new(data);
    let palette_count = r.read_u8()? as usize;
    if !(1..=127).contains(&palette_count) {
        return Err(Error::InvalidValue {
            field: "CLEARCODEC RLEX paletteCount",
            value: palette_count.to_string(),
        });
    }
    let mut palette = Vec::with_capacity(palette_count);
    for _ in 0..palette_count {
        let b = r.read_u8()?;
        let g = r.read_u8()?;
        let red = r.read_u8()?;
        palette.push([red, g, b]);
    }
    let num_bits = floor_log2(palette_count as u32 - 1) + 1;
    let stop_mask = (1u32 << num_bits) - 1;

    let pixel_count = w * h;
    let mut pixel_index = 0usize;
    while !r.is_empty() {
        let tmp = r.read_u8()? as u32;
        let first = r.read_u8()? as u32;
        let run_length = read_run_length(&mut r, first)? as usize;

        let suite_depth = (tmp >> num_bits) as u8;
        let stop_index = (tmp & stop_mask) as u8;
        let start_index = stop_index.wrapping_sub(suite_depth);

        if start_index as usize >= palette_count || stop_index as usize >= palette_count {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC RLEX palette index",
                value: format!("start={start_index} stop={stop_index} count={palette_count}"),
            });
        }

        if pixel_index + run_length > pixel_count {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC RLEX run",
                value: format!("{pixel_index} + {run_length} > {pixel_count}"),
            });
        }
        let color = palette[start_index as usize];
        for _ in 0..run_length {
            let x = pixel_index % w;
            let y = pixel_index / w;
            write_pixel(canvas, w, x0 + x, y0 + y, color);
            pixel_index += 1;
        }
        let suite_len = suite_depth as usize + 1;
        if pixel_index + suite_len > pixel_count {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC RLEX suite",
                value: format!("{pixel_index} + {suite_len} > {pixel_count}"),
            });
        }
        for k in 0..suite_len {
            let color = palette[start_index as usize + k];
            let x = pixel_index % w;
            let y = pixel_index / w;
            write_pixel(canvas, w, x0 + x, y0 + y, color);
            pixel_index += 1;
        }
    }
    if pixel_index != pixel_count {
        return Err(Error::InvalidValue {
            field: "CLEARCODEC RLEX pixel count",
            value: format!("{pixel_index} != {pixel_count}"),
        });
    }
    Ok(())
}

/// Decode `subcodecsData`: independent sub-tiles, each raw BGR24, RLEX, or
/// (not implemented here) NSCodec-compressed.
fn decode_subcodecs(data: &[u8], w: usize, h: usize, canvas: &mut [u8]) -> Result<()> {
    let mut r = Reader::new(data);
    while !r.is_empty() {
        let x_start = r.read_u16_le()? as usize;
        let y_start = r.read_u16_le()? as usize;
        let tile_w = r.read_u16_le()? as usize;
        let tile_h = r.read_u16_le()? as usize;
        let bitmap_len = r.read_u32_le()? as usize;
        let subcodec_id = r.read_u8()?;

        if x_start + tile_w > w || y_start + tile_h > h {
            return Err(Error::InvalidValue {
                field: "CLEARCODEC subcodec tile bounds",
                value: format!("[{x_start}+{tile_w}, {y_start}+{tile_h}] outside {w}x{h}"),
            });
        }
        let tile_data = r.read_bytes(bitmap_len)?;

        match subcodec_id {
            0 => {
                // Uncompressed BGR24, row-major, tile_w*tile_h*3 bytes.
                if bitmap_len != tile_w * tile_h * 3 {
                    return Err(Error::InvalidValue {
                        field: "CLEARCODEC subcodec 0 (raw) length",
                        value: format!("{bitmap_len} != {}", tile_w * tile_h * 3),
                    });
                }
                let mut tr = Reader::new(tile_data);
                for y in 0..tile_h {
                    for x in 0..tile_w {
                        let b = tr.read_u8()?;
                        let g = tr.read_u8()?;
                        let red = tr.read_u8()?;
                        write_pixel(canvas, w, x_start + x, y_start + y, [red, g, b]);
                    }
                }
            }
            2 => {
                decode_subcode_rlex(tile_data, tile_w, tile_h, canvas, x_start, y_start)?;
            }
            other => {
                return Err(Error::InvalidValue {
                    field: "CLEARCODEC subcodecId",
                    value: format!("{other} (NSCodec sub-tiles are not implemented)"),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(
        flags: u8,
        seq: u8,
        glyph_index: Option<u16>,
        residual: &[u8],
        bands: &[u8],
        subcodec: &[u8],
    ) -> Vec<u8> {
        let mut out = vec![flags, seq];
        if let Some(idx) = glyph_index {
            out.extend_from_slice(&idx.to_le_bytes());
        }
        out.extend_from_slice(&(residual.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bands.len() as u32).to_le_bytes());
        out.extend_from_slice(&(subcodec.len() as u32).to_le_bytes());
        out.extend_from_slice(residual);
        out.extend_from_slice(bands);
        out.extend_from_slice(subcodec);
        out
    }

    #[test]
    fn decode_residual_flat_fill() {
        let residual = [10u8, 20, 30, 4]; // b,g,r, run=4 (2x2 canvas).
        let data = stream(0, 0, None, &residual, &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        let pixels = dec.decode(&data, 2, 2).unwrap();
        for chunk in pixels.chunks(4) {
            assert_eq!(chunk, &[30, 20, 10, 255]);
        }
    }

    #[test]
    fn decode_residual_extended_run_length() {
        // run length 0xFF -> read u16 extension = 400 pixels (20x20).
        let mut residual = vec![1u8, 2, 3, 0xFF];
        residual.extend_from_slice(&400u16.to_le_bytes());
        let data = stream(0, 0, None, &residual, &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        let pixels = dec.decode(&data, 20, 20).unwrap();
        assert_eq!(pixels.len(), 20 * 20 * 4);
        for chunk in pixels.chunks(4) {
            assert_eq!(chunk, &[3, 2, 1, 255]);
        }
    }

    #[test]
    fn decode_subcodec_raw_tile() {
        let mut subcodec = Vec::new();
        subcodec.extend_from_slice(&0u16.to_le_bytes()); // xStart
        subcodec.extend_from_slice(&0u16.to_le_bytes()); // yStart
        subcodec.extend_from_slice(&2u16.to_le_bytes()); // width
        subcodec.extend_from_slice(&1u16.to_le_bytes()); // height
        subcodec.extend_from_slice(&6u32.to_le_bytes()); // bitmapDataByteCount
        subcodec.push(0); // subcodecId = uncompressed
        subcodec.extend_from_slice(&[1, 2, 3, 4, 5, 6]); // two BGR pixels

        let data = stream(0, 0, None, &[], &[], &subcodec);
        let mut dec = ClearCodecDecoder::new();
        let pixels = dec.decode(&data, 2, 1).unwrap();
        assert_eq!(pixels, vec![3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn decode_subcodec_rlex_tile() {
        // Palette of 2 colors; one run: tmp packs (suiteDepth<<numBits)|stopIndex.
        // numBits = floor_log2(2-1)+1 = floor_log2(1)+1 = 0+1 = 1.
        // suiteDepth=0, stopIndex=1 -> startIndex=1. tmp = (0<<1)|1 = 1.
        // runLengthFactor=4 -> writes color[1] x4 (a 2x2 tile), then suite of
        // 1 pixel (suiteDepth+1=1) at index 1 -> total 5 pixels... use a 5px
        // (5x1) tile instead to keep it exact.
        let mut subcodec = Vec::new();
        subcodec.extend_from_slice(&0u16.to_le_bytes());
        subcodec.extend_from_slice(&0u16.to_le_bytes());
        subcodec.extend_from_slice(&5u16.to_le_bytes()); // width
        subcodec.extend_from_slice(&1u16.to_le_bytes()); // height
        let mut rlex = vec![2u8]; // paletteCount = 2
        rlex.extend_from_slice(&[0, 0, 255]); // palette[0] = BGR -> red
        rlex.extend_from_slice(&[255, 0, 0]); // palette[1] = BGR -> blue
        rlex.push(1); // tmp: suiteDepth=0, stopIndex=1
        rlex.push(4); // runLengthFactor = 4
        subcodec.extend_from_slice(&(rlex.len() as u32).to_le_bytes());
        subcodec.push(2); // subcodecId = RLEX
        subcodec.extend_from_slice(&rlex);

        let data = stream(0, 0, None, &[], &[], &subcodec);
        let mut dec = ClearCodecDecoder::new();
        let pixels = dec.decode(&data, 5, 1).unwrap();
        // 4 pixels of palette[1] (blue: r=0,g=0,b=255), then 1 pixel suite
        // starting at index 1 (also palette[1]).
        for chunk in pixels.chunks(4) {
            assert_eq!(chunk, &[0, 0, 255, 255]);
        }
    }

    #[test]
    fn decode_bands_short_vbar_cache_miss_then_hit() {
        // Band covers a single column (xStart=xEnd=0), rows 0..4
        // (yStart=0,yEnd=3 -> vBarHeight=4), background=[7,8,9] (cr,cg,cb
        // order on the wire is cb,cg,cr).
        let mut bands = Vec::new();
        bands.extend_from_slice(&0u16.to_le_bytes()); // xStart
        bands.extend_from_slice(&0u16.to_le_bytes()); // xEnd
        bands.extend_from_slice(&0u16.to_le_bytes()); // yStart
        bands.extend_from_slice(&3u16.to_le_bytes()); // yEnd
        bands.push(9); // cb
        bands.push(8); // cg
        bands.push(7); // cr
                       // SHORT_VBAR_CACHE_MISS: top 2 bits = 00. vBarYOn=1 (low byte),
                       // vBarYOff=3 (bits 8-13) -> shortPixelCount = 2.
        let header: u16 = 1 | (3 << 8);
        bands.extend_from_slice(&header.to_le_bytes());
        bands.extend_from_slice(&[1, 2, 3]); // pixel0 BGR
        bands.extend_from_slice(&[4, 5, 6]); // pixel1 BGR

        let data = stream(0, 0, None, &[], &bands, &[]);
        let mut dec = ClearCodecDecoder::new();
        let pixels = dec.decode(&data, 1, 4).unwrap();
        // Row0: background (vBarYOn=1 means row0 < yOn). Row1,2: short bar
        // pixels (BGR [1,2,3]->rgb[3,2,1], [4,5,6]->rgb[6,5,4]). Row3:
        // background again (3 >= yOn(1)+shortCount(2)=3).
        assert_eq!(&pixels[0..4], &[7, 8, 9, 255]);
        assert_eq!(&pixels[4..8], &[3, 2, 1, 255]);
        assert_eq!(&pixels[8..12], &[6, 5, 4, 255]);
        assert_eq!(&pixels[12..16], &[7, 8, 9, 255]);
        assert_eq!(dec.vbar_cursor, 1);
        assert_eq!(dec.short_vbar_cursor, 1);
    }

    #[test]
    fn decode_bands_long_vbar_cache_hit_replays_stored_column() {
        let mut bands = Vec::new();
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&1u16.to_le_bytes()); // vBarHeight = 2
        bands.extend_from_slice(&[0, 0, 0]); // background (unused on hit)
                                             // SHORT_VBAR_CACHE_MISS to populate VBarStorage[0] with 2 raw pixels.
        let header: u16 = 2 << 8; // vBarYOn=0, vBarYOff=2 -> count=2
        bands.extend_from_slice(&header.to_le_bytes());
        bands.extend_from_slice(&[10, 20, 30]);
        bands.extend_from_slice(&[40, 50, 60]);

        let data = stream(0, 0, None, &[], &bands, &[]);
        let mut dec = ClearCodecDecoder::new();
        let first = dec.decode(&data, 1, 2).unwrap();
        assert_eq!(&first[0..4], &[30, 20, 10, 255]);
        assert_eq!(&first[4..8], &[60, 50, 40, 255]);

        // Second message: a fresh band referencing VBarStorage[0] directly
        // via VBAR_CACHE_HIT (top bit set, index 0 -> header 0x8000).
        let mut bands2 = Vec::new();
        bands2.extend_from_slice(&0u16.to_le_bytes());
        bands2.extend_from_slice(&0u16.to_le_bytes());
        bands2.extend_from_slice(&0u16.to_le_bytes());
        bands2.extend_from_slice(&1u16.to_le_bytes());
        bands2.extend_from_slice(&[0, 0, 0]);
        bands2.extend_from_slice(&0x8000u16.to_le_bytes());
        let data2 = stream(0, 1, None, &[], &bands2, &[]);
        let second = dec.decode(&data2, 1, 2).unwrap();
        assert_eq!(second, first);
    }

    #[test]
    fn decode_glyph_miss_records_then_hit_replays() {
        let residual = [1u8, 2, 3, 4]; // 2x2 flat fill.
        let miss = stream(FLAG_GLYPH_INDEX, 0, Some(7), &residual, &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        let rendered = dec.decode(&miss, 2, 2).unwrap();
        assert_eq!(rendered.len(), 16);

        let hit = stream(FLAG_GLYPH_INDEX | FLAG_GLYPH_HIT, 1, Some(7), &[], &[], &[]);
        let replayed = dec.decode(&hit, 2, 2).unwrap();
        assert_eq!(replayed, rendered);
    }

    #[test]
    fn decode_rejects_glyph_hit_without_glyph_index() {
        let data = stream(FLAG_GLYPH_HIT, 0, None, &[], &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        assert!(dec.decode(&data, 2, 2).is_err());
    }

    #[test]
    fn decode_rejects_glyph_cache_miss() {
        let data = stream(FLAG_GLYPH_INDEX | FLAG_GLYPH_HIT, 0, Some(3), &[], &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        assert!(dec.decode(&data, 2, 2).is_err());
    }

    #[test]
    fn decode_rejects_unexpected_sequence_number() {
        // A fresh decoder adopts whatever seqNumber the first message
        // carries (matching FreeRDP's reconnect-friendly behavior), so the
        // rejection path only bites once a sequence has been established.
        let first = stream(0, 5, None, &[1, 2, 3, 1], &[], &[]);
        let mut dec = ClearCodecDecoder::new();
        dec.decode(&first, 1, 1).unwrap();

        let skipped = stream(0, 200, None, &[1, 2, 3, 1], &[], &[]);
        assert!(dec.decode(&skipped, 1, 1).is_err());
    }

    #[test]
    fn decode_rejects_nscodec_subtile() {
        let mut subcodec = Vec::new();
        subcodec.extend_from_slice(&0u16.to_le_bytes());
        subcodec.extend_from_slice(&0u16.to_le_bytes());
        subcodec.extend_from_slice(&1u16.to_le_bytes());
        subcodec.extend_from_slice(&1u16.to_le_bytes());
        subcodec.extend_from_slice(&0u32.to_le_bytes());
        subcodec.push(1); // subcodecId = NSCodec (unsupported).

        let data = stream(0, 0, None, &[], &[], &subcodec);
        let mut dec = ClearCodecDecoder::new();
        assert!(dec.decode(&data, 1, 1).is_err());
    }

    #[test]
    fn decode_cache_reset_rewinds_cursors() {
        let mut bands = Vec::new();
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&0u16.to_le_bytes());
        bands.extend_from_slice(&0u16.to_le_bytes()); // vBarHeight = 1
        bands.extend_from_slice(&[0, 0, 0]);
        let header: u16 = 1 << 8; // yOn=0, yOff=1 -> count=1
        bands.extend_from_slice(&header.to_le_bytes());
        bands.extend_from_slice(&[9, 9, 9]);

        let data = stream(0, 0, None, &[], &bands, &[]);
        let mut dec = ClearCodecDecoder::new();
        dec.decode(&data, 1, 1).unwrap();
        assert_eq!(dec.vbar_cursor, 1);

        let data2 = stream(FLAG_CACHE_RESET, 1, None, &[], &bands, &[]);
        dec.decode(&data2, 1, 1).unwrap();
        assert_eq!(dec.vbar_cursor, 1); // rewound to 0, then advanced by 1 again.
    }

    #[test]
    fn decode_truncated_buffer_is_rejected_not_panicking() {
        let data = [0u8]; // header alone, missing seqNumber.
        let mut dec = ClearCodecDecoder::new();
        assert!(dec.decode(&data, 4, 4).is_err());
    }
}
