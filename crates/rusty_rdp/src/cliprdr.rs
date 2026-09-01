//! Clipboard Virtual Channel Extension (MS-RDPECLIP), std-only.
//!
//! Clipboard redirection rides on the static virtual channel named
//! `"cliprdr"` (registered via [`crate::net::EstablishConfig::extra_channels`]
//! and framed by [`crate::vchan`], exactly like any other static channel —
//! unlike [`crate::gfx`]/[`crate::rfx`], it does not go through a dynamic
//! channel). This module is the wire codec for the PDUs carried on it.
//!
//! ## Initialization sequence (MS-RDPECLIP 1.3.2.1)
//!
//! 1. Server sends [`CapsPdu`] (optional — absence means default capabilities).
//! 2. Server sends [`MonitorReadyPdu`].
//! 3. Client sends [`CapsPdu`] (optional, same rule).
//! 4. Client sends [`FormatListPdu`] announcing what's on its clipboard.
//! 5. Server (or client) replies [`FormatListResponsePdu`], and may later
//!    send [`FormatDataRequestPdu`] for one of the announced formats,
//!    answered with [`FormatDataResponsePdu`].
//!
//! ## What's implemented
//!
//! The core PDUs needed for text clipboard sharing: [`MonitorReadyPdu`],
//! [`CapsPdu`]/[`GeneralCapabilitySet`], [`FormatListPdu`] (the Long Format
//! Name variant only — [`CapsPdu`] always advertises
//! [`CB_USE_LONG_FORMAT_NAMES`], which this module always sets, sidestepping
//! the ambiguous Short Format Name variant), [`FormatListResponsePdu`],
//! [`FormatDataRequestPdu`], and [`FormatDataResponsePdu`]. File copy/paste
//! is implemented too: announce files by putting [`CFSTR_FILEDESCRIPTORW`]
//! in a [`FormatListPdu`] entry, answer a [`FormatDataRequestPdu`] for it
//! with a [`FileList`] (via [`FormatDataResponsePdu::as_file_list`] on the
//! receiving end), then use [`FileContentsRequestPdu`] /
//! [`FileContentsResponsePdu`] to pull a listed file's size or byte ranges,
//! optionally bracketed by [`LockClipDataPdu`]/[`UnlockClipDataPdu`] to keep
//! its data available across clipboard changes.
//!
//! **Not yet implemented:** `CB_TEMP_DIRECTORY` and the Short Format Name
//! variant of the Format List PDU.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The static virtual channel name clipboard redirection registers under.
pub const CLIPRDR_CHANNEL_NAME: &str = "cliprdr";

// msgType values (MS-RDPECLIP 2.2.1, the CLIPRDR_HEADER's msgType field).
const CB_MONITOR_READY: u16 = 0x0001;
const CB_FORMAT_LIST: u16 = 0x0002;
const CB_FORMAT_LIST_RESPONSE: u16 = 0x0003;
const CB_FORMAT_DATA_REQUEST: u16 = 0x0004;
const CB_FORMAT_DATA_RESPONSE: u16 = 0x0005;
const CB_CLIP_CAPS: u16 = 0x0007;
const CB_FILECONTENTS_REQUEST: u16 = 0x0008;
const CB_FILECONTENTS_RESPONSE: u16 = 0x0009;
const CB_LOCK_CLIPDATA: u16 = 0x000A;
const CB_UNLOCK_CLIPDATA: u16 = 0x000B;

// msgFlags values.
const CB_RESPONSE_OK: u16 = 0x0001;
const CB_RESPONSE_FAIL: u16 = 0x0002;

// dwFlags values (CLIPRDR_FILECONTENTS_REQUEST).
const FILECONTENTS_SIZE: u32 = 0x0000_0001;
const FILECONTENTS_RANGE: u32 = 0x0000_0002;

// flags values (CLIPRDR_FILEDESCRIPTOR).
const FD_WRITESTIME: u32 = 0x0000_0020;
const FD_ATTRIBUTES: u32 = 0x0000_0004;
const FD_FILESIZE: u32 = 0x0000_0040;
const FD_SHOWPROGRESSUI: u32 = 0x0000_4000;

/// The registered Clipboard Format name for a file list (MS-RDPECLIP
/// 1.3.1.1.5): announce this in a [`FormatListPdu`] entry to offer files on
/// the clipboard, and interpret the format's data via
/// [`FormatDataResponsePdu::as_file_list`].
pub const CFSTR_FILEDESCRIPTORW: &str = "FileGroupDescriptorW";

/// Well-known Clipboard Format ID: plain ANSI text.
pub const CF_TEXT: u32 = 1;
/// Well-known Clipboard Format ID: plain UTF-16LE text.
pub const CF_UNICODETEXT: u32 = 13;

/// `CB_CAPSTYPE_GENERAL`, the only capability set type this module encodes
/// or interprets.
const CB_CAPSTYPE_GENERAL: u16 = 0x0001;
/// `CB_CAPS_VERSION_2`.
const CB_CAPS_VERSION_2: u32 = 0x0000_0002;

/// `CB_USE_LONG_FORMAT_NAMES` — this module always sets it when encoding a
/// [`GeneralCapabilitySet`], and always emits/expects the Long Format Name
/// variant of [`FormatListPdu`] regardless of what the peer advertises.
pub const CB_USE_LONG_FORMAT_NAMES: u32 = 0x0000_0002;
/// `CB_STREAM_FILECLIP_ENABLED`.
pub const CB_STREAM_FILECLIP_ENABLED: u32 = 0x0000_0004;
/// `CB_FILECLIP_NO_FILE_PATHS`.
pub const CB_FILECLIP_NO_FILE_PATHS: u32 = 0x0000_0008;
/// `CB_CAN_LOCK_CLIPDATA`.
pub const CB_CAN_LOCK_CLIPDATA: u32 = 0x0000_0010;
/// `CB_HUGE_FILE_SUPPORT_ENABLED`.
pub const CB_HUGE_FILE_SUPPORT_ENABLED: u32 = 0x0000_0020;

fn wrap(msg_type: u16, msg_flags: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(8 + body.len());
    w.write_u16_le(msg_type);
    w.write_u16_le(msg_flags);
    w.write_u32_le(body.len() as u32);
    w.write_bytes(body);
    w.into_vec()
}

/// Read the `CLIPRDR_HEADER`, check `msgType` matches `expected`, and
/// return `(msgFlags, body reader)`.
fn unwrap<'a>(buf: &'a [u8], expected: u16) -> Result<(u16, Reader<'a>)> {
    let mut r = Reader::new(buf);
    let msg_type = r.read_u16_le()?;
    let msg_flags = r.read_u16_le()?;
    let data_len = r.read_u32_le()? as usize;
    if msg_type != expected {
        return Err(Error::InvalidValue {
            field: "CLIPRDR_HEADER msgType",
            value: format!("0x{msg_type:04X} (expected 0x{expected:04X})"),
        });
    }
    if data_len != r.remaining() {
        return Err(Error::InvalidLength {
            field: "CLIPRDR_HEADER dataLen",
            length: data_len,
        });
    }
    Ok((msg_flags, r))
}

/// Peek the `msgType` of an encoded PDU without consuming it, to pick the
/// right decoder.
pub fn decode_msg_type(buf: &[u8]) -> Result<u16> {
    let mut r = Reader::new(buf);
    Ok(r.read_u16_le()?)
}

fn read_wchar_z(r: &mut Reader<'_>) -> Result<String> {
    let mut units = Vec::new();
    loop {
        let u = r.read_u16_le()?;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    Ok(String::from_utf16_lossy(&units))
}

fn write_wchar_z(w: &mut Writer, s: &str) {
    for u in s.encode_utf16() {
        w.write_u16_le(u);
    }
    w.write_u16_le(0);
}

/// `CLIPRDR_MONITOR_READY` — sent by the server once initialized, after any
/// [`CapsPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonitorReadyPdu;

impl MonitorReadyPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        wrap(CB_MONITOR_READY, 0, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MonitorReadyPdu> {
        unwrap(buf, CB_MONITOR_READY)?;
        Ok(MonitorReadyPdu)
    }
}

/// `CLIPRDR_GENERAL_CAPABILITY` (`CB_CAPSTYPE_GENERAL`) — the only
/// capability set this module builds or interprets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralCapabilitySet {
    /// `CB_CAPS_VERSION_1` or `CB_CAPS_VERSION_2`; informational only.
    pub version: u32,
    /// `CB_USE_LONG_FORMAT_NAMES` / `CB_STREAM_FILECLIP_ENABLED` / etc.
    pub general_flags: u32,
}

impl Default for GeneralCapabilitySet {
    fn default() -> Self {
        GeneralCapabilitySet {
            version: CB_CAPS_VERSION_2,
            general_flags: CB_USE_LONG_FORMAT_NAMES,
        }
    }
}

impl GeneralCapabilitySet {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u16_le(CB_CAPSTYPE_GENERAL);
        w.write_u16_le(12); // lengthCapability: this set is always 12 bytes.
        w.write_u32_le(self.version);
        w.write_u32_le(self.general_flags);
    }
}

/// One entry of `CLIPRDR_CAPS`'s `capabilitySets` array. Only the General
/// Capability Set is interpreted; any other type is preserved raw so a
/// caller can inspect it, but this module does not otherwise act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    /// `CB_CAPSTYPE_GENERAL`.
    General(GeneralCapabilitySet),
    /// Any other `capabilitySetType`, preserved as raw capability data.
    Other {
        /// The unrecognized `capabilitySetType`.
        set_type: u16,
        /// The raw bytes following `lengthCapability`.
        data: Vec<u8>,
    },
}

impl CapabilitySet {
    fn encode_into(&self, w: &mut Writer) {
        match self {
            CapabilitySet::General(g) => g.encode_into(w),
            CapabilitySet::Other { set_type, data } => {
                w.write_u16_le(*set_type);
                w.write_u16_le((4 + data.len()) as u16);
                w.write_bytes(data);
            }
        }
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<CapabilitySet> {
        let set_type = r.read_u16_le()?;
        let length = r.read_u16_le()? as usize;
        let data_len = length.checked_sub(4).ok_or(Error::InvalidLength {
            field: "CLIPRDR_CAPS_SET lengthCapability",
            length,
        })?;
        if set_type == CB_CAPSTYPE_GENERAL && data_len == 8 {
            let version = r.read_u32_le()?;
            let general_flags = r.read_u32_le()?;
            Ok(CapabilitySet::General(GeneralCapabilitySet {
                version,
                general_flags,
            }))
        } else {
            let data = r.read_bytes(data_len)?.to_vec();
            Ok(CapabilitySet::Other { set_type, data })
        }
    }
}

/// `CLIPRDR_CAPS` — exchanges capability information. Optional on the wire;
/// an endpoint that never sends one is assumed to use the default
/// [`GeneralCapabilitySet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsPdu {
    /// The advertised capability sets.
    pub sets: Vec<CapabilitySet>,
}

impl Default for CapsPdu {
    fn default() -> Self {
        CapsPdu {
            sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
        }
    }
}

impl CapsPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.sets.len() as u16);
        body.write_u16_le(0); // pad1
        for set in &self.sets {
            set.encode_into(&mut body);
        }
        wrap(CB_CLIP_CAPS, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapsPdu> {
        let (_flags, mut r) = unwrap(buf, CB_CLIP_CAPS)?;
        let count = r.read_u16_le()?;
        let _pad1 = r.read_u16_le()?;
        let mut sets = Vec::with_capacity(count as usize);
        for _ in 0..count {
            sets.push(CapabilitySet::decode_from(&mut r)?);
        }
        Ok(CapsPdu { sets })
    }

    /// The general capability set, if present (or the spec's implied
    /// default of all-zero flags/version 1 if this PDU carries no General
    /// Capability Set at all).
    pub fn general(&self) -> GeneralCapabilitySet {
        self.sets
            .iter()
            .find_map(|s| match s {
                CapabilitySet::General(g) => Some(*g),
                _ => None,
            })
            .unwrap_or(GeneralCapabilitySet {
                version: 0,
                general_flags: 0,
            })
    }
}

/// `CLIPRDR_FORMAT_LIST` (Long Format Name variant) — announces the
/// Clipboard Format ID/name pairs available on the sender's local
/// clipboard. An empty list indicates the clipboard has been emptied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatListPdu {
    /// `(formatId, name)` pairs; `name` is empty for formats with no name
    /// (encoded on the wire as a single NUL).
    pub formats: Vec<(u32, String)>,
}

impl FormatListPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        for (id, name) in &self.formats {
            body.write_u32_le(*id);
            write_wchar_z(&mut body, name);
        }
        wrap(CB_FORMAT_LIST, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatListPdu> {
        let (_flags, mut r) = unwrap(buf, CB_FORMAT_LIST)?;
        let mut formats = Vec::new();
        while !r.is_empty() {
            let id = r.read_u32_le()?;
            let name = read_wchar_z(&mut r)?;
            formats.push((id, name));
        }
        Ok(FormatListPdu { formats })
    }
}

/// `CLIPRDR_FORMAT_LIST_RESPONSE` — acknowledges a [`FormatListPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatListResponsePdu {
    /// `true` for `CB_RESPONSE_OK`, `false` for `CB_RESPONSE_FAIL`.
    pub ok: bool,
}

impl FormatListResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.ok {
            CB_RESPONSE_OK
        } else {
            CB_RESPONSE_FAIL
        };
        wrap(CB_FORMAT_LIST_RESPONSE, flags, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatListResponsePdu> {
        let (flags, _r) = unwrap(buf, CB_FORMAT_LIST_RESPONSE)?;
        Ok(FormatListResponsePdu {
            ok: flags & CB_RESPONSE_OK != 0,
        })
    }
}

/// `CLIPRDR_FORMAT_DATA_REQUEST` — requests the data for one of the
/// formats previously announced in a [`FormatListPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatDataRequestPdu {
    /// The requested Clipboard Format ID.
    pub requested_format_id: u32,
}

impl FormatDataRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.requested_format_id);
        wrap(CB_FORMAT_DATA_REQUEST, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatDataRequestPdu> {
        let (_flags, mut r) = unwrap(buf, CB_FORMAT_DATA_REQUEST)?;
        Ok(FormatDataRequestPdu {
            requested_format_id: r.read_u32_le()?,
        })
    }
}

/// `CLIPRDR_FORMAT_DATA_RESPONSE` — replies to a [`FormatDataRequestPdu`]
/// with the requested clipboard data (generic bytes; this module does not
/// interpret the Packed Metafile/Palette payload variants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataResponsePdu {
    /// `true` for `CB_RESPONSE_OK`, `false` for `CB_RESPONSE_FAIL`.
    pub ok: bool,
    /// The requested format's data (empty when `ok` is `false`).
    pub data: Vec<u8>,
}

impl FormatDataResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.ok {
            CB_RESPONSE_OK
        } else {
            CB_RESPONSE_FAIL
        };
        wrap(CB_FORMAT_DATA_RESPONSE, flags, &self.data)
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatDataResponsePdu> {
        let (flags, mut r) = unwrap(buf, CB_FORMAT_DATA_RESPONSE)?;
        let data = r.read_bytes(r.remaining())?.to_vec();
        Ok(FormatDataResponsePdu {
            ok: flags & CB_RESPONSE_OK != 0,
            data,
        })
    }

    /// Decode `data` as UTF-16LE text (`CF_UNICODETEXT`), stripping one
    /// trailing NUL terminator if present.
    pub fn as_unicode_text(&self) -> String {
        let mut units: Vec<u16> = self
            .data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16_lossy(&units)
    }

    /// Decode `data` as a [`FileList`] (`CFSTR_FILEDESCRIPTORW`).
    pub fn as_file_list(&self) -> Result<FileList> {
        FileList::decode(&self.data)
    }
}

/// `CLIPRDR_FILEDESCRIPTOR` — describes one file in a [`FileList`]. Always
/// 592 bytes on the wire; `attributes`/`last_write_time`/`file_size` are
/// `None` when the corresponding validity flag isn't set (the underlying
/// bytes are still present on the wire, but not meaningful).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDescriptor {
    /// `FILE_ATTRIBUTE_*` flags, if valid.
    pub attributes: Option<u32>,
    /// Number of 100-nanosecond intervals since 1601-01-01 of the file's
    /// last write, if valid.
    pub last_write_time: Option<u64>,
    /// File size in bytes, if valid.
    pub file_size: Option<u64>,
    /// `FD_SHOWPROGRESSUI` — whether a progress indicator should be shown
    /// while copying this file.
    pub show_progress_ui: bool,
    /// The file's name (up to 259 UTF-16 code units; longer names are
    /// truncated on encode, matching the wire format's fixed 260-code-unit
    /// budget including the NUL terminator).
    pub file_name: String,
}

impl FileDescriptor {
    fn encode(&self, w: &mut Writer) {
        let mut flags = 0u32;
        if self.attributes.is_some() {
            flags |= FD_ATTRIBUTES;
        }
        if self.last_write_time.is_some() {
            flags |= FD_WRITESTIME;
        }
        if self.file_size.is_some() {
            flags |= FD_FILESIZE;
        }
        if self.show_progress_ui {
            flags |= FD_SHOWPROGRESSUI;
        }
        w.write_u32_le(flags);
        w.write_bytes(&[0u8; 32]); // reserved1
        w.write_u32_le(self.attributes.unwrap_or(0));
        w.write_bytes(&[0u8; 16]); // reserved2
        w.write_u64_le(self.last_write_time.unwrap_or(0));
        let size = self.file_size.unwrap_or(0);
        w.write_u32_le((size >> 32) as u32); // fileSizeHigh
        w.write_u32_le(size as u32); // fileSizeLow

        let mut units: Vec<u16> = self.file_name.encode_utf16().collect();
        units.truncate(259);
        units.resize(260, 0);
        for u in units {
            w.write_u16_le(u);
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<FileDescriptor> {
        let flags = r.read_u32_le()?;
        r.read_bytes(32)?; // reserved1
        let attributes = r.read_u32_le()?;
        r.read_bytes(16)?; // reserved2
        let last_write_time = r.read_u64_le()?;
        let file_size_high = r.read_u32_le()? as u64;
        let file_size_low = r.read_u32_le()? as u64;

        let mut units = Vec::with_capacity(260);
        for _ in 0..260 {
            units.push(r.read_u16_le()?);
        }
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        let file_name = String::from_utf16_lossy(&units[..end]);

        Ok(FileDescriptor {
            attributes: (flags & FD_ATTRIBUTES != 0).then_some(attributes),
            last_write_time: (flags & FD_WRITESTIME != 0).then_some(last_write_time),
            file_size: (flags & FD_FILESIZE != 0).then_some((file_size_high << 32) | file_size_low),
            show_progress_ui: flags & FD_SHOWPROGRESSUI != 0,
            file_name,
        })
    }
}

/// `CLIPRDR_FILELIST` — the payload of a [`FormatDataResponsePdu`] for the
/// [`CFSTR_FILEDESCRIPTORW`] format: the list of files placed on the
/// clipboard by a copy operation. Not wrapped in a `CLIPRDR_HEADER` itself
/// — decode it out of a [`FormatDataResponsePdu`]'s `data` via
/// [`FormatDataResponsePdu::as_file_list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileList {
    /// The files on the clipboard, in the order later
    /// [`FileContentsRequestPdu::lindex`] values index into.
    pub files: Vec<FileDescriptor>,
}

impl FileList {
    /// Encode to bytes (suitable as a [`FormatDataResponsePdu::data`]).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.files.len() as u32);
        for f in &self.files {
            f.encode(&mut w);
        }
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FileList> {
        let mut r = Reader::new(buf);
        let count = r.read_u32_le()? as usize;
        let mut files = Vec::with_capacity(count);
        for _ in 0..count {
            files.push(FileDescriptor::decode(&mut r)?);
        }
        Ok(FileList { files })
    }
}

/// The operation a [`FileContentsRequestPdu`] asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileContentsOperation {
    /// `FILECONTENTS_SIZE` — request the file's total size.
    Size,
    /// `FILECONTENTS_RANGE` — request up to `cb_requested` bytes starting
    /// at `position`.
    Range {
        /// Byte offset into the file to start reading from.
        position: u64,
        /// Maximum number of bytes to read.
        cb_requested: u32,
    },
}

/// `CLIPRDR_FILECONTENTS_REQUEST` — requests either the size of, or a byte
/// range from, one of the files in a previously received [`FileList`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileContentsRequestPdu {
    /// Correlates this request with the matching
    /// [`FileContentsResponsePdu`].
    pub stream_id: u32,
    /// Index of the target file in the [`FileList`].
    pub lindex: i32,
    /// The requested operation.
    pub operation: FileContentsOperation,
    /// Identifies File Stream data tagged by a prior [`LockClipDataPdu`],
    /// if this request targets locked data.
    pub clip_data_id: Option<u32>,
}

impl FileContentsRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.stream_id);
        body.write_u32_le(self.lindex as u32);
        match self.operation {
            FileContentsOperation::Size => {
                body.write_u32_le(FILECONTENTS_SIZE);
                body.write_u32_le(0); // nPositionLow
                body.write_u32_le(0); // nPositionHigh
                body.write_u32_le(8); // cbRequested
            }
            FileContentsOperation::Range {
                position,
                cb_requested,
            } => {
                body.write_u32_le(FILECONTENTS_RANGE);
                body.write_u32_le(position as u32); // nPositionLow
                body.write_u32_le((position >> 32) as u32); // nPositionHigh
                body.write_u32_le(cb_requested);
            }
        }
        if let Some(clip_data_id) = self.clip_data_id {
            body.write_u32_le(clip_data_id);
        }
        wrap(CB_FILECONTENTS_REQUEST, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FileContentsRequestPdu> {
        let (_flags, mut r) = unwrap(buf, CB_FILECONTENTS_REQUEST)?;
        let stream_id = r.read_u32_le()?;
        let lindex = r.read_u32_le()? as i32;
        let dw_flags = r.read_u32_le()?;
        let position_low = r.read_u32_le()?;
        let position_high = r.read_u32_le()?;
        let cb_requested = r.read_u32_le()?;
        let operation = match dw_flags {
            FILECONTENTS_SIZE => FileContentsOperation::Size,
            FILECONTENTS_RANGE => FileContentsOperation::Range {
                position: ((position_high as u64) << 32) | position_low as u64,
                cb_requested,
            },
            other => {
                return Err(Error::InvalidValue {
                    field: "CLIPRDR_FILECONTENTS_REQUEST dwFlags",
                    value: format!("0x{other:08X}"),
                });
            }
        };
        let clip_data_id = if r.is_empty() {
            None
        } else {
            Some(r.read_u32_le()?)
        };
        Ok(FileContentsRequestPdu {
            stream_id,
            lindex,
            operation,
            clip_data_id,
        })
    }
}

/// `CLIPRDR_FILECONTENTS_RESPONSE` — replies to a [`FileContentsRequestPdu`]
/// with either the file's size (`FILECONTENTS_SIZE`, as an 8-byte
/// little-endian integer) or the requested byte range
/// (`FILECONTENTS_RANGE`); which one depends on the corresponding
/// request's [`FileContentsOperation`], which this PDU does not repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsResponsePdu {
    /// Matches the triggering [`FileContentsRequestPdu::stream_id`].
    pub stream_id: u32,
    /// `true` for `CB_RESPONSE_OK`, `false` for `CB_RESPONSE_FAIL`.
    pub ok: bool,
    /// The response payload: an 8-byte file size, a range's raw bytes, or
    /// empty on failure.
    pub data: Vec<u8>,
}

impl FileContentsResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.ok {
            CB_RESPONSE_OK
        } else {
            CB_RESPONSE_FAIL
        };
        let mut body = Writer::with_capacity(4 + self.data.len());
        body.write_u32_le(self.stream_id);
        body.write_bytes(&self.data);
        wrap(CB_FILECONTENTS_RESPONSE, flags, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FileContentsResponsePdu> {
        let (flags, mut r) = unwrap(buf, CB_FILECONTENTS_RESPONSE)?;
        let stream_id = r.read_u32_le()?;
        let data = r.read_bytes(r.remaining())?.to_vec();
        Ok(FileContentsResponsePdu {
            stream_id,
            ok: flags & CB_RESPONSE_OK != 0,
            data,
        })
    }

    /// Interpret `data` as a `FILECONTENTS_SIZE` response: an 8-byte
    /// little-endian file size.
    pub fn as_file_size(&self) -> Result<u64> {
        if self.data.len() != 8 {
            return Err(Error::InvalidLength {
                field: "CLIPRDR_FILECONTENTS_RESPONSE requestedFileContentsData",
                length: self.data.len(),
            });
        }
        let mut r = Reader::new(&self.data);
        Ok(r.read_u64_le()?)
    }
}

/// `CLIPRDR_LOCK_CLIPDATA` — requests that the peer retain File Stream data
/// tagged with `clip_data_id` until a matching [`UnlockClipDataPdu`],
/// even if its own clipboard contents change in the meantime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockClipDataPdu {
    /// Tags the File Stream data to retain.
    pub clip_data_id: u32,
}

impl LockClipDataPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.clip_data_id);
        wrap(CB_LOCK_CLIPDATA, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<LockClipDataPdu> {
        let (_flags, mut r) = unwrap(buf, CB_LOCK_CLIPDATA)?;
        Ok(LockClipDataPdu {
            clip_data_id: r.read_u32_le()?,
        })
    }
}

/// `CLIPRDR_UNLOCK_CLIPDATA` — releases a [`LockClipDataPdu`]'s hold on
/// File Stream data tagged with `clip_data_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlockClipDataPdu {
    /// Tags the File Stream data to release.
    pub clip_data_id: u32,
}

impl UnlockClipDataPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.clip_data_id);
        wrap(CB_UNLOCK_CLIPDATA, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<UnlockClipDataPdu> {
        let (_flags, mut r) = unwrap(buf, CB_UNLOCK_CLIPDATA)?;
        Ok(UnlockClipDataPdu {
            clip_data_id: r.read_u32_le()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_ready_wire_shape_and_roundtrip() {
        let pdu = MonitorReadyPdu;
        assert_eq!(
            pdu.encode(),
            vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(MonitorReadyPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn decode_msg_type_reads_without_consuming() {
        let pdu = MonitorReadyPdu.encode();
        assert_eq!(decode_msg_type(&pdu).unwrap(), CB_MONITOR_READY);
    }

    #[test]
    fn wrong_msg_type_is_rejected() {
        let pdu = MonitorReadyPdu.encode();
        assert!(FormatListPdu::decode(&pdu).is_err());
    }

    #[test]
    fn truncated_data_len_is_rejected() {
        let mut pdu = MonitorReadyPdu.encode();
        pdu[4] = 5; // claim 5 bytes of body that aren't there
        assert!(MonitorReadyPdu::decode(&pdu).is_err());
    }

    #[test]
    fn caps_pdu_default_roundtrip() {
        let pdu = CapsPdu::default();
        let decoded = CapsPdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.general().general_flags, CB_USE_LONG_FORMAT_NAMES);
    }

    #[test]
    fn caps_pdu_general_wire_shape() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::General(GeneralCapabilitySet {
                version: CB_CAPS_VERSION_2,
                general_flags: CB_USE_LONG_FORMAT_NAMES | CB_STREAM_FILECLIP_ENABLED,
            })],
        };
        let encoded = pdu.encode();
        // header(8) + cCapabilitiesSets(2) + pad1(2) + capsSet(12) = 24.
        assert_eq!(encoded.len(), 24);
        assert_eq!(&encoded[0..2], &[0x07, 0x00]); // CB_CLIP_CAPS
        assert_eq!(&encoded[8..10], &[0x01, 0x00]); // cCapabilitiesSets = 1
        assert_eq!(&encoded[12..14], &[0x01, 0x00]); // capabilitySetType = GENERAL
        assert_eq!(&encoded[14..16], &[0x0C, 0x00]); // lengthCapability = 12
    }

    #[test]
    fn caps_pdu_preserves_unknown_set() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::Other {
                set_type: 0x00FF,
                data: vec![0xAA, 0xBB, 0xCC],
            }],
        };
        assert_eq!(CapsPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn caps_pdu_no_general_set_reports_default() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::Other {
                set_type: 0x00FF,
                data: vec![],
            }],
        };
        assert_eq!(pdu.general().general_flags, 0);
    }

    #[test]
    fn format_list_roundtrip_multiple_entries() {
        let pdu = FormatListPdu {
            formats: vec![
                (CF_UNICODETEXT, String::new()),
                (CF_TEXT, String::new()),
                (0xC000, "HTML Format".to_string()),
            ],
        };
        assert_eq!(FormatListPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn format_list_empty_is_the_clipboard_emptied_signal() {
        let pdu = FormatListPdu { formats: vec![] };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 8); // header only, zero-length body.
        assert_eq!(FormatListPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn format_list_wire_shape_unnamed_format() {
        // formatId then a lone UTF-16 NUL when there's no name.
        let pdu = FormatListPdu {
            formats: vec![(CF_TEXT, String::new())],
        };
        let encoded = pdu.encode();
        assert_eq!(&encoded[8..12], &[0x01, 0x00, 0x00, 0x00]); // formatId=1 LE
        assert_eq!(&encoded[12..14], &[0x00, 0x00]); // lone NUL
    }

    #[test]
    fn format_list_response_roundtrip() {
        for ok in [true, false] {
            let pdu = FormatListResponsePdu { ok };
            assert_eq!(FormatListResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
        }
    }

    #[test]
    fn format_data_request_roundtrip() {
        let pdu = FormatDataRequestPdu {
            requested_format_id: CF_UNICODETEXT,
        };
        assert_eq!(FormatDataRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn format_data_response_roundtrip_and_unicode_text() {
        let mut data = Vec::new();
        for u in "hello".encode_utf16() {
            data.extend_from_slice(&u.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes()); // NUL terminator
        let pdu = FormatDataResponsePdu { ok: true, data };
        let decoded = FormatDataResponsePdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.as_unicode_text(), "hello");
    }

    #[test]
    fn format_data_response_failure_has_no_data() {
        let pdu = FormatDataResponsePdu {
            ok: false,
            data: vec![],
        };
        assert_eq!(FormatDataResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    /// Simulate the full initialization handshake end to end, matching
    /// MS-RDPECLIP 1.3.2.1: caps exchange, monitor ready, then a format
    /// list announcing Unicode text, answered, then the data round trip.
    #[test]
    fn full_initialization_and_text_transfer_sequence() {
        let server_caps = CapsPdu::default().encode();
        assert_eq!(decode_msg_type(&server_caps).unwrap(), CB_CLIP_CAPS);

        let monitor_ready = MonitorReadyPdu.encode();
        assert_eq!(decode_msg_type(&monitor_ready).unwrap(), CB_MONITOR_READY);

        let client_caps = CapsPdu::default().encode();
        let general = CapsPdu::decode(&client_caps).unwrap().general();
        assert_ne!(general.general_flags & CB_USE_LONG_FORMAT_NAMES, 0);

        let format_list = FormatListPdu {
            formats: vec![(CF_UNICODETEXT, String::new())],
        }
        .encode();
        let formats = FormatListPdu::decode(&format_list).unwrap().formats;
        assert_eq!(formats, vec![(CF_UNICODETEXT, String::new())]);

        let response = FormatListResponsePdu { ok: true }.encode();
        assert!(FormatListResponsePdu::decode(&response).unwrap().ok);

        let request = FormatDataRequestPdu {
            requested_format_id: CF_UNICODETEXT,
        }
        .encode();
        let requested = FormatDataRequestPdu::decode(&request)
            .unwrap()
            .requested_format_id;
        assert_eq!(requested, CF_UNICODETEXT);

        let mut text_data = Vec::new();
        for u in "clipboard test".encode_utf16() {
            text_data.extend_from_slice(&u.to_le_bytes());
        }
        text_data.extend_from_slice(&0u16.to_le_bytes());
        let data_response = FormatDataResponsePdu {
            ok: true,
            data: text_data,
        }
        .encode();
        let decoded = FormatDataResponsePdu::decode(&data_response).unwrap();
        assert_eq!(decoded.as_unicode_text(), "clipboard test");
    }

    #[test]
    fn file_descriptor_roundtrip_with_all_fields_valid() {
        let fd = FileDescriptor {
            attributes: Some(0x20), // FILE_ATTRIBUTE_ARCHIVE
            last_write_time: Some(132_000_000_000_000_000),
            file_size: Some(0x1_0000_0002),
            show_progress_ui: true,
            file_name: "notes.txt".to_string(),
        };
        let mut w = Writer::new();
        fd.encode(&mut w);
        assert_eq!(w.as_slice().len(), 592);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(FileDescriptor::decode(&mut r).unwrap(), fd);
    }

    #[test]
    fn file_descriptor_roundtrip_with_no_valid_fields() {
        let fd = FileDescriptor {
            attributes: None,
            last_write_time: None,
            file_size: None,
            show_progress_ui: false,
            file_name: "a".to_string(),
        };
        let mut w = Writer::new();
        fd.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(FileDescriptor::decode(&mut r).unwrap(), fd);
    }

    #[test]
    fn file_descriptor_long_name_is_truncated_on_encode() {
        let fd = FileDescriptor {
            attributes: None,
            last_write_time: None,
            file_size: None,
            show_progress_ui: false,
            file_name: "x".repeat(300),
        };
        let mut w = Writer::new();
        fd.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        let decoded = FileDescriptor::decode(&mut r).unwrap();
        assert_eq!(decoded.file_name.len(), 259);
    }

    #[test]
    fn file_list_roundtrip() {
        let list = FileList {
            files: vec![
                FileDescriptor {
                    attributes: None,
                    last_write_time: None,
                    file_size: Some(20),
                    show_progress_ui: false,
                    file_name: "file1.txt".to_string(),
                },
                FileDescriptor {
                    attributes: None,
                    last_write_time: None,
                    file_size: Some(10),
                    show_progress_ui: false,
                    file_name: "file2.txt".to_string(),
                },
            ],
        };
        assert_eq!(FileList::decode(&list.encode()).unwrap(), list);
    }

    #[test]
    fn file_list_empty_roundtrip() {
        let list = FileList { files: vec![] };
        assert_eq!(FileList::decode(&list.encode()).unwrap(), list);
    }

    #[test]
    fn format_data_response_as_file_list() {
        let list = FileList {
            files: vec![FileDescriptor {
                attributes: None,
                last_write_time: None,
                file_size: Some(5),
                show_progress_ui: false,
                file_name: "a.bin".to_string(),
            }],
        };
        let response = FormatDataResponsePdu {
            ok: true,
            data: list.encode(),
        };
        assert_eq!(response.as_file_list().unwrap(), list);
    }

    #[test]
    fn file_contents_request_size_roundtrip() {
        let pdu = FileContentsRequestPdu {
            stream_id: 7,
            lindex: 0,
            operation: FileContentsOperation::Size,
            clip_data_id: None,
        };
        assert_eq!(FileContentsRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn file_contents_request_range_roundtrip_with_clip_data_id() {
        let pdu = FileContentsRequestPdu {
            stream_id: 7,
            lindex: -1,
            operation: FileContentsOperation::Range {
                position: 0x1_0000_0000,
                cb_requested: 4096,
            },
            clip_data_id: Some(42),
        };
        assert_eq!(FileContentsRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn file_contents_request_rejects_invalid_dw_flags() {
        // Hand-craft a request with dwFlags = 0 (neither SIZE nor RANGE).
        let mut body = Writer::new();
        body.write_u32_le(1); // streamId
        body.write_u32_le(0); // lindex
        body.write_u32_le(0); // dwFlags (invalid)
        body.write_u32_le(0); // nPositionLow
        body.write_u32_le(0); // nPositionHigh
        body.write_u32_le(8); // cbRequested
        let pdu = wrap(CB_FILECONTENTS_REQUEST, 0, body.as_slice());
        assert!(FileContentsRequestPdu::decode(&pdu).is_err());
    }

    #[test]
    fn file_contents_response_size_roundtrip() {
        let mut data = Vec::new();
        data.extend_from_slice(&12345u64.to_le_bytes());
        let pdu = FileContentsResponsePdu {
            stream_id: 7,
            ok: true,
            data,
        };
        let decoded = FileContentsResponsePdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.as_file_size().unwrap(), 12345);
    }

    #[test]
    fn file_contents_response_range_roundtrip() {
        let pdu = FileContentsResponsePdu {
            stream_id: 7,
            ok: true,
            data: vec![1, 2, 3, 4, 5],
        };
        let decoded = FileContentsResponsePdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert!(decoded.as_file_size().is_err());
    }

    #[test]
    fn file_contents_response_failure_roundtrip() {
        let pdu = FileContentsResponsePdu {
            stream_id: 3,
            ok: false,
            data: vec![],
        };
        assert_eq!(FileContentsResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn lock_and_unlock_clip_data_roundtrip() {
        let lock = LockClipDataPdu { clip_data_id: 99 };
        assert_eq!(LockClipDataPdu::decode(&lock.encode()).unwrap(), lock);

        let unlock = UnlockClipDataPdu { clip_data_id: 99 };
        assert_eq!(UnlockClipDataPdu::decode(&unlock.encode()).unwrap(), unlock);
    }

    #[test]
    fn file_copy_paste_end_to_end_sequence() {
        // Announce a file on the clipboard.
        let announce = FormatListPdu {
            formats: vec![(100, CFSTR_FILEDESCRIPTORW.to_string())],
        };
        let decoded_announce = FormatListPdu::decode(&announce.encode()).unwrap();
        let (format_id, format_name) = &decoded_announce.formats[0];
        assert_eq!(format_name, CFSTR_FILEDESCRIPTORW);

        // Peer requests the file list for that format.
        let list_request = FormatDataRequestPdu {
            requested_format_id: *format_id,
        };
        assert_eq!(
            FormatDataRequestPdu::decode(&list_request.encode())
                .unwrap()
                .requested_format_id,
            *format_id
        );

        // Reply with a one-file list.
        let list = FileList {
            files: vec![FileDescriptor {
                attributes: None,
                last_write_time: None,
                file_size: Some(15),
                show_progress_ui: false,
                file_name: "report.pdf".to_string(),
            }],
        };
        let list_response = FormatDataResponsePdu {
            ok: true,
            data: list.encode(),
        };
        let decoded_list = FormatDataResponsePdu::decode(&list_response.encode())
            .unwrap()
            .as_file_list()
            .unwrap();
        assert_eq!(decoded_list, list);

        // Lock, then request the first 15 bytes of file index 0.
        let lock = LockClipDataPdu { clip_data_id: 1 };
        assert_eq!(LockClipDataPdu::decode(&lock.encode()).unwrap(), lock);

        let contents_request = FileContentsRequestPdu {
            stream_id: 1,
            lindex: 0,
            operation: FileContentsOperation::Range {
                position: 0,
                cb_requested: 15,
            },
            clip_data_id: Some(1),
        };
        let decoded_request = FileContentsRequestPdu::decode(&contents_request.encode()).unwrap();
        assert_eq!(decoded_request, contents_request);

        let contents_response = FileContentsResponsePdu {
            stream_id: 1,
            ok: true,
            data: b"first 15 bytes\0".to_vec(),
        };
        let decoded_response =
            FileContentsResponsePdu::decode(&contents_response.encode()).unwrap();
        assert_eq!(decoded_response.data, contents_response.data);

        let unlock = UnlockClipDataPdu { clip_data_id: 1 };
        assert_eq!(UnlockClipDataPdu::decode(&unlock.encode()).unwrap(), unlock);
    }
}
