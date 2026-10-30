//! Typed decode and encode of the packets ConDrv delivers.
//!
//! Function codes, API numbers, NTSTATUS values and message layouts are
//! transcribed from microsoft/terminal at commit 09beddc6
//! (`dep/Console/condrv.h`, `conmsgl1.h`, `conmsgl2.h`,
//! `src/server/ApiMessage.h`, `IoSorter.cpp`, `ApiSorter.cpp`,
//! `precomp.h`; see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.
//!
//! A packet's variable input, read from offset 0, starts with what the
//! function carries: `CONSOLE_SERVER_MSG` for CONNECT,
//! `CD_CREATE_OBJECT_INFORMATION` for CREATE_OBJECT, the raw bytes for
//! RAW_WRITE, and for USER_DEFINED a `CONSOLE_MSG_HEADER`, the API's
//! fixed struct (`ApiDescriptorSize` bytes), then its variable data. The
//! fixed struct goes back through `CD_IO_COMPLETE.Write`; variable output
//! is written at offset `ApiDescriptorSize`.

use rusty_wire::Reader;

pub(crate) const CONNECT: u32 = 1;
pub(crate) const DISCONNECT: u32 = 2;
pub(crate) const CREATE_OBJECT: u32 = 3;
pub(crate) const CLOSE_OBJECT: u32 = 4;
pub(crate) const RAW_WRITE: u32 = 5;
pub(crate) const RAW_READ: u32 = 6;
pub(crate) const USER_DEFINED: u32 = 7;
pub(crate) const RAW_FLUSH: u32 = 8;

pub(crate) const STATUS_SUCCESS: i32 = 0;
/// A read ended by Ctrl+C, Ctrl+Break or its handle closing.
pub(crate) const STATUS_ALERTED: i32 = 0x0000_0101;
pub(crate) const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001_u32 as i32;
pub(crate) const STATUS_NOT_IMPLEMENTED: i32 = 0xC000_0002_u32 as i32;
pub(crate) const STATUS_INVALID_HANDLE: i32 = 0xC000_0008_u32 as i32;
pub(crate) const STATUS_INVALID_PARAMETER: i32 = 0xC000_000D_u32 as i32;
pub(crate) const STATUS_NO_MEMORY: i32 = 0xC000_0017_u32 as i32;
pub(crate) const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023_u32 as i32;
/// A read ended because its client disconnected.
pub(crate) const STATUS_THREAD_IS_TERMINATING: i32 = 0xC000_004B_u32 as i32;
pub(crate) const STATUS_ILLEGAL_FUNCTION: i32 = 0xC000_00AF_u32 as i32;
pub(crate) const STATUS_NOT_SUPPORTED: i32 = 0xC000_00BB_u32 as i32;
pub(crate) const STATUS_INVALID_BUFFER_SIZE: i32 = 0xC000_0206_u32 as i32;

/// What conhost puts on the wire for a failed HRESULT:
/// `NTSTATUS_FROM_HRESULT` keeps only its low 16 bits, as
/// `0xC007_xxxx`. So E_INVALIDARG is `0xC007_0057`, and even an
/// NTSTATUS wrapped in an HRESULT loses its facility.
pub(crate) const fn from_hresult(hr: u32) -> i32 {
    (0xC007_0000 | (hr & 0xFFFF)) as i32
}

/// E_INVALIDARG.
pub(crate) const STATUS_FROM_INVALIDARG: i32 = from_hresult(0x8007_0057);
/// HRESULT_FROM_WIN32(ERROR_SHARING_VIOLATION).
pub(crate) const STATUS_FROM_SHARING_VIOLATION: i32 = from_hresult(0x8007_0020);
/// E_HANDLE: no such object, or not the kind the API needs.
pub(crate) const STATUS_FROM_HANDLE: i32 = from_hresult(0x8007_0006);
/// E_ACCESSDENIED: the handle lacks the access the API needs.
pub(crate) const STATUS_FROM_ACCESS_DENIED: i32 = from_hresult(0x8007_0005);

/// `CONSOLE_API_NUMBER_L1` / `_L2`: `layer << 24 | index`.
pub(crate) mod api {
    pub(crate) const GET_CP: u32 = 0x0100_0000;
    pub(crate) const GET_MODE: u32 = 0x0100_0001;
    pub(crate) const SET_MODE: u32 = 0x0100_0002;
    pub(crate) const GET_NUMBER_OF_INPUT_EVENTS: u32 = 0x0100_0003;
    pub(crate) const GET_CONSOLE_INPUT: u32 = 0x0100_0004;
    pub(crate) const READ_CONSOLE: u32 = 0x0100_0005;
    pub(crate) const WRITE_CONSOLE: u32 = 0x0100_0006;
    pub(crate) const GET_LANG_ID: u32 = 0x0100_0008;
    pub(crate) const GENERATE_CTRL_EVENT: u32 = 0x0200_0001;
    pub(crate) const FLUSH_INPUT_BUFFER: u32 = 0x0200_0003;
    pub(crate) const SET_CP: u32 = 0x0200_0004;
    pub(crate) const GET_CURSOR_INFO: u32 = 0x0200_0005;
    pub(crate) const SET_CURSOR_INFO: u32 = 0x0200_0006;
    pub(crate) const GET_SCREEN_BUFFER_INFO: u32 = 0x0200_0007;
    pub(crate) const SET_CURSOR_POSITION: u32 = 0x0200_000A;
    pub(crate) const SET_TEXT_ATTRIBUTE: u32 = 0x0200_000D;
    pub(crate) const GET_TITLE: u32 = 0x0200_0014;
    pub(crate) const SET_TITLE: u32 = 0x0200_0015;
    pub(crate) const GET_CURRENT_FONT: u32 = 0x0300_0004;
    pub(crate) const GET_CONSOLE_WINDOW: u32 = 0x0300_001F;
}

/// `CD_IO_OBJECT_TYPE_*`.
pub(crate) const OBJECT_CURRENT_INPUT: u32 = 1;
pub(crate) const OBJECT_CURRENT_OUTPUT: u32 = 2;
pub(crate) const OBJECT_NEW_OUTPUT: u32 = 3;
pub(crate) const OBJECT_GENERIC: u32 = 4;

pub(crate) const GENERIC_READ: u32 = 0x8000_0000;
pub(crate) const GENERIC_WRITE: u32 = 0x4000_0000;
pub(crate) const FILE_SHARE_READ: u32 = 0x1;
pub(crate) const FILE_SHARE_WRITE: u32 = 0x2;

/// A USER_DEFINED message: `CONSOLE_MSG_HEADER`, then the API's fixed
/// struct and its variable data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ApiMsg<'a> {
    pub number: u32,
    pub fixed: &'a [u8],
    pub variable: &'a [u8],
}

/// Split a USER_DEFINED payload. `None` when the header is short or its
/// `ApiDescriptorSize` runs past the payload.
pub(crate) fn api_msg(payload: &[u8]) -> Option<ApiMsg<'_>> {
    let mut r = Reader::new(payload);
    let number = r.read_u32_le().ok()?;
    let size = r.read_u32_le().ok()? as usize;
    let fixed = r.read_bytes(size).ok()?;
    Some(ApiMsg {
        number,
        fixed,
        variable: r.peek_remaining(),
    })
}

/// `CD_CREATE_OBJECT_INFORMATION { ObjectType, ShareMode, DesiredAccess }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CreateObject {
    pub object_type: u32,
    pub share_mode: u32,
    pub desired_access: u32,
}

pub(crate) fn create_object(payload: &[u8]) -> Option<CreateObject> {
    let mut r = Reader::new(payload);
    let object_type = r.read_u32_le().ok()?;
    let share_mode = r.read_u32_le().ok()?;
    let desired_access = r.read_u32_le().ok()?;
    Some(CreateObject {
        object_type,
        share_mode,
        desired_access,
    })
}

/// `CONSOLE_SERVER_MSG`, 1340 bytes: the counted strings, each a USHORT
/// length in bytes and then its WCHAR array.
const SERVER_MSG_LEN: usize = 1340;
/// (length offset, array offset, array units) of Title, ApplicationName
/// and CurrentDirectory.
const SERVER_MSG_STRINGS: [(usize, usize, usize); 3] =
    [(34, 36, 261), (558, 560, 128), (816, 818, 261)];

/// `ProcessGroupId` in `CONSOLE_SERVER_MSG`.
const SERVER_MSG_GROUP_AT: usize = 28;

/// What CONNECT's `CONSOLE_SERVER_MSG` tells the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerMsg {
    pub title: Vec<u16>,
    /// `ProcessGroupId`, which GenerateConsoleCtrlEvent targets.
    pub group: u32,
}

/// Decode CONNECT's `CONSOLE_SERVER_MSG`. Like conhost, every counted
/// string must fit its array with room for a NUL, and be NUL-terminated
/// at its length; otherwise, or when the message is short,
/// `STATUS_INVALID_BUFFER_SIZE`.
pub(crate) fn server_msg_info(payload: &[u8]) -> Result<ServerMsg, i32> {
    let invalid = STATUS_INVALID_BUFFER_SIZE;
    let msg = payload.get(..SERVER_MSG_LEN).ok_or(invalid)?;
    let mut strings = Vec::with_capacity(3);
    for (len_at, array_at, units) in SERVER_MSG_STRINGS {
        let len = usize::from(u16::from_le_bytes([msg[len_at], msg[len_at + 1]]));
        let array = utf16_units(&msg[array_at..array_at + units * 2]);
        if len > (units - 1) * 2 || array[len / 2] != 0 {
            return Err(invalid);
        }
        strings.push(array[..len / 2].to_vec());
    }
    Ok(ServerMsg {
        title: strings.swap_remove(0),
        group: u32_at(msg, SERVER_MSG_GROUP_AT).unwrap_or(0),
    })
}

/// Bytes in a `CD_CONNECTION_INFORMATION`.
pub(crate) const CONNECTION_INFORMATION_LEN: usize = 24;

/// `CD_CONNECTION_INFORMATION { Process, Input, Output }`, the cookies
/// ConDrv echoes back in later descriptors.
pub(crate) fn connection_information(process: usize, input: usize, output: usize) -> Vec<u8> {
    [process, input, output]
        .iter()
        .flat_map(|v| (*v as u64).to_le_bytes())
        .collect()
}

/// Read a little-endian ULONG at `at` of a fixed struct.
pub(crate) fn u32_at(fixed: &[u8], at: usize) -> Option<u32> {
    let bytes = fixed.get(at..at + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Read a BOOLEAN at `at` of a fixed struct.
pub(crate) fn bool_at(fixed: &[u8], at: usize) -> Option<bool> {
    fixed.get(at).map(|b| *b != 0)
}

/// Read a little-endian USHORT (or COORD half) at `at` of a fixed struct.
pub(crate) fn u16_at(fixed: &[u8], at: usize) -> Option<u16> {
    let bytes = fixed.get(at..at + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

/// Overwrite the USHORT at `at`. The caller has checked the length.
pub(crate) fn put_u16(fixed: &mut [u8], at: usize, v: u16) {
    fixed[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// Overwrite the ULONG at `at`. The caller has checked the length.
pub(crate) fn put_u32(fixed: &mut [u8], at: usize, v: u32) {
    fixed[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// UTF-16LE bytes as code units; a trailing odd byte is dropped.
pub(crate) fn utf16_units(bytes: &[u8]) -> Vec<u16> {
    let (units, _odd) = bytes.as_chunks::<2>();
    units.iter().map(|c| u16::from_le_bytes(*c)).collect()
}

#[cfg(test)]
/// A CONSOLE_SERVER_MSG with `title` and empty other strings.
pub(crate) fn server_msg(title: &str) -> Vec<u8> {
    server_msg_in_group(title, 0)
}

#[cfg(test)]
/// [`server_msg`] with a `ProcessGroupId`.
pub(crate) fn server_msg_in_group(title: &str, group: u32) -> Vec<u8> {
    let mut m = vec![0; SERVER_MSG_LEN];
    put_u32(&mut m, SERVER_MSG_GROUP_AT, group);
    let units: Vec<u16> = title.encode_utf16().collect();
    m[34..36].copy_from_slice(&((units.len() * 2) as u16).to_le_bytes());
    for (i, u) in units.iter().enumerate() {
        m[36 + i * 2..38 + i * 2].copy_from_slice(&u.to_le_bytes());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_msg_splits_header_fixed_and_variable() {
        let mut p = Vec::new();
        p.extend(api::WRITE_CONSOLE.to_le_bytes());
        p.extend(8u32.to_le_bytes());
        p.extend([0, 0, 0, 0, 1, 0, 0, 0]);
        p.extend(b"hi");
        let m = api_msg(&p).expect("well-formed");
        assert_eq!(m.number, api::WRITE_CONSOLE);
        assert_eq!(m.fixed, &[0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(m.variable, b"hi");
    }

    #[test]
    fn api_msg_rejects_a_short_header_or_an_oversized_descriptor() {
        assert_eq!(api_msg(&[1, 0, 0]), None);
        let mut p = Vec::new();
        p.extend(api::GET_MODE.to_le_bytes());
        p.extend(9u32.to_le_bytes());
        p.extend([0; 8]);
        assert_eq!(api_msg(&p), None, "ApiDescriptorSize past the payload");
        let mut p = Vec::new();
        p.extend(api::GET_MODE.to_le_bytes());
        p.extend(u32::MAX.to_le_bytes());
        assert_eq!(api_msg(&p), None);
    }

    #[test]
    fn create_object_needs_all_three_fields() {
        let mut p = Vec::new();
        p.extend(OBJECT_GENERIC.to_le_bytes());
        p.extend(3u32.to_le_bytes());
        p.extend(GENERIC_WRITE.to_le_bytes());
        assert_eq!(
            create_object(&p),
            Some(CreateObject {
                object_type: OBJECT_GENERIC,
                share_mode: 3,
                desired_access: GENERIC_WRITE
            })
        );
        assert_eq!(create_object(&p[..11]), None);
    }

    #[test]
    fn server_msg_title_reads_the_counted_title() {
        let title: Vec<u16> = "cmd.exe".encode_utf16().collect();
        let info = |m: &[u8]| server_msg_info(m).map(|i| i.title);
        assert_eq!(info(&server_msg("cmd.exe")), Ok(title));
        assert_eq!(info(&server_msg("")), Ok(Vec::new()));
        assert_eq!(
            server_msg_info(&server_msg_in_group("", 4242)).map(|i| i.group),
            Ok(4242)
        );
        let longest = "x".repeat(260);
        assert_eq!(
            server_msg_info(&server_msg(&longest)).map(|i| i.title.len()),
            Ok(260)
        );
    }

    #[test]
    fn server_msg_title_rejects_what_conhost_rejects() {
        let invalid = Err(STATUS_INVALID_BUFFER_SIZE);
        assert_eq!(
            server_msg_info(&server_msg("x")[..SERVER_MSG_LEN - 1]),
            invalid
        );
        let mut m = server_msg("abc");
        m[34..36].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(server_msg_info(&m), invalid, "no NUL at the length");
        let mut m = server_msg("");
        m[34..36].copy_from_slice(&522u16.to_le_bytes());
        assert_eq!(server_msg_info(&m), invalid, "no room for the NUL");
        let mut m = server_msg("");
        m[560] = b'a';
        assert_eq!(
            server_msg_info(&m),
            invalid,
            "ApplicationName is checked too"
        );
    }

    #[test]
    fn from_hresult_keeps_the_low_word_only() {
        assert_eq!(STATUS_FROM_INVALIDARG as u32, 0xC007_0057);
        assert_eq!(STATUS_FROM_SHARING_VIOLATION as u32, 0xC007_0020);
        // HRESULT_FROM_NT(STATUS_NOT_SUPPORTED), as GetConsoleLangId sends.
        assert_eq!(from_hresult(0xD000_00BB) as u32, 0xC007_00BB);
    }

    #[test]
    fn connection_information_is_three_little_endian_pointers() {
        let info = connection_information(1, 2, 3);
        assert_eq!(info.len(), 24);
        assert_eq!(&info[8..16], &2u64.to_le_bytes());
    }

    #[test]
    fn field_readers_check_bounds() {
        assert_eq!(u32_at(&[1, 0, 0, 0], 0), Some(1));
        assert_eq!(u32_at(&[1, 0, 0], 0), None);
        assert_eq!(bool_at(&[0, 2], 1), Some(true));
        assert_eq!(bool_at(&[0], 1), None);
        assert_eq!(utf16_units(&[b'h', 0, b'i', 0, 7]), vec![0x68, 0x69]);
    }
}
