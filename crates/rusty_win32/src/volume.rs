//! Drive/volume enumeration (`GetLogicalDrives`/`GetDriveTypeW`/
//! `GetVolumeInformationW`) — a distinctly Windows-shaped primitive with no
//! Unix analog at all: Windows' multi-root filesystem model (`C:\`, `D:\`,
//! …) has no single mount table the way Unix's single-rooted tree does.
//! Nothing in [`crate::fs`]/[`crate::path`] currently helps a shell offer
//! drive-letter tab-completion or a `df`-equivalent listing of mounted
//! volumes; this module is that primitive. No current `rush` call site
//! needs this yet — added per the round-2 capability assessment's own
//! "flagged for completeness, not because any consumer currently wants it"
//! framing.

use crate::error::Win32Error;
use crate::handle::RawHandle;

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// `GetDriveTypeW`'s return value doesn't identify a drive letter's type
/// (unknown root, no medium in a removable/CD-ROM drive, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveType {
    /// The drive type cannot be determined.
    Unknown,
    /// `root_path` doesn't name an existing root directory.
    NoRootDir,
    /// A removable-media drive (floppy, USB flash, etc.).
    Removable,
    /// A fixed (non-removable) disk.
    Fixed,
    /// A remote (network-mapped) drive.
    Remote,
    /// A CD-ROM/DVD drive.
    CdRom,
    /// A RAM disk.
    RamDisk,
}

const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

/// A generous starting buffer size, in UTF-16 units, for
/// [`volume_information`]'s volume-name and file-system-name buffers —
/// comfortably larger than either's own documented maximum (`MAX_PATH`
/// for the volume name, a short fixed set of file-system names like
/// `"NTFS"`/`"FAT32"`), so a single call is always enough in practice.
const NAME_BUFFER_LEN: usize = 261;

/// Microsoft's own documented minimum buffer size for
/// `FindFirstVolumeW`/`FindNextVolumeW`'s volume-name output — comfortably
/// larger than a GUID volume path (`\\?\Volume{xxxxxxxx-xxxx-xxxx-xxxx-
/// xxxxxxxxxxxx}\`, 49 characters plus a NUL) ever needs.
const VOLUME_NAME_BUFFER_LEN: usize = 50;

/// `INVALID_HANDLE_VALUE` — the sentinel `FindFirstVolumeW` returns instead
/// of `NULL` on failure, the same convention `fs::read_dir`'s
/// `FindFirstFileW` uses.
const INVALID_HANDLE_VALUE: isize = -1;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root_path_name: *const u16) -> u32;
    fn GetVolumeInformationW(
        root_path_name: *const u16,
        volume_name_buffer: *mut u16,
        volume_name_size: u32,
        volume_serial_number: *mut u32,
        maximum_component_length: *mut u32,
        file_system_flags: *mut u32,
        file_system_name_buffer: *mut u16,
        file_system_name_size: u32,
    ) -> i32;
    fn GetDiskFreeSpaceExW(
        directory_name: *const u16,
        free_bytes_available_to_caller: *mut u64,
        total_number_of_bytes: *mut u64,
        total_number_of_free_bytes: *mut u64,
    ) -> i32;
    fn FindFirstVolumeW(volume_name: *mut u16, buffer_length: u32) -> RawHandle;
    fn FindNextVolumeW(find_volume: RawHandle, volume_name: *mut u16, buffer_length: u32) -> i32;
    fn FindVolumeClose(find_volume: RawHandle) -> i32;
    fn GetVolumePathNameW(
        file_name: *const u16,
        volume_path_name: *mut u16,
        buffer_length: u32,
    ) -> i32;
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Every drive letter currently present (`A`..`Z`) — `GetLogicalDrives`'s
/// bitmask (bit 0 for `A`, bit 25 for `Z`) decoded into the letters
/// themselves, in ascending order. "Present" here matches whatever
/// `GetLogicalDrives` itself reports (e.g. a removable drive's letter can
/// appear even with no medium inserted) — this function doesn't second-
/// guess it by also checking [`drive_type`] or media presence.
pub fn logical_drives() -> Vec<char> {
    // SAFETY: `GetLogicalDrives` takes no arguments and has no
    // precondition.
    let mask = unsafe { GetLogicalDrives() };
    (0..26)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| (b'A' + bit as u8) as char)
        .collect()
}

/// The type of the drive named by `root_path` (e.g. `"C:\\"`) —
/// `GetDriveTypeW`. Never fails: an unrecognized or nonexistent
/// `root_path` reports [`DriveType::Unknown`]/[`DriveType::NoRootDir`]
/// rather than a `Win32Error`, matching `GetDriveTypeW`'s own documented
/// contract (it has no `GetLastError` failure mode at all).
pub fn drive_type(root_path: &str) -> DriveType {
    let wide = to_wide(root_path);
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string.
    let raw = unsafe { GetDriveTypeW(wide.as_ptr()) };
    match raw {
        DRIVE_NO_ROOT_DIR => DriveType::NoRootDir,
        DRIVE_REMOVABLE => DriveType::Removable,
        DRIVE_FIXED => DriveType::Fixed,
        DRIVE_REMOTE => DriveType::Remote,
        DRIVE_CDROM => DriveType::CdRom,
        DRIVE_RAMDISK => DriveType::RamDisk,
        _ => DriveType::Unknown,
    }
}

/// [`volume_information`]'s result — `GetVolumeInformationW`'s fields, the
/// closest Windows analog of a Unix `statvfs`'s filesystem-identity
/// portion (though not its free-space fields, which `GetVolumeInformationW`
/// doesn't report at all — that's `GetDiskFreeSpaceExW`, out of this
/// module's current scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInformation {
    /// The volume's label, e.g. `"Windows"` — empty if unlabeled.
    pub volume_name: String,
    /// A serial number assigned when the volume was formatted, unique per
    /// formatting (not a stable hardware identifier).
    pub serial_number: u32,
    /// The longest file-name component the file system supports (in UTF-16
    /// units), e.g. `255` for NTFS.
    pub maximum_component_length: u32,
    /// Raw `FILE_*` capability bits (e.g. whether the file system preserves
    /// case, supports named streams, …) — exposed as-is, the same way this
    /// crate's other raw bitmask fields are (`fs::FILE_ATTRIBUTE_*`,
    /// `console::ENABLE_*`): deciding what a caller does with them is the
    /// caller's policy, not this crate's.
    pub file_system_flags: u32,
    /// The file system's name, e.g. `"NTFS"`/`"FAT32"`.
    pub file_system_name: String,
}

/// Identify the file system mounted at `root_path` (e.g. `"C:\\"`) —
/// `GetVolumeInformationW`.
pub fn volume_information(root_path: &str) -> Result<VolumeInformation, Win32Error> {
    let wide = to_wide(root_path);
    let mut volume_name_buf = alloc::vec![0u16; NAME_BUFFER_LEN];
    let mut file_system_name_buf = alloc::vec![0u16; NAME_BUFFER_LEN];
    let mut serial_number: u32 = 0;
    let mut maximum_component_length: u32 = 0;
    let mut file_system_flags: u32 = 0;

    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
    // `volume_name_buf`/`file_system_name_buf` are valid,
    // `NAME_BUFFER_LEN`-element writable buffers matched by the
    // `..._size` arguments naming their exact lengths;
    // `serial_number`/`maximum_component_length`/`file_system_flags` are
    // valid out-pointers.
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            volume_name_buf.as_mut_ptr(),
            volume_name_buf.len() as u32,
            &mut serial_number,
            &mut maximum_component_length,
            &mut file_system_flags,
            file_system_name_buf.as_mut_ptr(),
            file_system_name_buf.len() as u32,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(VolumeInformation {
        volume_name: from_wide(&volume_name_buf),
        serial_number,
        maximum_component_length,
        file_system_flags,
        file_system_name: from_wide(&file_system_name_buf),
    })
}

/// Free/total space for the volume `root_path` (e.g. `"C:\\"`) resolves
/// onto — `GetDiskFreeSpaceExW`, the primitive a `df`-style builtin needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskFreeSpace {
    /// Bytes available to the calling process's own user — can be less
    /// than [`total_free_bytes`](Self::total_free_bytes) under per-user
    /// disk quotas.
    pub free_bytes_available_to_caller: u64,
    /// The volume's total capacity in bytes.
    pub total_bytes: u64,
    /// Total free bytes on the volume, ignoring any per-user quota.
    pub total_free_bytes: u64,
}

/// Free/total space for the volume `root_path` names — `GetDiskFreeSpaceExW`.
/// `root_path` can be a drive root (`"C:\\"`), a UNC share, or any directory
/// on the volume (the API resolves it to the containing volume itself).
pub fn disk_free_space(root_path: &str) -> Result<DiskFreeSpace, Win32Error> {
    let wide = to_wide(root_path);
    let mut free_bytes_available_to_caller: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free_bytes: u64 = 0;
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; the three
    // out-pointers are valid, correctly-sized locals.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available_to_caller,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(DiskFreeSpace {
        free_bytes_available_to_caller,
        total_bytes,
        total_free_bytes,
    })
}

/// Enumerates every volume on the system by its stable GUID path
/// (`\\?\Volume{GUID}\`), independent of drive-letter assignment —
/// `FindFirstVolumeW`/`FindNextVolumeW`, closing the search handle via
/// `FindVolumeClose` on drop (including on an early `break`), the same
/// shape [`crate::fs::read_dir`]'s `ReadDir` already established for
/// `FindFirstFileW`/`FindNextFileW`/`FindClose`. A GUID volume path stays
/// stable across reboots/drive-letter reassignment, unlike
/// [`logical_drives`]'s drive letters.
#[derive(Debug)]
pub struct FindVolumes {
    handle: RawHandle,
    // `FindFirstVolumeW` already produced the first entry by the time this
    // struct exists — `next()` returns it before ever calling
    // `FindNextVolumeW`, the same "the opening call already returned data"
    // shape `fs::ReadDir`/`process::list_processes` use.
    pending: Option<String>,
}

impl Iterator for FindVolumes {
    type Item = Result<String, Win32Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(name) = self.pending.take() {
            return Some(Ok(name));
        }
        let mut buf = alloc::vec![0u16; VOLUME_NAME_BUFFER_LEN];
        // SAFETY: `self.handle` is a valid, currently-open search handle
        // (from `find_volumes`, not yet closed); `buf` is a valid,
        // `VOLUME_NAME_BUFFER_LEN`-element writable buffer matched by the
        // `buffer_length` argument naming its exact length.
        let found = unsafe { FindNextVolumeW(self.handle, buf.as_mut_ptr(), buf.len() as u32) };
        if found == 0 {
            // `FindNextFileW`'s own end-of-enumeration convention, applied
            // the same way here: a `FALSE` return with
            // `GetLastError() == ERROR_NO_MORE_FILES` means "nothing left,"
            // not a real error.
            let err = Win32Error::last();
            return if err == Win32Error::ERROR_NO_MORE_FILES {
                None
            } else {
                Some(Err(err))
            };
        }
        Some(Ok(from_wide(&buf)))
    }
}

impl Drop for FindVolumes {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a valid, currently-open search handle,
        // closed exactly once here and never used again after.
        let _ = unsafe { FindVolumeClose(self.handle) };
    }
}

/// Start enumerating every volume on the system — `FindFirstVolumeW`.
pub fn find_volumes() -> Result<FindVolumes, Win32Error> {
    let mut buf = alloc::vec![0u16; VOLUME_NAME_BUFFER_LEN];
    // SAFETY: `buf` is a valid, `VOLUME_NAME_BUFFER_LEN`-element writable
    // buffer matched by the `buffer_length` argument naming its exact
    // length.
    let handle = unsafe { FindFirstVolumeW(buf.as_mut_ptr(), buf.len() as u32) };
    if handle.is_null() || handle as isize == INVALID_HANDLE_VALUE {
        return Err(Win32Error::last());
    }
    Ok(FindVolumes {
        handle,
        pending: Some(from_wide(&buf)),
    })
}

/// Maps `path` (any file or directory path, not just a drive root) to the
/// root path of the volume it's on — `GetVolumePathNameW`, the reverse
/// direction of [`volume_information`]/[`disk_free_space`]'s own root-path
/// parameter.
pub fn volume_path_name(path: &str) -> Result<String, Win32Error> {
    let wide = to_wide(path);
    let mut buf = alloc::vec![0u16; NAME_BUFFER_LEN];
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; `buf` is a
    // valid, `NAME_BUFFER_LEN`-element writable buffer matched by the
    // `buffer_length` argument naming its exact length.
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(from_wide(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_drives_includes_the_system_drive() {
        let drives = logical_drives();
        // SystemDrive is documented to always be set in a real Windows
        // process's environment (e.g. "C:") — a stable, deterministic
        // drive letter to check for rather than assuming `C` specifically,
        // in case CI ever runs from a different system drive.
        let system_drive = std::env::var("SystemDrive")
            .expect("SystemDrive should be set in any real Windows process's environment");
        let letter = system_drive
            .chars()
            .next()
            .expect("SystemDrive should be non-empty")
            .to_ascii_uppercase();
        assert!(
            drives.contains(&letter),
            "logical_drives() should include the system drive {letter}, got: {drives:?}"
        );
    }

    #[test]
    fn drive_type_reports_fixed_for_the_system_drive() {
        let system_drive = std::env::var("SystemDrive")
            .expect("SystemDrive should be set in any real Windows process's environment");
        let root = alloc::format!("{system_drive}\\");
        assert_eq!(drive_type(&root), DriveType::Fixed);
    }

    #[test]
    fn drive_type_reports_no_root_dir_for_an_unassigned_letter() {
        // Every currently-assigned letter comes from `logical_drives()`;
        // the alphabet's complement is guaranteed unassigned right now,
        // rather than hardcoding a specific letter that might collide with
        // a real drive on some CI runner.
        let assigned = logical_drives();
        let unassigned = (b'A'..=b'Z')
            .map(|b| b as char)
            .find(|c| !assigned.contains(c))
            .expect("not every one of the 26 possible drive letters can be assigned at once");
        let root = alloc::format!("{unassigned}:\\");
        assert_eq!(drive_type(&root), DriveType::NoRootDir);
    }

    #[test]
    fn volume_information_reports_a_plausible_file_system_for_the_system_drive() {
        let system_drive = std::env::var("SystemDrive")
            .expect("SystemDrive should be set in any real Windows process's environment");
        let root = alloc::format!("{system_drive}\\");
        let info = volume_information(&root)
            .expect("GetVolumeInformationW should succeed for SystemDrive");
        assert!(
            !info.file_system_name.is_empty(),
            "the system drive should report a non-empty file system name"
        );
        assert!(
            info.maximum_component_length > 0,
            "a real file system should report a positive max component length"
        );
    }

    #[test]
    fn disk_free_space_reports_plausible_values_for_the_system_drive() {
        let system_drive = std::env::var("SystemDrive")
            .expect("SystemDrive should be set in any real Windows process's environment");
        let root = alloc::format!("{system_drive}\\");
        let space =
            disk_free_space(&root).expect("GetDiskFreeSpaceExW should succeed for SystemDrive");
        assert!(
            space.total_bytes > 0,
            "a real, existing volume should report a nonzero total size"
        );
        assert!(
            space.total_free_bytes <= space.total_bytes,
            "free space can't exceed total capacity"
        );
        assert!(
            space.free_bytes_available_to_caller <= space.total_free_bytes,
            "caller-available free space can't exceed the volume's total free space"
        );
    }

    #[test]
    fn find_volumes_reports_at_least_one_guid_volume_path() {
        let volumes: Vec<String> = find_volumes()
            .expect("FindFirstVolumeW should succeed")
            .collect::<Result<_, _>>()
            .expect("FindNextVolumeW should succeed for every entry");
        assert!(
            !volumes.is_empty(),
            "a real Windows machine should have at least one volume (the system drive's)"
        );
        for name in &volumes {
            assert!(
                name.starts_with(r"\\?\Volume{") && name.ends_with('\\'),
                "every entry should be a GUID volume path, got: {name:?}"
            );
        }
    }

    #[test]
    fn volume_path_name_resolves_the_windows_directory_to_the_system_drive_root() {
        let system_drive = std::env::var("SystemDrive")
            .expect("SystemDrive should be set in any real Windows process's environment");
        let windows_dir = std::env::var("SystemRoot")
            .expect("SystemRoot should be set in any real Windows process's environment");
        let root = volume_path_name(&windows_dir)
            .expect("GetVolumePathNameW should succeed for a well-known existing directory");
        assert_eq!(
            root.to_ascii_uppercase(),
            alloc::format!("{}\\", system_drive.to_ascii_uppercase())
        );
    }
}
