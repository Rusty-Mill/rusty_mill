//! `erosion`: the erosion profile's command (card #704 slice 2, D41, D42).
//!
//! ```text
//! erosion check --base COMMIT (--report-only | --max-concentration-bp N --max-duplication-bp N)
//! ```
//!
//! Run in a git checkout. It measures the Rust code of the `--base` commit
//! and of the working tree (tracked files plus untracked, unignored ones),
//! and prints one JSON line: both levels, the deltas, the three
//! worst-affected functions, and changed code it cannot measure.
//!
//! Exit status: 0 when within the limits, and always 0 with `--report-only`
//! (the deltas are still printed). 1 when a delta exceeds its limit. 2 when
//! it could not measure, including Rust that is not UTF-8 in either the base
//! or the working tree; with `--report-only` that is printed and exits 0
//! (unreadable Rust is listed as `unmeasured`), so a report-only profile
//! never blocks a claim.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

use rusty_erosion::adapter::rust::Parser;
use rusty_erosion::{Affected, FileClass, FileMeasure, Report, classify};

/// Functions listed in an escalation (D43).
const WORST: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    base: String,
    limits: Option<Limits>,
}

/// Largest allowed rise of each share, in basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Limits {
    concentration_bp: i32,
    duplication_bp: i32,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("erosion: {e}");
            return ExitCode::from(2);
        }
    };
    match check(&opts) {
        Ok((line, within)) => {
            println!("{line}");
            match within || opts.limits.is_none() {
                true => ExitCode::SUCCESS,
                false => ExitCode::from(1),
            }
        }
        Err(e) => {
            println!("{{\"erosion\":1,\"error\":{}}}", json_str(&e));
            match opts.limits {
                None => ExitCode::SUCCESS,
                Some(_) => ExitCode::from(2),
            }
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let usage = "usage: erosion check --base COMMIT (--report-only | --max-concentration-bp N --max-duplication-bp N)";
    let Some((cmd, rest)) = args.split_first() else {
        return Err(usage.into());
    };
    if cmd != "check" {
        return Err(usage.into());
    }
    let mut base = None;
    let mut report_only = false;
    let (mut conc, mut dup) = (None, None);
    let mut seen: Vec<&str> = Vec::new();
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        // A repeated flag is ambiguous: the host validates one `--base`, so
        // the binary must not run another (D82).
        if seen.contains(&flag.as_str()) {
            return Err(format!("{flag} given more than once"));
        }
        seen.push(flag);
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--base" => base = Some(value()?),
            "--report-only" => report_only = true,
            "--max-concentration-bp" => conc = Some(parse_bp(&value()?)?),
            "--max-duplication-bp" => dup = Some(parse_bp(&value()?)?),
            other => return Err(format!("unknown argument {other:?}; {usage}")),
        }
    }
    let base = base.ok_or("--base is required")?;
    if !is_commit_id(&base) {
        return Err(format!("--base {base:?} is not a full commit id"));
    }
    let limits = match (report_only, conc, dup) {
        (true, None, None) => None,
        (false, Some(c), Some(d)) => Some(Limits {
            concentration_bp: c,
            duplication_bp: d,
        }),
        (true, _, _) => return Err("--report-only takes no limits".into()),
        (false, _, _) => return Err(format!("give --report-only or both limits; {usage}")),
    };
    Ok(Options { base, limits })
}

fn parse_bp(s: &str) -> Result<i32, String> {
    s.parse()
        .map_err(|_| format!("{s:?} is not a whole number of basis points"))
}

/// 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
fn is_commit_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Measures both trees; returns the JSON line and whether the deltas are
/// within the limits (true when there are none).
fn check(opts: &Options) -> Result<(String, bool), String> {
    // Every git and file path below is relative to the repository root, so
    // work from there wherever the profile was started.
    let top = git(&["rev-parse", "--show-toplevel"])?;
    let top = String::from_utf8(top).map_err(|_| "repository root is not UTF-8".to_string())?;
    std::env::set_current_dir(top.trim_end_matches('\n'))
        .map_err(|e| format!("repository root: {e}"))?;
    git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{}^{{commit}}", opts.base),
    ])?;
    let (changed, mangled) = changed_paths(&opts.base)?;
    let mut parser = Parser::new().map_err(|e| e.to_string())?;
    let base_snap = base_snapshot(&opts.base)?;
    let cand_snap = worktree(&changed, &mangled)?;
    let base_measures = measure_all(&mut parser, &base_snap.rust)?;
    let base = Report::build(
        base_measures.values().cloned().collect(),
        base_snap.unmeasured().cloned().collect(),
    );
    // Unchanged files are the base's entries; changed ones come from disk.
    let mut cand_measures = base_measures;
    cand_measures.retain(|path, _| !changed.contains(path));
    cand_measures.extend(measure_all(&mut parser, &cand_snap.rust)?);
    let cand_unmeasured = base_snap
        .unmeasured()
        .filter(|p| !changed.contains(*p))
        .chain(cand_snap.unmeasured())
        .cloned()
        .collect();
    let cand = Report::build(cand_measures.into_values().collect(), cand_unmeasured);
    let bad_rust: BTreeSet<&String> = base_snap
        .unreadable
        .iter()
        .chain(&cand_snap.unreadable)
        .collect();
    if opts.limits.is_some() && !bad_rust.is_empty() {
        // Supported code that cannot be read leaves a snapshot incomplete, a
        // measurement failure rather than `unmeasured` (D45 covers other
        // languages only): fail closed.
        return Err(format!(
            "cannot measure Rust (not a regular file, or a name or contents that are not UTF-8): {}",
            bad_rust
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // Changed code in another language (deleted included), and unreadable
    // Rust in either snapshot.
    let unmeasured_paths: BTreeSet<&String> = cand_snap.other.iter().chain(bad_rust).collect();
    let changed_unmeasured: Vec<&String> = unmeasured_paths.into_iter().collect();
    let delta = base.delta(&cand);
    let within = opts.limits.is_none_or(|l| {
        delta.concentration_bp <= l.concentration_bp && delta.duplication_bp <= l.duplication_bp
    });
    let line = render(opts, &base, &cand, within, &changed_unmeasured);
    Ok((line, within))
}

fn measure_all(
    parser: &mut Parser,
    files: &[(String, String)],
) -> Result<HashMap<String, FileMeasure>, String> {
    let mut out = HashMap::new();
    for (path, src) in files {
        out.insert(
            path.clone(),
            parser.measure(path, src).map_err(|e| e.to_string())?,
        );
    }
    Ok(out)
}

/// One side of the comparison, as far as v0 can read it.
#[derive(Debug, Default)]
struct Snapshot {
    /// Rust source: regular files whose names and contents are UTF-8.
    rust: Vec<(String, String)>,
    /// Rust it cannot measure as source: not a regular file, or a name or
    /// contents that are not UTF-8.
    unreadable: BTreeSet<String>,
    /// Code in another language (D45): listed, never read.
    other: BTreeSet<String>,
}

impl Snapshot {
    fn add_rust(&mut self, path: String, bytes: Vec<u8>) {
        match String::from_utf8(bytes) {
            Ok(src) => self.rust.push((path, src)),
            Err(_) => {
                self.unreadable.insert(path);
            }
        }
    }

    /// What its report lists as unmeasured.
    fn unmeasured(&self) -> impl Iterator<Item = &String> {
        self.other.iter().chain(&self.unreadable)
    }
}

/// Paths the working tree changes against `base`: modified, added, deleted
/// and untracked-unignored. The second set holds those whose names are not
/// UTF-8, as lossy text for diagnostics only: they cannot be named exactly,
/// so are never opened, and never mixed with the exact names.
fn changed_paths(base: &str) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    let mut raw = git(&["diff", "--name-only", "--no-renames", "-z", base, "--"])?;
    raw.push(0);
    raw.extend(git(&["ls-files", "-z", "--others", "--exclude-standard"])?);
    let mut all = BTreeSet::new();
    let mut mangled = BTreeSet::new();
    for name in split_z(&raw) {
        match name {
            Ok(path) => all.insert(path),
            Err(lossy) => mangled.insert(lossy),
        };
    }
    Ok((all, mangled))
}

/// `base`'s tree. Rust blobs are read by object id, so no path name can
/// break the batch framing; a Rust entry that is not a regular file (a
/// symlink's blob is its target path) or has a name that is not UTF-8 is
/// unreadable. Submodules are skipped; other languages are only listed.
fn base_snapshot(base: &str) -> Result<Snapshot, String> {
    let mut snap = Snapshot::default();
    let mut wanted = Vec::new();
    for entry in git(&["ls-tree", "-r", "-z", base])?.split(|b| *b == 0) {
        if entry.is_empty() {
            continue;
        }
        // `<mode> <type> <object>\t<path>`
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or("git ls-tree: bad entry")?;
        let meta = std::str::from_utf8(&entry[..tab]).map_err(|_| "git ls-tree: bad entry")?;
        let (path, exact) = match std::str::from_utf8(&entry[tab + 1..]) {
            Ok(p) => (p.to_owned(), true),
            Err(_) => (
                String::from_utf8_lossy(&entry[tab + 1..]).into_owned(),
                false,
            ),
        };
        let fields: Vec<&str> = meta.split(' ').collect();
        let [mode, _, oid] = fields[..] else {
            return Err(format!("git ls-tree: bad entry {meta:?}"));
        };
        match classify(&path) {
            FileClass::Ignored => {}
            FileClass::Unmeasured => {
                snap.other.insert(path);
            }
            FileClass::Rust if mode == "160000" => {}
            FileClass::Rust if exact && matches!(mode, "100644" | "100755") => {
                wanted.push((path, oid.to_owned()));
            }
            FileClass::Rust => {
                snap.unreadable.insert(path);
            }
        }
    }
    let oids: Vec<&str> = wanted.iter().map(|(_, o)| o.as_str()).collect();
    for ((path, _), bytes) in wanted.iter().zip(blobs(&oids)?) {
        snap.add_rust(path.clone(), bytes);
    }
    Ok(snap)
}

/// The changed files in the working tree, read as git sees them: Rust is
/// read only when it is a regular file reached through real directories of
/// this repository (a symlinked or replaced ancestor, or a gitlink one,
/// means git sees it deleted). A gitlink itself is skipped, as in the base,
/// and anything else that is there is unreadable. Gitlinks come from the
/// index, not from finding a `.git`: the parent can still track files in a
/// directory that holds a nested repository. Other languages are not touched at all
/// (D45), so nothing about them, not even a failed lookup, decides the
/// result.
fn worktree(changed: &BTreeSet<String>, mangled: &BTreeSet<String>) -> Result<Snapshot, String> {
    let gitlinks = index_gitlinks()?;
    let mut snap = Snapshot::default();
    // Names that are not UTF-8 are listed by their lossy text, never opened.
    for name in mangled {
        match classify(name) {
            FileClass::Ignored => {}
            FileClass::Unmeasured => {
                snap.other.insert(name.clone());
            }
            FileClass::Rust => {
                snap.unreadable.insert(name.clone());
            }
        }
    }
    for path in changed {
        match classify(path) {
            FileClass::Ignored => {}
            FileClass::Unmeasured => {
                snap.other.insert(path.clone());
            }
            FileClass::Rust if gitlinks.contains(path) => {}
            FileClass::Rust => match entry(path, &gitlinks)? {
                Entry::Absent => {}
                Entry::Special => {
                    snap.unreadable.insert(path.clone());
                }
                Entry::File(bytes) => snap.add_rust(path.clone(), bytes),
            },
        }
    }
    Ok(snap)
}

/// Index entries with mode `160000`: the candidate's submodules.
fn index_gitlinks() -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for entry in git(&["ls-files", "-s", "-z"])?.split(|b| *b == 0) {
        // `<mode> <object> <stage>\t<path>`
        let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
            continue;
        };
        // Only exact names: they are matched against exact source paths,
        // and a lossy one could equal a different, real directory.
        let exact = std::str::from_utf8(&entry[tab + 1..]);
        if let (true, Ok(path)) = (entry.starts_with(b"160000 "), exact) {
            out.insert(path.to_owned());
        }
    }
    Ok(out)
}

/// A working-tree path, without following any symlink.
enum Entry {
    /// Deleted, as git sees it.
    Absent,
    /// There, but not a regular file or directory: a symlink, FIFO or socket.
    Special,
    File(Vec<u8>),
}

fn entry(path: &str, gitlinks: &BTreeSet<String>) -> Result<Entry, String> {
    let gone = |e: &std::io::Error| {
        matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    };
    let mut dir = std::path::PathBuf::new();
    let mut parts: Vec<&str> = path.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    let mut prefix = String::new();
    for part in parts {
        dir.push(part);
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        // Under a gitlink is the submodule's tree, not this one's.
        if gitlinks.contains(&prefix) {
            return Ok(Entry::Absent);
        }
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Ok(Entry::Absent),
            Err(e) if gone(&e) => return Ok(Entry::Absent),
            Err(e) => return Err(format!("{path}: {e}")),
        }
    }
    dir.push(name);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.is_file() => std::fs::read(&dir)
            .map(Entry::File)
            .map_err(|e| format!("{path}: {e}")),
        // A file since replaced by a directory: git sees it deleted.
        Ok(m) if m.is_dir() => Ok(Entry::Absent),
        Ok(_) => Ok(Entry::Special),
        Err(e) if gone(&e) => Ok(Entry::Absent),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Blobs by object id through one `git cat-file --batch`, in order.
fn blobs(oids: &[&str]) -> Result<Vec<Vec<u8>>, String> {
    let mut child = git_cmd()
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git cat-file: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("git cat-file: no stdin")?;
    let request: String = oids.iter().map(|o| format!("{o}\n")).collect();
    let writer = std::thread::spawn(move || stdin.write_all(request.as_bytes()));
    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .ok_or("git cat-file: no stdout")?
        .read_to_end(&mut raw)
        .map_err(|e| format!("git cat-file: {e}"))?;
    let wrote = writer
        .join()
        .map_err(|_| "git cat-file: writer panicked".to_string())?;
    wrote.map_err(|e| format!("git cat-file: {e}"))?;
    let status = child.wait().map_err(|e| format!("git cat-file: {e}"))?;
    if !status.success() {
        return Err(format!("git cat-file exited {status}"));
    }
    parse_batch(oids, &raw)
}

/// Splits `git cat-file --batch` output: one blob per requested object, in
/// order. A missing object or any other type is a measurement failure.
fn parse_batch(oids: &[&str], mut raw: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::with_capacity(oids.len());
    for oid in oids {
        let nl = raw
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("git cat-file: truncated header")?;
        let header = std::str::from_utf8(&raw[..nl]).map_err(|_| "git cat-file: bad header")?;
        let fields: Vec<&str> = header.split(' ').collect();
        let [got, "blob", size] = fields[..] else {
            return Err(format!("git cat-file: {oid}: {header}"));
        };
        if got != *oid {
            return Err(format!("git cat-file: asked for {oid}, got {header}"));
        }
        let size: usize = size
            .parse()
            .map_err(|_| format!("git cat-file: bad size in {header}"))?;
        let body = raw
            .get(nl + 1..nl + 1 + size)
            .ok_or("git cat-file: truncated body")?;
        out.push(body.to_vec());
        raw = raw.get(nl + 2 + size..).unwrap_or_default();
    }
    Ok(out)
}

fn git(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = git_cmd()
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {}: {}", args.join(" "), err.trim()));
    }
    Ok(out.stdout)
}

/// git without system or user configuration: the profile's sandbox cannot
/// read `/etc/gitconfig`, and a user's settings (`diff.noprefix`, external
/// diff drivers) must not change what is measured.
fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

/// NUL-separated names: `Ok` when UTF-8, else `Err` with lossy text.
fn split_z(raw: &[u8]) -> Vec<Result<String, String>> {
    raw.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8(s.to_vec()).map_err(|_| String::from_utf8_lossy(s).into_owned()))
        .collect()
}

/// The one JSON line the run log carries for the inbox (slice 3).
fn render(
    opts: &Options,
    base: &Report,
    cand: &Report,
    within: bool,
    changed: &[&String],
) -> String {
    let d = base.delta(cand);
    let share = |name: &str, b: u32, c: u32, delta: i32| {
        format!("\"{name}\":{{\"base\":{b},\"candidate\":{c},\"delta\":{delta}}}")
    };
    let affected = |a: &Affected| {
        format!(
            "{{\"key\":{},\"base_cc\":{},\"cc\":{}}}",
            json_str(&a.key),
            a.base_cc,
            a.cc
        )
    };
    let list = |items: Vec<String>| format!("[{}]", items.join(","));
    let mode = match opts.limits {
        None => "\"report-only\"".to_string(),
        Some(l) => format!(
            "{{\"max_concentration_bp\":{},\"max_duplication_bp\":{}}}",
            l.concentration_bp, l.duplication_bp
        ),
    };
    let fields = [
        "\"erosion\":1".to_string(),
        format!("\"base\":{}", json_str(&opts.base)),
        format!("\"mode\":{mode}"),
        // No limits to be within while report-only.
        match opts.limits {
            None => "\"within_limits\":null".to_string(),
            Some(_) => format!("\"within_limits\":{within}"),
        },
        share(
            "concentration_bp",
            base.concentration.bp(),
            cand.concentration.bp(),
            d.concentration_bp,
        ),
        share(
            "duplication_bp",
            base.duplication.bp(),
            cand.duplication.bp(),
            d.duplication_bp,
        ),
        format!(
            "\"total_cc\":{},\"loc\":{},\"functions\":{}",
            d.total_cc, d.loc, d.functions
        ),
        format!(
            "\"worst_affected\":{}",
            list(
                base.worst_affected(cand, WORST)
                    .iter()
                    .map(affected)
                    .collect()
            )
        ),
        format!(
            "\"unmeasured\":{}",
            list(changed.iter().map(|p| json_str(p)).collect())
        ),
        format!(
            "\"parse_errors\":{}",
            list(cand.parse_errors.iter().map(|p| json_str(p)).collect())
        ),
    ];
    format!("{{{}}}", fields.join(","))
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    const C: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn parses_report_only_and_limits() {
        let ro = parse_args(&args(&format!("check --base {C} --report-only")));
        assert_eq!(
            ro,
            Ok(Options {
                base: C.into(),
                limits: None
            })
        );
        let lim = parse_args(&args(&format!(
            "check --base {C} --max-concentration-bp 150 --max-duplication-bp 100"
        )));
        let want = Limits {
            concentration_bp: 150,
            duplication_bp: 100,
        };
        assert_eq!(lim.map(|o| o.limits), Ok(Some(want)));
    }

    #[test]
    fn refuses_ambiguous_or_missing_modes() {
        for bad in [
            format!("check --base {C}"),
            format!("check --base {C} --max-concentration-bp 1"),
            format!("check --base {C} --report-only --max-duplication-bp 1"),
            "check --base HEAD --report-only".to_string(),
            format!("check --base {C} --report-only --oops"),
            format!("check --base {C} --base {} --report-only", "f".repeat(40)),
            format!("check --base {C} --report-only --report-only"),
            "measure".to_string(),
        ] {
            assert!(parse_args(&args(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("a\"b\\c\nd"), "\"a\\\"b\\\\c\\u000ad\"");
    }

    #[test]
    fn batch_output_is_split_per_object() {
        let raw = b"aaa blob 3\nfoo\nccc blob 0\n\n";
        assert_eq!(
            parse_batch(&["aaa", "ccc"], raw),
            Ok(vec![b"foo".to_vec(), vec![]])
        );
        assert!(parse_batch(&["aaa"], b"aaa blob 9\nfoo\n").is_err());
        // A missing object, another type or a reordered reply fails.
        assert!(parse_batch(&["bbb"], b"bbb missing\n").is_err());
        assert!(parse_batch(&["aaa"], b"aaa tree 0\n\n").is_err());
        assert!(parse_batch(&["aaa", "ccc"], b"ccc blob 0\n\naaa blob 0\n\n").is_err());
    }
}
