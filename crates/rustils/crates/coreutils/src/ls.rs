//! `ls` over the platform traits: name listing, sorted for determinism
//! (the trait leaves order unspecified — see `docs/behavior/fs.md`).
//!
//! `ls_long`/`format_long` (coreutils gap backlog #65) render the
//! `-l` long-format line real `ls -l` produces — 10-char type+
//! permission string, link count, owner, group, size, and a
//! modification date, followed by the name (with `-> target` for
//! symlinks) — using the `Metadata::nlink`/`modified` and
//! `UnixMode::permissions` fields the gap backlog's #63/#64 work
//! added. Deliberately does *not* print `ls -l`'s leading "total N"
//! block-count line: `Metadata` has no allocated-block-count field,
//! and no consumer has asked for one (RFC v2 §3) — the per-entry line
//! format is this feature's whole point, not a byte-for-byte clone of
//! every `ls` output detail.

use std::ffi::OsString;
use std::time::SystemTime;

use platform::error::Result;
use platform::fs::{Dir, DirEntry, FileType, Metadata, UnixMode};

/// List entries of `dir`, sorted by name.
pub fn ls(dir: &dyn Dir) -> Result<Vec<DirEntry>> {
    let mut entries = dir.read_dir()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Render an entry the way the CLI prints it (dirs get a trailing slash).
pub fn render(entry: &DirEntry) -> String {
    let name = entry.name.to_string_lossy();
    match entry.file_type {
        FileType::Dir => format!("{name}/"),
        _ => name.into_owned(),
    }
}

/// Everything a long-format render needs for one entry, gathered up
/// front so `format_long` itself is pure and independently testable.
pub struct LongEntry {
    pub name: OsString,
    pub file_type: FileType,
    pub metadata: Metadata,
    pub unix_mode: Option<UnixMode>,
    pub symlink_target: Option<OsString>,
}

/// Lists `dir` (sorted, matching [`ls`]) and gathers each entry's
/// metadata/mode/symlink-target — the data `format_long` renders.
pub fn ls_long(dir: &dyn Dir) -> Result<Vec<LongEntry>> {
    let mut entries = dir.read_dir()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
        .into_iter()
        .map(|entry| {
            let metadata = dir.metadata(&entry.name)?;
            let unix_mode = dir.unix_mode(&entry.name)?;
            let symlink_target = if entry.file_type == FileType::Symlink {
                Some(dir.read_link(&entry.name)?)
            } else {
                None
            };
            Ok(LongEntry {
                name: entry.name,
                file_type: entry.file_type,
                metadata,
                unix_mode,
                symlink_target,
            })
        })
        .collect()
}

/// Renders `entries` the way real `ls -l` does, one line per entry,
/// columns aligned to the widest value present (matching `ls -l`'s own
/// per-listing alignment — not a fixed width).
///
/// `now` (the reference time "recent" is judged against) and
/// `resolve_user`/`resolve_group` (numeric id → display string) are
/// passed in rather than read from the OS, so this stays deterministic
/// and testable against `MockDir` — the real `rls -l` binary passes
/// `SystemTime::now()` and `platform_linux`'s
/// `user_name`/`group_name` (numeric-string fallback baked into the
/// closure when resolution fails or there's no `unix_mode` to resolve
/// at all, e.g. on Windows).
pub fn format_long(
    entries: &[LongEntry],
    now: SystemTime,
    mut resolve_user: impl FnMut(u32) -> String,
    mut resolve_group: impl FnMut(u32) -> String,
) -> Vec<String> {
    struct Rendered {
        type_perm: String,
        nlink: String,
        owner: String,
        group: String,
        size: String,
        date: String,
        name: String,
    }

    let rendered: Vec<Rendered> = entries
        .iter()
        .map(|e| Rendered {
            type_perm: type_perm_string(e.file_type, e.unix_mode),
            nlink: e.metadata.nlink.to_string(),
            owner: e
                .unix_mode
                .map_or_else(|| "-".to_string(), |m| resolve_user(m.uid)),
            group: e
                .unix_mode
                .map_or_else(|| "-".to_string(), |m| resolve_group(m.gid)),
            size: e.metadata.len.to_string(),
            date: format_date(e.metadata.modified, now),
            name: match &e.symlink_target {
                Some(target) => {
                    format!(
                        "{} -> {}",
                        e.name.to_string_lossy(),
                        target.to_string_lossy()
                    )
                }
                None => e.name.to_string_lossy().into_owned(),
            },
        })
        .collect();

    let nlink_w = rendered.iter().map(|r| r.nlink.len()).max().unwrap_or(0);
    let owner_w = rendered.iter().map(|r| r.owner.len()).max().unwrap_or(0);
    let group_w = rendered.iter().map(|r| r.group.len()).max().unwrap_or(0);
    let size_w = rendered.iter().map(|r| r.size.len()).max().unwrap_or(0);

    rendered
        .iter()
        .map(|r| {
            format!(
                "{} {:>nlink_w$} {:<owner_w$} {:<group_w$} {:>size_w$} {} {}",
                r.type_perm, r.nlink, r.owner, r.group, r.size, r.date, r.name,
            )
        })
        .collect()
}

/// The 10-character type+permission string (`-rwxr-xr-x`, `drwxr-xr-x`,
/// `lrwxrwxrwx`, …). `None` (Windows: no POSIX permission concept at
/// all) renders as all dashes after the type character — an honest
/// "nothing to show" rather than a fabricated `rwx` guess, the same
/// choice `Dir::unix_mode` itself already makes by returning `None`
/// there instead of a zeroed-out `Some`.
fn type_perm_string(file_type: FileType, unix_mode: Option<UnixMode>) -> String {
    let type_char = match file_type {
        FileType::File => '-',
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        _ => '?', // FileType is #[non_exhaustive]; Other and anything future.
    };
    let mut s = String::with_capacity(10);
    s.push(type_char);
    let Some(mode) = unix_mode else {
        s.push_str("---------");
        return s;
    };
    let bits = mode.permissions;
    let triplet = |read_bit, write_bit, exec_bit, special: bool, lower: char, upper: char| {
        let mut t = String::with_capacity(3);
        t.push(if bits & read_bit != 0 { 'r' } else { '-' });
        t.push(if bits & write_bit != 0 { 'w' } else { '-' });
        let exec = bits & exec_bit != 0;
        t.push(match (special, exec) {
            (true, true) => lower,
            (true, false) => upper,
            (false, true) => 'x',
            (false, false) => '-',
        });
        t
    };
    s.push_str(&triplet(0o400, 0o200, 0o100, mode.setuid, 's', 'S'));
    s.push_str(&triplet(0o040, 0o020, 0o010, mode.setgid, 's', 'S'));
    s.push_str(&triplet(0o004, 0o002, 0o001, mode.sticky, 't', 'T'));
    s
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `modified`'s seconds-since-epoch as a signed `i64` — `SystemTime`'s
/// own `duration_since` errors on a time before its argument (i.e. a
/// pre-1970 `modified`), since `Duration` can't be negative; this
/// folds that into a plain signed count instead of propagating the
/// error for what real files essentially never hit anyway.
fn unix_seconds(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Days since the Unix epoch (1970-01-01) → (year, month, day),
/// proleptic Gregorian calendar. Howard Hinnant's public-domain
/// `civil_from_days` algorithm — re-derived here rather than adding a
/// date/time crate dependency for one calendar conversion, matching
/// `docs/versioning.md`'s general "vendor the small thing" bias.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn ymdhm_from_unix_secs(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let secs_of_day = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;
    (y, m, d, hour, minute)
}

/// `ls -l`'s own date rendering: `"Mon DD HH:MM"` for a file modified
/// within about the last 6 months (and not in the future), else
/// `"Mon DD  YYYY"` — both exactly 12 characters, by construction, so
/// this field needs no per-listing column-width logic of its own,
/// unlike the others `format_long` aligns.
fn format_date(modified: SystemTime, now: SystemTime) -> String {
    const SIX_MONTHS_SECS: i64 = 6 * 30 * 24 * 60 * 60;
    let modified_secs = unix_seconds(modified);
    let now_secs = unix_seconds(now);
    let recent = now_secs >= modified_secs && now_secs - modified_secs < SIX_MONTHS_SECS;
    let (year, month, day, hour, minute) = ymdhm_from_unix_secs(modified_secs);
    let mon = MONTH_NAMES[(month - 1) as usize];
    if recent {
        format!("{mon} {day:2} {hour:02}:{minute:02}")
    } else {
        format!("{mon} {day:2}  {year}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_mock::MockDir;
    use std::ffi::OsStr;

    #[test]
    fn ls_sorts_and_types() {
        let root = MockDir::root()
            .with_file("b.txt", "x")
            .with_file("a.txt", "y");
        root.create_dir(OsStr::new("z-dir")).expect("mkdir");
        let entries = ls(&root).expect("ls");
        let rendered: Vec<_> = entries.iter().map(render).collect();
        assert_eq!(rendered, vec!["a.txt", "b.txt", "z-dir/"]);
    }

    /// `now` fixed well after the epoch (`MockDir`'s deterministic
    /// `modified` value) so every mock entry lands in the "old" —
    /// year-shown — branch of `format_date`, keeping this test's
    /// expected strings independent of when it happens to run.
    fn fixed_now() -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(365 * 24 * 60 * 60)
    }

    #[test]
    fn format_long_matches_real_ls_l_shape() {
        let root = MockDir::root().with_file("a.txt", "hello!");
        root.create_dir(OsStr::new("d")).expect("mkdir");
        let entries = ls_long(&root).expect("ls_long");
        let lines = format_long(
            &entries,
            fixed_now(),
            |uid| uid.to_string(),
            |gid| gid.to_string(),
        );

        // MockDir::unix_mode is always `Some(UnixMode::default())`
        // (permissions 0, uid/gid 0) and `metadata` always reports
        // `nlink: 1`/`modified: UNIX_EPOCH` — deterministic, so the
        // exact rendered strings are assertable, not just their shape.
        assert_eq!(
            lines,
            vec![
                "---------- 1 0 0 6 Jan  1  1970 a.txt",
                "d--------- 1 0 0 0 Jan  1  1970 d",
            ]
        );
    }

    #[test]
    fn format_long_renders_symlink_targets() {
        let root = MockDir::root();
        root.symlink(OsStr::new("a.txt"), OsStr::new("link"))
            .expect("symlink");
        let entries = ls_long(&root).expect("ls_long");
        let lines = format_long(
            &entries,
            fixed_now(),
            |uid| uid.to_string(),
            |gid| gid.to_string(),
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("link -> a.txt"), "got: {}", lines[0]);
        assert_eq!(&lines[0][0..1], "l", "type char must be 'l' for a symlink");
    }

    #[test]
    fn format_long_uses_the_supplied_name_resolver() {
        let root = MockDir::root().with_file("a.txt", "x");
        let entries = ls_long(&root).expect("ls_long");
        let lines = format_long(
            &entries,
            fixed_now(),
            |_uid| "alice".to_string(),
            |_gid| "staff".to_string(),
        );
        assert!(lines[0].contains(" alice staff "), "got: {}", lines[0]);
    }

    #[test]
    fn format_date_switches_to_the_year_after_six_months() {
        // `now` = 1970-07-20. 170 days earlier is 1970-01-31 (under 6
        // months: recent, clock time); 190 days earlier is 1970-01-11
        // (over 6 months: year shown instead).
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(200 * 24 * 60 * 60);
        let recent = now - std::time::Duration::from_secs(170 * 24 * 60 * 60);
        assert_eq!(format_date(recent, now), "Jan 31 00:00");
        let old = now - std::time::Duration::from_secs(190 * 24 * 60 * 60);
        assert_eq!(format_date(old, now), "Jan 11  1970");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0 by definition.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-03-01 is a well-known fixed point for this algorithm
        // (the day right after the era boundary it's built around).
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
    }
}
