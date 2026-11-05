//! `rusty_orch watch` (#599 maintenance): look at each shipped app once and
//! post what changed under it as claims on the app's maintenance task.
//!
//! - The registry is one `factory/apps/<app>.json` per app (D80). The human
//!   opens each maintenance task with `bbp open --mode claims`; `watch` never
//!   opens a task.
//! - Each [`Watch`] adapter is a CLI call (D58): `cargo deny` advisories,
//!   `cargo update --dry-run`, and the toolchain against the watched-versions
//!   file. The canary re-run on the released ref is a later step.
//! - Findings are posted by the task's `watcher` role (D79) through its
//!   `bbp mcp` session. A claim never accepts on the watcher's word alone
//!   (`min_authors`), and a human still merges.
//! - Posted once (D83). Each finding has a stable `key` and a `slot`, the
//!   thing it is about. Advisories and yanked crates are their own slot; the
//!   update set is one slot per app (`updates`), and so is the toolchain
//!   (`toolchain:rust`). The `watch_finding` table in orch-store keeps, per
//!   app and slot, the key last posted and its claim:
//!   - same key: nothing is posted, and the report gives the claim;
//!   - a new key in a slot that has a claim (a new update set, a new watched
//!     Rust, or an earlier key coming back): a new claim that `supersedes`
//!     the slot's claim, so accepting it retires the stale one;
//!   - an empty slot: a new claim.
//!
//!   Details that change under the same key (an advisory's title or
//!   severity, the app's current pin) are in [`Finding::detail`], reported
//!   and never posted, so the claim text never goes stale.
//! - Crash safety. Before a post is sent, its exact arguments are saved in
//!   the slot as pending; once BBP answers, the claim replaces it. A run that
//!   dies in between leaves the pending post, and the next run resends it
//!   before anything else in that slot: BBP replays it if it landed (R15:
//!   the same op and bytes) or posts it now. Either way its claim becomes the
//!   slot's, so the next finding supersedes it instead of orphaning it. A
//!   post is identified by the app, the key and the slot's previous claim
//!   (the op id is a digest of all three, and refs cite the opening brief
//!   `art:1`), so a key that comes back after another (A, B, A) is a new op
//!   over B, not a conflict with A's first post.

use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_cli::CommandRunner;
use orch_store::{PendingPost, Store, WatchFinding, WatchSlot};
use rusty_bbp::{MsgId, Response};
use rusty_serde::{Deserialize, Serialize, Value};

use crate::args::ArgsError;
use crate::lens::bbp::{CallError, Child, Rpc};

pub const USAGE: &str = "usage: rusty_orch watch --factory DIR --bbp-dir DIR --state DIR [--bbp PATH] [--app NAME] [--dry-run]\n\nFor each app in DIR/apps/*.json (or only NAME), run the watch adapters in its repo and post\neach new finding as a claim on its maintenance task, as the task's `watcher` role. --state is\nthe orch state dir that remembers what was posted: a finding already posted is not posted\nagain, and a newer one in its slot supersedes it. Prints a JSON report. --dry-run posts nothing.";

/// The role that posts findings (D79).
pub const WATCHER: &str = "watcher";

/// How long one adapter command may run.
const ADAPTER_DEADLINE: Duration = Duration::from_secs(600);

/// One shipped app (`factory/apps/<app>.json`, D80).
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
    /// every posted field are built from it alone.
    pub key: String,
    /// What the finding is about: a newer key in the same slot replaces it.
    pub slot: String,
    /// The claim's domain.
    pub domain: &'static str,
    /// The claim text: a function of `key` only.
    pub text: String,
    /// What may change under the same key; in the report, never posted.
    pub detail: String,
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
        let title = a["title"].as_str().unwrap_or_default();
        let severity = a["cvss"]
            .as_str()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default();
        let key = format!("advisory:{id}:{package}");
        return Some(Finding {
            slot: key.clone(),
            key,
            domain: "security",
            text: format!("Dependency {package} is affected by {id}; update or replace it."),
            detail: format!("{title}{severity}"),
        });
    }
    if fields["code"].as_str() != Some("yanked") {
        return None;
    }
    let (name, version) = (krate["name"].as_str()?, krate["version"].as_str()?);
    let key = format!("yanked:{name}:{version}");
    Some(Finding {
        slot: key.clone(),
        key,
        domain: "dependencies",
        text: format!("Dependency {name} {version} was yanked; move to a published version."),
        detail: String::new(),
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
        slot: "updates".to_owned(),
        domain: "dependencies",
        text: format!(
            "The lockfile can take {} dependency updates: {shown}{more}. Run cargo update and re-run the tests.",
            moves.len()
        ),
        detail: String::new(),
    })
}

/// Updates named in an update claim's text; the rest are counted.
const SHOWN: usize = 8;

/// A finding when `rust-toolchain.toml`'s channel is not `watched`, keyed by
/// `watched` alone: moving the pin to another old version is still the same
/// thing to do.
pub fn toolchain(file: &str, watched: &str) -> Option<Finding> {
    let channel = channel(file)?;
    (channel != watched).then(|| Finding {
        key: format!("toolchain:rust:{watched}"),
        slot: "toolchain:rust".to_owned(),
        domain: "platform",
        text: format!(
            "Rust {watched} is the current toolchain and the app pins another. Move it to {watched} and re-run its tests."
        ),
        detail: format!("pins {channel}"),
    })
}

/// `[toolchain] channel` from a `rust-toolchain.toml`: a basic or literal
/// TOML string, with an optional trailing comment. Anything else (a bare or
/// escaped value, a key outside `[toolchain]`) is not read as a channel.
fn channel(file: &str) -> Option<String> {
    let mut table = "";
    for line in file.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = name.trim();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if table != "toolchain" || k.trim() != "channel" {
            continue;
        }
        let v = v.trim_start();
        let quote = v.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let (value, rest) = v[1..].split_once(quote)?;
        let rest = rest.trim_start();
        let clean = !(quote == '"' && value.contains('\\'));
        return (clean && (rest.is_empty() || rest.starts_with('#'))).then(|| value.to_owned());
    }
    None
}

// ---------------------------------------------------------------- posting

/// What became of one finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Posted {
    pub finding: Finding,
    /// The claim; `None` in a dry run.
    pub claim: Option<MsgId>,
    /// Whether this run posted it (false: an earlier run did).
    pub new: bool,
    /// The slot's earlier claim that this one replaces once accepted.
    pub supersedes: Option<MsgId>,
}

/// The `watch_finding` table (D58): per app and slot, the finding last
/// posted and its claim.
pub trait Seen {
    fn get(&self, app: &str, slot: &str) -> Result<WatchSlot, String>;
    fn put(&mut self, app: &str, slot: &str, row: &WatchSlot) -> Result<(), String>;
}

impl Seen for Store {
    fn get(&self, app: &str, slot: &str) -> Result<WatchSlot, String> {
        self.watch_slot(app, slot).map_err(|e| e.to_string())
    }
    fn put(&mut self, app: &str, slot: &str, row: &WatchSlot) -> Result<(), String> {
        self.save_watch_slot(app, slot, row)
            .map_err(|e| e.to_string())
    }
}

/// Post `finding` unless its slot already holds it, superseding the slot's
/// claim. The post is written down before it is sent and settled after, so
/// a run that dies in between is finished by the next one (see [`recover`]).
pub fn record(
    rpc: &mut dyn Rpc,
    seen: &mut dyn Seen,
    app: &str,
    finding: Finding,
) -> Result<Posted, String> {
    let slot = finding.slot.clone();
    let mut row = recover(rpc, seen, app, &slot)?;
    if let Some(p) = row.posted.as_ref().filter(|p| p.key == finding.key) {
        return Ok(Posted {
            claim: Some(MsgId(p.claim)),
            finding,
            new: false,
            supersedes: None,
        });
    }
    let supersedes = row.posted.as_ref().map(|p| MsgId(p.claim));
    let args = post_args(app, &finding, supersedes);
    row.pending = Some(PendingPost {
        key: finding.key.clone(),
        post: rusty_serde::json::to_string(&args).map_err(|e| e.to_string())?,
    });
    seen.put(app, &slot, &row)
        .map_err(|e| format!("not sent, the pending post was not saved: {e}"))?;
    let claim = match send(rpc, &args)? {
        Sent::Posted(c) => c,
        Sent::Refused(e) => {
            row.pending = None;
            let cleared = seen.put(app, &slot, &row);
            return Err(match cleared {
                Ok(()) => e,
                Err(c) => format!("{e}; the pending post stays until a later run: {c}"),
            });
        }
    };
    row.posted = Some(WatchFinding {
        key: finding.key.clone(),
        claim: claim.0,
    });
    row.pending = None;
    seen.put(app, &slot, &row)
        .map_err(|e| format!("{claim} posted, recorded next run: {e}"))?;
    Ok(Posted {
        finding,
        claim: Some(claim),
        new: true,
        supersedes,
    })
}

/// The slot, with a post an earlier run wrote down but did not settle
/// resent first. BBP replays it if it landed (the same op and bytes) or
/// posts it now, and either way its claim becomes the slot's, so the next
/// finding supersedes it rather than leaving it orphaned. A refused resend
/// never posted; it is dropped and reported.
pub fn recover(
    rpc: &mut dyn Rpc,
    seen: &mut dyn Seen,
    app: &str,
    slot: &str,
) -> Result<WatchSlot, String> {
    let mut row = seen.get(app, slot)?;
    let Some(p) = row.pending.take() else {
        return Ok(row);
    };
    let args: Value = rusty_serde::json::from_str(&p.post)
        .map_err(|e| format!("pending post for {}: {e}", p.key))?;
    let refused = match send(rpc, &args)? {
        Sent::Posted(c) => {
            row.posted = Some(WatchFinding {
                key: p.key.clone(),
                claim: c.0,
            });
            None
        }
        Sent::Refused(e) => Some(e),
    };
    seen.put(app, slot, &row)
        .map_err(|e| format!("pending post for {} not settled: {e}", p.key))?;
    match refused {
        None => Ok(row),
        Some(e) => Err(format!(
            "pending post for {} was refused and dropped: {e}",
            p.key
        )),
    }
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

/// The op id of posting `finding` on `app` over the slot's claim
/// `supersedes`: the same post always maps to it, and nothing else does.
pub fn op_id(app: &str, finding: &Finding, supersedes: Option<MsgId>) -> String {
    let prior = supersedes.map_or(String::new(), |c| c.0.to_string());
    let digest = rusty_bbp::Sha256::of(format!("{app}\n{}\n{prior}", finding.key).as_bytes());
    format!("watch-{}", &digest.hex()[..32])
}

/// The brief `task_opened` stores first. A claim cites it rather than the
/// current brief, so a repeated post stays byte-identical (R15).
const OPENING_BRIEF: &str = "art:1";

/// The `post` arguments for `finding` as a low-stakes, reversible claim,
/// citing the opening brief. Every field comes from `app`, `finding.key` and
/// `supersedes`, so a repeat is byte-identical and replays (R15).
pub fn post_args(app: &str, finding: &Finding, supersedes: Option<MsgId>) -> Value {
    let mut fields = vec![
        ("op", op_id(app, finding, supersedes).into()),
        ("kind", "claim".into()),
        ("text", finding.text.as_str().into()),
        ("domains", Value::Seq(vec![finding.domain.into()])),
        ("stakes", "low".into()),
        ("reversible", Value::Bool(true)),
        ("refs", Value::Seq(vec![OPENING_BRIEF.into()])),
    ];
    if let Some(x) = supersedes {
        fields.push(("supersedes", Value::UInt(x.0)));
    }
    map(fields)
}

/// Post `finding` (see [`post_args`]); a refusal is an error.
pub fn post(
    rpc: &mut dyn Rpc,
    app: &str,
    finding: &Finding,
    supersedes: Option<MsgId>,
) -> Result<MsgId, String> {
    match send(rpc, &post_args(app, finding, supersedes))? {
        Sent::Posted(id) => Ok(id),
        Sent::Refused(e) => Err(e),
    }
}

/// How a sent post ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Posted(MsgId),
    /// BBP answered and refused it: nothing was posted.
    Refused(String),
}

/// Send `post` once more if the first answer was lost. `Err` means no answer
/// arrived, so it may or may not have landed.
pub fn send(rpc: &mut dyn Rpc, args: &Value) -> Result<Sent, String> {
    let reply = match rpc.call("post", args.clone()) {
        Err(CallError::Lost(_)) => rpc.call("post", args.clone()),
        first => first,
    };
    let text = match reply {
        Ok(t) => t,
        Err(CallError::Answered(e)) => return Ok(Sent::Refused(e)),
        Err(CallError::Lost(e)) => return Err(format!("no answer: {e}")),
    };
    match rusty_serde::json::from_str::<Response>(&text) {
        Ok(Response::Posted(id)) => Ok(Sent::Posted(id)),
        Ok(Response::Rejected(r)) => Ok(Sent::Refused(format!("{:?}: {}", r.code, r.detail))),
        Ok(other) => Err(format!("post answered {other:?}")),
        Err(e) => Err(format!("unreadable answer: {e}")),
    }
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// A claims session and the table of what it posted.
pub type Session<'a> = (&'a mut dyn Rpc, &'a mut dyn Seen);

/// Watch `app` with `adapters`; record each finding through `session` unless
/// it is `None` (a dry run).
pub fn watch_app(
    app: &App,
    adapters: &[&dyn Watch],
    run: &dyn CommandRunner,
    mut session: Option<Session<'_>>,
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
            let Some((rpc, seen)) = session.as_mut() else {
                report.findings.push(Posted {
                    finding,
                    claim: None,
                    new: false,
                    supersedes: None,
                });
                continue;
            };
            let key = finding.key.clone();
            match record(&mut **rpc, &mut **seen, &app.app, finding) {
                Ok(p) => report.findings.push(p),
                Err(e) => report.errors.push(format!("post {key}: {e}")),
            }
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
    /// The orch state dir holding the `watch_finding` table.
    pub state: PathBuf,
    pub bbp: PathBuf,
    pub app: Option<String>,
    pub dry_run: bool,
}

/// Parse `--factory DIR --bbp-dir DIR --state DIR [--bbp PATH] [--app NAME] [--dry-run]`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let (mut factory, mut bbp_dir, mut state, mut bbp, mut app) = (None, None, None, None, None);
    let mut dry_run = false;
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
            "--state" => &mut state,
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
        state: need(state, "--state")?.into(),
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
    let mut store = match options.dry_run {
        true => None,
        false => Some(Store::open(&options.state).map_err(|e| e.to_string())?),
    };
    let mut reports = Vec::new();
    for app in &apps {
        let report = match store.as_mut() {
            None => watch_app(app, &adapters, runner, None),
            Some(store) => match Child::spawn(&options.bbp, &options.bbp_dir, &app.task, WATCHER) {
                Ok(mut child) => watch_app(app, &adapters, runner, Some((&mut child, store))),
                Err(e) => AppReport {
                    app: app.app.clone(),
                    task: app.task.clone(),
                    findings: Vec::new(),
                    errors: vec![e],
                },
            },
        };
        reports.push(report);
    }
    let clean = reports.iter().all(|r| r.errors.is_empty());
    let json = rusty_serde::json::to_string(&reports).map_err(|e| e.to_string())?;
    Ok((json, clean))
}

#[cfg(all(test, target_os = "linux"))]
mod e2e;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use orch_cli::fake::{exit, FakeCommand};
    use rusty_bbp::{Code, Rejection};
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

    /// A claims session with BBP's R15: a repeated op replays only with the
    /// same payload, and a changed payload is refused with `OpConflict`.
    /// Answers only `post`, so reading the current brief would panic.
    #[derive(Default)]
    struct Board {
        posts: Vec<Value>,
        ops: BTreeMap<String, (Value, Response)>,
        next: u64,
        /// Refuse the next post, as BBP does a `bad_claim`.
        refuse: bool,
    }

    impl Board {
        fn claims(&self) -> u64 {
            self.next
        }
    }

    impl Rpc for Board {
        fn call(&mut self, tool: &str, args: Value) -> Result<String, CallError> {
            assert_eq!(tool, "post", "the watcher only posts");
            let op = args["op"].as_str().unwrap().to_owned();
            self.posts.push(args.clone());
            if std::mem::take(&mut self.refuse) {
                let no = Rejection::new(Code::BadClaim, "supersedes target closed");
                return Ok(rusty_serde::json::to_string(&Response::Rejected(no)).unwrap());
            }
            let resp = match self.ops.get(&op) {
                // BBP sees the wire bytes, so compare those.
                Some((first, resp)) if wire(first) == wire(&args) => resp.clone(),
                Some(_) => Response::Rejected(Rejection::new(
                    Code::OpConflict,
                    "op reused with a different payload",
                )),
                None => {
                    self.next += 1;
                    let fresh = Response::Posted(MsgId(10 + self.next));
                    self.ops.insert(op, (args, fresh.clone()));
                    fresh
                }
            };
            Ok(rusty_serde::json::to_string(&resp).unwrap())
        }
    }

    fn wire(v: &Value) -> String {
        rusty_serde::json::to_string(v).unwrap()
    }

    /// One `cargo deny` advisory line.
    fn advisory(title: &str, cvss: &str) -> String {
        format!(
            r#"{{"type":"diagnostic","fields":{{"severity":"error","advisory":{{"id":"RUSTSEC-2026-0001","package":"foo","title":"{title}","cvss":"{cvss}"}}}}}}"#
        )
    }

    #[test]
    fn deny_findings_are_advisories_per_package_and_yanked_versions() {
        let f = advisories(DENY);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].key, "advisory:RUSTSEC-2026-0001:foo");
        assert_eq!(f[1].key, "yanked:bar:0.2.0");
        assert_eq!(f[0].domain, "security");
        assert_eq!(f[0].detail, "Use after free");
        assert!(!f[0].text.contains("Use after free"), "{}", f[0].text);
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

    /// A `rust-toolchain.toml` pinning `channel`.
    pub(super) fn pin(channel: &str) -> String {
        format!("[toolchain]\nchannel = \"{channel}\"\n")
    }

    /// The update-set finding for one move of `foo`.
    pub(super) fn set(to: &str) -> Finding {
        updates(&format!("    Updating foo v1.0.0 -> {to}\n")).unwrap()
    }

    #[test]
    fn toolchain_flags_only_a_different_channel() {
        let pinned = "[toolchain]\nchannel = \"1.98.1\"\ncomponents = [\"clippy\"]\n";
        let f = toolchain(pinned, "1.99.0").expect("finding");
        assert_eq!(f.key, "toolchain:rust:1.99.0");
        assert_eq!(f.detail, "pins 1.98.1");
        let moved = toolchain(&pin("1.98.2"), "1.99.0").expect("finding");
        assert_eq!((&moved.key, &moved.text), (&f.key, &f.text), "same finding");
        assert!(toolchain(pinned, "1.98.1").is_none());
        assert!(toolchain("[toolchain]\n", "1.99.0").is_none());
    }

    #[test]
    fn the_channel_is_read_as_toml() {
        let current = [
            "[toolchain]\nchannel = '1.99.0'\n",
            "[toolchain]\nchannel = \"1.99.0\" # pinned\n",
            "# rust\n[toolchain]\n  channel='1.99.0'  # pinned\nprofile = \"minimal\"\n",
        ];
        for file in current {
            assert_eq!(channel(file).as_deref(), Some("1.99.0"), "{file:?}");
            assert!(toolchain(file, "1.99.0").is_none(), "{file:?}");
        }
        let old = toolchain("[toolchain]\nchannel = '1.98.1' # old\n", "1.99.0").unwrap();
        assert_eq!(old.detail, "pins 1.98.1");
        let unread = [
            "channel = \"1.98.1\"\n",
            "[other]\nchannel = \"1.98.1\"\n",
            "[toolchain]\nchannel = 1.98\n",
            "[toolchain]\nchannel = \"1.98.1\" trailing\n",
            "[toolchain]\nchannel = \"1.98\\u002e1\"\n",
        ];
        for file in unread {
            assert_eq!(channel(file), None, "{file:?}");
        }
    }

    /// The `watch_finding` table in memory. `fail_after: Some(n)` lets `n`
    /// writes through and loses the next one.
    #[derive(Default)]
    pub(super) struct MemSeen {
        rows: BTreeMap<(String, String), WatchSlot>,
        pub(super) fail_after: Option<usize>,
    }

    impl Seen for MemSeen {
        fn get(&self, app: &str, slot: &str) -> Result<WatchSlot, String> {
            Ok(self
                .rows
                .get(&(app.into(), slot.into()))
                .cloned()
                .unwrap_or_default())
        }
        fn put(&mut self, app: &str, slot: &str, row: &WatchSlot) -> Result<(), String> {
            match self.fail_after {
                Some(0) => {
                    self.fail_after = None;
                    return Err("disk full".into());
                }
                Some(n) => self.fail_after = Some(n - 1),
                None => {}
            }
            self.rows.insert((app.into(), slot.into()), row.clone());
            Ok(())
        }
    }

    #[test]
    fn a_repeated_post_is_byte_identical_and_replays() {
        let f = &updates(UPDATE).expect("finding");
        assert_eq!(op_id("demo", f, None), op_id("demo", f, None));
        assert_ne!(op_id("demo", f, None), op_id("other", f, None));
        assert_ne!(op_id("demo", f, None), op_id("demo", f, Some(MsgId(4))));
        let mut board = Board::default();
        let first = post(&mut board, "demo", f, None).unwrap();
        let again = post(&mut board, "demo", f, None).unwrap();
        assert_eq!(first, again, "the second post replays the first");
        let p = &board.posts[0];
        assert_eq!(p["kind"].as_str(), Some("claim"));
        assert_eq!(p["stakes"].as_str(), Some("low"));
        assert_eq!(p["refs"][0].as_str(), Some("art:1"));
        assert!(p.get("supersedes").is_none());
        assert_eq!(p, &board.posts[1], "same payload, so BBP replays it");
        assert_eq!(board.claims(), 1);
    }

    #[test]
    fn changed_details_keep_the_first_claim_and_post_nothing() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let first = advisories(&advisory("Use after free", "CVSS:3.1/AV:N")).remove(0);
        let revised = advisories(&advisory("Use after free in parser", "CVSS:3.1/AV:L")).remove(0);
        assert_ne!(first.detail, revised.detail);
        let a = record(&mut board, &mut seen, "demo", first).unwrap();
        let b = record(&mut board, &mut seen, "demo", revised).unwrap();
        assert!(a.new && !b.new, "title and severity changed");
        assert_eq!(b.claim, a.claim);
        let pinned = |c: &str| toolchain(&pin(c), "1.99.0").unwrap();
        let t = record(&mut board, &mut seen, "demo", pinned("1.98.1")).unwrap();
        let moved = record(&mut board, &mut seen, "demo", pinned("1.98.2")).unwrap();
        assert!(!moved.new && moved.claim == t.claim, "the pin moved");
        assert_eq!(board.posts.len(), 2, "one post per finding");
    }

    #[test]
    fn a_newer_finding_in_a_slot_supersedes_the_open_claim() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let first = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        let next = record(&mut board, &mut seen, "demo", set("v1.0.2")).unwrap();
        assert!(next.new);
        assert_eq!(next.supersedes, first.claim);
        let sup = |i: usize| board.posts[i]["supersedes"].as_u64();
        assert_eq!(sup(1), first.claim.map(|c| c.0));
        let third = record(&mut board, &mut seen, "demo", set("v1.0.3")).unwrap();
        assert_eq!(third.supersedes, next.claim, "always the slot's latest");
        let rust = |w: &str| toolchain(&pin("1.98.1"), w).unwrap();
        let old = record(&mut board, &mut seen, "demo", rust("1.99.0")).unwrap();
        let new = record(&mut board, &mut seen, "demo", rust("1.100.0")).unwrap();
        assert_eq!(new.supersedes, old.claim, "a new watched Rust");
        assert_eq!(old.supersedes, None, "slots are separate");
        let other = record(&mut board, &mut seen, "other", set("v1.0.1")).unwrap();
        assert_eq!(other.supersedes, None, "apps are separate");
    }

    #[test]
    fn a_post_whose_record_was_lost_is_settled_before_the_slot_moves_on() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let a = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        // B posts, but its settling write is lost.
        seen.fail_after = Some(1);
        let e = record(&mut board, &mut seen, "demo", set("v1.0.2")).unwrap_err();
        assert!(e.contains("posted, recorded next run"), "{e}");
        let b = board.next;
        // The next run sees C: B is recovered first, and C supersedes it.
        let c = record(&mut board, &mut seen, "demo", set("v1.0.3")).unwrap();
        assert_eq!(c.supersedes, Some(MsgId(10 + b)), "not A ({:?})", a.claim);
        assert_eq!(board.claims(), 3, "B's resend replayed");
        assert_eq!(
            wire(&board.posts[1]),
            wire(&board.posts[2]),
            "byte for byte"
        );
        // Seeing B itself again after a lost write posts nothing new.
        seen.fail_after = Some(1);
        record(&mut board, &mut seen, "demo", set("v1.0.4")).unwrap_err();
        let again = record(&mut board, &mut seen, "demo", set("v1.0.4")).unwrap();
        assert!(!again.new && board.claims() == 4, "{again:?}");
    }

    #[test]
    fn a_post_that_was_never_written_down_is_never_sent() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        seen.fail_after = Some(0);
        let e = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap_err();
        assert!(e.contains("not sent"), "{e}");
        assert!(board.posts.is_empty());
    }

    #[test]
    fn a_refused_post_leaves_the_slot_as_it_was() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let a = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        board.refuse = true;
        let e = record(&mut board, &mut seen, "demo", set("v1.0.2")).unwrap_err();
        assert!(e.contains("BadClaim"), "{e}");
        let row = seen.get("demo", "updates").unwrap();
        assert_eq!(row.pending, None);
        assert_eq!(row.posted.map(|p| p.claim), a.claim.map(|c| c.0));
        // A pending post whose resend is refused is dropped and reported
        // once; the run after that goes on from the slot's claim.
        seen.fail_after = Some(1);
        record(&mut board, &mut seen, "demo", set("v1.0.3")).unwrap_err();
        board.refuse = true;
        let e = record(&mut board, &mut seen, "demo", set("v1.0.4")).unwrap_err();
        assert!(e.contains("refused and dropped"), "{e}");
        let next = record(&mut board, &mut seen, "demo", set("v1.0.4")).unwrap();
        assert!(next.new && next.supersedes == a.claim, "{next:?}");
    }

    #[test]
    fn a_key_that_returns_supersedes_the_slots_claim_without_a_conflict() {
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let a = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        let b = record(&mut board, &mut seen, "demo", set("v1.0.2")).unwrap();
        let back = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        assert!(back.new, "{back:?}");
        assert_eq!(back.supersedes, b.claim);
        assert_ne!(back.claim, a.claim, "a new claim over B, not A's op");
        let again = record(&mut board, &mut seen, "demo", set("v1.0.1")).unwrap();
        assert!(!again.new && again.claim == back.claim, "then it settles");
        let rust = |w: &str| toolchain(&pin("1.98.1"), w).unwrap();
        let old = record(&mut board, &mut seen, "demo", rust("1.99.0")).unwrap();
        record(&mut board, &mut seen, "demo", rust("1.100.0")).unwrap();
        let rolled_back = record(&mut board, &mut seen, "demo", rust("1.99.0")).unwrap();
        assert!(
            rolled_back.new && rolled_back.claim != old.claim,
            "watched Rust rolled back"
        );
    }

    #[test]
    fn the_board_refuses_a_changed_payload_under_one_op() {
        let mut board = Board::default();
        let f = updates(UPDATE).expect("finding");
        post(&mut board, "demo", &f, None).unwrap();
        let drifted = Finding {
            text: format!("{} Now.", f.text),
            ..f.clone()
        };
        let e = post(&mut board, "demo", &drifted, None).unwrap_err();
        assert!(e.contains("OpConflict"), "{e}");
    }

    #[test]
    fn a_failing_adapter_is_reported_and_the_others_still_run() {
        let dir = std::env::temp_dir().join(format!("orch_watch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rust-toolchain.toml"), pin("1.98.1")).unwrap();
        let run = FakeCommand::new(Ok(exit(101, "", "error: no Cargo.lock\n")));
        let tc = Toolchain(Watched {
            rust: Some("1.99.0".into()),
        });
        let adapters: [&dyn Watch; 2] = [&Updates, &tc];
        let (mut board, mut seen) = (Board::default(), MemSeen::default());
        let r = watch_app(&app(&dir), &adapters, &run, Some((&mut board, &mut seen)));
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
        let ok = o(&[
            "--factory",
            "f",
            "--bbp-dir",
            "d",
            "--state",
            "s",
            "--dry-run",
        ])
        .unwrap();
        assert_eq!(ok.state, PathBuf::from("s"));
        assert!(
            matches!(o(&["--factory", "f", "--bbp-dir", "d"]), Err(ArgsError::Usage(m)) if m.contains("--state"))
        );
        assert!(ok.dry_run);
        assert_eq!(ok.bbp, PathBuf::from("bbp"));
        assert!(
            matches!(o(&["--factory", "f"]), Err(ArgsError::Usage(m)) if m.contains("--bbp-dir"))
        );
        assert!(matches!(o(&["--app"]), Err(ArgsError::Usage(_))));
        assert!(matches!(o(&["--help"]), Err(ArgsError::Help)));
    }
}
