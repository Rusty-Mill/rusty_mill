//! Real Cargo workspaces through the installed command; inspection cannot
//! mutate tracked files, initialize state or invoke provider CLIs.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "orch-inspect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temporary workspace");
        Self(path)
    }
    fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        fs::write(path, text).expect("fixture");
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scan(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusty_orch"))
        .args(["inspect", "--repo"])
        .arg(path)
        // Goal-run environment must have no effect on inspection.
        .env("ORCH_BBP", "missing-config.json")
        .env("RUSTY_ORCH_STATE", path.join("must-not-exist"))
        .output()
        .expect("inspection process")
}

#[test]
fn cargo_workspace_discovery_reports_all_feature_edges_without_changes() {
    let workspace = Workspace::new();
    // stdin script mode must retain -I: target-repository modules must never
    // shadow either the stdlib or the compiled-in first-party helpers.
    for module in [
        "json",
        "affected_crates",
        "repository_intelligence",
        "sitecustomize",
    ] {
        workspace.write(
            &format!("{module}.py"),
            "raise RuntimeError('target code executed')\n",
        );
    }
    workspace.write("Cargo.toml", "[workspace]\nmembers = [\"crates/foundation/base\", \"crates/apps/app\"]\nresolver = \"2\"\n");
    workspace.write("crates/foundation/base/Cargo.toml", "[package]\nname = \"base\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"foundation\"\n");
    workspace.write("crates/foundation/base/src/lib.rs", "pub fn base() {}\n");
    workspace.write("crates/apps/app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"apps\"\n[dependencies]\nrenamed = { package = \"base\", path = \"../../foundation/base\", optional = true }\n[features]\nextra = [\"dep:renamed\"]\n");
    workspace.write(
        "crates/apps/app/src/lib.rs",
        "compile_error!(\"inspection must never compile workspace sources\");\n",
    );
    assert!(Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .status()
        .expect("lockfile")
        .success());
    let lock = fs::read(workspace.0.join("Cargo.lock")).expect("lock");
    let first = scan(&workspace.0);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: rusty_json::Value =
        rusty_json::Value::parse(std::str::from_utf8(&first.stdout).expect("utf8")).expect("JSON");
    let crates = report["crates"].as_array().expect("crates");
    assert_eq!(crates.len(), 2);
    let app = crates
        .iter()
        .find(|c| c["name"].as_str() == Some("app"))
        .expect("app");
    assert_eq!(app["dependencies"].as_array().expect("edges").len(), 1);
    assert!(report["findings"].as_array().expect("findings").is_empty());
    let second = scan(&workspace.0);
    assert!(
        second.status.success(),
        "repeated inspection failed ({}): {}",
        second.status,
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        lock,
        fs::read(workspace.0.join("Cargo.lock")).expect("unchanged lock")
    );
    assert!(!workspace.0.join("target").exists());
    assert!(!workspace.0.join("must-not-exist").exists());
    assert!(!workspace.0.join("__pycache__").exists());
}

#[test]
fn missing_lockfile_is_an_error_with_no_partial_report_or_writes() {
    let workspace = Workspace::new();
    workspace.write(
        "Cargo.toml",
        "[package]\nname = \"unlocked\"\nversion = \"0.1.0\"\n",
    );
    workspace.write("src/lib.rs", "");
    let output = scan(&workspace.0);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cargo metadata failed"));
    assert!(!workspace.0.join("Cargo.lock").exists());
}

#[test]
fn forbidden_build_edge_has_evidence_and_reverse_impact_in_real_workspace() {
    let workspace = Workspace::new();
    workspace.write("Cargo.toml", "[workspace]\nmembers = [\"crates/foundation/base\", \"crates/apps/app\", \"crates/tools/check\"]\nresolver = \"2\"\n");
    for (name, layer) in [("base", "foundation"), ("app", "apps"), ("check", "tools")] {
        let dependencies = match name {
            "base" => "[build-dependencies]\napp = { path = \"../../apps/app\" }\n",
            "check" => "[target.'cfg(unix)'.dev-dependencies]\nbase = { path = \"../../foundation/base\" }\n",
            _ => "",
        };
        workspace.write(&format!("crates/{layer}/{name}/Cargo.toml"), &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"{layer}\"\n{dependencies}"
        ));
        workspace.write(&format!("crates/{layer}/{name}/src/lib.rs"), "");
    }
    assert!(Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .status()
        .expect("lockfile")
        .success());
    let output = scan(&workspace.0);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = rusty_json::Value::parse(std::str::from_utf8(&output.stdout).expect("utf8"))
        .expect("report");
    let findings = report["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["rule"].as_str(), Some("dependency-layer"));
    assert_eq!(
        findings[0]["impact"]["crates_to_validate"]
            .as_array()
            .expect("impact")
            .len(),
        3
    );
    assert_eq!(
        findings[0]["evidence"][0]["declaration"]["kinds"][0]["kind"].as_str(),
        Some("build")
    );
}

fn git(workspace: &Workspace, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(&workspace.0)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn proposed_change_is_revision_bound_and_detects_requirement_edits() {
    let workspace = Workspace::new();
    let inputs = Workspace::new();
    workspace.write("Cargo.toml", "[workspace]\nmembers = [\"crates/foundation/atomic\", \"crates/apps/orch\", \"crates/apps/remind\", \"crates/apps/board\", \"crates/apps/other\"]\nresolver = \"2\"\n");
    for (name, directory, layer) in [
        ("rusty_atomic_file", "atomic", "foundation"),
        ("orch", "orch", "apps"),
        ("remind", "remind", "apps"),
        ("board", "board", "apps"),
        ("other", "other", "apps"),
    ] {
        let dependency = if name == "rusty_atomic_file" || name == "other" {
            ""
        } else {
            "[dependencies]\nrenamed = { package = \"rusty_atomic_file\", path = \"../../foundation/atomic\", optional = true }\n"
        };
        workspace.write(&format!("crates/{layer}/{directory}/Cargo.toml"), &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"{layer}\"\n{dependency}"));
        workspace.write(
            &format!("crates/{layer}/{directory}/src/lib.rs"),
            "compile_error!(\"inspection must never build sources\");\n",
        );
    }
    assert!(Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&workspace.0)
        .output()
        .expect("lockfile")
        .status
        .success());
    git(&workspace, &["init"]);
    git(&workspace, &["add", "."]);
    git(
        &workspace,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ],
    );
    inputs.write("projects.json", r#"{"repos":{"mill":{"orch":["crates/apps/orch"],"remindme":["crates/apps/remind"],"millboard":["crates/apps/board"],"unrelated":["crates/apps/other"],"inactive":["plugins/inactive"]}}}"#);
    let snapshot = |revision: &str| {
        format!(
            r#"{{"schema_version":1,"source":"remind_me","complete":true,"retrieved_context":[],"records":[{{"id":"durable","updated_at":"{revision}","content":"Retain crash-safe writes","status":"archived","metadata":{{"factory_requirement":{{"binding":true,"repository":"mill","projects":["inactive"],"packages":["rusty_atomic_file"],"validation":["inactive persistence suite"]}}}}}}]}}"#
        )
    };
    inputs.write("requirements.json", &snapshot("1"));
    inputs.write("request.json", r#"{"schema_version":1,"repository":"mill","changes":[{"package":"rusty_atomic_file"}],"projects":"projects.json","requirements":"requirements.json"}"#);
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_rusty_orch"))
            .args(["inspect", "--repo"])
            .arg(&workspace.0)
            .arg("--impact")
            .arg(inputs.0.join("request.json"))
            .env("ORCH_BBP", "missing-config.json")
            .env("RUSTY_ORCH_STATE", workspace.0.join("must-not-exist"))
            .output()
            .expect("impact process")
    };
    let first = invoke();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json = rusty_json::Value::parse(std::str::from_utf8(&first.stdout).expect("UTF8"))
        .expect("report");
    assert_eq!(
        first_json["source_revision"]
            .as_str()
            .expect("revision")
            .len(),
        40
    );
    let apps = first_json["affected_applications"]
        .as_array()
        .expect("apps");
    for app in ["orch", "remindme", "millboard", "inactive"] {
        assert!(
            apps.iter().any(|v| v.as_str() == Some(app)),
            "missing {app}"
        );
    }
    assert!(!first_json["validation"]["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .any(|v| v.as_str() == Some("unrelated")));
    assert_eq!(
        first_json["requirements"][0]["id"].as_str(),
        Some("durable")
    );
    assert_eq!(first.stdout, invoke().stdout);
    inputs.write(
        "previous.json",
        std::str::from_utf8(&first.stdout).expect("UTF8"),
    );
    inputs.write("request.json", r#"{"schema_version":1,"repository":"mill","changes":[{"package":"rusty_atomic_file"}],"projects":"projects.json","requirements":"requirements.json","previous":"previous.json"}"#);
    inputs.write("requirements.json", &snapshot("2"));
    let changed = invoke();
    assert!(
        changed.status.success(),
        "{}",
        String::from_utf8_lossy(&changed.stderr)
    );
    let report = rusty_json::Value::parse(std::str::from_utf8(&changed.stdout).expect("UTF8"))
        .expect("report");
    assert_eq!(report["changed_requirements"][0].as_str(), Some("durable"));
    assert!(!workspace.0.join("target").exists());
    assert!(!workspace.0.join("must-not-exist").exists());
    workspace.write("crates/apps/orch/src/lib.rs", "// uncommitted change\n");
    let dirty = invoke();
    assert!(!dirty.status.success());
    assert!(dirty.stdout.is_empty());
    assert!(String::from_utf8_lossy(&dirty.stderr).contains("clean checkout"));
}
