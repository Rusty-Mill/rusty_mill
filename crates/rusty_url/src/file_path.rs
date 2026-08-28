//! Conversion between `file:` URLs and `std::path::Path`, backing
//! [`crate::Url::from_file_path`]/[`crate::Url::from_directory_path`]/
//! [`crate::Url::to_file_path`]. Platform-specific: Unix-like systems treat
//! paths as raw bytes with a single root; Windows paths carry a drive
//! letter or UNC-share prefix, which becomes the URL's host.

use std::path::{Path, PathBuf};

use crate::host::HostInternal;

#[cfg(any(unix, target_os = "redox", target_os = "wasi", target_os = "hermit"))]
pub(crate) fn path_to_file_url_segments(
    path: &Path,
    serialization: &mut String,
) -> Result<(u32, HostInternal), ()> {
    use crate::parser::{to_u32, SPECIAL_PATH_SEGMENT};
    use crate::percent_encode::percent_encode;
    #[cfg(target_os = "hermit")]
    use std::os::hermit::ffi::OsStrExt;
    #[cfg(any(unix, target_os = "redox"))]
    use std::os::unix::ffi::OsStrExt;

    if !path.is_absolute() {
        return Err(());
    }
    let host_end = to_u32(serialization.len()).map_err(|_| ())?;
    let mut empty = true;
    // Skip the root component.
    for component in path.components().skip(1) {
        empty = false;
        serialization.push('/');
        #[cfg(not(target_os = "wasi"))]
        let bytes = component.as_os_str().as_bytes().to_vec();
        #[cfg(target_os = "wasi")]
        let bytes = component.as_os_str().to_string_lossy().as_bytes().to_vec();
        let encoded = percent_encode(&bytes, SPECIAL_PATH_SEGMENT);
        serialization.push_str(std::str::from_utf8(&encoded).expect("percent-encoding is ASCII"));
    }
    if empty {
        // A URL's path must not be empty.
        serialization.push('/');
    }
    Ok((host_end, HostInternal::None))
}

#[cfg(windows)]
pub(crate) fn path_to_file_url_segments(
    path: &Path,
    serialization: &mut String,
) -> Result<(u32, HostInternal), ()> {
    path_to_file_url_segments_windows(path, serialization)
}

#[cfg(windows)]
pub(crate) fn path_to_file_url_segments_windows(
    path: &Path,
    serialization: &mut String,
) -> Result<(u32, HostInternal), ()> {
    use crate::host::Host;
    use crate::parser::{is_windows_drive_letter, to_u32, PATH_SEGMENT};
    use crate::percent_encode::percent_encode;
    use std::fmt::Write as _;
    use std::path::{Component, Prefix};

    if !path.is_absolute() {
        return Err(());
    }
    let mut components = path.components();

    let host_start = serialization.len() + 1;
    let host_end;
    let host_internal;

    match components.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                host_end = to_u32(serialization.len()).map_err(|_| ())?;
                host_internal = HostInternal::None;
                serialization.push('/');
                serialization.push(letter as char);
                serialization.push(':');
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                let host = Host::parse(server.to_str().ok_or(())?).map_err(|_| ())?;
                write!(serialization, "{host}").map_err(|_| ())?;
                host_end = to_u32(serialization.len()).map_err(|_| ())?;
                host_internal = host.into();
                serialization.push('/');
                let share = share.to_str().ok_or(())?;
                let encoded = percent_encode(share.as_bytes(), PATH_SEGMENT);
                serialization
                    .push_str(std::str::from_utf8(&encoded).expect("percent-encoding is ASCII"));
            }
            _ => return Err(()),
        },
        _ => return Err(()),
    }

    let mut path_only_has_prefix = true;
    for component in components {
        if component == Component::RootDir {
            continue;
        }
        path_only_has_prefix = false;
        let component = component.as_os_str().to_str().ok_or(())?;
        serialization.push('/');
        let encoded = percent_encode(component.as_bytes(), PATH_SEGMENT);
        serialization.push_str(std::str::from_utf8(&encoded).expect("percent-encoding is ASCII"));
    }

    // A windows drive letter must end with a slash.
    if serialization.len() > host_start
        && is_windows_drive_letter(&serialization[host_start..])
        && path_only_has_prefix
    {
        serialization.push('/');
    }

    Ok((host_end, host_internal))
}

#[cfg(any(unix, target_os = "redox", target_os = "wasi", target_os = "hermit"))]
pub(crate) fn file_url_segments_to_pathbuf(
    host: Option<&str>,
    segments: std::str::Split<'_, char>,
) -> Result<PathBuf, ()> {
    use crate::percent_encode::percent_decode;
    #[cfg(not(target_os = "wasi"))]
    use std::ffi::OsStr;
    #[cfg(target_os = "hermit")]
    use std::os::hermit::ffi::OsStrExt;
    #[cfg(any(unix, target_os = "redox"))]
    use std::os::unix::ffi::OsStrExt;

    if host.is_some() {
        return Err(());
    }

    let mut bytes = Vec::new();
    for segment in segments {
        bytes.push(b'/');
        bytes.extend_from_slice(&percent_decode(segment.as_bytes()));
    }

    // A windows drive letter must end with a slash.
    if bytes.len() > 2
        && bytes[bytes.len() - 2].is_ascii_alphabetic()
        && matches!(bytes[bytes.len() - 1], b':' | b'|')
    {
        bytes.push(b'/');
    }

    #[cfg(not(target_os = "wasi"))]
    let path = PathBuf::from(OsStr::from_bytes(&bytes));
    #[cfg(target_os = "wasi")]
    let path = String::from_utf8(bytes)
        .map(PathBuf::from)
        .map_err(|_| ())?;

    debug_assert!(
        path.is_absolute(),
        "to_file_path() failed to produce an absolute Path"
    );
    Ok(path)
}

#[cfg(windows)]
pub(crate) fn file_url_segments_to_pathbuf(
    host: Option<&str>,
    segments: std::str::Split<'_, char>,
) -> Result<PathBuf, ()> {
    file_url_segments_to_pathbuf_windows(host, segments)
}

#[cfg(any(windows, test))]
pub(crate) fn file_url_segments_to_pathbuf_windows(
    host: Option<&str>,
    mut segments: std::str::Split<'_, char>,
) -> Result<PathBuf, ()> {
    use crate::parser::ascii_alpha;
    use crate::percent_encode::percent_decode;

    let mut string = String::new();
    if let Some(host) = host {
        string.push_str(r"\\");
        string.push_str(host);
    } else {
        let first = segments.next().ok_or(())?;
        match first.len() {
            2 => {
                if !first.starts_with(ascii_alpha) || first.as_bytes()[1] != b':' {
                    return Err(());
                }
                string.push_str(first);
            }
            4 => {
                if !first.starts_with(ascii_alpha) {
                    return Err(());
                }
                let bytes = first.as_bytes();
                if bytes[1] != b'%' || bytes[2] != b'3' || (bytes[3] != b'a' && bytes[3] != b'A') {
                    return Err(());
                }
                string.push_str(&first[0..1]);
                string.push(':');
            }
            _ => return Err(()),
        }
    }

    for segment in segments {
        string.push('\\');
        match std::str::from_utf8(&percent_decode(segment.as_bytes())) {
            Ok(s) => string.push_str(s),
            Err(_) => return Err(()),
        }
    }

    Ok(PathBuf::from(string))
}
