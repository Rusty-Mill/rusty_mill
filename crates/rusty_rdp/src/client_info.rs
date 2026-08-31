//! Client Info PDU — `TS_INFO_PACKET` (MS-RDPBCGR 2.2.1.11.1.1).
//!
//! After security commencement the client sends its logon information:
//! domain, user name, password, the program to launch and its working
//! directory, plus assorted behaviour flags. RDP 5+ appends an *extended
//! info* block with the client's network address, time zone, and performance
//! hints.
//!
//! Strings are UTF-16LE and NUL-terminated; each has a preceding `cb*` field
//! giving its byte length **excluding** the terminator. This module always
//! sets `INFO_UNICODE`; the ANSI form is not produced.
//!
//! The `TS_INFO_PACKET` this module encodes is the *plaintext* body — it is
//! normally wrapped in a Basic Security Header with `SEC_INFO_PKT` and
//! RC4-encrypted by [`crate::security`].

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

// INFO_* flags (MS-RDPBCGR 2.2.1.11.1.1).
/// `INFO_MOUSE` — the client supports a mouse.
pub const INFO_MOUSE: u32 = 0x0000_0001;
/// `INFO_DISABLECTRLALTDEL` — Ctrl+Alt+Del is not required at logon.
pub const INFO_DISABLECTRLALTDEL: u32 = 0x0000_0002;
/// `INFO_UNICODE` — the string fields are UTF-16LE (always set here).
pub const INFO_UNICODE: u32 = 0x0000_0010;
/// `INFO_MAXIMIZESHELL` — maximise the initial application.
pub const INFO_MAXIMIZESHELL: u32 = 0x0000_0020;
/// `INFO_LOGONNOTIFY` — request a logon notification.
pub const INFO_LOGONNOTIFY: u32 = 0x0000_0040;
/// `INFO_COMPRESSION` — bulk compression is supported.
pub const INFO_COMPRESSION: u32 = 0x0000_0080;
/// `INFO_ENABLEWINDOWSKEY` — the Windows key is enabled.
pub const INFO_ENABLEWINDOWSKEY: u32 = 0x0000_0100;
/// `INFO_LOGONERRORS` — request logon error notifications.
pub const INFO_LOGONERRORS: u32 = 0x0100_0000;

/// A reasonable default flag set for an interactive Unicode client.
pub const DEFAULT_INFO_FLAGS: u32 = INFO_MOUSE
    | INFO_DISABLECTRLALTDEL
    | INFO_UNICODE
    | INFO_MAXIMIZESHELL
    | INFO_LOGONNOTIFY
    | INFO_ENABLEWINDOWSKEY;

/// `AF_INET` — the address family value for an IPv4 client address.
pub const AF_INET: u16 = 0x0002;

/// Length of the `clientTimeZone` (`TS_TIME_ZONE_INFORMATION`) field.
const TIME_ZONE_LEN: usize = 172;

/// The RDP 5+ extended info block (`TS_EXTENDED_INFO_PACKET`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedInfo {
    /// Address family (`AF_INET`).
    pub address_family: u16,
    /// Client IP address as text (may be empty).
    pub client_address: String,
    /// Client directory (may be empty).
    pub client_dir: String,
    /// `performanceFlags` (wallpaper/theme suppression, etc.).
    pub performance_flags: u32,
    /// Client session id (0 for a new session).
    pub client_session_id: u32,
}

impl Default for ExtendedInfo {
    fn default() -> Self {
        ExtendedInfo {
            address_family: AF_INET,
            client_address: String::new(),
            client_dir: String::new(),
            performance_flags: 0,
            client_session_id: 0,
        }
    }
}

impl ExtendedInfo {
    fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.address_family);
        write_utf16_field(w, &self.client_address);
        write_utf16_field(w, &self.client_dir);
        // clientTimeZone: zero-filled TS_TIME_ZONE_INFORMATION (UTC/no DST).
        w.write_bytes(&[0u8; TIME_ZONE_LEN]);
        w.write_u32_le(self.client_session_id);
        w.write_u32_le(self.performance_flags);
    }

    fn decode(r: &mut Reader<'_>) -> Result<ExtendedInfo> {
        let address_family = r.read_u16_le()?;
        let client_address = read_utf16_field(r)?;
        let client_dir = read_utf16_field(r)?;
        r.skip(TIME_ZONE_LEN)?;
        let client_session_id = r.read_u32_le()?;
        let performance_flags = r.read_u32_le()?;
        Ok(ExtendedInfo {
            address_family,
            client_address,
            client_dir,
            performance_flags,
            client_session_id,
        })
    }
}

/// The Client Info PDU body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// ANSI code page (0 selects the default).
    pub code_page: u32,
    /// `INFO_*` behaviour flags.
    pub flags: u32,
    /// Logon domain (may be empty).
    pub domain: String,
    /// Logon user name.
    pub username: String,
    /// Logon password (may be empty; often left to interactive logon).
    pub password: String,
    /// Program to launch on connect (may be empty for the default shell).
    pub alternate_shell: String,
    /// Working directory for `alternate_shell` (may be empty).
    pub working_dir: String,
    /// RDP 5+ extended info; `None` produces an RDP 4-style packet.
    pub extended: Option<ExtendedInfo>,
}

impl ClientInfo {
    /// Build a logon packet with the default flags and the RDP 5+ extended
    /// info block.
    pub fn new(domain: &str, username: &str, password: &str) -> Self {
        ClientInfo {
            code_page: 0,
            flags: DEFAULT_INFO_FLAGS,
            domain: domain.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            alternate_shell: String::new(),
            working_dir: String::new(),
            extended: Some(ExtendedInfo::default()),
        }
    }

    /// Encode the `TS_INFO_PACKET` body.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.code_page);
        w.write_u32_le(self.flags | INFO_UNICODE);

        // Byte lengths excluding the NUL terminators.
        w.write_u16_le(utf16_byte_len(&self.domain));
        w.write_u16_le(utf16_byte_len(&self.username));
        w.write_u16_le(utf16_byte_len(&self.password));
        w.write_u16_le(utf16_byte_len(&self.alternate_shell));
        w.write_u16_le(utf16_byte_len(&self.working_dir));

        write_utf16_terminated(&mut w, &self.domain);
        write_utf16_terminated(&mut w, &self.username);
        write_utf16_terminated(&mut w, &self.password);
        write_utf16_terminated(&mut w, &self.alternate_shell);
        write_utf16_terminated(&mut w, &self.working_dir);

        if let Some(ext) = &self.extended {
            ext.encode(&mut w);
        }
        w.into_vec()
    }

    /// Decode a `TS_INFO_PACKET` body.
    pub fn decode(buf: &[u8]) -> Result<ClientInfo> {
        let mut r = Reader::new(buf);
        let code_page = r.read_u32_le()?;
        let flags = r.read_u32_le()?;

        let cb_domain = r.read_u16_le()? as usize;
        let cb_username = r.read_u16_le()? as usize;
        let cb_password = r.read_u16_le()? as usize;
        let cb_shell = r.read_u16_le()? as usize;
        let cb_workdir = r.read_u16_le()? as usize;

        let domain = read_utf16_len(&mut r, cb_domain)?;
        let username = read_utf16_len(&mut r, cb_username)?;
        let password = read_utf16_len(&mut r, cb_password)?;
        let alternate_shell = read_utf16_len(&mut r, cb_shell)?;
        let working_dir = read_utf16_len(&mut r, cb_workdir)?;

        let extended = if r.remaining() > 0 {
            Some(ExtendedInfo::decode(&mut r)?)
        } else {
            None
        };

        Ok(ClientInfo {
            code_page,
            flags,
            domain,
            username,
            password,
            alternate_shell,
            working_dir,
            extended,
        })
    }
}

// ---------------------------------------------------------------------------
// UTF-16LE string helpers
// ---------------------------------------------------------------------------

/// Byte length of `s` as UTF-16LE, excluding any terminator.
fn utf16_byte_len(s: &str) -> u16 {
    (s.encode_utf16().count() * 2) as u16
}

/// Write `s` as UTF-16LE followed by a NUL terminator.
fn write_utf16_terminated(w: &mut Writer, s: &str) {
    for unit in s.encode_utf16() {
        w.write_u16_le(unit);
    }
    w.write_u16_le(0);
}

/// Read `cb` bytes of UTF-16LE plus a 2-byte NUL terminator, dropping it.
fn read_utf16_len(r: &mut Reader<'_>, cb: usize) -> Result<String> {
    let bytes = r.read_bytes(cb)?;
    let s = decode_utf16le(bytes);
    r.skip(2)?; // NUL terminator
    Ok(s)
}

/// Extended-info string field: `cb` (including terminator) then the string.
fn write_utf16_field(w: &mut Writer, s: &str) {
    // cbField counts the terminator as well.
    let cb = (s.encode_utf16().count() * 2 + 2) as u16;
    w.write_u16_le(cb);
    write_utf16_terminated(w, s);
}

fn read_utf16_field(r: &mut Reader<'_>) -> Result<String> {
    let cb = r.read_u16_le()? as usize;
    if cb < 2 {
        return Err(Error::InvalidLength {
            field: "extended info string",
            length: cb,
        });
    }
    let bytes = r.read_bytes(cb)?;
    // The field length includes the NUL terminator; drop the last unit.
    Ok(decode_utf16le(&bytes[..cb - 2]))
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_extended() {
        let info = ClientInfo::new("CORP", "alice", "s3cret");
        let bytes = info.to_vec();
        let decoded = ClientInfo::decode(&bytes).unwrap();
        assert_eq!(decoded.domain, "CORP");
        assert_eq!(decoded.username, "alice");
        assert_eq!(decoded.password, "s3cret");
        assert_eq!(decoded.extended, Some(ExtendedInfo::default()));
        // INFO_UNICODE is forced on.
        assert!(decoded.flags & INFO_UNICODE != 0);
    }

    #[test]
    fn empty_strings_roundtrip() {
        let info = ClientInfo::new("", "user", "");
        let decoded = ClientInfo::decode(&info.to_vec()).unwrap();
        assert_eq!(decoded.domain, "");
        assert_eq!(decoded.password, "");
        assert_eq!(decoded.username, "user");
    }

    #[test]
    fn cb_fields_exclude_terminator() {
        let info = ClientInfo::new("AB", "user", "");
        let bytes = info.to_vec();
        // Layout: codePage(4) flags(4) then cbDomain at offset 8.
        // "AB" is 2 UTF-16 units = 4 bytes, terminator excluded.
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 4);
    }

    #[test]
    fn rdp4_style_without_extended() {
        let mut info = ClientInfo::new("", "user", "");
        info.extended = None;
        let decoded = ClientInfo::decode(&info.to_vec()).unwrap();
        assert_eq!(decoded.extended, None);
        assert_eq!(decoded.username, "user");
    }

    #[test]
    fn unicode_content_roundtrip() {
        let info = ClientInfo::new("dömäin", "üser", "pä55");
        let decoded = ClientInfo::decode(&info.to_vec()).unwrap();
        assert_eq!(decoded.domain, "dömäin");
        assert_eq!(decoded.username, "üser");
        assert_eq!(decoded.password, "pä55");
    }
}
