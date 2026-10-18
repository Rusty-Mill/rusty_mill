//! Explicit, single-task maintenance intake over the existing BBP host.
//! Inspection is always fresh; the write-ahead journal and BBP namespace survive
//! process death. Human actions remain on BBP's fenced human channel.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::num::NonZeroU32;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use orch_cli::{CommandRunner, StdCommand};
use orch_core::board::Board;
use orch_core::task::{Plan, Role, Task, TaskSpec};
use orch_core::{GoalId, Ref, Text};
use orch_dispatch::{HostHandle, Progress, TaskHost};
use rusty_bbp::{ArtifactPayload, FsStore, Sha256, State, Store, TaskState};
use rusty_serde::json::{self, Value};
use rusty_serde::{Deserialize, Serialize};

use crate::args::ArgsError;

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// Maintenance has no environment-variable opt-in and never relays approval.
pub const USAGE: &str = "usage: rusty_orch maintain [--repo DIR] [--bbp CONFIG] [--execute] [--publish-pr]\n       [--confirm-benefit FINDING_ID REASON]\n\nDefault: read-only JSON plan. --execute requires BBP config and a clean Git repo.\nOne task per cycle; repeat to recover/poll. State is shared by Git worktrees.\nApprove plans and settle requests through bbp human; maintenance never approves\nor merges. Validated, reviewed candidates produce a local PR bundle for a human\nto publish, or use --execute --publish-pr to authorize draft publication. Requires gh for a current open-PR conflict check before task creation.";

/// Explicit maintenance options, independent of `run` and its routing defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub repo: PathBuf,
    pub bbp: Option<PathBuf>,
    pub execute: bool,
    /// Explicit human authorization to publish the validated candidate as a draft PR.
    pub publish_pr: bool,
    pub benefits: BTreeMap<String, String>,
}

/// Parse an opt-in command; execution is never the default.
pub fn parse(
    argv: impl IntoIterator<Item = String>,
    cwd: PathBuf,
) -> std::result::Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    let mut options = Options {
        repo: cwd,
        bbp: None,
        execute: false,
        publish_pr: false,
        benefits: BTreeMap::new(),
    };
    let mut flags = BTreeSet::new();
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Err(ArgsError::Help);
        }
        if flag != "--confirm-benefit" && !flags.insert(flag.clone()) {
            return Err(ArgsError::Usage(format!("duplicate option {flag}")));
        }
        let mut value = || {
            args.next()
                .filter(|v| !v.trim().is_empty() && !v.starts_with('-'))
                .ok_or_else(|| ArgsError::Usage(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "--repo" => options.repo = value()?.into(),
            "--bbp" => options.bbp = Some(value()?.into()),
            "--execute" => options.execute = true,
            "--publish-pr" => options.publish_pr = true,
            "--confirm-benefit" => {
                let id = value()?;
                let reason = value()?;
                if options.benefits.insert(id, reason).is_some() {
                    return Err(ArgsError::Usage("duplicate benefit confirmation".into()));
                }
            }
            _ => {
                return Err(ArgsError::Usage(format!(
                    "unexpected maintenance argument {flag:?}"
                )))
            }
        }
    }
    if options.publish_pr && !options.execute {
        return Err(ArgsError::Usage("--publish-pr requires --execute".into()));
    }
    Ok(options)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CrateInfo {
    id: String,
    name: String,
    manifest: String,
    reverse_dependencies: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Impact {
    crates_to_validate: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Finding {
    id: String,
    rule: String,
    affected_crates: Vec<String>,
    evidence: Vec<Value>,
    recommendation: String,
    impact: Impact,
}
#[derive(Debug, Clone, Deserialize)]
struct Inspection {
    schema_version: u32,
    workspace_root: String,
    crates: Vec<CrateInfo>,
    findings: Vec<Finding>,
}

/// Reviewable selection, including an explicit reason for every excluded finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub finding_id: String,
    pub rule: String,
    pub affected_crates: Vec<String>,
    pub validation_packages: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub evidence: Vec<Value>,
    pub benefit: String,
    pub instruction: String,
}
#[derive(Debug, Clone, Serialize)]
struct Exclusion {
    finding_id: String,
    reason: String,
}
#[derive(Debug, Serialize)]
struct DryRun {
    schema_version: u32,
    mode: String,
    selected: Option<Selection>,
    excluded: Vec<Exclusion>,
    state: Option<Value>,
}

fn relative_manifest(path: &str) -> bool {
    path.ends_with("/Cargo.toml")
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn select(
    report: &Inspection,
    benefits: &BTreeMap<String, String>,
    seen: &BTreeSet<String>,
    conflicts: &BTreeSet<String>,
) -> Result<(Option<Selection>, Vec<Exclusion>)> {
    if report.schema_version != 1 {
        return Err("unsupported inspection schema".into());
    }
    let crates: BTreeMap<_, _> = report.crates.iter().map(|c| (c.id.as_str(), c)).collect();
    if crates.len() != report.crates.len() {
        return Err("duplicate crate IDs in inspection".into());
    }
    let mut candidates = Vec::new();
    let mut excluded = Vec::new();
    for f in &report.findings {
        let candidate = assess(f, &crates, benefits, seen, conflicts);
        match candidate {
            Ok(c) => candidates.push(c),
            Err(reason) => excluded.push(Exclusion {
                finding_id: f.id.clone(),
                reason: reason.to_string(),
            }),
        }
    }
    // Fix explicit policy defects before advisory simplification; then minimize
    // the reverse closure, edit surface, and finally portable finding identity.
    candidates.sort_by_key(|c| {
        (
            u8::from(c.rule != "layer-classification"),
            c.validation_packages.len(),
            c.allowed_paths.len(),
            c.finding_id.clone(),
        )
    });
    let selected = candidates.first().cloned();
    excluded.extend(candidates.into_iter().skip(1).map(|c| Exclusion {
        finding_id: c.finding_id,
        reason: "one task per cycle; lower priority".into(),
    }));
    excluded.sort_by(|a, b| a.finding_id.cmp(&b.finding_id));
    Ok((selected, excluded))
}

fn assess(
    f: &Finding,
    crates: &BTreeMap<&str, &CrateInfo>,
    benefits: &BTreeMap<String, String>,
    seen: &BTreeSet<String>,
    conflicts: &BTreeSet<String>,
) -> Result<Selection> {
    if seen.contains(&f.id) {
        return Err("already attempted; recover the existing task instead".into());
    }
    let benefit = match f.rule.as_str() {
        "layer-classification" => "Restore the existing directory's declared architectural layer without changing dependencies or code".to_owned(),
        "duplicate-declaration" => benefits.get(&f.id).filter(|s| !s.trim().is_empty()).cloned().ok_or("advisory: explicit benefit confirmation required")?,
        _ => return Err("structural or dependency-policy change requires separate design; not low risk".into()),
    };
    if f.affected_crates.is_empty() || f.affected_crates.len() > 2 || f.evidence.is_empty() {
        return Err("edit scope must cover one or two crates with evidence".into());
    }
    let mut paths = BTreeSet::new();
    for id in &f.affected_crates {
        let c = crates.get(id.as_str()).ok_or("unknown affected crate")?;
        if !relative_manifest(&c.manifest) {
            return Err("unsupported manifest boundary".into());
        }
        if f.rule == "layer-classification" {
            let parts: Vec<_> = c.manifest.split('/').collect();
            if parts.len() < 4
                || parts[0] != "crates"
                || !["foundation", "platform", "libs", "tools", "apps"].contains(&parts[1])
            {
                return Err("layer repair requires an unambiguous directory layer".into());
            }
        }
        paths.insert(c.manifest.clone());
    }
    if f.rule == "duplicate-declaration" {
        paths.insert("Cargo.toml".into());
    }
    // Recompute closure from the report graph; never trust a truncated impact.
    let mut closure = BTreeSet::new();
    let mut pending = f.affected_crates.clone();
    while let Some(id) = pending.pop() {
        if !closure.insert(id.clone()) {
            continue;
        }
        let c = crates
            .get(id.as_str())
            .ok_or("unknown reverse dependency")?;
        pending.extend(c.reverse_dependencies.clone());
    }
    if closure != f.impact.crates_to_validate.iter().cloned().collect() {
        return Err("inspection impact differs from reverse closure".into());
    }
    if closure.len() > 8 {
        return Err("validation impact exceeds eight crates".into());
    }
    let mut packages = Vec::new();
    for id in closure {
        let c = crates.get(id.as_str()).ok_or("missing validation crate")?;
        if crates.values().filter(|other| other.name == c.name).count() != 1 {
            return Err("ambiguous package name".into());
        }
        packages.push(c.name.clone());
    }
    packages.sort();
    if paths.iter().any(|p| conflicts.contains(p)) {
        return Err("open PR touches an allowed edit path".into());
    }
    let allowed_paths: Vec<_> = paths.into_iter().collect();
    let instruction = format!("Maintenance finding {} ({})\nEvidence: {}\nConfirmed benefit: {}\nRecommendation: {}\nAffected crates: {:?}\nOnly edit: {:?}. Preserve public APIs, dependency versions, features, source semantics and Cargo.lock. No migrations, deletions, governance edits, push or merge. Confirm the repair is beneficial in the proposed spec; if uncertain, request human input. Obtain human plan approval before producing a candidate. Independently review the candidate. Validate packages {:?}, including reverse dependencies, with frozen Cargo test and clippy; fmt must pass. Do not broaden scope to make validation pass.", f.id, f.rule, json::to_string(&f.evidence)?, benefit, f.recommendation, f.affected_crates, allowed_paths, packages);
    Ok(Selection {
        finding_id: f.id.clone(),
        rule: f.rule.clone(),
        affected_crates: f.affected_crates.clone(),
        validation_packages: packages,
        allowed_paths,
        evidence: f.evidence.clone(),
        benefit,
        instruction,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Attempt {
    selection: Selection,
    base: String,
    config: orch_bbp::Config,
    profiles: String,
    namespace: String,
    handle: Option<String>,
    status: String,
    detail: String,
    pr_url: Option<String>,
    last_card: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    schema_version: u32,
    repo: String,
    attempts: Vec<Attempt>,
}

fn public_state(journal: &Journal) -> Result<Value> {
    // Freeze execution inputs privately for recovery, but never print launcher
    // arguments or the configured workload environment in a public report.
    let attempts = journal
        .attempts
        .iter()
        .map(|attempt| {
            Ok(Value::Map(vec![
                (
                    "selection".into(),
                    json::from_str(&json::to_string(&attempt.selection)?)?,
                ),
                ("base".into(), Value::String(attempt.base.clone())),
                ("namespace".into(), Value::String(attempt.namespace.clone())),
                (
                    "handle".into(),
                    attempt
                        .handle
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                ("status".into(), Value::String(attempt.status.clone())),
                ("detail".into(), Value::String(attempt.detail.clone())),
                (
                    "pr_url".into(),
                    attempt
                        .pr_url
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "last_card".into(),
                    attempt.last_card.clone().unwrap_or(Value::Null),
                ),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Map(vec![
        ("schema_version".into(), Value::UInt(1)),
        ("repo".into(), Value::String(journal.repo.clone())),
        ("attempts".into(), Value::Seq(attempts)),
    ]))
}

fn command(runner: &impl CommandRunner, repo: &Path, args: &[String]) -> Result<String> {
    let exit = runner.run_in(
        Some(repo),
        args,
        &[],
        Duration::from_secs(30),
        orch_bbp::SCRUB,
    )?;
    if exit.status != 0 {
        return Err(format!("{}: {}", args[0], orch_cli::excerpt(&exit.stderr)).into());
    }
    Ok(String::from_utf8(exit.stdout)?.trim().to_owned())
}
fn git(runner: &impl CommandRunner, repo: &Path, args: &[&str]) -> Result<String> {
    command(
        runner,
        repo,
        &std::iter::once("git")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
}
fn save(path: &Path, journal: &Journal) -> Result<()> {
    rusty_atomic_file::write_private(path, json::to_string(journal)?.as_bytes())?;
    Ok(())
}
fn load(path: &Path, repo: &Path) -> Result<Journal> {
    let state: Journal = if path.exists() {
        json::from_str(&fs::read_to_string(path)?)?
    } else {
        Journal {
            schema_version: 1,
            repo: repo.to_str().ok_or("non-UTF8 repo")?.into(),
            attempts: vec![],
        }
    };
    if state.schema_version != 1 || state.repo != repo.to_string_lossy() {
        return Err("maintenance journal schema/repository mismatch".into());
    }
    Ok(state)
}

fn profiles(input: &str, selected: &Selection) -> Result<String> {
    let mut set: BTreeMap<String, Value> = json::from_str(input)?;
    let mut profiles: Vec<Value> = match set.remove("profiles") {
        Some(Value::Seq(p)) if !p.is_empty() => p,
        _ => return Err("BBP profile set must contain validation profiles".into()),
    };
    profiles.insert(0, scope_profile(selected)?);
    for (name, args) in validation_commands(selected) {
        let profile = Value::Map(vec![
            ("name".into(), Value::String(name)),
            ("program".into(), Value::String("cargo".into())),
            (
                "args".into(),
                Value::Seq(args.into_iter().map(Value::String).collect()),
            ),
        ]);
        profiles.push(profile);
    }
    set.insert("profiles".into(), Value::Seq(profiles));
    Ok(json::to_string(&set)?)
}
fn scope_profile(selected: &Selection) -> Result<Value> {
    Ok(Value::Map(vec![
        ("name".into(), Value::String("maintenance-scope".into())),
        ("program".into(), Value::String("python3".into())),
        (
            "args".into(),
            Value::Seq(vec![
                Value::String("-I".into()),
                Value::String("-c".into()),
                Value::String(include_str!("maintenance_validation.py").into()),
                Value::String(json::to_string(selected)?),
            ]),
        ),
    ]))
}

fn validation_commands(selected: &Selection) -> Vec<(String, Vec<String>)> {
    let mut commands = vec![(
        "maintenance-fmt".into(),
        vec!["fmt".into(), "--all".into(), "--".into(), "--check".into()],
    )];
    for tool in ["test", "clippy"] {
        let mut args = vec![tool.into(), "--frozen".into(), "--all-features".into()];
        for package in &selected.validation_packages {
            args.extend(["-p".into(), package.clone()]);
        }
        if tool == "clippy" {
            args.extend(["--".into(), "-D".into(), "warnings".into()]);
        }
        commands.push((format!("maintenance-{tool}"), args));
    }
    commands
}
fn task(selected: &Selection) -> Result<(Task, Board)> {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let text = |s: &str| Text::new(s).ok_or("empty maintenance instruction");
    let id = plan.add(TaskSpec {
        role: Role::Implement,
        instruction: text(&selected.instruction)?,
        acceptance: vec![text("Bounded beneficial repair; human-approved plan; scoped passing validation and independent review")?],
        refs: selected.allowed_paths.iter().map(|p| text(p).map(Ref::Path)).collect::<std::result::Result<_, _>>()?,
        depends_on: vec![], max_calls: NonZeroU32::new(1).ok_or("invalid budget")?,
    })?;
    Ok((
        plan.get(id).ok_or("maintenance task missing")?.clone(),
        Board::new(plan.goal()),
    ))
}

fn open_pr_conflicts(runner: &impl CommandRunner, repo: &Path) -> Result<BTreeSet<String>> {
    // Authentication remains the gh CLI's own; credentials are not passed to BBP.
    let exit = runner.run_in(
        Some(repo),
        &[
            "gh".into(),
            "pr".into(),
            "list".into(),
            "--state".into(),
            "open".into(),
            "--limit".into(),
            "1000".into(),
            "--json".into(),
            "files,headRefName,url".into(),
        ],
        &[],
        Duration::from_secs(30),
        &[],
    )?;
    if exit.status != 0 {
        return Err(format!(
            "cannot establish current PR state: {}",
            orch_cli::excerpt(&exit.stderr)
        )
        .into());
    }
    let prs: Vec<Value> = json::from_str(std::str::from_utf8(&exit.stdout)?)?;
    if prs.len() >= 1000 {
        return Err("open PR inventory may be truncated".into());
    }
    let mut files = BTreeSet::new();
    for pr in prs {
        let entries = pr
            .get("files")
            .and_then(Value::as_seq)
            .ok_or("PR files missing")?;
        if entries.len() >= 100 {
            return Err("PR file inventory may be truncated; resolve before maintenance".into());
        }
        for f in entries {
            files.insert(
                f.get("path")
                    .and_then(Value::as_str)
                    .ok_or("PR file path missing")?
                    .to_owned(),
            );
        }
    }
    Ok(files)
}

/// Run one cycle. Dry-run reads no provider configuration and creates no files.
pub fn execute(options: &Options, runner: &impl CommandRunner) -> Result<String> {
    let repo = options.repo.canonicalize()?;
    let common = PathBuf::from(git(
        runner,
        &repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    // Resolve the inspected checkout separately from the shared Git identity.
    let source =
        PathBuf::from(git(runner, &repo, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let state_dir = common.join("rusty-orch-maintenance");
    let journal_path = state_dir.join("journal.json");
    let lock = if options.execute {
        if !cfg!(target_os = "linux") {
            return Err("maintenance execution requires sandboxed Linux BBP".into());
        }
        fs::create_dir_all(&state_dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(state_dir.join("lock"))?;
        file.try_lock()
            .map_err(|e| format!("another maintenance cycle owns this repository: {e}"))?;
        Some(file)
    } else {
        None
    };
    // Use common Git directory as identity so switching a worktree cannot fork state.
    let mut journal = load(&journal_path, &common)?;
    if let Some(index) = journal
        .attempts
        .iter()
        .position(|a| !matches!(a.status.as_str(), "success" | "cancelled"))
    {
        if !options.execute {
            return Ok(json::to_string(&DryRun {
                schema_version: 1,
                mode: "dry-run: recovery pending".into(),
                selected: None,
                excluded: vec![],
                state: Some(public_state(&journal)?),
            })?);
        }
        advance(&mut journal, index, &journal_path, &state_dir)?;
        publish_if_requested(
            options,
            runner,
            &mut journal,
            index,
            &journal_path,
            &state_dir,
        )?;
        return Ok(json::to_string(&public_state(&journal)?)?);
    }
    let inspected_base = git(runner, &source, &["rev-parse", "HEAD"])?;
    let inspection: Inspection = json::from_str(&crate::inspect::inspect(&source, runner)?)?;
    if Path::new(&inspection.workspace_root).canonicalize()? != source {
        return Err("inspection root mismatch".into());
    }
    let seen = journal
        .attempts
        .iter()
        .map(|a| a.selection.finding_id.clone())
        .collect();
    let conflicts = if options.execute {
        open_pr_conflicts(runner, &source)?
    } else {
        BTreeSet::new()
    };
    let (selected, excluded) = select(&inspection, &options.benefits, &seen, &conflicts)?;
    if !options.execute || selected.is_none() {
        return Ok(json::to_string(&DryRun {
            schema_version: 1,
            mode: if options.execute {
                "no eligible task"
            } else {
                "dry-run: PR/task conflicts checked at execution"
            }
            .into(),
            selected,
            excluded,
            state: None,
        })?);
    }
    if !git(runner, &source, &["status", "--porcelain"])?.is_empty() {
        return Err("maintenance requires a clean source worktree".into());
    }
    let selected = selected.ok_or("missing selection")?;
    let config_path = options
        .bbp
        .as_ref()
        .ok_or("--execute requires --bbp CONFIG")?;
    let mut config: orch_bbp::Config = json::from_str(&fs::read_to_string(config_path)?)?;
    config = config.absolute()?;
    config.validate()?;
    if config.repo.canonicalize()? != source {
        return Err("BBP config repository differs from inspection".into());
    }
    if config.assignments.values().any(|a| a.builtin_tools) {
        return Err(
            "maintenance agents must use BBP artifacts; built-in tools must be disabled".into(),
        );
    }
    for manifest in &selected.allowed_paths {
        let path = source.join(manifest);
        if !path.canonicalize()?.starts_with(&source)
            || fs::symlink_metadata(&path)?.file_type().is_symlink()
        {
            return Err("maintenance manifest must be a regular in-repository file".into());
        }
    }
    if active_bbp_task(&config)?.is_some() {
        return Err("existing nonterminal BBP task for this repository; resolve it before selecting maintenance".into());
    }
    let profiles = profiles(&fs::read_to_string(&config.profiles)?, &selected)?;
    let namespace = Sha256::of(
        format!(
            "maintenance-v1:{}:{}",
            common.display(),
            selected.finding_id
        )
        .as_bytes(),
    )
    .hex();
    config.profiles = state_dir.join(format!("{namespace}-profiles.json"));
    let base = git(runner, &source, &["rev-parse", "HEAD"])?;
    if base != inspected_base {
        return Err(
            "source revision changed during inspection; repeat maintenance on the current head"
                .into(),
        );
    }
    journal.attempts.push(Attempt {
        selection: selected,
        base,
        config,
        profiles,
        namespace,
        handle: None,
        status: "opening".into(),
        detail: "write-ahead intent; uncertain opens are adopted".into(),
        pr_url: None,
        last_card: None,
    });
    save(&journal_path, &journal)?;
    let index = journal.attempts.len() - 1;
    advance(&mut journal, index, &journal_path, &state_dir)?;
    publish_if_requested(
        options,
        runner,
        &mut journal,
        index,
        &journal_path,
        &state_dir,
    )?;
    drop(lock);
    Ok(json::to_string(&public_state(&journal)?)?)
}

fn advance(journal: &mut Journal, index: usize, path: &Path, state_dir: &Path) -> Result<()> {
    let attempt = &mut journal.attempts[index];
    // Restored from the journal, not the mutable config/profile files supplied
    // by this invocation. Intent is already durable before any external call.
    rusty_atomic_file::write_private(&attempt.config.profiles, attempt.profiles.as_bytes())?;
    let result = advance_host(attempt, state_dir);
    if let Err(error) = &result {
        attempt.status = "interrupted".into();
        attempt.detail = error.to_string();
    }
    save(path, journal)?;
    result
}
fn advance_host(attempt: &mut Attempt, state_dir: &Path) -> Result<()> {
    if attempt.handle.is_none()
        && git(&StdCommand, &attempt.config.repo, &["rev-parse", "HEAD"])? != attempt.base
    {
        return Err(
            "source revision changed during uncertain open; restore recorded base before recovery"
                .into(),
        );
    }
    let mut host = orch_bbp::BbpHost::new(
        attempt.config.clone(),
        attempt.namespace.clone(),
        attempt.selection.instruction.clone(),
    )?;
    drive(attempt, &mut host)?;
    observe(attempt)?;
    if attempt.status == "validated" {
        prepare(attempt, state_dir)?;
        attempt.status = "prepared".into();
    }
    Ok(())
}
fn drive(attempt: &mut Attempt, host: &mut impl TaskHost) -> Result<()> {
    let handle = match &attempt.handle {
        Some(h) => HostHandle(Text::new(h).ok_or("empty hosted handle")?),
        None => {
            let (task, board) = task(&attempt.selection)?;
            let handle = host
                .open(&task, &board)
                .map_err(|e| format!("BBP open: {e:?}"))?;
            attempt.handle = Some(handle.0.as_str().into());
            handle
        }
    };
    match host.poll(&handle).map_err(|e| format!("BBP poll: {e:?}"))? {
        Progress::Working => {
            attempt.status = "working".into();
            attempt.detail = "BBP owns execution; repeat maintain --execute to poll".into();
        }
        Progress::Gate(g) => {
            attempt.status = "waiting-human".into();
            attempt.detail = format!("Revision {}: {}", g.rev, g.prompt);
        }
        Progress::Cancelled { reason } => {
            attempt.status = "cancelled".into();
            attempt.detail = reason.as_str().into();
        }
        Progress::Closed { .. } => {
            attempt.status = "success".into();
            attempt.detail = "BBP records an externally supplied merge receipt".into();
        }
    }
    Ok(())
}

fn prepare(attempt: &Attempt, state_dir: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Handle {
        dir: String,
        task: String,
    }
    let handle: Handle = json::from_str(
        attempt
            .handle
            .as_ref()
            .and_then(|h| h.strip_prefix("bbp:"))
            .ok_or("missing BBP handle")?,
    )?;
    let mut store = FsStore::open(Path::new(&handle.dir))?;
    let id = rusty_bbp::TaskId(handle.task);
    store
        .refresh(&id)
        .map_err(|e| format!("BBP refresh: {e:?}"))?;
    let events = store
        .events(&id, 0)
        .map_err(|e| format!("BBP events: {e:?}"))?;
    let state = TaskState::fold(id, &events);
    if !matches!(state.state, State::MergeGate | State::Approved) {
        return Err(
            "PR preparation requires current independent review and passing validation".into(),
        );
    }
    let card = state.card();
    let candidate_id = card.candidate.ok_or("missing candidate")?;
    let (_, _, report_id) = card.run.ok_or("missing validation run")?;
    let report_id = report_id.ok_or("missing validation report")?;
    let record = state
        .artifacts
        .get(&report_id)
        .ok_or("missing report artifact")?;
    let ArtifactPayload::TestReport(report) = &record.payload else {
        return Err("invalid report payload".into());
    };
    if report.candidate != candidate_id
        || report.status != rusty_bbp::RunStatus::Passed
        || !report.consistent()
        || report.sandbox == "unconfined"
        || report.tree.is_none()
    {
        return Err("PR preparation requires scoped passing sandbox validation".into());
    }
    for name in std::iter::once("maintenance-scope".to_owned()).chain(
        validation_commands(&attempt.selection)
            .into_iter()
            .map(|(name, _)| name),
    ) {
        if !report
            .profiles
            .iter()
            .any(|p| p.name == name && p.exit_code == 0 && p.failed == 0)
        {
            return Err("maintenance validation profile missing".into());
        }
    }
    let candidate = state
        .artifacts
        .get(&candidate_id)
        .ok_or("missing candidate artifact")?;
    let ArtifactPayload::Candidate(candidate) = &candidate.payload else {
        return Err("invalid candidate payload".into());
    };
    if candidate.base != attempt.base {
        return Err("candidate base differs from inspected revision".into());
    }
    let mut patch = Vec::new();
    for diff in &candidate.diffs {
        let record = state.artifacts.get(diff).ok_or("missing candidate diff")?;
        let bytes = store
            .blob_get(&record.blob.sha)
            .map_err(|e| format!("BBP blob: {e:?}"))?;
        check_patch(
            std::str::from_utf8(&bytes)?,
            &attempt.selection.allowed_paths,
        )?;
        patch.extend(bytes);
        if !patch.ends_with(b"\n") {
            patch.push(b'\n');
        }
    }
    if patch.is_empty() {
        return Err("empty maintenance candidate".into());
    }
    let dir = state_dir.join("pr").join(&attempt.namespace);
    fs::create_dir_all(&dir)?;
    rusty_atomic_file::write_private(&dir.join("candidate.patch"), &patch)?;
    let body = format!("# Maintenance: {}\n\n{}\n\nBase: `{}`\nCandidate: art:{}; report: art:{}; tree: {:?}\nSandbox: {}\nValidation packages: {:?}\n\nHuman publication required (maintain --execute --publish-pr). Alternatively apply candidate.patch to a branch at the recorded base, review the diff, and open a PR. Never merge automatically. This bundle is not a published PR.\n", attempt.selection.finding_id, attempt.selection.instruction, candidate.base, candidate_id.0, report_id.0, report.tree, report.sandbox, attempt.selection.validation_packages);
    rusty_atomic_file::write_private(&dir.join("PR.md"), body.as_bytes())?;
    rusty_atomic_file::write_private(
        &dir.join("report.json"),
        json::to_string(report)?.as_bytes(),
    )?;
    Ok(())
}

fn check_patch(patch: &str, allowed: &[String]) -> Result<()> {
    let mut old = None;
    let mut files = 0;
    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("--- ") {
            let path = path
                .strip_prefix("a/")
                .ok_or("maintenance cannot add/delete or rename files")?;
            if !allowed.iter().any(|p| p == path) {
                return Err("candidate edits outside maintenance scope".into());
            }
            old = Some(path);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            let path = path
                .strip_prefix("b/")
                .ok_or("maintenance cannot add/delete or rename files")?;
            if old.take() != Some(path) {
                return Err("candidate path pair mismatch".into());
            }
            files += 1;
        }
    }
    if files == 0 || old.is_some() {
        return Err("candidate has no complete manifest patch".into());
    }
    Ok(())
}

/// Production entry point over fixed process arguments and scrubbed BBP commands.
pub fn run(options: &Options) -> Result<String> {
    execute(options, &StdCommand)
}

fn active_bbp_task(config: &orch_bbp::Config) -> Result<Option<String>> {
    if !config.dir.join("tasks").exists() {
        return Ok(None);
    }
    let store = FsStore::open(&config.dir)?;
    for task in store.task_ids() {
        let events = store
            .events(&task, 0)
            .map_err(|e| format!("BBP task inventory: {e:?}"))?;
        let state = TaskState::fold(task.clone(), &events);
        if Path::new(&state.repo).canonicalize()? == config.repo.canonicalize()?
            && !state.state.terminal()
        {
            return Ok(Some(task.0));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod tests;

fn observe(attempt: &mut Attempt) -> Result<()> {
    #[derive(Deserialize)]
    struct Handle {
        dir: String,
        task: String,
    }
    let h: Handle = json::from_str(
        attempt
            .handle
            .as_ref()
            .and_then(|s| s.strip_prefix("bbp:"))
            .ok_or("missing task handle")?,
    )?;
    let mut store = FsStore::open(Path::new(&h.dir))?;
    let task = rusty_bbp::TaskId(h.task);
    store
        .refresh(&task)
        .map_err(|e| format!("BBP observation: {e:?}"))?;
    let events = store
        .events(&task, 0)
        .map_err(|e| format!("BBP observation: {e:?}"))?;
    let view = rusty_bbp::TaskView::from_events(task, &events);
    attempt.last_card = Some(json::from_str(&json::to_string(&view)?)?);
    if matches!(view.card.state, State::MergeGate | State::Approved) {
        attempt.status = "validated".into();
    }
    if matches!(
        view.card.run,
        Some((
            _,
            Some(rusty_bbp::RunStatus::Failed | rusty_bbp::RunStatus::Error),
            _
        ))
    ) && attempt.status == "working"
    {
        attempt.status = "validation-failed".into();
        attempt.detail =
            "BBP retains the failed validation evidence and owns recovery within the same task"
                .into();
    }
    Ok(())
}

fn publish_if_requested(
    options: &Options,
    runner: &impl CommandRunner,
    journal: &mut Journal,
    index: usize,
    path: &Path,
    state_dir: &Path,
) -> Result<()> {
    let attempt = &mut journal.attempts[index];
    if !options.publish_pr || attempt.status != "prepared" {
        return Ok(());
    }
    let result = publish(attempt, runner, state_dir);
    match result {
        Ok(url) => attempt.pr_url = Some(url),
        Err(error) => {
            attempt.detail = format!("Draft PR publication interrupted: {error}. Repeat --execute --publish-pr to reconcile the same branch/PR.");
            save(path, journal)?;
            return Err(error);
        }
    }
    save(path, journal)
}

fn gh(runner: &impl CommandRunner, repo: &Path, args: &[String]) -> Result<String> {
    let argv: Vec<_> = std::iter::once("gh".into())
        .chain(args.iter().cloned())
        .collect();
    let out = runner.run_in(Some(repo), &argv, &[], Duration::from_secs(60), &[])?;
    if out.status != 0 {
        return Err(format!("gh: {}", orch_cli::excerpt(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?.trim().into())
}

fn publish(attempt: &Attempt, runner: &impl CommandRunner, state_dir: &Path) -> Result<String> {
    // prepare() has verified the current BBP gate, scoped profiles, sandbox,
    // independent review and exact base. No Git mutation occurs before this.
    let bundle = state_dir.join("pr").join(&attempt.namespace);
    let report: rusty_bbp::Report =
        json::from_str(&fs::read_to_string(bundle.join("report.json"))?)?;
    let expected = report.tree.ok_or("missing validated tree")?;
    let branch = format!("maintenance/{}", attempt.namespace);
    let checkout = attempt
        .config
        .work
        .join("maintenance-pr")
        .join(&attempt.namespace);
    let source = &attempt.config.repo;
    if !checkout.exists() {
        command(
            runner,
            source,
            &[
                "git".into(),
                "worktree".into(),
                "add".into(),
                "-b".into(),
                branch.clone(),
                checkout.to_str().ok_or("non-UTF8 PR checkout")?.into(),
                attempt.base.clone(),
            ],
        )?;
    }
    if git(runner, &checkout, &["symbolic-ref", "--short", "HEAD"])? != branch {
        return Err("PR checkout branch mismatch".into());
    }
    let head = git(runner, &checkout, &["rev-parse", "HEAD"])?;
    let tree = git(runner, &checkout, &["write-tree"])?;
    if Sha256::of(tree.as_bytes()) != expected {
        if head != attempt.base || !git(runner, &checkout, &["status", "--porcelain"])?.is_empty() {
            return Err(
                "PR checkout differs from base/validated tree; inspect it before retrying".into(),
            );
        }
        command(
            runner,
            &checkout,
            &[
                "git".into(),
                "apply".into(),
                "--index".into(),
                bundle
                    .join("candidate.patch")
                    .to_str()
                    .ok_or("non-UTF8 patch path")?
                    .into(),
            ],
        )?;
    }
    let tree = git(runner, &checkout, &["write-tree"])?;
    if Sha256::of(tree.as_bytes()) != expected {
        return Err("PR tree differs from sandbox validation".into());
    }
    if head == attempt.base {
        command(
            runner,
            &checkout,
            &[
                "git".into(),
                "commit".into(),
                "-m".into(),
                format!("maintenance: {}", attempt.selection.finding_id),
            ],
        )?;
    }
    let commit = git(runner, &checkout, &["rev-parse", "HEAD"])?;
    let committed_tree = git(runner, &checkout, &["rev-parse", "HEAD^{tree}"])?;
    if Sha256::of(committed_tree.as_bytes()) != expected {
        return Err("committed PR tree differs from sandbox validation".into());
    }
    let existing: Vec<Value> = json::from_str(&gh(
        runner,
        source,
        &[
            "pr".into(),
            "list".into(),
            "--head".into(),
            branch.clone(),
            "--state".into(),
            "all".into(),
            "--json".into(),
            "url,headRefOid".into(),
        ],
    )?)?;
    if let Some(pr) = existing.first() {
        if existing.len() != 1 || pr["headRefOid"].as_str() != Some(commit.as_str()) {
            return Err("existing maintenance PR differs from validated commit".into());
        }
        return Ok(pr["url"].as_str().ok_or("PR URL missing")?.into());
    }
    let conflicts = open_pr_conflicts(runner, source)?;
    if attempt
        .selection
        .allowed_paths
        .iter()
        .any(|p| conflicts.contains(p))
    {
        return Err("an open PR now conflicts with this maintenance candidate".into());
    }
    // Fixed branch and head checks make a lost push/create reply recoverable.
    command(
        runner,
        &checkout,
        &[
            "git".into(),
            "push".into(),
            "origin".into(),
            format!("{commit}:refs/heads/{branch}"),
        ],
    )?;
    let repo: Value = json::from_str(&gh(
        runner,
        source,
        &[
            "repo".into(),
            "view".into(),
            "--json".into(),
            "defaultBranchRef".into(),
        ],
    )?)?;
    let base = repo["defaultBranchRef"]["name"]
        .as_str()
        .ok_or("default branch missing")?;
    gh(
        runner,
        source,
        &[
            "pr".into(),
            "create".into(),
            "--draft".into(),
            "--head".into(),
            branch,
            "--base".into(),
            base.into(),
            "--title".into(),
            format!("maintenance: {}", attempt.selection.finding_id),
            "--body-file".into(),
            bundle
                .join("PR.md")
                .to_str()
                .ok_or("non-UTF8 PR body")?
                .into(),
        ],
    )
}
