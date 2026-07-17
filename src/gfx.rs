//! RDP Graphics Pipeline Extension (MS-RDPEGFX), std-only.
//!
//! RDPGFX is the transport for the modern bitmap codecs (RemoteFX,
//! RemoteFX Progressive, the AVC420/AVC444 H.264 profiles, ClearCodec,
//! Planar) and rides inside a [`crate::dvc`] dynamic channel named
//! [`RDPGFX_DVC_CHANNEL_NAME`] — open it via
//! [`crate::net::EstablishConfig::extra_channels`] with [`crate::dvc`]
//! carrying it over the `"DRDYNVC"` static channel, and feed the resulting
//! [`crate::dvcman::DvcEvent::Data`] payloads to [`decode_cmd_id`] to route
//! them to the right PDU decoder.
//!
//! Every RDPGFX PDU starts with an 8-byte `RDPGFX_HEADER` (`cmdId`, `flags`,
//! `pduLength`, all little-endian) that this module writes and strips on
//! every message via internal helpers shared by every PDU type below.
//!
//! ## What's implemented
//!
//! Capability negotiation ([`CapsAdvertisePdu`] / [`CapsConfirmPdu`] /
//! [`Capset`]), surface lifecycle ([`CreateSurfacePdu`] /
//! [`DeleteSurfacePdu`]), the two bitmap-carrying PDUs
//! ([`WireToSurface1Pdu`] / [`WireToSurface2Pdu`]), and frame
//! sequencing/flow-control ([`StartFramePdu`] / [`EndFramePdu`] /
//! [`FrameAcknowledgePdu`]).
//!
//! **Not yet implemented:** the cache PDUs (`SURFACETOCACHE`,
//! `CACHETOSURFACE`, `EVICTCACHEENTRY`, `CACHEIMPORTOFFER`/`REPLY`), surface
//! composition (`SOLIDFILL`, `SURFACETOSURFACE`), output mapping
//! (`RESETGRAPHICS`, `MAPSURFACETO*`), and — the largest remaining piece —
//! the bitmap codecs themselves (`bitmapData` is carried opaquely here as
//! raw bytes; decoding RemoteFX/AVC/ClearCodec payloads is future work).

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The DVC channel name RDPGFX registers itself under (MS-RDPEGFX 1.3.1),
/// opened over [`crate::dvc::DRDYNVC_CHANNEL_NAME`].
pub const RDPGFX_DVC_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Graphics";

/// Length of the `RDPGFX_HEADER` in bytes.
pub const HEADER_LEN: usize = 8;

// cmdId values (MS-RDPEGFX 2.2.1.5).
/// `RDPGFX_CMDID_WIRETOSURFACE_1`.
pub const CMDID_WIRETOSURFACE_1: u16 = 0x0001;
/// `RDPGFX_CMDID_WIRETOSURFACE_2`.
pub const CMDID_WIRETOSURFACE_2: u16 = 0x0002;
/// `RDPGFX_CMDID_CREATESURFACE`.
pub const CMDID_CREATESURFACE: u16 = 0x0009;
/// `RDPGFX_CMDID_DELETESURFACE`.
pub const CMDID_DELETESURFACE: u16 = 0x000A;
/// `RDPGFX_CMDID_STARTFRAME`.
pub const CMDID_STARTFRAME: u16 = 0x000B;
/// `RDPGFX_CMDID_ENDFRAME`.
pub const CMDID_ENDFRAME: u16 = 0x000C;
/// `RDPGFX_CMDID_FRAMEACKNOWLEDGE`.
pub const CMDID_FRAMEACKNOWLEDGE: u16 = 0x000D;
/// `RDPGFX_CMDID_CAPSADVERTISE`.
pub const CMDID_CAPSADVERTISE: u16 = 0x0012;
/// `RDPGFX_CMDID_CAPSCONFIRM`.
pub const CMDID_CAPSCONFIRM: u16 = 0x0013;

// RDPGFX_CODECID_* values (MS-RDPEGFX 2.2.2.1 / 2.2.2.2).
/// `RDPGFX_CODECID_UNCOMPRESSED`.
pub const CODECID_UNCOMPRESSED: u16 = 0x0000;
/// `RDPGFX_CODECID_CAVIDEO` — RemoteFX.
pub const CODECID_CAVIDEO: u16 = 0x0003;
/// `RDPGFX_CODECID_CAPROGRESSIVE` — RemoteFX Progressive (wire-to-surface-2 only).
pub const CODECID_CAPROGRESSIVE: u16 = 0x0009;
/// `RDPGFX_CODECID_CLEARCODEC`.
pub const CODECID_CLEARCODEC: u16 = 0x0008;
/// `RDPGFX_CODECID_PLANAR`.
pub const CODECID_PLANAR: u16 = 0x000A;
/// `RDPGFX_CODECID_AVC420`.
pub const CODECID_AVC420: u16 = 0x000B;
/// `RDPGFX_CODECID_ALPHA`.
pub const CODECID_ALPHA: u16 = 0x000C;
/// `RDPGFX_CODECID_AVC444`.
pub const CODECID_AVC444: u16 = 0x000E;
/// `RDPGFX_CODECID_AVC444V2`.
pub const CODECID_AVC444V2: u16 = 0x000F;

// RDPGFX_CAPVERSION_* values (MS-RDPEGFX 2.2.1.6).
/// `RDPGFX_CAPVERSION_8`.
pub const CAPVERSION_8: u32 = 0x0008_0004;
/// `RDPGFX_CAPVERSION_81`.
pub const CAPVERSION_81: u32 = 0x0008_0105;
/// `RDPGFX_CAPVERSION_10`.
pub const CAPVERSION_10: u32 = 0x000A_0002;
/// `RDPGFX_CAPVERSION_101`.
pub const CAPVERSION_101: u32 = 0x000A_0100;
/// `RDPGFX_CAPVERSION_102`.
pub const CAPVERSION_102: u32 = 0x000A_0200;
/// `RDPGFX_CAPVERSION_103`.
pub const CAPVERSION_103: u32 = 0x000A_0301;
/// `RDPGFX_CAPVERSION_104`.
pub const CAPVERSION_104: u32 = 0x000A_0400;
/// `RDPGFX_CAPVERSION_105`.
pub const CAPVERSION_105: u32 = 0x000A_0502;
/// `RDPGFX_CAPVERSION_106`.
pub const CAPVERSION_106: u32 = 0x000A_0600;
/// `RDPGFX_CAPVERSION_107`.
pub const CAPVERSION_107: u32 = 0x000A_0701;

/// `RDPGFX_PIXELFORMAT` (MS-RDPEGFX 2.2.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// `PIXEL_FORMAT_XRGB_8888` — 32bpp, no valid alpha.
    Xrgb8888,
    /// `PIXEL_FORMAT_ARGB_8888` — 32bpp, valid alpha.
    Argb8888,
}

impl PixelFormat {
    fn to_u8(self) -> u8 {
        match self {
            PixelFormat::Xrgb8888 => 0x20,
            PixelFormat::Argb8888 => 0x21,
        }
    }

    fn from_u8(v: u8) -> Result<PixelFormat> {
        match v {
            0x20 => Ok(PixelFormat::Xrgb8888),
            0x21 => Ok(PixelFormat::Argb8888),
            other => Err(Error::InvalidValue {
                field: "RDPGFX_PIXELFORMAT",
                value: format!("0x{other:02X}"),
            }),
        }
    }
}

/// `RDPGFX_RECT16` — a rectangle with exclusive right/bottom bounds
/// (MS-RDPEGFX 2.2.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect16 {
    /// Leftmost bound.
    pub left: u16,
    /// Upper bound.
    pub top: u16,
    /// Rightmost bound (exclusive).
    pub right: u16,
    /// Lower bound (exclusive).
    pub bottom: u16,
}

impl Rect16 {
    fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.left);
        w.write_u16_le(self.top);
        w.write_u16_le(self.right);
        w.write_u16_le(self.bottom);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Rect16> {
        Ok(Rect16 {
            left: r.read_u16_le()?,
            top: r.read_u16_le()?,
            right: r.read_u16_le()?,
            bottom: r.read_u16_le()?,
        })
    }
}

fn wrap(cmd_id: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_LEN + body.len());
    w.write_u16_le(cmd_id);
    w.write_u16_le(0); // flags, MUST be zero
    w.write_u32_le((HEADER_LEN + body.len()) as u32);
    w.write_bytes(body);
    w.into_vec()
}

/// Read the `RDPGFX_HEADER`, check `cmdId` matches `expected`, and return a
/// reader positioned at the start of the body (validating `pduLength`
/// against the buffer actually supplied).
fn unwrap<'a>(buf: &'a [u8], expected: u16) -> Result<Reader<'a>> {
    let mut r = Reader::new(buf);
    let cmd_id = r.read_u16_le()?;
    let _flags = r.read_u16_le()?;
    let pdu_length = r.read_u32_le()? as usize;
    if cmd_id != expected {
        return Err(Error::InvalidValue {
            field: "RDPGFX_HEADER cmdId",
            value: format!("0x{cmd_id:04X} (expected 0x{expected:04X})"),
        });
    }
    if pdu_length != buf.len() {
        return Err(Error::InvalidLength {
            field: "RDPGFX_HEADER pduLength",
            length: pdu_length,
        });
    }
    Ok(r)
}

/// Peek the `cmdId` of an encoded PDU without consuming it, to pick the
/// right decoder.
pub fn decode_cmd_id(buf: &[u8]) -> Result<u16> {
    let mut r = Reader::new(buf);
    r.read_u16_le()
}

/// `RDPGFX_CAPSET` (MS-RDPEGFX 2.2.1.6) — one capability set entry: a
/// version tag plus version-specific opaque data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capset {
    /// One of the `RDPGFX_CAPVERSION_*` constants.
    pub version: u32,
    /// Version-specific capability data (typically a 4-byte flags word).
    pub data: Vec<u8>,
}

impl Capset {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u32_le(self.version);
        w.write_u32_le(self.data.len() as u32);
        w.write_bytes(&self.data);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<Capset> {
        let version = r.read_u32_le()?;
        let len = r.read_u32_le()? as usize;
        let data = r.read_bytes(len)?.to_vec();
        Ok(Capset { version, data })
    }
}

/// `RDPGFX_CAPS_ADVERTISE_PDU` — sent by the client to advertise the
/// capability sets it supports, most-preferred first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsAdvertisePdu {
    /// The advertised capability sets.
    pub caps_sets: Vec<Capset>,
}

impl CapsAdvertisePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.caps_sets.len() as u16);
        for set in &self.caps_sets {
            set.encode_into(&mut body);
        }
        wrap(CMDID_CAPSADVERTISE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapsAdvertisePdu> {
        let mut r = unwrap(buf, CMDID_CAPSADVERTISE)?;
        let count = r.read_u16_le()?;
        let mut caps_sets = Vec::with_capacity(count as usize);
        for _ in 0..count {
            caps_sets.push(Capset::decode_from(&mut r)?);
        }
        Ok(CapsAdvertisePdu { caps_sets })
    }
}

/// `RDPGFX_CAPS_CONFIRM_PDU` — sent by the server to confirm the single
/// capability set it selected from the client's advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsConfirmPdu {
    /// The selected capability set.
    pub caps_set: Capset,
}

impl CapsConfirmPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        self.caps_set.encode_into(&mut body);
        wrap(CMDID_CAPSCONFIRM, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapsConfirmPdu> {
        let mut r = unwrap(buf, CMDID_CAPSCONFIRM)?;
        let caps_set = Capset::decode_from(&mut r)?;
        Ok(CapsConfirmPdu { caps_set })
    }
}

/// `RDPGFX_CREATE_SURFACE_PDU` — instructs the client to create an
/// off-screen surface of the given dimensions and pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateSurfacePdu {
    /// ID to assign to the new surface.
    pub surface_id: u16,
    /// Surface width in pixels.
    pub width: u16,
    /// Surface height in pixels.
    pub height: u16,
    /// Pixel format of the surface.
    pub pixel_format: PixelFormat,
}

impl CreateSurfacePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u16_le(self.width);
        body.write_u16_le(self.height);
        body.write_u8(self.pixel_format.to_u8());
        wrap(CMDID_CREATESURFACE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CreateSurfacePdu> {
        let mut r = unwrap(buf, CMDID_CREATESURFACE)?;
        Ok(CreateSurfacePdu {
            surface_id: r.read_u16_le()?,
            width: r.read_u16_le()?,
            height: r.read_u16_le()?,
            pixel_format: PixelFormat::from_u8(r.read_u8()?)?,
        })
    }
}

/// `RDPGFX_DELETE_SURFACE_PDU` — instructs the client to delete a surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteSurfacePdu {
    /// ID of the surface to delete.
    pub surface_id: u16,
}

impl DeleteSurfacePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        wrap(CMDID_DELETESURFACE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeleteSurfacePdu> {
        let mut r = unwrap(buf, CMDID_DELETESURFACE)?;
        Ok(DeleteSurfacePdu {
            surface_id: r.read_u16_le()?,
        })
    }
}

/// `RDPGFX_WIRE_TO_SURFACE_PDU_1` — transfers codec-encoded bitmap data to a
/// destination surface. `bitmap_data` is carried opaquely; decoding it is up
/// to a codec module keyed on `codec_id` (only `CODECID_UNCOMPRESSED` can be
/// interpreted without one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireToSurface1Pdu {
    /// Destination surface.
    pub surface_id: u16,
    /// One of the `CODECID_*` constants.
    pub codec_id: u16,
    /// Pixel format of the decoded bitmap.
    pub pixel_format: PixelFormat,
    /// Target rectangle on the destination surface (a bounding box, not an
    /// exact target, for the AVC codecs).
    pub dest_rect: Rect16,
    /// Codec-specific encoded bitmap bytes.
    pub bitmap_data: Vec<u8>,
}

impl WireToSurface1Pdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u16_le(self.codec_id);
        body.write_u8(self.pixel_format.to_u8());
        self.dest_rect.encode(&mut body);
        body.write_u32_le(self.bitmap_data.len() as u32);
        body.write_bytes(&self.bitmap_data);
        wrap(CMDID_WIRETOSURFACE_1, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<WireToSurface1Pdu> {
        let mut r = unwrap(buf, CMDID_WIRETOSURFACE_1)?;
        let surface_id = r.read_u16_le()?;
        let codec_id = r.read_u16_le()?;
        let pixel_format = PixelFormat::from_u8(r.read_u8()?)?;
        let dest_rect = Rect16::decode(&mut r)?;
        let len = r.read_u32_le()? as usize;
        let bitmap_data = r.read_bytes(len)?.to_vec();
        Ok(WireToSurface1Pdu {
            surface_id,
            codec_id,
            pixel_format,
            dest_rect,
            bitmap_data,
        })
    }
}

/// `RDPGFX_WIRE_TO_SURFACE_PDU_2` — transfers bitmap data encoded with a
/// persistent compression context (RemoteFX Progressive only:
/// `CODECID_CAPROGRESSIVE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireToSurface2Pdu {
    /// Destination surface.
    pub surface_id: u16,
    /// One of the `CODECID_*` constants (`CODECID_CAPROGRESSIVE` per spec).
    pub codec_id: u16,
    /// Identifies the persistent compression context this data continues.
    pub codec_context_id: u32,
    /// Pixel format of the decoded bitmap.
    pub pixel_format: PixelFormat,
    /// Codec-specific encoded bitmap bytes.
    pub bitmap_data: Vec<u8>,
}

impl WireToSurface2Pdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u16_le(self.codec_id);
        body.write_u32_le(self.codec_context_id);
        body.write_u8(self.pixel_format.to_u8());
        body.write_u32_le(self.bitmap_data.len() as u32);
        body.write_bytes(&self.bitmap_data);
        wrap(CMDID_WIRETOSURFACE_2, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<WireToSurface2Pdu> {
        let mut r = unwrap(buf, CMDID_WIRETOSURFACE_2)?;
        let surface_id = r.read_u16_le()?;
        let codec_id = r.read_u16_le()?;
        let codec_context_id = r.read_u32_le()?;
        let pixel_format = PixelFormat::from_u8(r.read_u8()?)?;
        let len = r.read_u32_le()? as usize;
        let bitmap_data = r.read_bytes(len)?.to_vec();
        Ok(WireToSurface2Pdu {
            surface_id,
            codec_id,
            codec_context_id,
            pixel_format,
            bitmap_data,
        })
    }
}

/// `RDPGFX_START_FRAME_PDU` — marks the start of a logical frame; graphics
/// commands until the matching [`EndFramePdu`] belong to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartFramePdu {
    /// UTC timestamp packed per MS-RDPEGFX 2.2.2.11, or zero if unavailable.
    pub timestamp: u32,
    /// Unique ID assigned to this frame.
    pub frame_id: u32,
}

impl StartFramePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.timestamp);
        body.write_u32_le(self.frame_id);
        wrap(CMDID_STARTFRAME, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<StartFramePdu> {
        let mut r = unwrap(buf, CMDID_STARTFRAME)?;
        Ok(StartFramePdu {
            timestamp: r.read_u32_le()?,
            frame_id: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_END_FRAME_PDU` — marks the end of a logical frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndFramePdu {
    /// The ID from the matching [`StartFramePdu`].
    pub frame_id: u32,
}

impl EndFramePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.frame_id);
        wrap(CMDID_ENDFRAME, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<EndFramePdu> {
        let mut r = unwrap(buf, CMDID_ENDFRAME)?;
        Ok(EndFramePdu {
            frame_id: r.read_u32_le()?,
        })
    }
}

/// Sentinel `queueDepth` meaning no buffering information is available.
pub const QUEUE_DEPTH_UNAVAILABLE: u32 = 0x0000_0000;
/// Sentinel `queueDepth` opting out of further frame acknowledgements.
pub const SUSPEND_FRAME_ACKNOWLEDGEMENT: u32 = 0xFFFF_FFFF;

/// `RDPGFX_FRAME_ACKNOWLEDGE_PDU` — sent by the client after decoding a
/// frame, in response to [`EndFramePdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameAcknowledgePdu {
    /// Bytes of buffered, unprocessed graphics messages, or one of the
    /// `QUEUE_DEPTH_UNAVAILABLE`/`SUSPEND_FRAME_ACKNOWLEDGEMENT` sentinels.
    pub queue_depth: u32,
    /// The frame being acknowledged.
    pub frame_id: u32,
    /// Total frames decoded by the client since the connection began.
    pub total_frames_decoded: u32,
}

impl FrameAcknowledgePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.queue_depth);
        body.write_u32_le(self.frame_id);
        body.write_u32_le(self.total_frames_decoded);
        wrap(CMDID_FRAMEACKNOWLEDGE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FrameAcknowledgePdu> {
        let mut r = unwrap(buf, CMDID_FRAMEACKNOWLEDGE)?;
        Ok(FrameAcknowledgePdu {
            queue_depth: r.read_u32_le()?,
            frame_id: r.read_u32_le()?,
            total_frames_decoded: r.read_u32_le()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_wire_shape() {
        let pdu = DeleteSurfacePdu { surface_id: 0x0102 }.encode();
        // cmdId=0x000A LE, flags=0, pduLength=10 LE, surfaceId=0x0102 LE.
        assert_eq!(
            pdu,
            vec![0x0A, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x02, 0x01]
        );
    }

    #[test]
    fn decode_cmd_id_reads_without_consuming() {
        let pdu = EndFramePdu { frame_id: 7 }.encode();
        assert_eq!(decode_cmd_id(&pdu).unwrap(), CMDID_ENDFRAME);
        assert_eq!(EndFramePdu::decode(&pdu).unwrap().frame_id, 7);
    }

    #[test]
    fn wrong_cmd_id_is_rejected() {
        let pdu = EndFramePdu { frame_id: 7 }.encode();
        assert!(StartFramePdu::decode(&pdu).is_err());
    }

    #[test]
    fn truncated_pdu_length_is_rejected() {
        let mut pdu = EndFramePdu { frame_id: 7 }.encode();
        pdu.truncate(pdu.len() - 1);
        assert!(EndFramePdu::decode(&pdu).is_err());
    }

    #[test]
    fn capset_roundtrip() {
        let set = Capset {
            version: CAPVERSION_107,
            data: vec![0x01, 0x00, 0x00, 0x00],
        };
        let mut w = Writer::new();
        set.encode_into(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Capset::decode_from(&mut r).unwrap(), set);
    }

    #[test]
    fn caps_advertise_roundtrip_multiple_sets() {
        let pdu = CapsAdvertisePdu {
            caps_sets: vec![
                Capset {
                    version: CAPVERSION_107,
                    data: vec![0x01, 0x00, 0x00, 0x00],
                },
                Capset {
                    version: CAPVERSION_8,
                    data: vec![0x00, 0x00, 0x00, 0x00],
                },
            ],
        };
        assert_eq!(CapsAdvertisePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn caps_advertise_empty_roundtrip() {
        let pdu = CapsAdvertisePdu { caps_sets: vec![] };
        assert_eq!(CapsAdvertisePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn caps_confirm_roundtrip() {
        let pdu = CapsConfirmPdu {
            caps_set: Capset {
                version: CAPVERSION_107,
                data: vec![0xAA, 0xBB, 0xCC, 0xDD],
            },
        };
        assert_eq!(CapsConfirmPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn create_and_delete_surface_roundtrip() {
        let create = CreateSurfacePdu {
            surface_id: 1,
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Xrgb8888,
        };
        assert_eq!(CreateSurfacePdu::decode(&create.encode()).unwrap(), create);

        let delete = DeleteSurfacePdu { surface_id: 1 };
        assert_eq!(DeleteSurfacePdu::decode(&delete.encode()).unwrap(), delete);
    }

    #[test]
    fn pixel_format_rejects_unknown_value() {
        assert!(PixelFormat::from_u8(0x99).is_err());
    }

    #[test]
    fn wire_to_surface_1_roundtrip() {
        let pdu = WireToSurface1Pdu {
            surface_id: 3,
            codec_id: CODECID_UNCOMPRESSED,
            pixel_format: PixelFormat::Argb8888,
            dest_rect: Rect16 {
                left: 0,
                top: 0,
                right: 64,
                bottom: 64,
            },
            bitmap_data: vec![0x42; 64 * 64 * 4],
        };
        assert_eq!(WireToSurface1Pdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn wire_to_surface_2_roundtrip() {
        let pdu = WireToSurface2Pdu {
            surface_id: 3,
            codec_id: CODECID_CAPROGRESSIVE,
            codec_context_id: 99,
            pixel_format: PixelFormat::Xrgb8888,
            bitmap_data: vec![0x7F; 512],
        };
        assert_eq!(WireToSurface2Pdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn frame_lifecycle_roundtrip() {
        let start = StartFramePdu {
            timestamp: 0,
            frame_id: 42,
        };
        assert_eq!(StartFramePdu::decode(&start.encode()).unwrap(), start);

        let end = EndFramePdu { frame_id: 42 };
        assert_eq!(EndFramePdu::decode(&end.encode()).unwrap(), end);

        let ack = FrameAcknowledgePdu {
            queue_depth: QUEUE_DEPTH_UNAVAILABLE,
            frame_id: 42,
            total_frames_decoded: 1,
        };
        assert_eq!(FrameAcknowledgePdu::decode(&ack.encode()).unwrap(), ack);

        let suspend = FrameAcknowledgePdu {
            queue_depth: SUSPEND_FRAME_ACKNOWLEDGEMENT,
            frame_id: 42,
            total_frames_decoded: 1,
        };
        assert_eq!(
            FrameAcknowledgePdu::decode(&suspend.encode()).unwrap(),
            suspend
        );
    }

    #[test]
    fn rect16_roundtrip() {
        let rect = Rect16 {
            left: 1,
            top: 2,
            right: 3,
            bottom: 4,
        };
        let mut w = Writer::new();
        rect.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Rect16::decode(&mut r).unwrap(), rect);
    }
}
