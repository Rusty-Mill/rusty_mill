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
//! ([`WireToSurface1Pdu`] / [`WireToSurface2Pdu`]), frame
//! sequencing/flow-control ([`StartFramePdu`] / [`EndFramePdu`] /
//! [`FrameAcknowledgePdu`]), the bitmap cache PDUs
//! ([`SurfaceToCachePdu`] / [`CacheToSurfacePdu`] / [`EvictCacheEntryPdu`] /
//! [`CacheImportOfferPdu`] / [`CacheImportReplyPdu`]), surface composition
//! ([`SolidFillPdu`] / [`SurfaceToSurfacePdu`]), output mapping
//! ([`ResetGraphicsPdu`] / [`MapSurfaceToOutputPdu`] /
//! [`MapSurfaceToScaledOutputPdu`] / [`MapSurfaceToWindowPdu`] /
//! [`MapSurfaceToScaledWindowPdu`]), and the AVC420/AVC444 wrapper formats
//! ([`Avc420BitmapStream`] / [`Avc444BitmapStream`]) — region and
//! quantization metadata only, since decoding the H.264 bitstreams they
//! carry needs an actual H.264 decoder (see [`Avc420BitmapStream`]'s docs).
//!
//! The RemoteFX, RDP 6.0 Planar, and ClearCodec bitmap codecs decode all
//! the way to pixels, in [`crate::rfx`], [`crate::planar`], and
//! [`crate::clearcodec`] respectively; `bitmapData` for the other codec
//! IDs stays opaque.

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
/// `RDPGFX_CMDID_SOLIDFILL`.
pub const CMDID_SOLIDFILL: u16 = 0x0004;
/// `RDPGFX_CMDID_SURFACETOSURFACE`.
pub const CMDID_SURFACETOSURFACE: u16 = 0x0005;
/// `RDPGFX_CMDID_SURFACETOCACHE`.
pub const CMDID_SURFACETOCACHE: u16 = 0x0006;
/// `RDPGFX_CMDID_CACHETOSURFACE`.
pub const CMDID_CACHETOSURFACE: u16 = 0x0007;
/// `RDPGFX_CMDID_EVICTCACHEENTRY`.
pub const CMDID_EVICTCACHEENTRY: u16 = 0x0008;
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
/// `RDPGFX_CMDID_CACHEIMPORTOFFER`.
pub const CMDID_CACHEIMPORTOFFER: u16 = 0x0010;
/// `RDPGFX_CMDID_CACHEIMPORTREPLY`.
pub const CMDID_CACHEIMPORTREPLY: u16 = 0x0011;
/// `RDPGFX_CMDID_RESETGRAPHICS`.
pub const CMDID_RESETGRAPHICS: u16 = 0x000E;
/// `RDPGFX_CMDID_MAPSURFACETOOUTPUT`.
pub const CMDID_MAPSURFACETOOUTPUT: u16 = 0x000F;
/// `RDPGFX_CMDID_MAPSURFACETOWINDOW`.
pub const CMDID_MAPSURFACETOWINDOW: u16 = 0x0015;
/// `RDPGFX_CMDID_MAPSURFACETOSCALEDOUTPUT`.
pub const CMDID_MAPSURFACETOSCALEDOUTPUT: u16 = 0x0017;
/// `RDPGFX_CMDID_MAPSURFACETOSCALEDWINDOW`.
pub const CMDID_MAPSURFACETOSCALEDWINDOW: u16 = 0x0018;

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

/// `RDPGFX_POINT16` (MS-RDPEGFX 2.2.1.1) — a point relative to the origin
/// of a target surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point16 {
    /// X-coordinate.
    pub x: i16,
    /// Y-coordinate.
    pub y: i16,
}

impl Point16 {
    fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.x as u16);
        w.write_u16_le(self.y as u16);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Point16> {
        Ok(Point16 {
            x: r.read_u16_le()? as i16,
            y: r.read_u16_le()? as i16,
        })
    }
}

/// `RDPGFX_COLOR` (MS-RDPEGFX 2.2.1.3) — a 32bpp ARGB or XRGB color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color32 {
    /// Blue component.
    pub b: u8,
    /// Green component.
    pub g: u8,
    /// Red component.
    pub r: u8,
    /// Alpha component for ARGB; ignored for XRGB.
    pub xa: u8,
}

impl Color32 {
    fn encode(&self, w: &mut Writer) {
        w.write_u8(self.b);
        w.write_u8(self.g);
        w.write_u8(self.r);
        w.write_u8(self.xa);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Color32> {
        let b = r.read_u8()?;
        let g = r.read_u8()?;
        let red = r.read_u8()?;
        let xa = r.read_u8()?;
        Ok(Color32 { b, g, r: red, xa })
    }
}

/// `TS_MONITOR_PRIMARY` flag for [`MonitorDef::flags`] (MS-RDPBCGR
/// 2.2.1.3.6.1) — marks the monitor as the display containing the taskbar
/// and Start menu.
pub const TS_MONITOR_PRIMARY: u32 = 0x0000_0001;

/// `TS_MONITOR_DEF` (MS-RDPBCGR 2.2.1.3.6.1) — describes one monitor's
/// position and size within a [`ResetGraphicsPdu`]'s monitor layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorDef {
    /// Left bound, relative to the upper-left corner of the primary
    /// monitor.
    pub left: i32,
    /// Upper bound, relative to the upper-left corner of the primary
    /// monitor.
    pub top: i32,
    /// Right bound (inclusive), relative to the upper-left corner of the
    /// primary monitor.
    pub right: i32,
    /// Lower bound (inclusive), relative to the upper-left corner of the
    /// primary monitor.
    pub bottom: i32,
    /// Monitor flags; see [`TS_MONITOR_PRIMARY`].
    pub flags: u32,
}

impl MonitorDef {
    fn encode(&self, w: &mut Writer) {
        w.write_u32_le(self.left as u32);
        w.write_u32_le(self.top as u32);
        w.write_u32_le(self.right as u32);
        w.write_u32_le(self.bottom as u32);
        w.write_u32_le(self.flags);
    }

    fn decode(r: &mut Reader<'_>) -> Result<MonitorDef> {
        Ok(MonitorDef {
            left: r.read_u32_le()? as i32,
            top: r.read_u32_le()? as i32,
            right: r.read_u32_le()? as i32,
            bottom: r.read_u32_le()? as i32,
            flags: r.read_u32_le()?,
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
    Ok(r.read_u16_le()?)
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

/// `RDPGFX_SOLID_FILL_PDU` — instructs the client to fill a collection of
/// rectangles on a destination surface with a solid color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolidFillPdu {
    /// Destination surface.
    pub surface_id: u16,
    /// The fill color.
    pub fill_pixel: Color32,
    /// The rectangles to fill.
    pub fill_rects: Vec<Rect16>,
}

impl SolidFillPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        self.fill_pixel.encode(&mut body);
        body.write_u16_le(self.fill_rects.len() as u16);
        for rect in &self.fill_rects {
            rect.encode(&mut body);
        }
        wrap(CMDID_SOLIDFILL, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<SolidFillPdu> {
        let mut r = unwrap(buf, CMDID_SOLIDFILL)?;
        let surface_id = r.read_u16_le()?;
        let fill_pixel = Color32::decode(&mut r)?;
        let count = r.read_u16_le()?;
        let mut fill_rects = Vec::with_capacity(count as usize);
        for _ in 0..count {
            fill_rects.push(Rect16::decode(&mut r)?);
        }
        Ok(SolidFillPdu {
            surface_id,
            fill_pixel,
            fill_rects,
        })
    }
}

/// `RDPGFX_SURFACE_TO_SURFACE_PDU` — instructs the client to copy bitmap
/// data from a source surface to one or more points on a destination
/// surface (which may be the same surface, to replicate bitmap data within
/// it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceToSurfacePdu {
    /// Source surface.
    pub surface_id_src: u16,
    /// Destination surface.
    pub surface_id_dest: u16,
    /// Rectangle bounding the source bitmap.
    pub rect_src: Rect16,
    /// Target points on the destination surface to copy the source bitmap
    /// to.
    pub dest_pts: Vec<Point16>,
}

impl SurfaceToSurfacePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id_src);
        body.write_u16_le(self.surface_id_dest);
        self.rect_src.encode(&mut body);
        body.write_u16_le(self.dest_pts.len() as u16);
        for pt in &self.dest_pts {
            pt.encode(&mut body);
        }
        wrap(CMDID_SURFACETOSURFACE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<SurfaceToSurfacePdu> {
        let mut r = unwrap(buf, CMDID_SURFACETOSURFACE)?;
        let surface_id_src = r.read_u16_le()?;
        let surface_id_dest = r.read_u16_le()?;
        let rect_src = Rect16::decode(&mut r)?;
        let count = r.read_u16_le()?;
        let mut dest_pts = Vec::with_capacity(count as usize);
        for _ in 0..count {
            dest_pts.push(Point16::decode(&mut r)?);
        }
        Ok(SurfaceToSurfacePdu {
            surface_id_src,
            surface_id_dest,
            rect_src,
            dest_pts,
        })
    }
}

/// `RDPGFX_SURFACE_TO_CACHE_PDU` — instructs the client to copy a rectangle
/// of a surface into the bitmap cache, keyed by `cache_key` and addressed
/// by `cache_slot` for later [`CacheToSurfacePdu`] lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceToCachePdu {
    /// Source surface.
    pub surface_id: u16,
    /// Unique key identifying this cache entry.
    pub cache_key: u64,
    /// Cache slot to store the entry in.
    pub cache_slot: u16,
    /// Rectangle bounding the source bitmap.
    pub rect_src: Rect16,
}

impl SurfaceToCachePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u64_le(self.cache_key);
        body.write_u16_le(self.cache_slot);
        self.rect_src.encode(&mut body);
        wrap(CMDID_SURFACETOCACHE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<SurfaceToCachePdu> {
        let mut r = unwrap(buf, CMDID_SURFACETOCACHE)?;
        Ok(SurfaceToCachePdu {
            surface_id: r.read_u16_le()?,
            cache_key: r.read_u64_le()?,
            cache_slot: r.read_u16_le()?,
            rect_src: Rect16::decode(&mut r)?,
        })
    }
}

/// `RDPGFX_CACHE_TO_SURFACE_PDU` — instructs the client to copy a
/// previously cached bitmap ([`SurfaceToCachePdu`]) to one or more points
/// on a destination surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheToSurfacePdu {
    /// The cache slot to copy from.
    pub cache_slot: u16,
    /// Destination surface.
    pub surface_id: u16,
    /// Target points on the destination surface.
    pub dest_pts: Vec<Point16>,
}

impl CacheToSurfacePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.cache_slot);
        body.write_u16_le(self.surface_id);
        body.write_u16_le(self.dest_pts.len() as u16);
        for pt in &self.dest_pts {
            pt.encode(&mut body);
        }
        wrap(CMDID_CACHETOSURFACE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CacheToSurfacePdu> {
        let mut r = unwrap(buf, CMDID_CACHETOSURFACE)?;
        let cache_slot = r.read_u16_le()?;
        let surface_id = r.read_u16_le()?;
        let count = r.read_u16_le()?;
        let mut dest_pts = Vec::with_capacity(count as usize);
        for _ in 0..count {
            dest_pts.push(Point16::decode(&mut r)?);
        }
        Ok(CacheToSurfacePdu {
            cache_slot,
            surface_id,
            dest_pts,
        })
    }
}

/// `RDPGFX_EVICT_CACHE_ENTRY_PDU` — instructs the client to evict a bitmap
/// cache entry, freeing its slot for reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictCacheEntryPdu {
    /// The cache slot to evict.
    pub cache_slot: u16,
}

impl EvictCacheEntryPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.cache_slot);
        wrap(CMDID_EVICTCACHEENTRY, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<EvictCacheEntryPdu> {
        let mut r = unwrap(buf, CMDID_EVICTCACHEENTRY)?;
        Ok(EvictCacheEntryPdu {
            cache_slot: r.read_u16_le()?,
        })
    }
}

/// `RDPGFX_CACHE_ENTRY_METADATA` — one entry in a [`CacheImportOfferPdu`]:
/// a persistent-cache bitmap the client already holds from a prior
/// session, identified by its unique key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheEntryMetadata {
    /// Unique key identifying this cache entry (matches a prior
    /// [`SurfaceToCachePdu::cache_key`]).
    pub cache_key: u64,
    /// Size, in bytes, of the cached bitmap.
    pub bitmap_length: u32,
}

impl CacheEntryMetadata {
    fn encode(&self, w: &mut Writer) {
        w.write_u64_le(self.cache_key);
        w.write_u32_le(self.bitmap_length);
    }

    fn decode(r: &mut Reader<'_>) -> Result<CacheEntryMetadata> {
        Ok(CacheEntryMetadata {
            cache_key: r.read_u64_le()?,
            bitmap_length: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_CACHE_IMPORT_OFFER_PDU` — sent by the client after capability
/// exchange to offer bitmaps its persistent disk cache already holds from
/// a prior session, so the server can skip re-sending ones it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheImportOfferPdu {
    /// The offered cache entries.
    pub cache_entries: Vec<CacheEntryMetadata>,
}

impl CacheImportOfferPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.cache_entries.len() as u16);
        for entry in &self.cache_entries {
            entry.encode(&mut body);
        }
        wrap(CMDID_CACHEIMPORTOFFER, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CacheImportOfferPdu> {
        let mut r = unwrap(buf, CMDID_CACHEIMPORTOFFER)?;
        let count = r.read_u16_le()?;
        let mut cache_entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            cache_entries.push(CacheEntryMetadata::decode(&mut r)?);
        }
        Ok(CacheImportOfferPdu { cache_entries })
    }
}

/// `RDPGFX_CACHE_IMPORT_REPLY_PDU` — the server's answer to a
/// [`CacheImportOfferPdu`], assigning a cache slot to each offered entry
/// it accepted, in the same order they were offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheImportReplyPdu {
    /// The cache slot assigned to each accepted entry.
    pub cache_slots: Vec<u16>,
}

impl CacheImportReplyPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.cache_slots.len() as u16);
        for &slot in &self.cache_slots {
            body.write_u16_le(slot);
        }
        wrap(CMDID_CACHEIMPORTREPLY, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CacheImportReplyPdu> {
        let mut r = unwrap(buf, CMDID_CACHEIMPORTREPLY)?;
        let count = r.read_u16_le()?;
        let mut cache_slots = Vec::with_capacity(count as usize);
        for _ in 0..count {
            cache_slots.push(r.read_u16_le()?);
        }
        Ok(CacheImportReplyPdu { cache_slots })
    }
}

/// `RDPGFX_RESET_GRAPHICS_PDU` — sent by the server to reset the client's
/// graphics output buffer to a new monitor layout, e.g. after a display
/// resize or reconfiguration. Always exactly 340 bytes on the wire
/// regardless of `monitors.len()`; the remainder is ignored padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetGraphicsPdu {
    /// Width, in pixels, of the graphics output buffer. MUST be less than
    /// or equal to 32766.
    pub width: u32,
    /// Height, in pixels, of the graphics output buffer. MUST be less than
    /// or equal to 32766.
    pub height: u32,
    /// The new monitor layout. MUST contain at most 16 entries.
    pub monitors: Vec<MonitorDef>,
}

/// Fixed total size, in bytes, of an encoded [`ResetGraphicsPdu`]
/// (MS-RDPEGFX 2.2.2.14).
const RESET_GRAPHICS_PDU_LEN: usize = 340;
/// Maximum number of [`MonitorDef`] entries a [`ResetGraphicsPdu`] may
/// carry (MS-RDPEGFX 2.2.2.14).
const RESET_GRAPHICS_MAX_MONITORS: usize = 16;

impl ResetGraphicsPdu {
    /// Encode to bytes. `monitors` MUST have at most 16 entries, or the
    /// padding computation underflows and the returned buffer will not be
    /// 340 bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.width);
        body.write_u32_le(self.height);
        body.write_u32_le(self.monitors.len() as u32);
        for monitor in &self.monitors {
            monitor.encode(&mut body);
        }
        let used = HEADER_LEN + 12 + 20 * self.monitors.len();
        body.write_bytes(&vec![0u8; RESET_GRAPHICS_PDU_LEN.saturating_sub(used)]);
        wrap(CMDID_RESETGRAPHICS, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ResetGraphicsPdu> {
        if buf.len() != RESET_GRAPHICS_PDU_LEN {
            return Err(Error::InvalidLength {
                field: "RDPGFX_RESET_GRAPHICS_PDU pduLength",
                length: buf.len(),
            });
        }
        let mut r = unwrap(buf, CMDID_RESETGRAPHICS)?;
        let width = r.read_u32_le()?;
        let height = r.read_u32_le()?;
        let monitor_count = r.read_u32_le()?;
        if monitor_count as usize > RESET_GRAPHICS_MAX_MONITORS {
            return Err(Error::InvalidValue {
                field: "RDPGFX_RESET_GRAPHICS_PDU monitorCount",
                value: monitor_count.to_string(),
            });
        }
        let mut monitors = Vec::with_capacity(monitor_count as usize);
        for _ in 0..monitor_count {
            monitors.push(MonitorDef::decode(&mut r)?);
        }
        Ok(ResetGraphicsPdu {
            width,
            height,
            monitors,
        })
    }
}

/// `RDPGFX_MAP_SURFACE_TO_OUTPUT_PDU` — instructs the client to map a
/// surface to a rectangular area of the graphics output buffer at a fixed
/// (unscaled) origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSurfaceToOutputPdu {
    /// ID of the surface to map.
    pub surface_id: u16,
    /// X-coordinate, relative to the origin of the graphics output buffer,
    /// at which to map the top-left corner of the surface.
    pub output_origin_x: u32,
    /// Y-coordinate, relative to the origin of the graphics output buffer,
    /// at which to map the top-left corner of the surface.
    pub output_origin_y: u32,
}

impl MapSurfaceToOutputPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u16_le(0); // reserved, MUST be zero
        body.write_u32_le(self.output_origin_x);
        body.write_u32_le(self.output_origin_y);
        wrap(CMDID_MAPSURFACETOOUTPUT, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MapSurfaceToOutputPdu> {
        let mut r = unwrap(buf, CMDID_MAPSURFACETOOUTPUT)?;
        let surface_id = r.read_u16_le()?;
        let _reserved = r.read_u16_le()?;
        Ok(MapSurfaceToOutputPdu {
            surface_id,
            output_origin_x: r.read_u32_le()?,
            output_origin_y: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_MAP_SURFACE_TO_SCALED_OUTPUT_PDU` — instructs the client to map
/// a surface to a rectangular area of the graphics output buffer, scaled to
/// a target width/height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSurfaceToScaledOutputPdu {
    /// ID of the surface to map.
    pub surface_id: u16,
    /// X-coordinate, relative to the origin of the graphics output buffer,
    /// at which to map the top-left corner of the surface.
    pub output_origin_x: u32,
    /// Y-coordinate, relative to the origin of the graphics output buffer,
    /// at which to map the top-left corner of the surface.
    pub output_origin_y: u32,
    /// Width, in pixels, to which the surface MUST be scaled.
    pub target_width: u32,
    /// Height, in pixels, to which the surface MUST be scaled.
    pub target_height: u32,
}

impl MapSurfaceToScaledOutputPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u16_le(0); // reserved, MUST be zero
        body.write_u32_le(self.output_origin_x);
        body.write_u32_le(self.output_origin_y);
        body.write_u32_le(self.target_width);
        body.write_u32_le(self.target_height);
        wrap(CMDID_MAPSURFACETOSCALEDOUTPUT, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MapSurfaceToScaledOutputPdu> {
        let mut r = unwrap(buf, CMDID_MAPSURFACETOSCALEDOUTPUT)?;
        let surface_id = r.read_u16_le()?;
        let _reserved = r.read_u16_le()?;
        Ok(MapSurfaceToScaledOutputPdu {
            surface_id,
            output_origin_x: r.read_u32_le()?,
            output_origin_y: r.read_u32_le()?,
            target_width: r.read_u32_le()?,
            target_height: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_MAP_SURFACE_TO_WINDOW_PDU` — instructs the client to map a
/// surface to a RAIL window (Enhanced RemoteApp), unscaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSurfaceToWindowPdu {
    /// ID of the surface to map.
    pub surface_id: u16,
    /// ID of the RAIL window to associate with the mapping (see
    /// `[MS-RDPERP]` 2.2.1.3.1.1's `WindowId`).
    pub window_id: u64,
    /// Width of the rectangular region on the surface to which the window
    /// is mapped.
    pub mapped_width: u32,
    /// Height of the rectangular region on the surface to which the window
    /// is mapped.
    pub mapped_height: u32,
}

impl MapSurfaceToWindowPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u64_le(self.window_id);
        body.write_u32_le(self.mapped_width);
        body.write_u32_le(self.mapped_height);
        wrap(CMDID_MAPSURFACETOWINDOW, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MapSurfaceToWindowPdu> {
        let mut r = unwrap(buf, CMDID_MAPSURFACETOWINDOW)?;
        Ok(MapSurfaceToWindowPdu {
            surface_id: r.read_u16_le()?,
            window_id: r.read_u64_le()?,
            mapped_width: r.read_u32_le()?,
            mapped_height: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_MAP_SURFACE_TO_SCALED_WINDOW_PDU` — instructs the client to map
/// a surface to a RAIL window (Enhanced RemoteApp), scaled to a target
/// width/height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSurfaceToScaledWindowPdu {
    /// ID of the surface to map.
    pub surface_id: u16,
    /// ID of the RAIL window to associate with the mapping (see
    /// `[MS-RDPERP]` 2.2.1.3.1.1's `WindowId`).
    pub window_id: u64,
    /// Width of the rectangular region on the surface to which the window
    /// is mapped.
    pub mapped_width: u32,
    /// Height of the rectangular region on the surface to which the window
    /// is mapped.
    pub mapped_height: u32,
    /// Width, in pixels, of the target graphics output to which the
    /// surface will be scaled.
    pub target_width: u32,
    /// Height, in pixels, of the target graphics output to which the
    /// surface will be scaled.
    pub target_height: u32,
}

impl MapSurfaceToScaledWindowPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.surface_id);
        body.write_u64_le(self.window_id);
        body.write_u32_le(self.mapped_width);
        body.write_u32_le(self.mapped_height);
        body.write_u32_le(self.target_width);
        body.write_u32_le(self.target_height);
        wrap(CMDID_MAPSURFACETOSCALEDWINDOW, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MapSurfaceToScaledWindowPdu> {
        let mut r = unwrap(buf, CMDID_MAPSURFACETOSCALEDWINDOW)?;
        Ok(MapSurfaceToScaledWindowPdu {
            surface_id: r.read_u16_le()?,
            window_id: r.read_u64_le()?,
            mapped_width: r.read_u32_le()?,
            mapped_height: r.read_u32_le()?,
            target_width: r.read_u32_le()?,
            target_height: r.read_u32_le()?,
        })
    }
}

/// `RDPGFX_AVC420_QUANT_QUALITY` — one region's H.264 quantization/quality
/// hint within an [`Avc420MetaBlock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Avc420QuantQuality {
    /// Quantization parameter used to encode the region (0-63).
    pub qp: u8,
    /// `useQualityVal` flag: when set, `quality_val` should be used instead
    /// of `qp` to judge the region's quality.
    pub r: bool,
    /// Progressive-frame flag.
    pub p: bool,
    /// Quality value for the region, on a 0-100 scale.
    pub quality_val: u8,
}

impl Avc420QuantQuality {
    fn encode(&self, w: &mut Writer) {
        let qp_val = (self.qp & 0x3F) | ((self.r as u8) << 6) | ((self.p as u8) << 7);
        w.write_u8(qp_val);
        w.write_u8(self.quality_val);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Avc420QuantQuality> {
        let qp_val = r.read_u8()?;
        let quality_val = r.read_u8()?;
        Ok(Avc420QuantQuality {
            qp: qp_val & 0x3F,
            r: (qp_val >> 6) & 1 != 0,
            p: (qp_val >> 7) & 1 != 0,
            quality_val,
        })
    }
}

/// `RFX_AVC420_METABLOCK` — the region list an AVC420/AVC444 encoded
/// bitstream applies to, one [`Rect16`] and one [`Avc420QuantQuality`] per
/// region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avc420MetaBlock {
    /// The regions this bitstream's decoded frame should be cropped to and
    /// composited over.
    pub region_rects: Vec<Rect16>,
    /// Per-region quantization/quality hints, same length and order as
    /// `region_rects`.
    pub quant_quality_vals: Vec<Avc420QuantQuality>,
}

impl Avc420MetaBlock {
    fn encode(&self, w: &mut Writer) {
        w.write_u32_le(self.region_rects.len() as u32);
        for rect in &self.region_rects {
            rect.encode(w);
        }
        for qq in &self.quant_quality_vals {
            qq.encode(w);
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<Avc420MetaBlock> {
        let count = r.read_u32_le()? as usize;
        let mut region_rects = Vec::with_capacity(count);
        for _ in 0..count {
            region_rects.push(Rect16::decode(r)?);
        }
        let mut quant_quality_vals = Vec::with_capacity(count);
        for _ in 0..count {
            quant_quality_vals.push(Avc420QuantQuality::decode(r)?);
        }
        Ok(Avc420MetaBlock {
            region_rects,
            quant_quality_vals,
        })
    }
}

/// `RFX_AVC420_BITMAP_STREAM` — the wire format carried as
/// [`WireToSurface1Pdu::bitmap_data`] when `codec_id == `[`CODECID_AVC420`],
/// and nested (with a bounded length instead of running to the end of the
/// buffer) inside [`Avc444BitmapStream`].
///
/// This crate parses the region/quality metadata only; `bitstream` is
/// carried opaquely as raw ITU-T H.264 Annex B bytes — decoding it to
/// pixels requires an actual H.264 decoder, out of scope for this
/// dependency-free crate. Hand `bitstream` to an external decoder (e.g.
/// openh264, ffmpeg) keyed on the region metadata here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avc420BitmapStream {
    /// The regions and per-region quality hints this bitstream applies to.
    pub meta: Avc420MetaBlock,
    /// Opaque ITU-T H.264 Annex B encoded frame data.
    pub bitstream: Vec<u8>,
}

impl Avc420BitmapStream {
    /// Encode to bytes (not wrapped in an `RDPGFX_HEADER` — this is a
    /// codec-specific `bitmapData` payload, not a PDU in its own right).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.meta.encode(&mut w);
        w.write_bytes(&self.bitstream);
        w.into_vec()
    }

    /// Decode from bytes, taking every byte after the metadata as the
    /// encoded bitstream.
    pub fn decode(buf: &[u8]) -> Result<Avc420BitmapStream> {
        let mut r = Reader::new(buf);
        let meta = Avc420MetaBlock::decode(&mut r)?;
        let bitstream = r.read_bytes(r.remaining())?.to_vec();
        Ok(Avc420BitmapStream { meta, bitstream })
    }
}

/// The `LC` (layer composition) field of an [`Avc444BitmapStream`],
/// specifying which of its one or two [`Avc420BitmapStream`]s carry the
/// YUV420 luma frame vs. the Chroma420 residual frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Avc444LayerComposition {
    /// `stream1` carries the YUV420 frame, `stream2` carries the
    /// corresponding Chroma420 frame.
    LumaAndChroma,
    /// `stream1` carries the YUV420 frame only; the matching Chroma420
    /// frame will follow in a later message.
    LumaOnly,
    /// `stream1` carries the Chroma420 frame only, to be combined with a
    /// previously received YUV420 frame.
    ChromaOnly,
}

impl Avc444LayerComposition {
    fn from_lc(lc: u32) -> Result<Avc444LayerComposition> {
        match lc {
            0 => Ok(Avc444LayerComposition::LumaAndChroma),
            1 => Ok(Avc444LayerComposition::LumaOnly),
            2 => Ok(Avc444LayerComposition::ChromaOnly),
            other => Err(Error::InvalidValue {
                field: "RFX_AVC444_BITMAP_STREAM LC",
                value: other.to_string(),
            }),
        }
    }

    fn to_lc(self) -> u32 {
        match self {
            Avc444LayerComposition::LumaAndChroma => 0,
            Avc444LayerComposition::LumaOnly => 1,
            Avc444LayerComposition::ChromaOnly => 2,
        }
    }
}

/// `RFX_AVC444_BITMAP_STREAM` / `RFX_AVC444V2_BITMAP_STREAM` (identical
/// wrapper shape) — the wire format carried as
/// [`WireToSurface1Pdu::bitmap_data`] when `codec_id == `[`CODECID_AVC444`]
/// or [`CODECID_AVC444V2`]: up to two [`Avc420BitmapStream`]s carrying the
/// YUV420 and Chroma420 halves of a YUV444 frame. See
/// [`Avc420BitmapStream`] for why the encoded bitstreams are carried
/// opaquely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avc444BitmapStream {
    /// Which frame(s) `stream1`/`stream2` carry.
    pub lc: Avc444LayerComposition,
    /// The YUV420 frame, or the Chroma420 frame if `lc` is `ChromaOnly`.
    pub stream1: Avc420BitmapStream,
    /// The Chroma420 frame, present only when `lc` is `LumaAndChroma`.
    pub stream2: Option<Avc420BitmapStream>,
}

impl Avc444BitmapStream {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let stream1_encoded = self.stream1.encode();
        let mut w = Writer::new();
        let info = (stream1_encoded.len() as u32 & 0x3FFF_FFFF) | (self.lc.to_lc() << 30);
        w.write_u32_le(info);
        w.write_bytes(&stream1_encoded);
        if let Some(stream2) = &self.stream2 {
            w.write_bytes(&stream2.encode());
        }
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<Avc444BitmapStream> {
        let mut r = Reader::new(buf);
        let info = r.read_u32_le()?;
        let cb_stream1 = (info & 0x3FFF_FFFF) as usize;
        let lc = Avc444LayerComposition::from_lc(info >> 30)?;

        let stream1 = if matches!(lc, Avc444LayerComposition::LumaAndChroma) {
            // Bounded to exactly cb_stream1 bytes so the reader lands at
            // the start of stream2.
            let chunk = r.read_bytes(cb_stream1)?;
            Avc420BitmapStream::decode(chunk)?
        } else {
            // Only one bitstream is present: it runs to the end of the
            // buffer regardless of what cb_stream1 claims (matching
            // FreeRDP's reference decoder, which does not bound it here
            // either).
            Avc420BitmapStream::decode(r.read_bytes(r.remaining())?)?
        };

        let stream2 = if matches!(lc, Avc444LayerComposition::LumaAndChroma) {
            Some(Avc420BitmapStream::decode(r.read_bytes(r.remaining())?)?)
        } else {
            None
        };

        Ok(Avc444BitmapStream {
            lc,
            stream1,
            stream2,
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

    #[test]
    fn point16_roundtrip_including_negative() {
        let pt = Point16 { x: -5, y: 1080 };
        let mut w = Writer::new();
        pt.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Point16::decode(&mut r).unwrap(), pt);
    }

    #[test]
    fn color32_wire_shape_is_bgra_order() {
        let color = Color32 {
            b: 0x11,
            g: 0x22,
            r: 0x33,
            xa: 0x44,
        };
        let mut w = Writer::new();
        color.encode(&mut w);
        assert_eq!(w.as_slice(), &[0x11, 0x22, 0x33, 0x44]);
        let mut reader = Reader::new(w.as_slice());
        assert_eq!(Color32::decode(&mut reader).unwrap(), color);
    }

    #[test]
    fn solid_fill_roundtrip_multiple_rects() {
        let pdu = SolidFillPdu {
            surface_id: 1,
            fill_pixel: Color32 {
                b: 0xFF,
                g: 0x00,
                r: 0x00,
                xa: 0x00,
            },
            fill_rects: vec![
                Rect16 {
                    left: 0,
                    top: 0,
                    right: 64,
                    bottom: 64,
                },
                Rect16 {
                    left: 64,
                    top: 64,
                    right: 128,
                    bottom: 128,
                },
            ],
        };
        assert_eq!(SolidFillPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn solid_fill_roundtrip_empty_rects() {
        let pdu = SolidFillPdu {
            surface_id: 1,
            fill_pixel: Color32 {
                b: 0,
                g: 0,
                r: 0,
                xa: 0,
            },
            fill_rects: vec![],
        };
        assert_eq!(SolidFillPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn surface_to_surface_roundtrip() {
        let pdu = SurfaceToSurfacePdu {
            surface_id_src: 1,
            surface_id_dest: 2,
            rect_src: Rect16 {
                left: 0,
                top: 0,
                right: 32,
                bottom: 32,
            },
            dest_pts: vec![Point16 { x: 32, y: 0 }, Point16 { x: 64, y: 0 }],
        };
        assert_eq!(SurfaceToSurfacePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn surface_to_surface_same_surface_replication() {
        // Copying within the same surface (surfaceIdSrc == surfaceIdDest)
        // is explicitly a supported use of this PDU.
        let pdu = SurfaceToSurfacePdu {
            surface_id_src: 1,
            surface_id_dest: 1,
            rect_src: Rect16 {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            },
            dest_pts: vec![Point16 { x: 16, y: 0 }],
        };
        assert_eq!(SurfaceToSurfacePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn surface_to_cache_roundtrip() {
        let pdu = SurfaceToCachePdu {
            surface_id: 1,
            cache_key: 0x0102_0304_0506_0708,
            cache_slot: 7,
            rect_src: Rect16 {
                left: 0,
                top: 0,
                right: 64,
                bottom: 64,
            },
        };
        assert_eq!(SurfaceToCachePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn cache_to_surface_roundtrip() {
        let pdu = CacheToSurfacePdu {
            cache_slot: 7,
            surface_id: 1,
            dest_pts: vec![
                Point16 { x: 0, y: 0 },
                Point16 { x: 64, y: 0 },
                Point16 { x: 0, y: 64 },
            ],
        };
        assert_eq!(CacheToSurfacePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn evict_cache_entry_roundtrip() {
        let pdu = EvictCacheEntryPdu { cache_slot: 42 };
        assert_eq!(EvictCacheEntryPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn cache_entry_metadata_roundtrip() {
        let entry = CacheEntryMetadata {
            cache_key: 0xDEAD_BEEF_0000_0001,
            bitmap_length: 4096,
        };
        let mut w = Writer::new();
        entry.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(CacheEntryMetadata::decode(&mut r).unwrap(), entry);
    }

    #[test]
    fn cache_import_offer_roundtrip_multiple() {
        let pdu = CacheImportOfferPdu {
            cache_entries: vec![
                CacheEntryMetadata {
                    cache_key: 1,
                    bitmap_length: 100,
                },
                CacheEntryMetadata {
                    cache_key: 2,
                    bitmap_length: 200,
                },
            ],
        };
        assert_eq!(CacheImportOfferPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn cache_import_offer_roundtrip_empty() {
        let pdu = CacheImportOfferPdu {
            cache_entries: vec![],
        };
        assert_eq!(CacheImportOfferPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn cache_import_reply_roundtrip() {
        let pdu = CacheImportReplyPdu {
            cache_slots: vec![0, 1, 2, 3],
        };
        assert_eq!(CacheImportReplyPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn cache_import_reply_roundtrip_empty_when_none_accepted() {
        let pdu = CacheImportReplyPdu {
            cache_slots: vec![],
        };
        assert_eq!(CacheImportReplyPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    /// Simulate a full cache round trip: the client offers a persistent
    /// cache entry, the server accepts it and later tells the client to
    /// populate a surface from it (skipping a `WireToSurface1Pdu`
    /// retransmission), then composites and evicts it — matching
    /// MS-RDPEGFX 3.2.5.2's cache import flow layered on top of the
    /// existing create/delete-surface coverage.
    #[test]
    fn full_cache_and_composition_sequence() {
        let offer = CacheImportOfferPdu {
            cache_entries: vec![CacheEntryMetadata {
                cache_key: 0xAABB_CCDD,
                bitmap_length: 16 * 16 * 4,
            }],
        };
        let decoded_offer = CacheImportOfferPdu::decode(&offer.encode()).unwrap();
        assert_eq!(decoded_offer.cache_entries.len(), 1);

        // Server accepts the single offered entry into slot 0.
        let reply = CacheImportReplyPdu {
            cache_slots: vec![0],
        };
        let decoded_reply = CacheImportReplyPdu::decode(&reply.encode()).unwrap();
        let cache_slot = decoded_reply.cache_slots[0];

        // Server also caches a freshly-drawn surface region under a new key.
        let to_cache = SurfaceToCachePdu {
            surface_id: 1,
            cache_key: decoded_offer.cache_entries[0].cache_key,
            cache_slot,
            rect_src: Rect16 {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            },
        };
        assert_eq!(
            SurfaceToCachePdu::decode(&to_cache.encode()).unwrap(),
            to_cache
        );

        // Composite it onto the destination surface at two points.
        let to_surface = CacheToSurfacePdu {
            cache_slot,
            surface_id: 2,
            dest_pts: vec![Point16 { x: 0, y: 0 }, Point16 { x: 16, y: 0 }],
        };
        let decoded_to_surface = CacheToSurfacePdu::decode(&to_surface.encode()).unwrap();
        assert_eq!(decoded_to_surface.dest_pts.len(), 2);

        // Also composite surface-to-surface directly (no cache involved).
        let surf_to_surf = SurfaceToSurfacePdu {
            surface_id_src: 2,
            surface_id_dest: 2,
            rect_src: Rect16 {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            },
            dest_pts: vec![Point16 { x: 32, y: 0 }],
        };
        assert_eq!(
            SurfaceToSurfacePdu::decode(&surf_to_surf.encode()).unwrap(),
            surf_to_surf
        );

        // Finally evict the cache slot.
        let evict = EvictCacheEntryPdu { cache_slot };
        assert_eq!(EvictCacheEntryPdu::decode(&evict.encode()).unwrap(), evict);
    }

    #[test]
    fn monitor_def_roundtrip_with_negative_bounds() {
        let m = MonitorDef {
            left: -1920,
            top: 0,
            right: -1,
            bottom: 1079,
            flags: TS_MONITOR_PRIMARY,
        };
        let mut w = Writer::new();
        m.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(MonitorDef::decode(&mut r).unwrap(), m);
    }

    #[test]
    fn reset_graphics_roundtrip_is_always_340_bytes() {
        let pdu = ResetGraphicsPdu {
            width: 1920,
            height: 1080,
            monitors: vec![MonitorDef {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1079,
                flags: TS_MONITOR_PRIMARY,
            }],
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 340);
        assert_eq!(ResetGraphicsPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn reset_graphics_wire_shape_header_and_fixed_fields() {
        let pdu = ResetGraphicsPdu {
            width: 1024,
            height: 768,
            monitors: vec![],
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 340);
        // cmdId=0x000E LE, flags=0, pduLength=340 LE.
        assert_eq!(
            &encoded[0..8],
            &[0x0E, 0x00, 0x00, 0x00, 0x54, 0x01, 0x00, 0x00]
        );
        // width=1024 LE, height=768 LE, monitorCount=0 LE.
        assert_eq!(
            &encoded[8..20],
            &[0x00, 0x04, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        // Remainder is ignored padding out to 340 bytes total.
        assert_eq!(encoded.len() - 20, 320);
    }

    #[test]
    fn reset_graphics_roundtrip_with_max_monitors() {
        let monitors: Vec<MonitorDef> = (0..16)
            .map(|i| MonitorDef {
                left: i * 1920,
                top: 0,
                right: (i + 1) * 1920 - 1,
                bottom: 1079,
                flags: if i == 0 { TS_MONITOR_PRIMARY } else { 0 },
            })
            .collect();
        let pdu = ResetGraphicsPdu {
            width: 1920 * 16,
            height: 1080,
            monitors,
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 340);
        assert_eq!(ResetGraphicsPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn reset_graphics_rejects_wrong_total_length() {
        let mut encoded = ResetGraphicsPdu {
            width: 1024,
            height: 768,
            monitors: vec![],
        }
        .encode();
        encoded.pop();
        assert!(ResetGraphicsPdu::decode(&encoded).is_err());
    }

    #[test]
    fn reset_graphics_rejects_monitor_count_over_max() {
        // Hand-craft a 340-byte PDU whose monitorCount field claims 17
        // monitors (one more than the protocol maximum), independent of
        // how many MonitorDef entries actually follow.
        let mut w = Writer::new();
        w.write_u32_le(1024);
        w.write_u32_le(768);
        w.write_u32_le(17);
        w.write_bytes(&vec![0u8; 340 - 8 - 12]);
        let encoded = wrap(CMDID_RESETGRAPHICS, w.as_slice());
        assert_eq!(encoded.len(), 340);
        assert!(ResetGraphicsPdu::decode(&encoded).is_err());
    }

    #[test]
    fn map_surface_to_output_roundtrip() {
        let pdu = MapSurfaceToOutputPdu {
            surface_id: 5,
            output_origin_x: 100,
            output_origin_y: 200,
        };
        assert_eq!(MapSurfaceToOutputPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn map_surface_to_output_wire_shape() {
        let pdu = MapSurfaceToOutputPdu {
            surface_id: 5,
            output_origin_x: 100,
            output_origin_y: 200,
        };
        assert_eq!(
            pdu.encode(),
            vec![
                0x0F, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, // header
                0x05, 0x00, 0x00, 0x00, // surfaceId, reserved
                0x64, 0x00, 0x00, 0x00, // outputOriginX = 100
                0xC8, 0x00, 0x00, 0x00, // outputOriginY = 200
            ]
        );
    }

    #[test]
    fn map_surface_to_scaled_output_roundtrip() {
        let pdu = MapSurfaceToScaledOutputPdu {
            surface_id: 5,
            output_origin_x: 100,
            output_origin_y: 200,
            target_width: 1280,
            target_height: 720,
        };
        assert_eq!(
            MapSurfaceToScaledOutputPdu::decode(&pdu.encode()).unwrap(),
            pdu
        );
    }

    #[test]
    fn map_surface_to_window_roundtrip() {
        let pdu = MapSurfaceToWindowPdu {
            surface_id: 5,
            window_id: 0x0102_0304_0506_0708,
            mapped_width: 640,
            mapped_height: 480,
        };
        assert_eq!(MapSurfaceToWindowPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn map_surface_to_scaled_window_roundtrip() {
        let pdu = MapSurfaceToScaledWindowPdu {
            surface_id: 5,
            window_id: 0x0102_0304_0506_0708,
            mapped_width: 640,
            mapped_height: 480,
            target_width: 1280,
            target_height: 960,
        };
        assert_eq!(
            MapSurfaceToScaledWindowPdu::decode(&pdu.encode()).unwrap(),
            pdu
        );
    }

    #[test]
    fn output_mapping_cmd_ids_are_distinct_and_route_correctly() {
        let reset = ResetGraphicsPdu {
            width: 640,
            height: 480,
            monitors: vec![],
        }
        .encode();
        let to_output = MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        }
        .encode();
        let to_scaled_output = MapSurfaceToScaledOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
            target_width: 640,
            target_height: 480,
        }
        .encode();
        let to_window = MapSurfaceToWindowPdu {
            surface_id: 1,
            window_id: 1,
            mapped_width: 640,
            mapped_height: 480,
        }
        .encode();
        let to_scaled_window = MapSurfaceToScaledWindowPdu {
            surface_id: 1,
            window_id: 1,
            mapped_width: 640,
            mapped_height: 480,
            target_width: 640,
            target_height: 480,
        }
        .encode();

        assert_eq!(decode_cmd_id(&reset).unwrap(), CMDID_RESETGRAPHICS);
        assert_eq!(decode_cmd_id(&to_output).unwrap(), CMDID_MAPSURFACETOOUTPUT);
        assert_eq!(
            decode_cmd_id(&to_scaled_output).unwrap(),
            CMDID_MAPSURFACETOSCALEDOUTPUT
        );
        assert_eq!(decode_cmd_id(&to_window).unwrap(), CMDID_MAPSURFACETOWINDOW);
        assert_eq!(
            decode_cmd_id(&to_scaled_window).unwrap(),
            CMDID_MAPSURFACETOSCALEDWINDOW
        );

        // Cross-decoding with the wrong PDU type is rejected.
        assert!(MapSurfaceToWindowPdu::decode(&to_output).is_err());
        assert!(MapSurfaceToOutputPdu::decode(&to_window).is_err());
    }

    #[test]
    fn avc420_quant_quality_roundtrip_including_flag_bits() {
        let qq = Avc420QuantQuality {
            qp: 0x3F,
            r: true,
            p: true,
            quality_val: 77,
        };
        let mut w = Writer::new();
        qq.encode(&mut w);
        assert_eq!(w.as_slice(), &[0xFF, 77]);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Avc420QuantQuality::decode(&mut r).unwrap(), qq);
    }

    #[test]
    fn avc420_quant_quality_zero_flags() {
        let qq = Avc420QuantQuality {
            qp: 0x2A,
            r: false,
            p: false,
            quality_val: 0,
        };
        let mut w = Writer::new();
        qq.encode(&mut w);
        assert_eq!(w.as_slice(), &[0x2A, 0]);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(Avc420QuantQuality::decode(&mut r).unwrap(), qq);
    }

    #[test]
    fn avc420_bitmap_stream_roundtrip() {
        let stream = Avc420BitmapStream {
            meta: Avc420MetaBlock {
                region_rects: vec![
                    Rect16 {
                        left: 0,
                        top: 0,
                        right: 64,
                        bottom: 64,
                    },
                    Rect16 {
                        left: 64,
                        top: 0,
                        right: 128,
                        bottom: 64,
                    },
                ],
                quant_quality_vals: vec![
                    Avc420QuantQuality {
                        qp: 22,
                        r: false,
                        p: true,
                        quality_val: 80,
                    },
                    Avc420QuantQuality {
                        qp: 30,
                        r: true,
                        p: false,
                        quality_val: 60,
                    },
                ],
            },
            bitstream: vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xAB], // fake Annex B start code + NAL.
        };
        assert_eq!(
            Avc420BitmapStream::decode(&stream.encode()).unwrap(),
            stream
        );
    }

    #[test]
    fn avc420_bitmap_stream_empty_metadata_and_bitstream_roundtrip() {
        let stream = Avc420BitmapStream {
            meta: Avc420MetaBlock {
                region_rects: vec![],
                quant_quality_vals: vec![],
            },
            bitstream: vec![],
        };
        assert_eq!(
            Avc420BitmapStream::decode(&stream.encode()).unwrap(),
            stream
        );
    }

    fn sample_avc420_stream(tag: u8) -> Avc420BitmapStream {
        Avc420BitmapStream {
            meta: Avc420MetaBlock {
                region_rects: vec![Rect16 {
                    left: 0,
                    top: 0,
                    right: 32,
                    bottom: 32,
                }],
                quant_quality_vals: vec![Avc420QuantQuality {
                    qp: 20,
                    r: false,
                    p: true,
                    quality_val: 90,
                }],
            },
            bitstream: vec![tag; 10],
        }
    }

    #[test]
    fn avc444_bitmap_stream_luma_and_chroma_roundtrip() {
        let stream = Avc444BitmapStream {
            lc: Avc444LayerComposition::LumaAndChroma,
            stream1: sample_avc420_stream(0xAA),
            stream2: Some(sample_avc420_stream(0xBB)),
        };
        assert_eq!(
            Avc444BitmapStream::decode(&stream.encode()).unwrap(),
            stream
        );
    }

    #[test]
    fn avc444_bitmap_stream_luma_only_roundtrip() {
        let stream = Avc444BitmapStream {
            lc: Avc444LayerComposition::LumaOnly,
            stream1: sample_avc420_stream(0xCC),
            stream2: None,
        };
        let encoded = stream.encode();
        let decoded = Avc444BitmapStream::decode(&encoded).unwrap();
        assert_eq!(decoded, stream);
    }

    #[test]
    fn avc444_bitmap_stream_chroma_only_roundtrip() {
        let stream = Avc444BitmapStream {
            lc: Avc444LayerComposition::ChromaOnly,
            stream1: sample_avc420_stream(0xDD),
            stream2: None,
        };
        assert_eq!(
            Avc444BitmapStream::decode(&stream.encode()).unwrap(),
            stream
        );
    }

    #[test]
    fn avc444_bitmap_stream_rejects_invalid_lc_value() {
        // info word with LC=3 (invalid) in the top 2 bits, cbAvc420EncodedBitstream1=0.
        let info: u32 = 0b11 << 30;
        let mut w = Writer::new();
        w.write_u32_le(info);
        assert!(Avc444BitmapStream::decode(w.as_slice()).is_err());
    }

    #[test]
    fn avc444_bitmap_stream_wire_shape_lc_bits_are_top_two() {
        let stream = Avc444BitmapStream {
            lc: Avc444LayerComposition::ChromaOnly,
            stream1: sample_avc420_stream(0xEE),
            stream2: None,
        };
        let encoded = stream.encode();
        let info = u32::from_le_bytes(encoded[0..4].try_into().unwrap());
        assert_eq!(info >> 30, 2);
    }

    #[test]
    fn avc444_bitmap_stream_second_bitstream_only_present_for_luma_and_chroma() {
        let luma_only = Avc444BitmapStream {
            lc: Avc444LayerComposition::LumaOnly,
            stream1: sample_avc420_stream(0x11),
            stream2: None,
        }
        .encode();
        assert!(Avc444BitmapStream::decode(&luma_only)
            .unwrap()
            .stream2
            .is_none());

        let both = Avc444BitmapStream {
            lc: Avc444LayerComposition::LumaAndChroma,
            stream1: sample_avc420_stream(0x22),
            stream2: Some(sample_avc420_stream(0x33)),
        }
        .encode();
        assert!(Avc444BitmapStream::decode(&both).unwrap().stream2.is_some());
    }
}
