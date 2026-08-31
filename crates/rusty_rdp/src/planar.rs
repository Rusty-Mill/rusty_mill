//! RDP 6.0 "Planar" bitmap codec (MS-RDPEGDI 3.1.9), std-only.
//!
//! Decodes `RDP6_BITMAP_STREAM` — the wire format [`crate::gfx::WireToSurface1Pdu`]
//! and [`crate::gfx::WireToSurface2Pdu`] carry as opaque `bitmap_data` when
//! `codec_id == `[`crate::gfx::CODECID_PLANAR`] — to a flat, top-down RGBA8888
//! pixel buffer, matching [`crate::display::Framebuffer`]'s convention.
//!
//! The format splits a bitmap into four independently-compressed color
//! planes (alpha, luma-or-red, orange-chroma-or-green,
//! green-chroma-or-blue). Each plane is either sent as raw bytes or as a
//! sequence of `RDP6_RLE_SEGMENT`s: a scan-line run-length scheme where raw
//! bytes on the first row are absolute pixel values and every row after
//! that is delta-encoded against the pixel directly above it. Optionally
//! the bitmap is represented in the AYCoCg color space (rather than ARGB)
//! with the two chroma planes further compressed by a lossy bit-shift
//! ("color loss reduction") and/or 2×2 nearest-neighbor chroma
//! subsampling.
//!
//! This is decode-only, mirroring [`crate::rfx`]'s RemoteFX tile decoder:
//! encoding (the server-side direction) is not implemented.

use crate::cursor::Reader;
use crate::error::{Error, Result};

const FORMAT_HEADER_CLL_MASK: u8 = 0x07;
const FORMAT_HEADER_CS: u8 = 0x08;
const FORMAT_HEADER_RLE: u8 = 0x10;
const FORMAT_HEADER_NA: u8 = 0x20;

/// Decode an `RDP6_BITMAP_STREAM` to a `width * height * 4`-byte top-down
/// RGBA8888 pixel buffer.
pub fn decode(data: &[u8], width: u16, height: u16) -> Result<Vec<u8>> {
    let mut r = Reader::new(data);
    let format_header = r.read_u8()?;
    let cll = format_header & FORMAT_HEADER_CLL_MASK;
    let cs = format_header & FORMAT_HEADER_CS != 0;
    let rle = format_header & FORMAT_HEADER_RLE != 0;
    let no_alpha = format_header & FORMAT_HEADER_NA != 0;

    if cs && cll == 0 {
        return Err(Error::InvalidValue {
            field: "RDP6_BITMAP_STREAM FormatHeader",
            value: "chroma subsampling (CS) set without color loss (CLL)".to_string(),
        });
    }

    let w = width as usize;
    let h = height as usize;
    let sub_w = w / 2 + w % 2;
    let sub_h = h / 2 + h % 2;
    let (chroma_w, chroma_h) = if cs { (sub_w, sub_h) } else { (w, h) };

    let alpha_plane = if no_alpha {
        vec![0xFFu8; w * h]
    } else {
        decode_plane(&mut r, w, h, rle)?
    };
    let luma_or_red = decode_plane(&mut r, w, h, rle)?;
    let orange_or_green = decode_plane(&mut r, chroma_w, chroma_h, rle)?;
    let green_or_blue = decode_plane(&mut r, chroma_w, chroma_h, rle)?;
    if !rle {
        let _pad = r.read_u8()?;
    }

    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let luma = luma_or_red[idx] as i32;
            let (r8, g8, b8) = if cll == 0 {
                (luma as u8, orange_or_green[idx], green_or_blue[idx])
            } else {
                let (cx, cy) = if cs { (x / 2, y / 2) } else { (x, y) };
                let chroma_idx = cy * chroma_w + cx;
                let co = color_loss_expand(orange_or_green[chroma_idx], cll);
                let cg = color_loss_expand(green_or_blue[chroma_idx], cll);
                // Inverse AYCoCg->ARGB transform (MS-RDPEGDI 3.1.9.1.2) with
                // the documented R/B swap applied, compensating for a known
                // bug in Microsoft's encoder.
                (
                    clamp_u8(luma - co - cg),
                    clamp_u8(luma + cg),
                    clamp_u8(luma + co - cg),
                )
            };
            out[idx * 4] = r8;
            out[idx * 4 + 1] = g8;
            out[idx * 4 + 2] = b8;
            out[idx * 4 + 3] = alpha_plane[idx];
        }
    }
    Ok(out)
}

/// Reverse the color-loss reduction bit-shift (MS-RDPEGDI 3.1.9.1.4) on a
/// stored chroma byte, returning the reconstructed Co/2 or Cg/2 value used
/// directly by the inverse AYCoCg transform (the `/2` from the transform
/// matrix is folded into the shift amount).
fn color_loss_expand(raw: u8, cll: u8) -> i32 {
    let shift = cll - 1;
    let widened = (raw as i32) << shift;
    (widened as i8) as i32
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Decode one color plane (raw or RDP 6.0 RLE, MS-RDPEGDI 3.1.9.2 /
/// 2.2.2.5.1.1-2) to a `width * height` byte buffer in row-major order.
fn decode_plane(r: &mut Reader<'_>, width: usize, height: usize, rle: bool) -> Result<Vec<u8>> {
    if !rle {
        return Ok(r.read_bytes(width * height)?.to_vec());
    }
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let mut out = vec![0u8; width * height];
    for y in 0..height {
        let (prev_rows, rest) = out.split_at_mut(y * width);
        let cur_row = &mut rest[..width];
        let prev_row = if y == 0 {
            None
        } else {
            Some(&prev_rows[(y - 1) * width..y * width])
        };

        let mut x = 0usize;
        let mut last_raw: u8 = 0;
        let mut last_delta: i16 = 0;
        while x < width {
            let control = r.read_u8()?;
            let mut n_run = (control & 0x0F) as usize;
            let mut c_raw = ((control >> 4) & 0x0F) as usize;
            if n_run == 1 {
                n_run = c_raw + 16;
                c_raw = 0;
            } else if n_run == 2 {
                n_run = c_raw + 32;
                c_raw = 0;
            }
            if x + c_raw + n_run > width {
                return Err(Error::InvalidValue {
                    field: "RDP6_RLE_SEGMENT",
                    value: "segment overruns scan line".to_string(),
                });
            }

            match prev_row {
                None => {
                    for _ in 0..c_raw {
                        last_raw = r.read_u8()?;
                        cur_row[x] = last_raw;
                        x += 1;
                    }
                    for _ in 0..n_run {
                        cur_row[x] = last_raw;
                        x += 1;
                    }
                }
                Some(prev) => {
                    for _ in 0..c_raw {
                        last_delta = decode_delta(r.read_u8()?);
                        cur_row[x] = ((prev[x] as i16).wrapping_add(last_delta)) as u8;
                        x += 1;
                    }
                    for _ in 0..n_run {
                        cur_row[x] = ((prev[x] as i16).wrapping_add(last_delta)) as u8;
                        x += 1;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Decode one RDP 6.0 RLE delta byte (MS-RDPEGDI "Decoding Run-Length
/// Sequences") to the signed delta it represents, to be added (with 1-byte
/// wraparound arithmetic) to the pixel directly above.
fn decode_delta(d: u8) -> i16 {
    if d & 1 == 1 {
        -(((d >> 1) as i16) + 1)
    } else {
        (d >> 1) as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_plane_spec_worked_example() {
        // MS-RDPEGDI "Decoding Run-Length Sequences": a 6x3 plane.
        let data = [
            0x13, 0xFF, 0x20, 0xFE, 0xFD, 0x60, 0x01, 0x7D, 0xF5, 0xC2, 0x9A, 0x38, 0x60, 0x01,
            0x67, 0x8B, 0xA3, 0x78, 0xAF,
        ];
        let mut r = Reader::new(&data);
        let plane = decode_plane(&mut r, 6, 3, true).unwrap();
        assert_eq!(
            plane,
            vec![
                255, 255, 255, 255, 254, 253, //
                254, 192, 132, 96, 75, 25, //
                253, 140, 62, 14, 135, 193,
            ]
        );
        assert!(r.is_empty());
    }

    #[test]
    fn decode_plane_raw_is_a_straight_copy() {
        let data = [1, 2, 3, 4, 5, 6];
        let mut r = Reader::new(&data);
        let plane = decode_plane(&mut r, 3, 2, false).unwrap();
        assert_eq!(plane, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn decode_plane_rejects_segment_overrunning_scan_line() {
        // controlByte 0x90: cRawBytes=9, nRunLength=0 -- too wide for a
        // 4-pixel-wide plane.
        let data = [0x90, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut r = Reader::new(&data);
        assert!(decode_plane(&mut r, 4, 1, true).is_err());
    }

    #[test]
    fn decode_plane_extended_run_length_forms() {
        // Row 0, width 20: controlByte 0x01 -> nRunLength=1 (extended),
        // cRawBytes field (4) reinterpreted as +16 => run of 20, raw count 0.
        // Repeats the assumed-zero raw value across the whole row.
        let data = [0x41]; // nRunLength=1, cRawBytes=4 -> run = 16+4 = 20.
        let mut r = Reader::new(&data);
        let plane = decode_plane(&mut r, 20, 1, true).unwrap();
        assert_eq!(plane, vec![0u8; 20]);
    }

    #[test]
    fn decode_argb_mode_is_direct_rgb_with_no_transform() {
        // FormatHeader: CLL=0, CS=0, RLE=0, NA=1 (no alpha plane).
        let mut data = vec![FORMAT_HEADER_NA];
        data.extend_from_slice(&[10, 20, 30, 40]); // LumaOrRed = Red plane, 2x2
        data.extend_from_slice(&[50, 60, 70, 80]); // OrangeOrGreen = Green plane
        data.extend_from_slice(&[90, 100, 110, 120]); // GreenOrBlue = Blue plane
        data.push(0); // pad (RLE=0)

        let pixels = decode(&data, 2, 2).unwrap();
        assert_eq!(
            pixels,
            vec![
                10, 50, 90, 255, 20, 60, 100, 255, //
                30, 70, 110, 255, 40, 80, 120, 255,
            ]
        );
    }

    #[test]
    fn decode_aycocg_mode_gray_pixel_round_trips_to_itself() {
        // A flat gray image (R=G=B=128) forward-transforms to Y=128, Co=0,
        // Cg=0 exactly, so no color loss or subsampling artifacts can
        // perturb it -- a solid cross-check of the inverse transform wiring
        // (including the documented R/B swap) independent of any encoder.
        let mut data = vec![FORMAT_HEADER_NA | 1]; // CLL=1, NA=1.
        data.extend_from_slice(&[128, 128, 128, 128]); // Y plane, 2x2.
        data.extend_from_slice(&[0, 0, 0, 0]); // Co plane (already 0).
        data.extend_from_slice(&[0, 0, 0, 0]); // Cg plane (already 0).
        data.push(0); // pad.

        let pixels = decode(&data, 2, 2).unwrap();
        for chunk in pixels.chunks(4) {
            assert_eq!(chunk, &[128, 128, 128, 255]);
        }
    }

    #[test]
    fn decode_chroma_subsampling_upsamples_by_replication() {
        // 4x2 image, CLL=1, CS=1: chroma planes are 2x1 (ceil(4/2) x
        // ceil(2/2)), each subsampled value must appear in a 2x2 block of
        // output pixels.
        let mut data = vec![FORMAT_HEADER_NA | FORMAT_HEADER_CS | 1];
        data.extend_from_slice(&[128; 8]); // Y plane, 4x2, flat luma.
        data.extend_from_slice(&[4, 8]); // Co plane, 2x1 (subsampled).
        data.extend_from_slice(&[0, 0]); // Cg plane, 2x1 (subsampled).
        data.push(0); // pad.

        let pixels = decode(&data, 4, 2).unwrap();
        let co0 = color_loss_expand(4, 1);
        let co1 = color_loss_expand(8, 1);
        // Column pairs (0,1) share Co=co0, columns (2,3) share Co=co1, for
        // both rows (the plane is only 1 row tall after subsampling).
        for y in 0..2 {
            for x in 0..4 {
                let idx = (y * 4 + x) * 4;
                let co = if x < 2 { co0 } else { co1 };
                assert_eq!(pixels[idx], clamp_u8(128 - co)); // R = Y - Co - Cg(0)
                assert_eq!(pixels[idx + 2], clamp_u8(128 + co)); // B = Y + Co - Cg(0)
            }
        }
    }

    #[test]
    fn decode_rejects_subsampling_without_color_loss() {
        let data = [FORMAT_HEADER_CS]; // CS set, CLL=0.
        assert!(decode(&data, 2, 2).is_err());
    }

    #[test]
    fn decode_with_alpha_plane_present() {
        // FormatHeader: CLL=0, CS=0, RLE=0, NA=0 (alpha plane present).
        let mut data = vec![0u8];
        data.extend_from_slice(&[10, 20, 30, 40]); // Alpha plane, 2x2.
        data.extend_from_slice(&[1, 2, 3, 4]); // Red plane.
        data.extend_from_slice(&[5, 6, 7, 8]); // Green plane.
        data.extend_from_slice(&[9, 10, 11, 12]); // Blue plane.
        data.push(0); // pad.

        let pixels = decode(&data, 2, 2).unwrap();
        assert_eq!(
            pixels,
            vec![1, 5, 9, 10, 2, 6, 10, 20, 3, 7, 11, 30, 4, 8, 12, 40]
        );
    }

    #[test]
    fn decode_rle_aycocg_subsampled_full_pipeline() {
        // Exercises RLE + AYCoCg + chroma subsampling + no-alpha together,
        // using single all-RAW-no-RUN segments per row (nRunLength=0) so
        // the byte stream stays easy to hand-verify.
        let mut data = vec![FORMAT_HEADER_NA | FORMAT_HEADER_CS | FORMAT_HEADER_RLE | 1];
        // Y plane, 4x2, RLE: row0 absolute [100,110,120,130], row1 all-zero
        // deltas (repeats row0 exactly, since delta 0x00 -> +0).
        data.extend_from_slice(&[0x40, 100, 110, 120, 130]); // cRawBytes=4, nRunLength=0.
        data.extend_from_slice(&[0x40, 0x00, 0x00, 0x00, 0x00]);
        // Co plane, 2x1 subsampled, RLE: single absolute row [4, 8].
        data.extend_from_slice(&[0x20, 4, 8]); // cRawBytes=2, nRunLength=0.
                                               // Cg plane, 2x1 subsampled, RLE: single absolute row [0, 0].
        data.extend_from_slice(&[0x20, 0, 0]);
        // RLE mode: no pad byte.

        let mut r = Reader::new(&data[1..]);
        // Sanity-check the plane decoder in isolation first.
        let y_plane = decode_plane(&mut r, 4, 2, true).unwrap();
        assert_eq!(y_plane, vec![100, 110, 120, 130, 100, 110, 120, 130]);

        let pixels = decode(&data, 4, 2).unwrap();
        let co0 = color_loss_expand(4, 1);
        let co1 = color_loss_expand(8, 1);
        for y in 0..2 {
            for x in 0..4 {
                let idx = (y * 4 + x) * 4;
                let luma = if x < 2 { 100 } else { 120 } + if x % 2 == 0 { 0 } else { 10 };
                let co = if x < 2 { co0 } else { co1 };
                assert_eq!(pixels[idx], clamp_u8(luma - co));
                assert_eq!(pixels[idx + 1], clamp_u8(luma));
                assert_eq!(pixels[idx + 2], clamp_u8(luma + co));
                assert_eq!(pixels[idx + 3], 255);
            }
        }
    }

    #[test]
    fn decode_truncated_buffer_is_rejected_not_panicking() {
        let data = [FORMAT_HEADER_NA]; // Header only, no plane data at all.
        assert!(decode(&data, 4, 4).is_err());
    }

    #[test]
    fn decode_plane_truncated_rle_stream_is_rejected() {
        // Control byte promises 4 raw bytes but only 2 follow.
        let data = [0x40, 1, 2];
        let mut r = Reader::new(&data);
        assert!(decode_plane(&mut r, 4, 1, true).is_err());
    }

    #[test]
    fn decode_delta_matches_spec_examples() {
        // From the spec's worked example (0x01 following an absolute value
        // of 255 yields 254; see decode_plane_spec_worked_example).
        assert_eq!(decode_delta(0x01), -1);
        assert_eq!(decode_delta(0xC2), 97);
        assert_eq!(decode_delta(0x38), 28);
    }

    #[test]
    fn color_loss_expand_matches_convert_semantics() {
        // CLL=1 -> shift=0, so the stored byte is used directly as a
        // two's-complement signed value.
        assert_eq!(color_loss_expand(0x01, 1), 1);
        assert_eq!(color_loss_expand(0xFF, 1), -1);
        // CLL=2 -> shift=1, doubling (then truncating to 8 bits).
        assert_eq!(color_loss_expand(0x01, 2), 2);
    }
}
