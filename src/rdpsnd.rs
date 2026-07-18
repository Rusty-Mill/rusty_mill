//! Audio Output Virtual Channel Extension (MS-RDPEA), std-only.
//!
//! Audio redirection rides on the static virtual channel named `"rdpsnd"`
//! (registered via [`crate::net::EstablishConfig::extra_channels`] and
//! framed by [`crate::vchan`], like [`crate::cliprdr`]). This module is the
//! wire codec for the PDUs carried on it.
//!
//! ## Initialization sequence (MS-RDPEA 1.3.1)
//!
//! 1. Server sends [`AudioFormatsPdu`] listing its supported formats.
//! 2. Client optionally exchanges a training round: [`TrainingPdu`] /
//!    [`TrainingConfirmPdu`] (used to estimate network throughput).
//! 3. Client sends [`AudioFormatsPdu`] back with the formats both ends
//!    support, each entry's index into this list becoming the `format_no`
//!    used by later Wave PDUs.
//! 4. Server streams audio: [`encode_wave`] splits samples into the
//!    WaveInfo + Wave PDU pair (the first 4 bytes of every sample block are
//!    carried inside the WaveInfo PDU itself, a wire quirk this module
//!    hides — [`decode_wave`] reassembles the two back into one buffer).
//!    The client answers each with a [`WaveConfirmPdu`] once played out.
//! 5. Either side may send [`ClosePdu`] to end the stream.
//!
//! ## What's implemented
//!
//! The full non-UDP audio path: [`AudioFormat`]/[`AudioFormatsPdu`],
//! [`TrainingPdu`]/[`TrainingConfirmPdu`], [`encode_wave`]/[`decode_wave`]
//! (the WaveInfo/Wave PDU pair), [`WaveConfirmPdu`], [`ClosePdu`],
//! [`VolumePdu`]/[`PitchPdu`], and [`CryptKeyPdu`] (the key-distribution
//! PDU, sent over this virtual channel even though the audio data it keys
//! is not — see below).
//!
//! **Not yet implemented:** `SNDC_WAVE2` (the newer single-PDU wave format
//! with an explicit `dwAudioTimeStamp`), and everything that rides over
//! UDP rather than this virtual channel: `SNDC_WAVEENCRYPT` (the encrypted
//! wave data [`CryptKeyPdu`] keys) and `SNDC_UDPWAVE`/`UDPWAVELAST`.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The static virtual channel name audio redirection registers under.
pub const RDPSND_CHANNEL_NAME: &str = "rdpsnd";

// msgType values (MS-RDPEA 2.2.1, the SNDPROLOG header's msgType field).
const SNDC_CLOSE: u8 = 0x01;
const SNDC_WAVE: u8 = 0x02;
const SNDC_SETVOLUME: u8 = 0x03;
const SNDC_SETPITCH: u8 = 0x04;
const SNDC_WAVECONFIRM: u8 = 0x05;
const SNDC_TRAINING: u8 = 0x06;
const SNDC_FORMATS: u8 = 0x07;
const SNDC_CRYPTKEY: u8 = 0x08;

/// `WAVE_FORMAT_PCM` — the one format clients and servers are required to
/// support at minimum.
pub const WAVE_FORMAT_PCM: u16 = 0x0001;

/// `TSSNDCAPS_ALIVE` — required for any audio data to flow.
pub const TSSNDCAPS_ALIVE: u32 = 0x0000_0001;
/// `TSSNDCAPS_VOLUME`.
pub const TSSNDCAPS_VOLUME: u32 = 0x0000_0002;
/// `TSSNDCAPS_PITCH`.
pub const TSSNDCAPS_PITCH: u32 = 0x0000_0004;

fn wrap(msg_type: u8, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(4 + body.len());
    w.write_u8(msg_type);
    w.write_u8(0); // bPad
    w.write_u16_le(body.len() as u16);
    w.write_bytes(body);
    w.into_vec()
}

/// Read the `SNDPROLOG` header, check `msgType` matches `expected`, and
/// return a reader positioned at the start of the body. `BodySize` is
/// validated against the buffer for every message type except
/// `SNDC_WAVE`, whose `BodySize` has different semantics handled
/// separately by [`decode_wave`].
fn unwrap<'a>(buf: &'a [u8], expected: u8) -> Result<Reader<'a>> {
    let mut r = Reader::new(buf);
    let msg_type = r.read_u8()?;
    let _pad = r.read_u8()?;
    let body_size = r.read_u16_le()? as usize;
    if msg_type != expected {
        return Err(Error::InvalidValue {
            field: "SNDPROLOG msgType",
            value: format!("0x{msg_type:02X} (expected 0x{expected:02X})"),
        });
    }
    if msg_type != SNDC_WAVE && body_size != r.remaining() {
        return Err(Error::InvalidLength {
            field: "SNDPROLOG BodySize",
            length: body_size,
        });
    }
    Ok(r)
}

/// Peek the `msgType` of an encoded PDU without consuming it, to pick the
/// right decoder. Returns `SNDC_WAVE` (0x02) for a WaveInfo PDU; the
/// trailing Wave PDU has no header of its own (see [`decode_wave`]).
pub fn decode_msg_type(buf: &[u8]) -> Result<u8> {
    let mut r = Reader::new(buf);
    r.read_u8()
}

/// `AUDIO_FORMAT` — describes one supported audio format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormat {
    /// A WAVE form Registration Number ([RFC 2361]); `WAVE_FORMAT_PCM` at minimum.
    pub format_tag: u16,
    /// Number of channels.
    pub channels: u16,
    /// Samples per second.
    pub samples_per_sec: u32,
    /// Average bytes per second of encoded audio.
    pub avg_bytes_per_sec: u32,
    /// Minimum atomic unit of audio data for this format.
    pub block_align: u16,
    /// Bits needed to represent one sample.
    pub bits_per_sample: u16,
    /// Format-specific extra data (e.g. ADPCM coefficients).
    pub extra: Vec<u8>,
}

impl AudioFormat {
    /// A common CD-quality stereo 16-bit PCM format.
    pub fn pcm_stereo_44100_16() -> AudioFormat {
        AudioFormat {
            format_tag: WAVE_FORMAT_PCM,
            channels: 2,
            samples_per_sec: 44100,
            avg_bytes_per_sec: 44100 * 4,
            block_align: 4,
            bits_per_sample: 16,
            extra: Vec::new(),
        }
    }

    fn encode_into(&self, w: &mut Writer) {
        w.write_u16_le(self.format_tag);
        w.write_u16_le(self.channels);
        w.write_u32_le(self.samples_per_sec);
        w.write_u32_le(self.avg_bytes_per_sec);
        w.write_u16_le(self.block_align);
        w.write_u16_le(self.bits_per_sample);
        w.write_u16_le(self.extra.len() as u16);
        w.write_bytes(&self.extra);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<AudioFormat> {
        let format_tag = r.read_u16_le()?;
        let channels = r.read_u16_le()?;
        let samples_per_sec = r.read_u32_le()?;
        let avg_bytes_per_sec = r.read_u32_le()?;
        let block_align = r.read_u16_le()?;
        let bits_per_sample = r.read_u16_le()?;
        let cb_size = r.read_u16_le()? as usize;
        let extra = r.read_bytes(cb_size)?.to_vec();
        Ok(AudioFormat {
            format_tag,
            channels,
            samples_per_sec,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
            extra,
        })
    }
}

/// `SERVER_AUDIO_VERSION_AND_FORMATS` / `CLIENT_AUDIO_VERSION_AND_FORMATS`
/// — both directions share this shape (`SNDC_FORMATS`); which one a given
/// PDU is follows from the connection role, not anything in the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormatsPdu {
    /// `TSSNDCAPS_*` capability flags.
    pub flags: u32,
    /// Initial volume (left in the low word, right in the high word);
    /// meaningful only when `TSSNDCAPS_VOLUME` is set.
    pub volume: u32,
    /// Initial pitch multiplier, 16.16 fixed point; meaningful only when
    /// `TSSNDCAPS_PITCH` is set.
    pub pitch: u32,
    /// Client UDP port for audio, big-endian on the wire; 0 if unsupported.
    pub dgram_port: u16,
    /// Protocol version.
    pub version: u16,
    /// The offered/accepted audio formats.
    pub formats: Vec<AudioFormat>,
}

impl Default for AudioFormatsPdu {
    fn default() -> Self {
        AudioFormatsPdu {
            flags: TSSNDCAPS_ALIVE,
            volume: 0,
            pitch: 0,
            dgram_port: 0,
            version: 6,
            formats: vec![AudioFormat::pcm_stereo_44100_16()],
        }
    }
}

impl AudioFormatsPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.flags);
        body.write_u32_le(self.volume);
        body.write_u32_le(self.pitch);
        body.write_u16_be(self.dgram_port);
        body.write_u16_le(self.formats.len() as u16);
        body.write_u8(0); // cLastBlockConfirmed / cLastSentPacket, unused.
        body.write_u16_le(self.version);
        body.write_u8(0); // bPad
        for f in &self.formats {
            f.encode_into(&mut body);
        }
        wrap(SNDC_FORMATS, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<AudioFormatsPdu> {
        let mut r = unwrap(buf, SNDC_FORMATS)?;
        let flags = r.read_u32_le()?;
        let volume = r.read_u32_le()?;
        let pitch = r.read_u32_le()?;
        let dgram_port = r.read_u16_be()?;
        let count = r.read_u16_le()?;
        let _last_confirmed = r.read_u8()?;
        let version = r.read_u16_le()?;
        let _pad = r.read_u8()?;
        let mut formats = Vec::with_capacity(count as usize);
        for _ in 0..count {
            formats.push(AudioFormat::decode_from(&mut r)?);
        }
        Ok(AudioFormatsPdu {
            flags,
            volume,
            pitch,
            dgram_port,
            version,
            formats,
        })
    }
}

/// `SNDCLOSE` — ends the audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClosePdu;

impl ClosePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        wrap(SNDC_CLOSE, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClosePdu> {
        unwrap(buf, SNDC_CLOSE)?;
        Ok(ClosePdu)
    }
}

/// `SNDVOL` — sent by the server to set the volume applied to all
/// subsequently played audio data. Only sent if the client advertised
/// [`TSSNDCAPS_VOLUME`] in its [`AudioFormatsPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumePdu {
    /// Left-channel volume; `0x0000` is silence, `0xFFFF` is full volume,
    /// interpreted logarithmically.
    pub left: u16,
    /// Right-channel volume, same scale as `left`.
    pub right: u16,
}

impl VolumePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.left);
        body.write_u16_le(self.right);
        wrap(SNDC_SETVOLUME, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<VolumePdu> {
        let mut r = unwrap(buf, SNDC_SETVOLUME)?;
        Ok(VolumePdu {
            left: r.read_u16_le()?,
            right: r.read_u16_le()?,
        })
    }
}

/// `SNDPITCH` — sent by the server to set the pitch applied to all
/// subsequently played audio data. Only sent if the client advertised
/// [`TSSNDCAPS_PITCH`] in its [`AudioFormatsPdu`]; per MS-RDPEA, the client
/// MUST ignore this PDU regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PitchPdu {
    /// Signed integer part of the fixed-point pitch multiplier.
    pub integer_part: u16,
    /// Fractional part of the fixed-point pitch multiplier; `0x8000` is
    /// one-half, `0x4000` is one-quarter.
    pub fractional_part: u16,
}

impl PitchPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.fractional_part);
        body.write_u16_le(self.integer_part);
        wrap(SNDC_SETPITCH, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<PitchPdu> {
        let mut r = unwrap(buf, SNDC_SETPITCH)?;
        let fractional_part = r.read_u16_le()?;
        let integer_part = r.read_u16_le()?;
        Ok(PitchPdu {
            integer_part,
            fractional_part,
        })
    }
}

/// `SNDCRYPT` — sent by the server to distribute the symmetric key used to
/// encrypt audio data sent over UDP (Wave Encrypt / UDP Wave PDUs, neither
/// implemented by this module — see the module-level docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CryptKeyPdu {
    /// The 32-byte symmetric key.
    pub seed: [u8; 32],
}

impl CryptKeyPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(0); // Reserved, MUST be ignored on receipt.
        body.write_bytes(&self.seed);
        wrap(SNDC_CRYPTKEY, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CryptKeyPdu> {
        let mut r = unwrap(buf, SNDC_CRYPTKEY)?;
        let _reserved = r.read_u32_le()?;
        let seed: [u8; 32] = r.read_bytes(32)?.try_into().unwrap();
        Ok(CryptKeyPdu { seed })
    }
}

/// `SNDTRAINING` — sent by the server to help the client estimate
/// available bandwidth; `data` is `pack_size` bytes of arbitrary filler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPdu {
    /// Echoed back verbatim in [`TrainingConfirmPdu`].
    pub timestamp: u16,
    /// Size, in bytes, of `data` — the "pack size" being timed.
    pub pack_size: u16,
    /// Filler payload, `pack_size` bytes.
    pub data: Vec<u8>,
}

impl TrainingPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.timestamp);
        body.write_u16_le(self.pack_size);
        body.write_bytes(&self.data);
        wrap(SNDC_TRAINING, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<TrainingPdu> {
        let mut r = unwrap(buf, SNDC_TRAINING)?;
        let timestamp = r.read_u16_le()?;
        let pack_size = r.read_u16_le()?;
        let data = r.read_bytes(r.remaining())?.to_vec();
        Ok(TrainingPdu {
            timestamp,
            pack_size,
            data,
        })
    }
}

/// `SNDTRAININGCONFIRM` — the client's reply to a [`TrainingPdu`], echoing
/// its `timestamp`/`pack_size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainingConfirmPdu {
    /// Copied from the [`TrainingPdu`] being confirmed.
    pub timestamp: u16,
    /// Copied from the [`TrainingPdu`] being confirmed.
    pub pack_size: u16,
}

impl TrainingConfirmPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.timestamp);
        body.write_u16_le(self.pack_size);
        // SNDC_TRAINING is shared between the Training PDU and this reply;
        // BodySize is validated normally here since this is not SNDC_WAVE.
        wrap(SNDC_TRAINING, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<TrainingConfirmPdu> {
        let mut r = unwrap(buf, SNDC_TRAINING)?;
        Ok(TrainingConfirmPdu {
            timestamp: r.read_u16_le()?,
            pack_size: r.read_u16_le()?,
        })
    }
}

/// `SNDWAVECONFIRM` — sent by the client once it has finished playing a
/// wave sample delivered via [`encode_wave`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveConfirmPdu {
    /// Timestamp for latency measurement (see MS-RDPEA 3.2.5.2.1.6).
    pub timestamp: u16,
    /// Must equal the `block_no` of the WaveInfo PDU being confirmed.
    pub confirmed_block_no: u8,
}

impl WaveConfirmPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.timestamp);
        body.write_u8(self.confirmed_block_no);
        body.write_u8(0); // bPad
        wrap(SNDC_WAVECONFIRM, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<WaveConfirmPdu> {
        let mut r = unwrap(buf, SNDC_WAVECONFIRM)?;
        let timestamp = r.read_u16_le()?;
        let confirmed_block_no = r.read_u8()?;
        let _pad = r.read_u8()?;
        Ok(WaveConfirmPdu {
            timestamp,
            confirmed_block_no,
        })
    }
}

/// Split one audio sample into the wire's WaveInfo + Wave PDU pair
/// (MS-RDPEA 2.2.3.3/2.2.3.4): `sample` must be at least 4 bytes (a Wave
/// PDU always exists, even if empty, per the format's minimum framing).
/// `format_no` indexes the format list exchanged during initialization.
///
/// Returns `(wave_info_pdu_bytes, wave_pdu_bytes)` — send both, back to
/// back, on the channel.
pub fn encode_wave(
    timestamp: u16,
    format_no: u16,
    block_no: u8,
    sample: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    if sample.len() < 4 {
        return Err(Error::InvalidLength {
            field: "RDPSND wave sample",
            length: sample.len(),
        });
    }
    let mut info_body = Writer::new();
    info_body.write_u16_le(timestamp);
    info_body.write_u16_le(format_no);
    info_body.write_u8(block_no);
    info_body.write_bytes(&[0, 0, 0]); // bPad
    info_body.write_bytes(&sample[..4]);

    // BodySize for SNDC_WAVE is the *total* sample length (MS-RDPEA
    // 2.2.3.3), not this PDU's own body length like every other message.
    let mut info_pdu = Writer::with_capacity(4 + info_body.len());
    info_pdu.write_u8(SNDC_WAVE);
    info_pdu.write_u8(0);
    info_pdu.write_u16_le(sample.len() as u16);
    info_pdu.write_bytes(info_body.as_slice());

    let mut wave_pdu = Writer::with_capacity(4 + sample.len() - 4);
    wave_pdu.write_u32_le(0); // bPad
    wave_pdu.write_bytes(&sample[4..]);

    Ok((info_pdu.into_vec(), wave_pdu.into_vec()))
}

/// The decoded fields of a WaveInfo PDU: `(timestamp, format_no, block_no,
/// total_sample_len, first 4 bytes of the sample)`.
struct WaveInfoHeader {
    timestamp: u16,
    format_no: u16,
    block_no: u8,
    total_len: usize,
    first4: [u8; 4],
}

fn decode_wave_info(buf: &[u8]) -> Result<WaveInfoHeader> {
    let mut r = Reader::new(buf);
    let msg_type = r.read_u8()?;
    let _pad = r.read_u8()?;
    let total_len = r.read_u16_le()? as usize;
    if msg_type != SNDC_WAVE {
        return Err(Error::InvalidValue {
            field: "SNDPROLOG msgType",
            value: format!("0x{msg_type:02X} (expected 0x{SNDC_WAVE:02X})"),
        });
    }
    let timestamp = r.read_u16_le()?;
    let format_no = r.read_u16_le()?;
    let block_no = r.read_u8()?;
    r.skip(3)?; // bPad
    let first4: [u8; 4] = r.read_bytes(4)?.try_into().unwrap();
    Ok(WaveInfoHeader {
        timestamp,
        format_no,
        block_no,
        total_len,
        first4,
    })
}

/// Reassemble a WaveInfo + Wave PDU pair produced by [`encode_wave`] back
/// into `(timestamp, format_no, block_no, sample)`.
pub fn decode_wave(wave_info_pdu: &[u8], wave_pdu: &[u8]) -> Result<(u16, u16, u8, Vec<u8>)> {
    let info = decode_wave_info(wave_info_pdu)?;
    let mut r = Reader::new(wave_pdu);
    let _bpad = r.read_u32_le()?;
    let rest = r.read_bytes(r.remaining())?;
    let expected_rest = info.total_len.checked_sub(4).ok_or(Error::InvalidLength {
        field: "SNDWAVINFO BodySize",
        length: info.total_len,
    })?;
    if rest.len() != expected_rest {
        return Err(Error::InvalidLength {
            field: "SNDWAV data",
            length: rest.len(),
        });
    }
    let mut sample = Vec::with_capacity(info.total_len);
    sample.extend_from_slice(&info.first4);
    sample.extend_from_slice(rest);
    Ok((info.timestamp, info.format_no, info.block_no, sample))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_format_roundtrip() {
        let f = AudioFormat {
            format_tag: 0x0002, // WAVE_FORMAT_ADPCM
            channels: 2,
            samples_per_sec: 22050,
            avg_bytes_per_sec: 22311,
            block_align: 1024,
            bits_per_sample: 4,
            extra: vec![0xF4, 0x03, 0x07, 0x00],
        };
        let mut w = Writer::new();
        f.encode_into(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(AudioFormat::decode_from(&mut r).unwrap(), f);
    }

    #[test]
    fn audio_formats_pdu_default_roundtrip() {
        let pdu = AudioFormatsPdu::default();
        assert_eq!(AudioFormatsPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn audio_formats_pdu_matches_spec_wire_dump_fields() {
        // Cross-checked against the MS-RDPEA worked example: the first
        // format entry is 2-channel 16-bit PCM at 22050 Hz.
        let pdu = AudioFormatsPdu {
            flags: 0x008b_fb08,
            volume: 0x0009_f1e0,
            pitch: 0x771f_2770,
            dgram_port: 0,
            version: 5,
            formats: vec![AudioFormat {
                format_tag: WAVE_FORMAT_PCM,
                channels: 2,
                samples_per_sec: 22050,
                avg_bytes_per_sec: 88200,
                block_align: 4,
                bits_per_sample: 16,
                extra: vec![],
            }],
        };
        let decoded = AudioFormatsPdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.formats[0].samples_per_sec, 22050);
    }

    #[test]
    fn close_pdu_wire_shape_and_roundtrip() {
        let pdu = ClosePdu;
        assert_eq!(pdu.encode(), vec![0x01, 0x00, 0x00, 0x00]);
        assert_eq!(ClosePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn decode_msg_type_reads_without_consuming() {
        let pdu = ClosePdu.encode();
        assert_eq!(decode_msg_type(&pdu).unwrap(), SNDC_CLOSE);
    }

    #[test]
    fn wrong_msg_type_is_rejected() {
        let pdu = ClosePdu.encode();
        assert!(AudioFormatsPdu::decode(&pdu).is_err());
    }

    #[test]
    fn training_roundtrip() {
        let pdu = TrainingPdu {
            timestamp: 0x89da,
            pack_size: 1024,
            data: vec![0x42; 1024],
        };
        assert_eq!(TrainingPdu::decode(&pdu.encode()).unwrap(), pdu);

        let confirm = TrainingConfirmPdu {
            timestamp: 0x89da,
            pack_size: 1024,
        };
        assert_eq!(
            TrainingConfirmPdu::decode(&confirm.encode()).unwrap(),
            confirm
        );
    }

    #[test]
    fn wave_confirm_roundtrip() {
        let pdu = WaveConfirmPdu {
            timestamp: 0x1234,
            confirmed_block_no: 8,
        };
        assert_eq!(WaveConfirmPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn encode_wave_rejects_too_short_sample() {
        assert!(encode_wave(0, 0, 0, &[1, 2, 3]).is_err());
    }

    #[test]
    fn wave_roundtrip_matches_spec_dump_shape() {
        // WaveInfo PDU carries the first 4 bytes; Wave PDU the rest, per
        // the MS-RDPEA worked example (593-byte total sample).
        let sample: Vec<u8> = (0..593u32).map(|i| (i % 251) as u8).collect();
        let (info, wave) = encode_wave(0xadd7, 15, 8, &sample).unwrap();

        assert_eq!(decode_msg_type(&info).unwrap(), SNDC_WAVE);
        // header(4) + timestamp(2) + formatNo(2) + blockNo(1) + pad(3) + 4 = 16
        assert_eq!(info.len(), 16);

        let (timestamp, format_no, block_no, decoded) = decode_wave(&info, &wave).unwrap();
        assert_eq!(timestamp, 0xadd7);
        assert_eq!(format_no, 15);
        assert_eq!(block_no, 8);
        assert_eq!(decoded, sample);
    }

    #[test]
    fn wave_roundtrip_minimal_four_byte_sample() {
        let sample = [0xDE, 0xAD, 0xBE, 0xEF];
        let (info, wave) = encode_wave(1, 0, 0, &sample).unwrap();
        let (_, _, _, decoded) = decode_wave(&info, &wave).unwrap();
        assert_eq!(decoded, sample);
    }

    #[test]
    fn decode_wave_rejects_mismatched_pair() {
        let sample = vec![0xAA; 100];
        let (info, _wave) = encode_wave(0, 0, 0, &sample).unwrap();
        let (_other_info, other_wave) = encode_wave(0, 0, 0, &[0xBB; 50]).unwrap();
        assert!(decode_wave(&info, &other_wave).is_err());
    }

    /// Simulate the full non-UDP initialization + one audio sample round
    /// trip, matching MS-RDPEA 1.3.1: server formats, training, client
    /// formats, then a WaveInfo/Wave pair confirmed by the client.
    #[test]
    fn full_initialization_and_wave_transfer_sequence() {
        let server_formats = AudioFormatsPdu::default().encode();
        assert_eq!(decode_msg_type(&server_formats).unwrap(), SNDC_FORMATS);

        let training = TrainingPdu {
            timestamp: 100,
            pack_size: 8,
            data: vec![0; 8],
        }
        .encode();
        let t = TrainingPdu::decode(&training).unwrap();
        let confirm = TrainingConfirmPdu {
            timestamp: t.timestamp,
            pack_size: t.pack_size,
        }
        .encode();
        assert_eq!(
            TrainingConfirmPdu::decode(&confirm).unwrap(),
            TrainingConfirmPdu {
                timestamp: 100,
                pack_size: 8
            }
        );

        let client_formats = AudioFormatsPdu::default().encode();
        let formats = AudioFormatsPdu::decode(&client_formats).unwrap().formats;
        assert_eq!(formats.len(), 1);

        let sample: Vec<u8> = (0..2000u32).map(|i| (i % 200) as u8).collect();
        let (info, wave) = encode_wave(42, 0, 3, &sample).unwrap();
        let (_ts, format_no, block_no, decoded) = decode_wave(&info, &wave).unwrap();
        assert_eq!(format_no, 0);
        assert_eq!(decoded, sample);

        let confirm = WaveConfirmPdu {
            timestamp: 42,
            confirmed_block_no: block_no,
        }
        .encode();
        assert_eq!(
            WaveConfirmPdu::decode(&confirm).unwrap().confirmed_block_no,
            3
        );
    }

    #[test]
    fn volume_pdu_roundtrip() {
        let pdu = VolumePdu {
            left: 0xC000,
            right: 0x8000,
        };
        assert_eq!(VolumePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn volume_pdu_wire_shape_low_word_is_left_channel() {
        let pdu = VolumePdu {
            left: 0x1234,
            right: 0x5678,
        };
        let encoded = pdu.encode();
        // Header (msgType=0x03, pad=0, BodySize=4 LE), then left LE, right LE.
        assert_eq!(
            encoded,
            vec![0x03, 0x00, 0x04, 0x00, 0x34, 0x12, 0x78, 0x56]
        );
    }

    #[test]
    fn volume_full_and_silent_roundtrip() {
        let full = VolumePdu {
            left: 0xFFFF,
            right: 0xFFFF,
        };
        assert_eq!(VolumePdu::decode(&full.encode()).unwrap(), full);

        let silent = VolumePdu { left: 0, right: 0 };
        assert_eq!(VolumePdu::decode(&silent.encode()).unwrap(), silent);
    }

    #[test]
    fn pitch_pdu_roundtrip() {
        let pdu = PitchPdu {
            integer_part: 1,
            fractional_part: 0,
        };
        assert_eq!(PitchPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn pitch_pdu_no_change_multiplier_matches_spec_example() {
        // MS-RDPEA: 0x00010000 == a multiplier of 1.0 (no pitch change).
        let pdu = PitchPdu {
            integer_part: 1,
            fractional_part: 0,
        };
        let encoded = pdu.encode();
        let dw_pitch = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
        assert_eq!(dw_pitch, 0x0001_0000);
    }

    #[test]
    fn pitch_pdu_fifteen_point_five_matches_spec_example() {
        // MS-RDPEA: 0x000F8000 == a multiplier of 15.5.
        let pdu = PitchPdu {
            integer_part: 15,
            fractional_part: 0x8000,
        };
        let encoded = pdu.encode();
        let dw_pitch = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
        assert_eq!(dw_pitch, 0x000F_8000);
        assert_eq!(PitchPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn crypt_key_pdu_roundtrip() {
        let mut seed = [0u8; 32];
        for (i, b) in seed.iter_mut().enumerate() {
            *b = i as u8;
        }
        let pdu = CryptKeyPdu { seed };
        assert_eq!(CryptKeyPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn crypt_key_pdu_wire_shape_has_reserved_field_before_seed() {
        let pdu = CryptKeyPdu { seed: [0xAB; 32] };
        let encoded = pdu.encode();
        // Header(4) + Reserved(4) + Seed(32) = 40 bytes.
        assert_eq!(encoded.len(), 40);
        assert_eq!(&encoded[4..8], &[0, 0, 0, 0]); // Reserved, always encoded as zero.
        assert_eq!(&encoded[8..40], &[0xAB; 32]);
    }

    #[test]
    fn volume_pitch_cryptkey_msg_types_are_distinct() {
        let volume = VolumePdu { left: 1, right: 1 }.encode();
        let pitch = PitchPdu {
            integer_part: 1,
            fractional_part: 0,
        }
        .encode();
        let crypt_key = CryptKeyPdu { seed: [0; 32] }.encode();

        assert_eq!(decode_msg_type(&volume).unwrap(), SNDC_SETVOLUME);
        assert_eq!(decode_msg_type(&pitch).unwrap(), SNDC_SETPITCH);
        assert_eq!(decode_msg_type(&crypt_key).unwrap(), SNDC_CRYPTKEY);

        // Cross-decoding with the wrong PDU type is rejected.
        assert!(VolumePdu::decode(&pitch).is_err());
        assert!(PitchPdu::decode(&crypt_key).is_err());
    }
}
