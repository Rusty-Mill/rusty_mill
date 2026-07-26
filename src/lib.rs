//! `rpath`: Path translation and normalization engine for MSYS2/Git Bash/POSIX to Windows interop.

/// Path style representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    Posix,
    Windows,
}

/// Translates a POSIX-style path (e.g. `/c/Users/baile/dev`) to a Windows-style path (e.g. `C:\Users\baile\dev`).
/// Also translates pseudo-paths such as `/tmp`, `/dev/null`, `/usr/bin`.
pub fn posix_to_win32(posix_path: &str) -> String {
    let s = posix_path.trim();
    if s.is_empty() {
        return String::new();
    }

    // Special virtual device paths
    if s == "/dev/null" {
        return r"NUL".to_string();
    }

    // Check drive letter mapping: e.g. /c/ or /c or /C/
    let bytes = s.as_bytes();
    if bytes.starts_with(b"/") && bytes.len() >= 2 {
        let drive_char = bytes[1] as char;
        if drive_char.is_ascii_alphabetic() {
            if bytes.len() == 2 {
                return format!("{}:\\", drive_char.to_ascii_uppercase());
            } else if bytes[2] == b'/' {
                let rest = &s[3..];
                let win_rest = rest.replace('/', "\\");
                return format!("{}:\\{}", drive_char.to_ascii_uppercase(), win_rest);
            }
        }
    }

    // Pseudo-root directories: /tmp -> C:\Users\<user>\AppData\Local\Temp or .\tmp
    if s == "/tmp" || s.starts_with("/tmp/") {
        let temp_dir = std::env::temp_dir();
        let temp_str = temp_dir.to_string_lossy();
        if s == "/tmp" {
            return temp_str.to_string();
        } else {
            let rest = &s[5..].replace('/', "\\");
            return format!("{}\\{}", temp_str.trim_end_matches('\\'), rest);
        }
    }

    // If it's already a Windows path (e.g., C:\foo or C:/foo)
    if is_win32_path(s) {
        return s.replace('/', "\\");
    }

    // Otherwise, normalize forward slashes to backslashes
    s.replace('/', "\\")
}

/// Translates a Windows-style path (e.g. `C:\Users\baile\dev`) to a POSIX-style path (e.g. `/c/Users/baile/dev`).
pub fn win32_to_posix(win_path: &str) -> String {
    let s = win_path.trim();
    if s.is_empty() {
        return String::new();
    }

    if s.eq_ignore_ascii_case("NUL") {
        return "/dev/null".to_string();
    }

    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
        let drive_char = (bytes[0] as char).to_ascii_lowercase();
        let rest = if bytes.len() > 2 {
            let r = &s[2..];
            let r_posix = r.replace('\\', "/");
            if !r_posix.starts_with('/') {
                format!("/{}", r_posix)
            } else {
                r_posix
            }
        } else {
            "/".to_string()
        };
        return format!("/{}{}", drive_char, rest);
    }

    s.replace('\\', "/")
}

/// Helper to check if a path starts with a Windows drive letter (e.g. `C:` or `D:/`).
pub fn is_win32_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic()
}

/// Translates a PATH list string (colon `:` separated on POSIX, semicolon `;` separated on Windows).
pub fn convert_path_list(path_list: &str, target_style: PathStyle) -> String {
    if path_list.is_empty() {
        return String::new();
    }

    let entries: Vec<&str> = if path_list.contains(';') {
        path_list.split(';').collect()
    } else {
        path_list.split(':').collect()
    };

    let converted: Vec<String> = entries
        .into_iter()
        .map(|entry| match target_style {
            PathStyle::Windows => posix_to_win32(entry),
            PathStyle::Posix => win32_to_posix(entry),
        })
        .collect();

    match target_style {
        PathStyle::Windows => converted.join(";"),
        PathStyle::Posix => converted.join(":"),
    }
}

/// Normalizes a path string, resolving `.` and `..` components cleanly without hit to file system.
pub fn normalize_path(path: &str) -> String {
    let is_abs_posix = path.starts_with('/');
    let is_win_drive = is_win32_path(path);
    let sep = if is_win_drive || path.contains('\\') { '\\' } else { '/' };

    let parts = path.split(&['/', '\\'][..]);
    let mut stack: Vec<&str> = Vec::new();

    for part in parts {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if !stack.is_empty() && stack.last() != Some(&"..") {
                stack.pop();
            } else if !is_abs_posix && !is_win_drive {
                stack.push("..");
            }
        } else {
            stack.push(part);
        }
    }

    let joined = stack.join(&sep.to_string());
    if is_abs_posix && !joined.starts_with('/') {
        format!("/{}", joined)
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_to_win32() {
        assert_eq!(posix_to_win32("/c/dev/Rusty_Mill"), r"C:\dev\Rusty_Mill");
        assert_eq!(posix_to_win32("/d/work/project/file.txt"), r"D:\work\project\file.txt");
        assert_eq!(posix_to_win32("/c"), r"C:\");
        assert_eq!(posix_to_win32("/dev/null"), r"NUL");
    }

    #[test]
    fn test_win32_to_posix() {
        assert_eq!(win32_to_posix(r"C:\dev\Rusty_Mill"), "/c/dev/Rusty_Mill");
        assert_eq!(win32_to_posix(r"D:\work\project\file.txt"), "/d/work/project/file.txt");
        assert_eq!(win32_to_posix("C:"), "/c/");
        assert_eq!(win32_to_posix("NUL"), "/dev/null");
    }

    #[test]
    fn test_convert_path_list() {
        let posix_paths = "/c/bin:/d/tools";
        let win_paths = convert_path_list(posix_paths, PathStyle::Windows);
        assert_eq!(win_paths, r"C:\bin;D:\tools");

        let back_to_posix = convert_path_list(&win_paths, PathStyle::Posix);
        assert_eq!(back_to_posix, "/c/bin:/d/tools");
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/c/dev/./Rusty_Mill/../foo"), "/c/dev/foo");
        assert_eq!(normalize_path(r"C:\dev\.\Rusty_Mill\..\foo"), r"C:\dev\foo");
    }
}
