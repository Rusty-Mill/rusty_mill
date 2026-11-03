//! Golden sets (#597 section 3): small sets of real cases with known answers,
//! signed off once by the owner, run as `golden:` profiles of the holdout
//! suite so no model sees their inputs or outputs.
//!
//! ```text
//! golden/<intent>/
//!   cases/<case>/case.toml   cmd, args, stdin?, [expect] exit, stdout | stdout_file | json_file, stderr_contains?
//!   fixtures/...             or absolute host-only paths for private data
//!   MANIFEST                 "<sha256>  <path>" per file, then "set <sha256 of the lines above>"
//!   SIGNOFF                  digest = "<set digest>", signer = "...", date = "..."
//! ```
//!
//! A task opens with a golden set only if every file matches `MANIFEST` and
//! `SIGNOFF` names its set digest (`bad_golden` otherwise). `bbp golden-run`
//! is the generic case runner the `golden:` profile executes inside the
//! sandbox: it re-checks the sign-off, runs each case in the candidate's
//! tree and compares exactly, or as JSON.

use rusty_bbp::Sha256;
use rusty_codec::TomlValue;
use rusty_serde::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "MANIFEST";
pub const SIGNOFF: &str = "SIGNOFF";
/// In a case's `args`, replaced by the golden set's absolute directory.
pub const DIR_TOKEN: &str = "{golden}";

// ---------------------------------------------------------------- sign-off

/// The set digest of `dir` when its files match `MANIFEST` exactly and
/// `SIGNOFF` names that digest; otherwise why not.
pub fn check_signed(dir: &Path) -> Result<Sha256, String> {
    let text = read_text(&dir.join(MANIFEST))?;
    let (listed, digest) = parse_manifest(&text)?;
    let mut have = local_files(dir)?;
    for (path, _) in listed.iter().filter(|(p, _)| Path::new(p).is_absolute()) {
        let bytes = std::fs::read(path).map_err(|e| format!("fixture {path}: {e}"))?;
        have.insert(path.clone(), Sha256::of(&bytes));
    }
    if have != listed {
        let differ: Vec<&str> = listed
            .keys()
            .chain(have.keys())
            .filter(|p| listed.get(*p) != have.get(*p))
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(format!("files differ from MANIFEST: {}", differ.join(", ")));
    }
    let signed = signoff_digest(&read_text(&dir.join(SIGNOFF))?)?;
    if signed != digest {
        return Err(format!(
            "SIGNOFF names {}, the set is {}: it needs a new sign-off",
            signed.hex(),
            digest.hex()
        ));
    }
    Ok(digest)
}

/// `MANIFEST` text: each file's digest, and the set digest (SHA-256 of every
/// line before the `set` line, each ending in a newline), which must match.
/// Pure.
pub fn parse_manifest(text: &str) -> Result<(BTreeMap<String, Sha256>, Sha256), String> {
    let mut files = BTreeMap::new();
    let mut body = String::new();
    for line in text.lines() {
        if let Some(hex) = line.strip_prefix("set ") {
            let set = Sha256::from_hex(hex.trim()).ok_or("MANIFEST: bad set digest")?;
            if set != Sha256::of(body.as_bytes()) {
                return Err("MANIFEST: the set digest does not match its lines".into());
            }
            return Ok((files, set));
        }
        let (hex, path) = line
            .split_once("  ")
            .ok_or_else(|| format!("MANIFEST: bad line {line:?}"))?;
        let sha = Sha256::from_hex(hex).ok_or_else(|| format!("MANIFEST: bad digest {hex:?}"))?;
        if path.split('/').any(|s| s == "..") || path == MANIFEST || path == SIGNOFF {
            return Err(format!("MANIFEST: path {path:?} is not allowed"));
        }
        if files.insert(path.to_owned(), sha).is_some() {
            return Err(format!("MANIFEST: {path} listed twice"));
        }
        body.push_str(line);
        body.push('\n');
    }
    Err("MANIFEST: no set line".into())
}

/// `MANIFEST` text for `files`, ending in its set line. Pure.
pub fn manifest_text(files: &BTreeMap<String, Sha256>) -> String {
    let body: String = files
        .iter()
        .map(|(p, s)| format!("{}  {p}\n", s.hex()))
        .collect();
    format!("{body}set {}\n", Sha256::of(body.as_bytes()).hex())
}

fn signoff_digest(text: &str) -> Result<Sha256, String> {
    let doc = TomlValue::parse_str(text).map_err(|e| format!("SIGNOFF: {e}"))?;
    for key in ["signer", "date"] {
        doc.get(key)
            .and_then(TomlValue::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("SIGNOFF: no {key}"))?;
    }
    doc.get("digest")
        .and_then(TomlValue::as_str)
        .and_then(Sha256::from_hex)
        .ok_or_else(|| "SIGNOFF: no digest".to_owned())
}

/// Every regular file under `dir` except `MANIFEST` and `SIGNOFF`, by
/// relative path. A symlink or special file fails closed.
pub fn local_files(dir: &Path) -> Result<BTreeMap<String, Sha256>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                return Err(format!("{}: not a regular file", path.display()));
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", path.display()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if rel == MANIFEST || rel == SIGNOFF {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.insert(rel, Sha256::of(&bytes));
        }
    }
    Ok(out)
}

/// The set's absolute (host-only) fixture files, each granted to a holdout
/// run on its own: never its directory, whose other files nobody signed.
pub fn fixture_files(dir: &Path) -> Result<Vec<String>, String> {
    let (listed, _) = parse_manifest(&read_text(&dir.join(MANIFEST))?)?;
    Ok(listed
        .into_keys()
        .filter(|p| Path::new(p).is_absolute())
        .collect())
}

fn read_text(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

// ---------------------------------------------------------------- cases

/// One golden case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub expect: Expect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expect {
    pub exit: i32,
    pub out: Out,
    pub stderr_contains: Option<String>,
}

/// What stdout must be.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// Not compared.
    Any,
    /// Byte for byte.
    Exact(String),
    /// Equal as JSON, object keys in any order.
    Json(Value),
}

/// What a case's command did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// `None` when a signal ended it: no expected exit code matches that.
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

const CASE_KEYS: &[&str] = &["cmd", "args", "stdin", "expect"];
const EXPECT_KEYS: &[&str] = &[
    "exit",
    "stdout",
    "stdout_file",
    "json_file",
    "stderr_contains",
];

/// `v` is a table holding no key outside `allowed`: a misspelt or unknown
/// expectation would otherwise be ignored and pass anything.
fn only_keys(v: &TomlValue, allowed: &[&str]) -> Result<(), String> {
    let table = v.as_table().ok_or("not a table")?;
    match table.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown key {k:?}")),
        None => Ok(()),
    }
}

/// String `k` of `v`: absent is `None`, any other type an error, never a
/// default that would weaken the expectation.
fn opt_str(v: &TomlValue, k: &str) -> Result<Option<String>, String> {
    match v.get(k) {
        None => Ok(None),
        Some(x) => x
            .as_str()
            .map(|t| Some(t.to_owned()))
            .ok_or_else(|| format!("{k} must be a string")),
    }
}

/// Parse `case.toml` of case `name`. A `*_file` path names a file of the
/// case's directory that the signed `MANIFEST` lists, read and checked
/// against its digest: the oracle is signed like the cases.
pub fn parse_case(
    name: &str,
    golden: &Path,
    signed: &BTreeMap<String, Sha256>,
    text: &str,
) -> Result<Case, String> {
    let doc = TomlValue::parse_str(text).map_err(|e| format!("{name}: case.toml: {e}"))?;
    let expect = doc
        .get("expect")
        .ok_or_else(|| format!("{name}: no [expect]"))?;
    only_keys(&doc, CASE_KEYS).map_err(|e| format!("{name}: {e}"))?;
    only_keys(expect, EXPECT_KEYS).map_err(|e| format!("{name}: [expect]: {e}"))?;
    let s = |v: &TomlValue, k: &str| opt_str(v, k).map_err(|e| format!("{name}: {e}"));
    let cmd = s(&doc, "cmd")?.ok_or_else(|| format!("{name}: no cmd"))?;
    let dir_text = golden.to_str().ok_or("golden dir is not UTF-8")?;
    let args = match doc.get("args") {
        None => vec![],
        Some(v) => v
            .as_array()
            .ok_or_else(|| format!("{name}: args is not an array"))?
            .iter()
            .map(|a| a.as_str().map(|a| a.replace(DIR_TOKEN, dir_text)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("{name}: args must be strings"))?,
    };
    let exit = match expect.get("exit") {
        None => 0,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| format!("{name}: exit must be an integer"))?,
    };
    let file = |k: &str| -> Result<Option<String>, String> {
        s(expect, k)?
            .map(|f| signed_file(golden, signed, &format!("cases/{name}/{f}")))
            .transpose()
            .map_err(|e| format!("{name}: {k}: {e}"))
    };
    let out = match (
        s(expect, "stdout")?,
        file("stdout_file")?,
        file("json_file")?,
    ) {
        (None, None, None) => Out::Any,
        (Some(t), None, None) | (None, Some(t), None) => Out::Exact(t),
        (None, None, Some(j)) => Out::Json(
            rusty_serde::json::from_str(&j).map_err(|e| format!("{name}: json_file: {e}"))?,
        ),
        _ => {
            return Err(format!(
                "{name}: give one of stdout, stdout_file, json_file"
            ))
        }
    };
    Ok(Case {
        name: name.to_owned(),
        cmd,
        args,
        stdin: s(&doc, "stdin")?,
        expect: Expect {
            exit: i32::try_from(exit).map_err(|_| format!("{name}: exit out of range"))?,
            out,
            stderr_contains: s(expect, "stderr_contains")?,
        },
    })
}

/// Did `ran` meet `case`? `Err` says how it did not. Pure.
pub fn judge(case: &Case, ran: &Ran) -> Result<(), String> {
    let want = case.expect.exit;
    match ran.exit {
        None => return Err(format!("killed by a signal (want exit {want})")),
        Some(got) if got != want => return Err(format!("exit {got} (want {want})")),
        Some(_) => {}
    }
    match &case.expect.out {
        Out::Any => {}
        Out::Exact(want) if ran.stdout != want.as_bytes() => {
            return Err("stdout differs".into());
        }
        Out::Exact(_) => {}
        Out::Json(want) => {
            let text = std::str::from_utf8(&ran.stdout).map_err(|_| "stdout is not UTF-8")?;
            let got: Value = rusty_serde::json::from_str(text)
                .map_err(|e| format!("stdout is not JSON: {e}"))?;
            if canonical(got) != canonical(want.clone()) {
                return Err("stdout differs as JSON".into());
            }
        }
    }
    if let Some(needle) = &case.expect.stderr_contains {
        if !String::from_utf8_lossy(&ran.stderr).contains(needle.as_str()) {
            return Err(format!("stderr lacks {needle:?}"));
        }
    }
    Ok(())
}

/// The text of `rel`, a relative path inside the golden set that `signed`
/// lists, verified against its digest. Absolute paths, `.`, `..` and empty
/// segments are refused before anything is read.
fn signed_file(
    golden: &Path,
    signed: &BTreeMap<String, Sha256>,
    rel: &str,
) -> Result<String, String> {
    if rel.split('/').any(|seg| matches!(seg, "" | "." | "..")) {
        return Err(format!("{rel:?} must stay inside the case directory"));
    }
    let want = signed
        .get(rel)
        .ok_or_else(|| format!("{rel} is not in MANIFEST"))?;
    let bytes = std::fs::read(golden.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    if Sha256::of(&bytes) != *want {
        return Err(format!("{rel} differs from MANIFEST"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{rel} is not UTF-8"))
}

/// Object keys sorted, recursively, so JSON compares by content.
fn canonical(v: Value) -> Value {
    match v {
        Value::Map(mut m) => {
            m.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Map(m.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        Value::Seq(s) => Value::Seq(s.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// Every case of the golden set at `golden`, sorted by name; `signed` is
/// its verified `MANIFEST`.
pub fn cases(golden: &Path, signed: &BTreeMap<String, Sha256>) -> Result<Vec<Case>, String> {
    let root = golden.join("cases");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&root)
        .map_err(|e| format!("{}: {e}", root.display()))?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    names.sort();
    names
        .iter()
        .map(|d| {
            let name = d
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("case name is not UTF-8")?;
            parse_case(name, golden, signed, &read_text(&d.join("case.toml"))?)
        })
        .collect()
}

/// Run `case` in `cwd`. Input is fed on its own thread while the output is
/// drained, so a command that writes before it has read everything cannot
/// deadlock on a full pipe.
pub fn run(case: &Case, cwd: &Path) -> Result<Ran, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(&case.cmd)
        .args(&case.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", case.cmd))?;
    let input = case.stdin.clone().unwrap_or_default();
    let feeder = child.stdin.take().map(|mut pipe| {
        std::thread::spawn(move || {
            // A command that ignores its input closes the pipe early; that
            // is its choice, and its output is still judged.
            let _ = pipe.write_all(input.as_bytes());
        })
    });
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Some(feeder) = feeder {
        feeder
            .join()
            .map_err(|_| "stdin feeder panicked".to_owned())?;
    }
    Ok(Ran {
        exit: out.status.code(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// `bbp golden-run DIR`: check the sign-off, run every case in `cwd`, and
/// return the report and whether every case passed. Case outputs are never
/// printed, only verdicts.
pub fn run_all(golden: &Path, cwd: &Path) -> Result<(String, bool), String> {
    check_signed(golden).map_err(|e| format!("bad_golden: {e}"))?;
    let (signed, _) = parse_manifest(&read_text(&golden.join(MANIFEST))?)?;
    let mut report = String::new();
    let mut all = true;
    for case in cases(golden, &signed)? {
        let verdict = run(&case, cwd).and_then(|ran| judge(&case, &ran));
        all &= verdict.is_ok();
        match verdict {
            Ok(()) => report.push_str(&format!("pass {}\n", case.name)),
            Err(why) => report.push_str(&format!("FAIL {}: {why}\n", case.name)),
        }
    }
    Ok((report, all))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bbp_golden_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A byte-exact `cat` on every CI platform: copies `file`, or stdin.
    #[cfg(unix)]
    fn cat(file: Option<&str>) -> (String, Vec<String>) {
        (
            "/bin/cat".into(),
            file.into_iter().map(str::to_owned).collect(),
        )
    }

    /// Windows has no `cat`; PowerShell copies raw bytes, with no newline
    /// translation, so exact comparisons hold.
    #[cfg(windows)]
    fn cat(file: Option<&str>) -> (String, Vec<String>) {
        let src = file.map_or("[Console]::OpenStandardInput()".to_owned(), |f| {
            format!("[IO.File]::OpenRead('{f}')")
        });
        let script = format!("$o = [Console]::OpenStandardOutput(); {src}.CopyTo($o); $o.Flush()");
        let args = ["-NoProfile", "-NonInteractive", "-Command", script.as_str()];
        ("powershell".into(), args.map(str::to_owned).to_vec())
    }

    /// A case.toml running `cat` on `file` (or `stdin`), then `expect`.
    fn cat_case(file: Option<&str>, stdin: Option<&str>, expect: &str) -> String {
        let (cmd, args) = cat(file);
        let stdin = stdin.map_or(String::new(), |s| format!("stdin = {s:?}\n"));
        format!("cmd = {cmd:?}\nargs = {args:?}\n{stdin}[expect]\n{expect}")
    }

    /// A golden set with two cases, manifested and signed.
    fn golden(tag: &str) -> PathBuf {
        let g = tmp(tag);
        let write = |p: &str, t: &str| {
            let p = g.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        write(
            "cases/echo/case.toml",
            &cat_case(None, Some("hi\n"), "exit = 0\nstdout = \"hi\\n\"\n"),
        );
        write(
            "cases/json/case.toml",
            &cat_case(
                Some("{golden}/fixtures/a.json"),
                None,
                "json_file = \"want.json\"\n",
            ),
        );
        write("cases/json/want.json", "{\"b\": [1, 2], \"a\": true}");
        write("fixtures/a.json", "{\"a\": true, \"b\": [1, 2]}\n");
        sign(&g);
        g
    }

    fn sign(g: &Path) {
        let files = local_files(g).unwrap();
        let text = manifest_text(&files);
        let (_, set) = parse_manifest(&text).unwrap();
        std::fs::write(g.join(MANIFEST), text).unwrap();
        std::fs::write(
            g.join(SIGNOFF),
            format!(
                "digest = \"{}\"\nsigner = \"nano\"\ndate = \"2026-11-03\"\n",
                set.hex()
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_signed_set_passes_and_any_change_needs_a_new_signoff() {
        let g = golden("signed");
        let set = check_signed(&g).expect("signed");
        // A changed case changes the digest: the old sign-off no longer holds.
        std::fs::write(g.join("fixtures/a.json"), "{}").unwrap();
        let err = check_signed(&g).unwrap_err();
        assert!(err.contains("fixtures/a.json"), "{err}");
        let files = local_files(&g).unwrap();
        std::fs::write(g.join(MANIFEST), manifest_text(&files)).unwrap();
        let err = check_signed(&g).unwrap_err();
        assert!(err.contains("new sign-off"), "{err}");
        sign(&g);
        assert_ne!(check_signed(&g).unwrap(), set);
        // A planted file, or no SIGNOFF at all.
        std::fs::write(g.join("cases/extra.txt"), "x").unwrap();
        assert!(check_signed(&g).unwrap_err().contains("cases/extra.txt"));
        std::fs::remove_file(g.join("cases/extra.txt")).unwrap();
        std::fs::remove_file(g.join(SIGNOFF)).unwrap();
        assert!(check_signed(&g).is_err());
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn host_only_fixtures_are_hashed_by_absolute_path() {
        let g = golden("private");
        let private = tmp("private-data");
        std::fs::write(private.join("drive.bin"), "secret bytes").unwrap();
        let abs = private.join("drive.bin").to_str().unwrap().to_owned();
        let mut files = local_files(&g).unwrap();
        files.insert(abs.clone(), Sha256::of(b"secret bytes"));
        let text = manifest_text(&files);
        let (_, set) = parse_manifest(&text).unwrap();
        std::fs::write(g.join(MANIFEST), text).unwrap();
        std::fs::write(
            g.join(SIGNOFF),
            format!(
                "digest = \"{}\"\nsigner = \"nano\"\ndate = \"d\"\n",
                set.hex()
            ),
        )
        .unwrap();
        assert_eq!(check_signed(&g).unwrap(), set);
        assert_eq!(fixture_files(&g).unwrap(), std::slice::from_ref(&abs));
        std::fs::write(private.join("drive.bin"), "changed").unwrap();
        assert!(check_signed(&g).unwrap_err().contains(&abs));
        let _ = std::fs::remove_dir_all(&g);
        let _ = std::fs::remove_dir_all(&private);
    }

    #[test]
    fn manifests_refuse_tampering_and_escapes() {
        let mut files = BTreeMap::new();
        files.insert("a".to_owned(), Sha256::of(b"a"));
        let text = manifest_text(&files);
        assert!(parse_manifest(&text).is_ok());
        let forged = text.replace("  a\n", "  b\n");
        assert!(parse_manifest(&forged).unwrap_err().contains("set digest"));
        let mut esc = BTreeMap::new();
        esc.insert("../x".to_owned(), Sha256::of(b"x"));
        assert!(parse_manifest(&manifest_text(&esc)).is_err());
        assert!(parse_manifest("").is_err());
    }

    #[test]
    fn cases_compare_exactly_or_as_json() {
        let g = golden("run");
        let (report, all) = run_all(&g, &g).expect("run");
        assert!(all, "{report}");
        assert_eq!(report, "pass echo\npass json\n");
        // A wrong answer fails with a verdict, never the output itself.
        std::fs::write(
            g.join("cases/json/want.json"),
            "{\"a\": false, \"b\": [1, 2]}",
        )
        .unwrap();
        sign(&g);
        let (report, all) = run_all(&g, &g).expect("run");
        assert!(!all);
        assert!(
            report.contains("FAIL json: stdout differs as JSON"),
            "{report}"
        );
        assert!(!report.contains("true"), "{report}");
        // An unsigned set never runs.
        std::fs::remove_file(g.join(SIGNOFF)).unwrap();
        assert!(run_all(&g, &g).unwrap_err().starts_with("bad_golden"));
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn judge_checks_exit_stdout_and_stderr() {
        let case = Case {
            name: "c".into(),
            cmd: "x".into(),
            args: vec![],
            stdin: None,
            expect: Expect {
                exit: 2,
                out: Out::Exact("out".into()),
                stderr_contains: Some("warn".into()),
            },
        };
        let ran = |exit: i32, out: &str, err: &str| Ran {
            exit: Some(exit),
            stdout: out.as_bytes().to_vec(),
            stderr: err.as_bytes().to_vec(),
        };
        assert!(judge(&case, &ran(2, "out", "a warning")).is_ok());
        assert!(judge(&case, &ran(0, "out", "a warning"))
            .unwrap_err()
            .contains("exit 0"));
        assert!(judge(&case, &ran(2, "out\n", "a warning")).is_err());
        assert!(judge(&case, &ran(2, "out", ""))
            .unwrap_err()
            .contains("stderr"));
        // A signal is no exit code, so it never matches one, not even 128.
        let mut killed = ran(2, "out", "a warning");
        killed.exit = None;
        assert!(judge(&case, &killed).unwrap_err().contains("signal"));
    }

    /// #597 review: a field of the wrong type or an unknown key fails the
    /// case instead of falling back to a default that accepts anything.
    #[test]
    fn mistyped_or_unknown_expectations_are_refused() {
        let d = Path::new("/nonexistent");
        let none = BTreeMap::new();
        for bad in [
            "cmd = \"x\"\n[expect]\nstdout = [\"ok\"]\n",
            "cmd = \"x\"\n[expect]\nexit = \"1\"\n",
            "cmd = \"x\"\n[expect]\nstderr_contains = 1\n",
            "cmd = \"x\"\n[expect]\nstdotu = \"ok\"\n",
            "cmd = \"x\"\nstdin = 3\n[expect]\n",
            "cmd = \"x\"\nenv = \"y\"\n[expect]\n",
            "cmd = \"x\"\nexpect = 1\n",
            "cmd = 1\n[expect]\n",
        ] {
            assert!(parse_case("c", d, &none, bad).is_err(), "{bad}");
        }
        let fine = "cmd = \"x\"\nstdin = \"in\"\n[expect]\nexit = 1\nstdout = \"ok\"\n";
        assert!(parse_case("c", d, &none, fine).is_ok());
    }

    #[test]
    fn a_case_names_one_expected_output() {
        let d = Path::new("/nonexistent");
        let none = BTreeMap::new();
        let both = "cmd = \"x\"\n[expect]\nstdout = \"a\"\njson_file = \"b\"\n";
        assert!(parse_case("c", d, &none, both).is_err());
        let any = "cmd = \"x\"\n[expect]\nexit = 3\n";
        let c = parse_case("c", d, &none, any).unwrap();
        assert_eq!(c.expect.out, Out::Any);
        assert_eq!(c.expect.exit, 3);
        assert!(parse_case("c", d, &none, "[expect]\n").is_err());
    }

    #[test]
    fn expected_output_comes_only_from_signed_files_of_the_case() {
        let g = golden("oracle");
        let (signed, _) = parse_manifest(&read_text(&g.join(MANIFEST)).unwrap()).unwrap();
        let outside = tmp("oracle-outside");
        std::fs::write(outside.join("want.json"), "{}").unwrap();
        let abs = outside.join("want.json");
        for f in [
            abs.to_str().unwrap(),
            "../echo/case.toml",
            "./want.json",
            "nope.json",
        ] {
            let text = format!("cmd = \"x\"\n[expect]\njson_file = \"{f}\"\n");
            assert!(parse_case("json", &g, &signed, &text).is_err(), "{f}");
        }
        // A signed file changed after the check is refused too.
        std::fs::write(g.join("cases/json/want.json"), "{}").unwrap();
        let text = "cmd = \"x\"\n[expect]\njson_file = \"want.json\"\n";
        let err = parse_case("json", &g, &signed, text).unwrap_err();
        assert!(err.contains("differs from MANIFEST"), "{err}");
        let _ = std::fs::remove_dir_all(&g);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn large_input_streams_through_a_case_without_deadlock() {
        let input: String = "line of input\n".repeat(64 * 1024);
        let (cmd, args) = cat(None);
        let case = Case {
            name: "cat".into(),
            cmd,
            args,
            stdin: Some(input.clone()),
            expect: Expect {
                exit: 0,
                out: Out::Exact(input),
                stderr_contains: None,
            },
        };
        let ran = run(&case, &std::env::temp_dir()).expect("run");
        assert_eq!(judge(&case, &ran), Ok(()));
    }
}
