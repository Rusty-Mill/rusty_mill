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
//! it could not measure; with `--report-only` that is printed and exits 0,
//! so a report-only profile never blocks a claim.

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
    let changed = changed_paths(&opts.base)?;
    let mut parser = Parser::new().map_err(|e| e.to_string())?;
    let base_files = blobs(&opts.base, &tree_paths(&opts.base)?)?;
    let base_measures = measure_all(&mut parser, &base_files)?;
    let base = Report::build(
        base_measures.values().cloned().collect(),
        unmeasured(&base_files),
    );
    let cand_files = worktree(&changed)?;
    let mut cand_measures = base_measures;
    cand_measures.retain(|path, _| !changed.contains(path));
    cand_measures.extend(measure_all(&mut parser, &cand_files)?);
    let mut cand_unmeasured = unmeasured(&base_files);
    cand_unmeasured.retain(|p| !changed.contains(p));
    cand_unmeasured.extend(unmeasured(&cand_files));
    let cand = Report::build(cand_measures.into_values().collect(), cand_unmeasured);
    // Changed code it cannot measure: another language (deleted included),
    // or Rust that is not UTF-8.
    let unreadable = unmeasured(&cand_files);
    let changed_unmeasured: Vec<&String> = changed
        .iter()
        .filter(|p| classify(p) == FileClass::Unmeasured || unreadable.contains(p))
        .collect();
    let delta = base.delta(&cand);
    let within = opts.limits.is_none_or(|l| {
        delta.concentration_bp <= l.concentration_bp && delta.duplication_bp <= l.duplication_bp
    });
    let line = render(opts, &base, &cand, within, &changed_unmeasured);
    Ok((line, within))
}

fn measure_all(
    parser: &mut Parser,
    files: &[(String, Vec<u8>)],
) -> Result<HashMap<String, FileMeasure>, String> {
    let mut out = HashMap::new();
    for (path, bytes) in files {
        if classify(path) != FileClass::Rust {
            continue;
        }
        let Ok(src) = std::str::from_utf8(bytes) else {
            continue;
        };
        out.insert(
            path.clone(),
            parser.measure(path, src).map_err(|e| e.to_string())?,
        );
    }
    Ok(out)
}

/// Code files v0 cannot measure, including Rust that is not UTF-8.
fn unmeasured(files: &[(String, Vec<u8>)]) -> Vec<String> {
    files
        .iter()
        .filter(|(p, b)| match classify(p) {
            FileClass::Unmeasured => true,
            FileClass::Rust => std::str::from_utf8(b).is_err(),
            FileClass::Ignored => false,
        })
        .map(|(p, _)| p.clone())
        .collect()
}

/// Paths the working tree changes against `base`: modified, added, deleted
/// and untracked-unignored.
fn changed_paths(base: &str) -> Result<BTreeSet<String>, String> {
    let mut out = split_z(&git(&[
        "diff",
        "--name-only",
        "--no-renames",
        "-z",
        base,
        "--",
    ])?);
    out.extend(split_z(&git(&[
        "ls-files",
        "-z",
        "--others",
        "--exclude-standard",
    ])?));
    Ok(out.into_iter().collect())
}

/// Measured or unmeasured paths in `base`'s tree.
fn tree_paths(base: &str) -> Result<Vec<String>, String> {
    let all = split_z(&git(&["ls-tree", "-r", "-z", "--name-only", base])?);
    Ok(all
        .into_iter()
        .filter(|p| classify(p) != FileClass::Ignored)
        .collect())
}

/// The changed files' working-tree contents; deleted ones are left out.
fn worktree(changed: &BTreeSet<String>) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for path in changed.iter().filter(|p| classify(p) != FileClass::Ignored) {
        match std::fs::read(path) {
            Ok(bytes) => out.push((path.clone(), bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{path}: {e}")),
        }
    }
    Ok(out)
}

/// `base:path` blobs through one `git cat-file --batch`.
fn blobs(base: &str, paths: &[String]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut child = git_cmd()
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git cat-file: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("git cat-file: no stdin")?;
    let request: String = paths.iter().map(|p| format!("{base}:{p}\n")).collect();
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
    parse_batch(paths, &raw)
}

/// Splits `git cat-file --batch` output, one entry per requested path.
fn parse_batch(paths: &[String], mut raw: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let nl = raw
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("git cat-file: truncated header")?;
        let header = std::str::from_utf8(&raw[..nl]).map_err(|_| "git cat-file: bad header")?;
        let fields: Vec<&str> = header.split(' ').collect();
        if let [_, "missing"] = fields[..] {
            // A gitlink (submodule) has no blob in this repository.
            raw = &raw[nl + 1..];
            continue;
        }
        let [_, kind, size] = fields[..] else {
            return Err(format!("git cat-file: {path}: {header}"));
        };
        let size: usize = size
            .parse()
            .map_err(|_| format!("git cat-file: bad size in {header}"))?;
        let body = raw
            .get(nl + 1..nl + 1 + size)
            .ok_or("git cat-file: truncated body")?;
        if kind == "blob" {
            out.push((path.clone(), body.to_vec()));
        }
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

fn split_z(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
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
    fn batch_output_is_split_per_path() {
        let raw = b"aaa blob 3\nfoo\nbbb missing\nccc blob 0\n\n";
        let paths = ["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        let got = parse_batch(&paths, raw);
        let want = vec![
            ("a.rs".to_string(), b"foo".to_vec()),
            ("c.rs".to_string(), vec![]),
        ];
        assert_eq!(got, Ok(want));
        assert!(parse_batch(&paths, b"aaa blob 9\nfoo\n").is_err());
    }
}
