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

/// Directories holding the set's absolute (host-only) fixtures: a holdout
/// run must be able to read them.
pub fn fixture_roots(dir: &Path) -> Result<Vec<String>, String> {
    let (listed, _) = parse_manifest(&read_text(&dir.join(MANIFEST))?)?;
    let mut roots: Vec<String> = listed
        .keys()
        .filter(|p| Path::new(p).is_absolute())
        .filter_map(|p| Path::new(p).parent()?.to_str().map(str::to_owned))
        .collect();
    roots.sort();
    roots.dedup();
    Ok(roots)
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
    pub exit: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Parse `case.toml` of case `name`; `*_file` paths are read from `dir`.
pub fn parse_case(name: &str, dir: &Path, golden: &Path, text: &str) -> Result<Case, String> {
    let doc = TomlValue::parse_str(text).map_err(|e| format!("{name}: case.toml: {e}"))?;
    let s = |v: &TomlValue, k: &str| v.get(k).and_then(TomlValue::as_str).map(str::to_owned);
    let cmd = s(&doc, "cmd").ok_or_else(|| format!("{name}: no cmd"))?;
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
    let expect = doc
        .get("expect")
        .ok_or_else(|| format!("{name}: no [expect]"))?;
    let exit = expect.get("exit").and_then(TomlValue::as_i64).unwrap_or(0);
    let file = |k: &str| -> Result<Option<String>, String> {
        s(expect, k).map(|f| read_text(&dir.join(f))).transpose()
    };
    let out = match (
        s(expect, "stdout"),
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
        stdin: s(&doc, "stdin"),
        expect: Expect {
            exit: i32::try_from(exit).map_err(|_| format!("{name}: exit out of range"))?,
            out,
            stderr_contains: s(expect, "stderr_contains"),
        },
    })
}

/// Did `ran` meet `case`? `Err` says how it did not. Pure.
pub fn judge(case: &Case, ran: &Ran) -> Result<(), String> {
    if ran.exit != case.expect.exit {
        return Err(format!("exit {} (want {})", ran.exit, case.expect.exit));
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

/// Every case of the golden set at `golden`, sorted by name.
pub fn cases(golden: &Path) -> Result<Vec<Case>, String> {
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
            parse_case(name, d, golden, &read_text(&d.join("case.toml"))?)
        })
        .collect()
}

/// Run `case` in `cwd`.
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
    if let Some(mut pipe) = child.stdin.take() {
        // A command that ignores its input closes the pipe early; that is
        // its choice, and its output is still judged.
        let _ = pipe.write_all(case.stdin.as_deref().unwrap_or("").as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(Ran {
        exit: out.status.code().unwrap_or(128),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// `bbp golden-run DIR`: check the sign-off, run every case in `cwd`, and
/// return the report and whether every case passed. Case outputs are never
/// printed, only verdicts.
pub fn run_all(golden: &Path, cwd: &Path) -> Result<(String, bool), String> {
    check_signed(golden).map_err(|e| format!("bad_golden: {e}"))?;
    let mut report = String::new();
    let mut all = true;
    for case in cases(golden)? {
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
            "cmd = \"/bin/echo\"\nargs = [\"hi\"]\n[expect]\nexit = 0\nstdout = \"hi\\n\"\n",
        );
        write(
            "cases/json/case.toml",
            "cmd = \"/bin/cat\"\nargs = [\"{golden}/fixtures/a.json\"]\n[expect]\njson_file = \"want.json\"\n",
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
        assert_eq!(fixture_roots(&g).unwrap(), [private.to_str().unwrap()]);
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
        let ran = |exit, out: &str, err: &str| Ran {
            exit,
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
    }

    #[test]
    fn a_case_names_one_expected_output() {
        let d = Path::new("/nonexistent");
        let both = "cmd = \"x\"\n[expect]\nstdout = \"a\"\njson_file = \"b\"\n";
        assert!(parse_case("c", d, d, both).is_err());
        let none = "cmd = \"x\"\n[expect]\nexit = 3\n";
        let c = parse_case("c", d, d, none).unwrap();
        assert_eq!(c.expect.out, Out::Any);
        assert_eq!(c.expect.exit, 3);
        assert!(parse_case("c", d, d, "[expect]\n").is_err());
    }
}
