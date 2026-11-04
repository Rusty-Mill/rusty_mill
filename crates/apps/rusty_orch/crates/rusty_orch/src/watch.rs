//! `rusty_orch watch` (#599 maintenance): look at each shipped app once and
//! post what changed under it as claims on the app's maintenance task.
//!
//! - The registry is one `factory/apps/<app>.json` per app (D68). The human
//!   opens each maintenance task with `bbp open --mode claims`; `watch` never
//!   opens a task.
//! - Each [`Watch`] adapter is a CLI call (D58): `cargo deny` advisories,
//!   `cargo update --dry-run`, and the toolchain against the watched-versions
//!   file. The canary re-run on the released ref is a later step.
//! - Findings are posted by the task's `watcher` role (D67) through its
//!   `bbp mcp` session. A claim never accepts on the watcher's word alone
//!   (`min_authors`), and a human still merges.
//! - Posted once: each finding's op id is derived from the app and the
//!   finding's key, and its text from the key alone, so posting it again on a
//!   later day replays the first post instead of adding a claim (BBP R15).

use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_cli::CommandRunner;
use rusty_bbp::{MsgId, Response};
use rusty_serde::{Deserialize, Serialize, Value};

use crate::args::ArgsError;
use crate::lens::bbp::{CallError, Child, Rpc};

pub const USAGE: &str = "usage: rusty_orch watch --factory DIR --bbp-dir DIR [--bbp PATH] [--app NAME] [--dry-run]\n\nFor each app in DIR/apps/*.json (or only NAME), run the watch adapters in its repo and post\neach new finding as a claim on its maintenance task, as the task's `watcher` role. Prints a\nJSON report. --dry-run posts nothing. Findings already posted replay instead of repeating.";

/// The role that posts findings (D67).
pub const WATCHER: &str = "watcher";

/// How long one adapter command may run.
const ADAPTER_DEADLINE: Duration = Duration::from_secs(600);

/// One shipped app (`factory/apps/<app>.json`, D68).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct App {
    pub app: String,
    /// The app's checkout on the BBP host.
    pub repo: String,
    /// Its long-lived claims-mode maintenance task.
    pub task: String,
    /// The ref the app shipped from; the canary re-runs it.
    pub released_ref: String,
    /// The test profile the canary runs.
    pub profile: String,
    /// The most the app's watch may spend per run, in dollars (canary).
    pub watch_usd: f64,
}

/// Versions the watcher compares apps against (`factory/watched-versions.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Watched {
    /// The current stable Rust toolchain, e.g. `1.99.0`.
    #[rusty_serde(default)]
    pub rust: Option<String>,
}

/// Something that changed under an app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Stable identity, e.g. `advisory:RUSTSEC-2026-0001:foo`; the op id and
    /// the claim text are built from it alone.
    pub key: String,
    /// The claim's domain.
    pub domain: &'static str,
    pub text: String,
}

/// One watch adapter (D58's `Watch` port).
pub trait Watch {
    fn name(&self) -> &'static str;
    fn findings(&self, app: &App, run: &dyn CommandRunner) -> Result<Vec<Finding>, String>;
}

/// `cargo deny check advisories`: one finding per advisory and package.
pub struct Advisories;
/// `cargo update --dry-run`: one finding per dependency with a newer version.
pub struct Updates;
/// The app's `rust-toolchain.toml` channel against the watched Rust version.
pub struct Toolchain(pub Watched);

impl Watch for Advisories {
    fn name(&self) -> &'static str {
        "cargo-deny"
    }
    fn findings(&self, app: &App, run: &dyn CommandRunner) -> Result<Vec<Finding>, String> {
        let argv = [
            "cargo",
            "deny",
            "--color",
            "never",
            "--format",
            "json",
            "check",
            "advisories",
        ];
        let (status, stderr) = command(run, Path::new(&app.repo), &argv)?;
        let found = advisories(&stderr);
        if status != 0 && found.is_empty() {
            return Err(format!("cargo deny exited {status}: {}", tail(&stderr)));
        }
        Ok(found)
    }
}

impl Watch for Updates {
    fn name(&self) -> &'static str {
        "cargo-update"
    }
    fn findings(&self, app: &App, run: &dyn CommandRunner) -> Result<Vec<Finding>, String> {
        let (status, stderr) = command(
            run,
            Path::new(&app.repo),
            &["cargo", "update", "--dry-run", "--color", "never"],
        )?;
        if status != 0 {
            return Err(format!("cargo update exited {status}: {}", tail(&stderr)));
        }
        Ok(updates(&stderr).into_iter().collect())
    }
}

impl Watch for Toolchain {
    fn name(&self) -> &'static str {
        "toolchain"
    }
    fn findings(&self, app: &App, _: &dyn CommandRunner) -> Result<Vec<Finding>, String> {
        let Some(watched) = &self.0.rust else {
            return Ok(Vec::new());
        };
        let file = Path::new(&app.repo).join("rust-toolchain.toml");
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", file.display())),
        };
        Ok(toolchain(&text, watched).into_iter().collect())
    }
}

/// Run `argv` in `repo`; its status and stderr (cargo reports there).
fn command(run: &dyn CommandRunner, repo: &Path, argv: &[&str]) -> Result<(i32, String), String> {
    let argv: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
    let exit = run
        .run_in(Some(repo), &argv, &[], ADAPTER_DEADLINE, &[])
        .map_err(|e| format!("{}: {e}", argv.join(" ")))?;
    Ok((
        exit.status,
        String::from_utf8_lossy(&exit.stderr).into_owned(),
    ))
}

fn tail(s: &str) -> String {
    s.lines().rev().take(3).collect::<Vec<_>>().join(" / ")
}

/// Findings from `cargo deny --format json` diagnostics, one JSON object per
/// line: each advisory (`fields.advisory`, RustSec metadata) per package, and
/// each yanked crate version (`code: yanked`). Other lines are ignored.
pub fn advisories(stderr: &str) -> Vec<Finding> {
    let mut out: Vec<Finding> = stderr
        .lines()
        .filter_map(|l| rusty_serde::json::from_str::<Value>(l).ok())
        .filter_map(|v| diagnostic(&v["fields"]))
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out.dedup_by(|a, b| a.key == b.key);
    out
}

fn diagnostic(fields: &Value) -> Option<Finding> {
    let krate = &fields["graphs"][0]["Krate"];
    let a = &fields["advisory"];
    if let Some(id) = a["id"].as_str() {
        let package = a["package"].as_str().or_else(|| krate["name"].as_str())?;
        let title = a["title"].as_str().unwrap_or("security advisory");
        return Some(Finding {
            key: format!("advisory:{id}:{package}"),
            domain: "security",
            text: format!(
                "Dependency {package} is affected by {id} ({title}); update or replace it."
            ),
        });
    }
    if fields["code"].as_str() != Some("yanked") {
        return None;
    }
    let (name, version) = (krate["name"].as_str()?, krate["version"].as_str()?);
    Some(Finding {
        key: format!("yanked:{name}:{version}"),
        domain: "dependencies",
        text: format!("Dependency {name} {version} was yanked; move to a published version."),
    })
}

/// Newer versions from `cargo update --dry-run` (`Updating foo v1.0.0 ->
/// v1.0.1`), as one finding for the whole set: one `cargo update` takes them
/// all, so they are one change to review. The key is a digest of the set, so
/// a new claim is posted only when the set changes.
pub fn updates(stderr: &str) -> Option<Finding> {
    let mut moves: Vec<String> = stderr
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Updating ")?;
            let (name_from, to) = rest.split_once(" -> ")?;
            let (name, from) = name_from.split_once(' ')?;
            Some(format!("{name} {from} -> {to}"))
        })
        .collect();
    if moves.is_empty() {
        return None;
    }
    moves.sort();
    let digest = rusty_bbp::Sha256::of(moves.join("\n").as_bytes()).hex();
    let shown = moves
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = match moves.len().saturating_sub(SHOWN) {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    Some(Finding {
        key: format!("updates:{}", &digest[..16]),
        domain: "dependencies",
        text: format!(
            "The lockfile can take {} dependency updates: {shown}{more}. Run cargo update and re-run the tests.",
            moves.len()
        ),
    })
}

/// Updates named in an update claim's text; the rest are counted.
const SHOWN: usize = 8;

/// A finding when `rust-toolchain.toml`'s channel is not `watched`.
pub fn toolchain(file: &str, watched: &str) -> Option<Finding> {
    let channel = file.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "channel").then(|| v.trim().trim_matches('"').to_owned())
    })?;
    (channel != watched).then(|| Finding {
        key: format!("toolchain:rust:{watched}"),
        domain: "platform",
        text: format!("Rust {watched} is the current toolchain; the app pins {channel}. Move it and re-run its tests."),
    })
}

// ---------------------------------------------------------------- posting

/// What became of one finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Posted {
    pub finding: Finding,
    /// The claim; `None` in a dry run.
    pub claim: Option<MsgId>,
}

/// One app's watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppReport {
    pub app: String,
    pub task: String,
    pub findings: Vec<Posted>,
    /// Adapters that failed, with why. The others still ran.
    pub errors: Vec<String>,
}

/// The op id of `finding` on `app`: the same finding always maps to it.
pub fn op_id(app: &str, finding: &Finding) -> String {
    let digest = rusty_bbp::Sha256::of(format!("{app}\n{}", finding.key).as_bytes());
    format!("watch-{}", &digest.hex()[..32])
}

/// Post `finding` as a low-stakes, reversible claim on the session's task,
/// citing the task's brief.
pub fn post(rpc: &mut dyn Rpc, app: &str, finding: &Finding) -> Result<MsgId, String> {
    let brief = match answer(rpc.call("task_card", map(vec![])))? {
        Response::Card(c) => c.brief.ok_or("task_card: the task has no brief")?,
        other => return Err(format!("task_card answered {other:?}")),
    };
    let args = map(vec![
        ("op", op_id(app, finding).into()),
        ("kind", "claim".into()),
        ("text", finding.text.as_str().into()),
        ("domains", Value::Seq(vec![finding.domain.into()])),
        ("stakes", "low".into()),
        ("reversible", Value::Bool(true)),
        ("refs", Value::Seq(vec![format!("art:{}", brief.0).into()])),
    ]);
    let reply = match rpc.call("post", args.clone()) {
        Err(CallError::Lost(_)) => rpc.call("post", args),
        first => first,
    };
    match answer(reply)? {
        Response::Posted(id) => Ok(id),
        other => Err(format!("post answered {other:?}")),
    }
}

fn answer(reply: Result<String, CallError>) -> Result<Response, String> {
    let text = reply.map_err(|e| e.to_string())?;
    match rusty_serde::json::from_str::<Response>(&text) {
        Ok(Response::Rejected(r)) => Err(format!("{:?}: {}", r.code, r.detail)),
        Ok(r) => Ok(r),
        Err(e) => Err(format!("unreadable answer: {e}")),
    }
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Watch `app` with `adapters`; post each finding through `session` unless
/// it is `None` (a dry run).
pub fn watch_app(
    app: &App,
    adapters: &[&dyn Watch],
    run: &dyn CommandRunner,
    mut session: Option<&mut dyn Rpc>,
) -> AppReport {
    let mut report = AppReport {
        app: app.app.clone(),
        task: app.task.clone(),
        findings: Vec::new(),
        errors: Vec::new(),
    };
    for w in adapters {
        let found = match w.findings(app, run) {
            Ok(f) => f,
            Err(e) => {
                report.errors.push(format!("{}: {e}", w.name()));
                continue;
            }
        };
        for finding in found {
            let claim = match session.as_deref_mut() {
                None => None,
                Some(rpc) => match post(rpc, &app.app, &finding) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        report.errors.push(format!("post {}: {e}", finding.key));
                        continue;
                    }
                },
            };
            report.findings.push(Posted { finding, claim });
        }
    }
    report
}

// ---------------------------------------------------------------- CLI

/// What to watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub factory: PathBuf,
    pub bbp_dir: PathBuf,
    pub bbp: PathBuf,
    pub app: Option<String>,
    pub dry_run: bool,
}

/// Parse `--factory DIR --bbp-dir DIR [--bbp PATH] [--app NAME] [--dry-run]`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let (mut factory, mut bbp_dir, mut bbp, mut app, mut dry_run) = (None, None, None, None, false);
    let mut args = argv.into_iter();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--dry-run" => {
                dry_run = true;
                continue;
            }
            "--factory" => &mut factory,
            "--bbp-dir" => &mut bbp_dir,
            "--bbp" => &mut bbp,
            "--app" => &mut app,
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        };
        if slot.is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
        *slot = Some(value);
    }
    let need = |v: Option<String>, flag: &str| {
        v.ok_or_else(|| ArgsError::Usage(format!("{flag} is required")))
    };
    Ok(Options {
        factory: need(factory, "--factory")?.into(),
        bbp_dir: need(bbp_dir, "--bbp-dir")?.into(),
        bbp: bbp.unwrap_or_else(|| "bbp".to_owned()).into(),
        app,
        dry_run,
    })
}

/// The apps in `factory/apps/*.json`, by file name; only `only` if given.
pub fn apps(factory: &Path, only: Option<&str>) -> Result<Vec<App>, String> {
    let dir = factory.join("apps");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let app: App =
            rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        if p.file_stem().and_then(|s| s.to_str()) != Some(app.app.as_str()) {
            return Err(format!("{}: names app {:?}", p.display(), app.app));
        }
        if only.is_none_or(|o| o == app.app) {
            out.push(app);
        }
    }
    if let Some(o) = only.filter(|_| out.is_empty()) {
        return Err(format!("no app {o:?} in {}", dir.display()));
    }
    Ok(out)
}

/// `factory/watched-versions.json`, or nothing watched if it is absent.
pub fn watched(factory: &Path) -> Result<Watched, String> {
    let path = factory.join("watched-versions.json");
    match std::fs::read_to_string(&path) {
        Ok(t) => rusty_serde::json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Watched::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Watch every app; the reports as JSON, and whether every adapter and post
/// succeeded.
pub fn run(options: &Options, runner: &dyn CommandRunner) -> Result<(String, bool), String> {
    let apps = apps(&options.factory, options.app.as_deref())?;
    let toolchain = Toolchain(watched(&options.factory)?);
    let adapters: [&dyn Watch; 3] = [&Advisories, &Updates, &toolchain];
    let mut reports = Vec::new();
    for app in &apps {
        let report = if options.dry_run {
            watch_app(app, &adapters, runner, None)
        } else {
            match Child::spawn(&options.bbp, &options.bbp_dir, &app.task, WATCHER) {
                Ok(mut child) => watch_app(app, &adapters, runner, Some(&mut child)),
                Err(e) => AppReport {
                    app: app.app.clone(),
                    task: app.task.clone(),
                    findings: Vec::new(),
                    errors: vec![e],
                },
            }
        };
        reports.push(report);
    }
    let clean = reports.iter().all(|r| r.errors.is_empty());
    let json = rusty_serde::json::to_string(&reports).map_err(|e| e.to_string())?;
    Ok((json, clean))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use orch_cli::fake::{exit, FakeCommand};
    use rusty_bbp::{ArtId, TaskId, TaskState};
    use std::collections::BTreeMap;

    const DENY: &str = r#"{"type":"diagnostic","fields":{"severity":"error","message":"x","advisory":{"id":"RUSTSEC-2026-0001","package":"foo","title":"Use after free"}}}
{"type":"summary","fields":{}}
{"fields":{"code":"yanked","graphs":[{"Krate":{"name":"bar","version":"0.2.0"},"parents":[]}],"message":"detected yanked crate","severity":"warning"},"type":"diagnostic"}
{"fields":{"code":"unmaintained","message":"m","severity":"warning"},"type":"diagnostic"}
{"type":"diagnostic","fields":{"severity":"error","advisory":{"id":"RUSTSEC-2026-0001","package":"foo","title":"Use after free"}}}
"#;

    const UPDATE: &str = "    Updating crates.io index\n     Locking 2 packages to latest compatible versions\n    Updating foo v1.0.0 -> v1.0.1\n    Updating bar v0.3.2 -> v0.3.4\nwarning: not updating the lock file due to the dry run\n";

    fn app(repo: &Path) -> App {
        App {
            app: "demo".into(),
            repo: repo.display().to_string(),
            task: "maint-demo".into(),
            released_ref: "v1.0.0".into(),
            profile: "unit".into(),
            watch_usd: 1.0,
        }
    }

    /// A claims session that answers `task_card` and each `post` in turn,
    /// recording the post arguments.
    #[derive(Default)]
    struct Board {
        posts: Vec<Value>,
        answers: BTreeMap<String, Response>,
        next: u64,
    }

    impl Rpc for Board {
        fn call(&mut self, tool: &str, args: Value) -> Result<String, CallError> {
            let resp = match tool {
                "task_card" => {
                    let mut st = TaskState::new(TaskId::from("maint-demo"));
                    st.brief = Some(ArtId(1));
                    Response::Card(st.card())
                }
                "post" => {
                    let op = args["op"].as_str().unwrap().to_owned();
                    self.posts.push(args);
                    self.next += 1;
                    let fresh = Response::Posted(MsgId(10 + self.next));
                    self.answers.entry(op).or_insert(fresh).clone()
                }
                other => panic!("unexpected {other}"),
            };
            Ok(rusty_serde::json::to_string(&resp).unwrap())
        }
    }

    #[test]
    fn deny_findings_are_advisories_per_package_and_yanked_versions() {
        let f = advisories(DENY);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].key, "advisory:RUSTSEC-2026-0001:foo");
        assert_eq!(f[1].key, "yanked:bar:0.2.0");
        assert_eq!(f[0].domain, "security");
        assert!(f[0].text.contains("Use after free"), "{}", f[0].text);
        assert!(advisories("not json\n").is_empty());
    }

    #[test]
    fn updates_are_one_finding_keyed_by_the_set() {
        let f = updates(UPDATE).expect("finding");
        assert!(f.key.starts_with("updates:"), "{}", f.key);
        assert!(
            f.text
                .contains("2 dependency updates: bar v0.3.2 -> v0.3.4, foo v1.0.0 -> v1.0.1."),
            "{}",
            f.text
        );
        let reordered = "    Updating bar v0.3.2 -> v0.3.4\n    Updating foo v1.0.0 -> v1.0.1\n";
        assert_eq!(updates(reordered), Some(f.clone()), "order does not matter");
        let other = "    Updating foo v1.0.0 -> v1.0.2\n";
        assert_ne!(updates(other).expect("finding").key, f.key);
        assert_eq!(updates("    Updating crates.io index\n"), None);
        let many: String = (0..10)
            .map(|i| format!("    Updating c{i} v0.1.0 -> v0.1.1\n"))
            .collect();
        assert!(updates(&many)
            .expect("finding")
            .text
            .contains(" and 2 more."));
    }

    #[test]
    fn toolchain_flags_only_a_different_channel() {
        let pinned = "[toolchain]\nchannel = \"1.98.1\"\ncomponents = [\"clippy\"]\n";
        let f = toolchain(pinned, "1.99.0").expect("finding");
        assert_eq!(f.key, "toolchain:rust:1.99.0");
        assert!(toolchain(pinned, "1.98.1").is_none());
        assert!(toolchain("[toolchain]\n", "1.99.0").is_none());
    }

    #[test]
    fn a_finding_posts_once_under_a_stable_op() {
        let f = &updates(UPDATE).expect("finding");
        assert_eq!(op_id("demo", f), op_id("demo", f));
        assert_ne!(op_id("demo", f), op_id("other", f));
        let mut board = Board::default();
        let first = post(&mut board, "demo", f).unwrap();
        let again = post(&mut board, "demo", f).unwrap();
        assert_eq!(first, again, "the second post replays the first");
        let p = &board.posts[0];
        assert_eq!(p["kind"].as_str(), Some("claim"));
        assert_eq!(p["stakes"].as_str(), Some("low"));
        assert_eq!(p["refs"][0].as_str(), Some("art:1"));
        assert_eq!(p, &board.posts[1], "same payload, so BBP replays it");
    }

    #[test]
    fn a_failing_adapter_is_reported_and_the_others_still_run() {
        let dir = std::env::temp_dir().join(format!("orch_watch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rust-toolchain.toml"), "channel = \"1.98.1\"\n").unwrap();
        let run = FakeCommand::new(Ok(exit(101, "", "error: no Cargo.lock\n")));
        let tc = Toolchain(Watched {
            rust: Some("1.99.0".into()),
        });
        let adapters: [&dyn Watch; 2] = [&Updates, &tc];
        let mut board = Board::default();
        let r = watch_app(&app(&dir), &adapters, &run, Some(&mut board));
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert!(
            r.errors[0].starts_with("cargo-update: cargo update exited 101"),
            "{:?}",
            r.errors
        );
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].claim, Some(MsgId(11)));
        assert_eq!(run.cwds.lock().unwrap()[0].as_deref(), Some(dir.as_path()));
        let dry = watch_app(&app(&dir), &adapters, &run, None);
        assert_eq!(dry.findings[0].claim, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn advisories_found_with_a_failing_exit_are_findings_not_errors() {
        let run = FakeCommand::new(Ok(exit(1, "", DENY)));
        let f = Advisories
            .findings(&app(Path::new("/nonexistent")), &run)
            .unwrap();
        assert_eq!(f.len(), 2);
        let broken = FakeCommand::new(Ok(exit(2, "", "error: no deny.toml\n")));
        assert!(Advisories.findings(&app(Path::new("/x")), &broken).is_err());
    }

    #[test]
    fn the_registry_needs_file_and_app_names_to_match() {
        let dir = std::env::temp_dir().join(format!("orch_watch_reg_{}", std::process::id()));
        let apps_dir = dir.join("apps");
        std::fs::create_dir_all(&apps_dir).unwrap();
        let body = |name: &str| {
            format!(
                r#"{{"app":"{name}","repo":"/srv/{name}","task":"m-{name}","released_ref":"v1","profile":"unit","watch_usd":0.5}}"#
            )
        };
        std::fs::write(apps_dir.join("a.json"), body("a")).unwrap();
        std::fs::write(apps_dir.join("b.json"), body("b")).unwrap();
        std::fs::write(apps_dir.join("notes.txt"), "ignored").unwrap();
        let all = apps(&dir, None).unwrap();
        assert_eq!(
            all.iter().map(|a| a.app.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(apps(&dir, Some("b")).unwrap()[0].task, "m-b");
        assert!(apps(&dir, Some("c")).unwrap_err().contains("no app \"c\""));
        std::fs::write(apps_dir.join("c.json"), body("x")).unwrap();
        assert!(apps(&dir, None).unwrap_err().contains("names app \"x\""));
        assert_eq!(watched(&dir).unwrap(), Watched::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_need_the_factory_and_bbp_dir() {
        let o = |a: &[&str]| options(a.iter().map(|s| (*s).to_owned()));
        let ok = o(&["--factory", "f", "--bbp-dir", "d", "--dry-run"]).unwrap();
        assert!(ok.dry_run);
        assert_eq!(ok.bbp, PathBuf::from("bbp"));
        assert!(
            matches!(o(&["--factory", "f"]), Err(ArgsError::Usage(m)) if m.contains("--bbp-dir"))
        );
        assert!(matches!(o(&["--app"]), Err(ArgsError::Usage(_))));
        assert!(matches!(o(&["--help"]), Err(ArgsError::Help)));
    }
}
