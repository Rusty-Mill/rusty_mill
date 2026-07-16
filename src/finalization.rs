//! Connection-finalization PDUs (MS-RDPBCGR 2.2.1.14–2.2.1.22).
//!
//! After the capability exchange both peers run a short, fixed handshake that
//! marks the session fully active. Every PDU here is a Share Data PDU
//! ([`crate::pdu`]); they differ only in their `pduType2` sub-type and a small
//! fixed body.
//!
//! The client sends, in order:
//!
//! 1. **Synchronize** (`PDUTYPE2_SYNCHRONIZE`)
//! 2. **Control — Cooperate** (`PDUTYPE2_CONTROL`)
//! 3. **Control — Request Control**
//! 4. **Font List** (`PDUTYPE2_FONTLIST`)
//!
//! and the server answers with Synchronize, Control Cooperate, Control
//! **Granted Control**, and a **Font Map** (`PDUTYPE2_FONTMAP`). Once the Font
//! Map arrives the session is ready for input and output PDUs.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::pdu::{
    ShareDataHeader, PDUTYPE2_CONTROL, PDUTYPE2_FONTLIST, PDUTYPE2_FONTMAP, PDUTYPE2_SYNCHRONIZE,
};

/// `SYNCMSGTYPE_SYNC` — the only Synchronize `messageType`.
pub const SYNCMSGTYPE_SYNC: u16 = 0x0001;

// Control PDU actions (2.2.1.15.1).
/// `CTRLACTION_REQUEST_CONTROL`.
pub const CTRLACTION_REQUEST_CONTROL: u16 = 0x0001;
/// `CTRLACTION_GRANTED_CONTROL`.
pub const CTRLACTION_GRANTED_CONTROL: u16 = 0x0002;
/// `CTRLACTION_DETACH`.
pub const CTRLACTION_DETACH: u16 = 0x0003;
/// `CTRLACTION_COOPERATE`.
pub const CTRLACTION_COOPERATE: u16 = 0x0004;

// Font List / Font Map flags.
/// `FONTLIST_FIRST`.
pub const FONTLIST_FIRST: u16 = 0x0001;
/// `FONTLIST_LAST`.
pub const FONTLIST_LAST: u16 = 0x0002;

/// `TS_SYNCHRONIZE_PDU` (2.2.1.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynchronizePdu {
    /// `targetUser` — the MCS channel id of the other party.
    pub target_user: u16,
}

impl SynchronizePdu {
    /// Create a Synchronize PDU targeting `target_user`.
    pub fn new(target_user: u16) -> Self {
        SynchronizePdu { target_user }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(4);
        w.write_u16_le(SYNCMSGTYPE_SYNC);
        w.write_u16_le(self.target_user);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<SynchronizePdu> {
        let mut r = Reader::new(body);
        let _message_type = r.read_u16_le()?;
        Ok(SynchronizePdu {
            target_user: r.read_u16_le()?,
        })
    }
}

/// `TS_CONTROL_PDU` (2.2.1.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPdu {
    /// `action` (`CTRLACTION_*`).
    pub action: u16,
    /// `grantId`.
    pub grant_id: u16,
    /// `controlId`.
    pub control_id: u32,
}

impl ControlPdu {
    /// A Control Cooperate PDU.
    pub fn cooperate() -> Self {
        ControlPdu {
            action: CTRLACTION_COOPERATE,
            grant_id: 0,
            control_id: 0,
        }
    }

    /// A Control Request Control PDU (client asks for input control).
    pub fn request_control() -> Self {
        ControlPdu {
            action: CTRLACTION_REQUEST_CONTROL,
            grant_id: 0,
            control_id: 0,
        }
    }

    /// A Control Granted Control PDU (server grants input control).
    pub fn granted_control(grant_id: u16, control_id: u32) -> Self {
        ControlPdu {
            action: CTRLACTION_GRANTED_CONTROL,
            grant_id,
            control_id,
        }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(8);
        w.write_u16_le(self.action);
        w.write_u16_le(self.grant_id);
        w.write_u32_le(self.control_id);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<ControlPdu> {
        let mut r = Reader::new(body);
        Ok(ControlPdu {
            action: r.read_u16_le()?,
            grant_id: r.read_u16_le()?,
            control_id: r.read_u32_le()?,
        })
    }
}

/// `TS_FONT_LIST_PDU` (2.2.1.18). Modern clients send an empty list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontListPdu {
    /// `numberFonts` (0 for the empty list clients send).
    pub number_fonts: u16,
    /// `totalNumFonts`.
    pub total_num_fonts: u16,
    /// `listFlags`.
    pub list_flags: u16,
    /// `entrySize`.
    pub entry_size: u16,
}

impl Default for FontListPdu {
    fn default() -> Self {
        FontListPdu {
            number_fonts: 0,
            total_num_fonts: 0,
            list_flags: FONTLIST_FIRST | FONTLIST_LAST,
            entry_size: 0x0032,
        }
    }
}

impl FontListPdu {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(8);
        w.write_u16_le(self.number_fonts);
        w.write_u16_le(self.total_num_fonts);
        w.write_u16_le(self.list_flags);
        w.write_u16_le(self.entry_size);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<FontListPdu> {
        let mut r = Reader::new(body);
        Ok(FontListPdu {
            number_fonts: r.read_u16_le()?,
            total_num_fonts: r.read_u16_le()?,
            list_flags: r.read_u16_le()?,
            entry_size: r.read_u16_le()?,
        })
    }
}

/// `TS_FONT_MAP_PDU` (2.2.1.22). The server's empty map ends finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMapPdu {
    /// `numberEntries`.
    pub number_entries: u16,
    /// `totalNumEntries`.
    pub total_num_entries: u16,
    /// `mapFlags`.
    pub map_flags: u16,
    /// `entrySize`.
    pub entry_size: u16,
}

impl Default for FontMapPdu {
    fn default() -> Self {
        FontMapPdu {
            number_entries: 0,
            total_num_entries: 0,
            map_flags: FONTLIST_FIRST | FONTLIST_LAST,
            entry_size: 0x0004,
        }
    }
}

impl FontMapPdu {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(8);
        w.write_u16_le(self.number_entries);
        w.write_u16_le(self.total_num_entries);
        w.write_u16_le(self.map_flags);
        w.write_u16_le(self.entry_size);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<FontMapPdu> {
        let mut r = Reader::new(body);
        Ok(FontMapPdu {
            number_entries: r.read_u16_le()?,
            total_num_entries: r.read_u16_le()?,
            map_flags: r.read_u16_le()?,
            entry_size: r.read_u16_le()?,
        })
    }
}

/// One connection-finalization PDU, tagged by its Share Data `pduType2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationPdu {
    /// Synchronize PDU.
    Synchronize(SynchronizePdu),
    /// Control PDU (cooperate / request / granted).
    Control(ControlPdu),
    /// Font List PDU.
    FontList(FontListPdu),
    /// Font Map PDU.
    FontMap(FontMapPdu),
}

impl FinalizationPdu {
    /// The Share Data `pduType2` this PDU encodes as.
    pub fn pdu_type2(&self) -> u8 {
        match self {
            FinalizationPdu::Synchronize(_) => PDUTYPE2_SYNCHRONIZE,
            FinalizationPdu::Control(_) => PDUTYPE2_CONTROL,
            FinalizationPdu::FontList(_) => PDUTYPE2_FONTLIST,
            FinalizationPdu::FontMap(_) => PDUTYPE2_FONTMAP,
        }
    }

    fn body(&self) -> Vec<u8> {
        match self {
            FinalizationPdu::Synchronize(p) => p.encode_body(),
            FinalizationPdu::Control(p) => p.encode_body(),
            FinalizationPdu::FontList(p) => p.encode_body(),
            FinalizationPdu::FontMap(p) => p.encode_body(),
        }
    }

    /// Encode as a Share Data PDU for `share_id`, sent from `pdu_source`.
    pub fn encode(&self, share_id: u32, pdu_source: u16) -> Result<Vec<u8>> {
        let body = self.body();
        ShareDataHeader::new(share_id, self.pdu_type2(), body.len()).encode(pdu_source, &body)
    }

    /// Decode a Share Data PDU, returning `(pdu_source, share_id, pdu)`.
    ///
    /// Returns [`Error::InvalidValue`] for a `pduType2` that is not a
    /// finalization PDU.
    pub fn decode(buf: &[u8]) -> Result<(u16, u32, FinalizationPdu)> {
        let (source, header, body) = ShareDataHeader::decode(buf)?;
        let pdu = match header.pdu_type2 {
            PDUTYPE2_SYNCHRONIZE => {
                FinalizationPdu::Synchronize(SynchronizePdu::decode_body(body)?)
            }
            PDUTYPE2_CONTROL => FinalizationPdu::Control(ControlPdu::decode_body(body)?),
            PDUTYPE2_FONTLIST => FinalizationPdu::FontList(FontListPdu::decode_body(body)?),
            PDUTYPE2_FONTMAP => FinalizationPdu::FontMap(FontMapPdu::decode_body(body)?),
            other => {
                return Err(Error::InvalidValue {
                    field: "finalization pduType2",
                    value: other.to_string(),
                });
            }
        };
        Ok((source, header.share_id, pdu))
    }
}

/// The ordered finalization PDUs a client sends after Confirm Active.
pub fn client_finalization_sequence(server_channel: u16) -> [FinalizationPdu; 4] {
    [
        FinalizationPdu::Synchronize(SynchronizePdu::new(server_channel)),
        FinalizationPdu::Control(ControlPdu::cooperate()),
        FinalizationPdu::Control(ControlPdu::request_control()),
        FinalizationPdu::FontList(FontListPdu::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(pdu: FinalizationPdu) {
        let bytes = pdu.encode(0x0001_00EA, 1007).unwrap();
        let (source, share_id, decoded) = FinalizationPdu::decode(&bytes).unwrap();
        assert_eq!(source, 1007);
        assert_eq!(share_id, 0x0001_00EA);
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn synchronize_roundtrip() {
        roundtrip(FinalizationPdu::Synchronize(SynchronizePdu::new(1002)));
    }

    #[test]
    fn control_variants_roundtrip() {
        roundtrip(FinalizationPdu::Control(ControlPdu::cooperate()));
        roundtrip(FinalizationPdu::Control(ControlPdu::request_control()));
        roundtrip(FinalizationPdu::Control(ControlPdu::granted_control(
            1, 1002,
        )));
    }

    #[test]
    fn font_list_and_map_roundtrip() {
        roundtrip(FinalizationPdu::FontList(FontListPdu::default()));
        roundtrip(FinalizationPdu::FontMap(FontMapPdu::default()));
    }

    #[test]
    fn control_body_layout() {
        let pdu = FinalizationPdu::Control(ControlPdu::granted_control(0x0001, 0x0000_03EA));
        let bytes = pdu.encode(0x1234, 1002).unwrap();
        let (_, _, decoded) = FinalizationPdu::decode(&bytes).unwrap();
        let FinalizationPdu::Control(c) = decoded else {
            panic!("expected control");
        };
        assert_eq!(c.action, CTRLACTION_GRANTED_CONTROL);
        assert_eq!(c.grant_id, 1);
        assert_eq!(c.control_id, 1002);
    }

    #[test]
    fn client_sequence_is_ordered() {
        let seq = client_finalization_sequence(1002);
        assert!(matches!(seq[0], FinalizationPdu::Synchronize(_)));
        assert!(matches!(
            seq[1],
            FinalizationPdu::Control(ControlPdu {
                action: CTRLACTION_COOPERATE,
                ..
            })
        ));
        assert!(matches!(
            seq[2],
            FinalizationPdu::Control(ControlPdu {
                action: CTRLACTION_REQUEST_CONTROL,
                ..
            })
        ));
        assert!(matches!(seq[3], FinalizationPdu::FontList(_)));
    }

    #[test]
    fn decode_rejects_non_finalization_type() {
        // A Share Data PDU with pduType2 = PDUTYPE2_INPUT (28) is not one of
        // the finalization PDUs.
        let body = [0u8; 4];
        let bytes = ShareDataHeader::new(0x1234, crate::pdu::PDUTYPE2_INPUT, body.len())
            .encode(1007, &body)
            .unwrap();
        assert!(matches!(
            FinalizationPdu::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "finalization pduType2",
                ..
            }
        ));
    }
}
