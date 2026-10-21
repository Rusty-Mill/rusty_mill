//! Exporting ownership must not open services, stores or credentials.
use rusty_mill_board::projects::Projects;
use std::fs;
use std::process::Command;
use uuid::Uuid;

#[test]
fn export_reuses_projects_validation_and_opens_no_board_store() {
    let dir = std::env::temp_dir().join(format!("mill-project-export-{}", Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("directory");
    let file = dir.join("projects.json");
    let registry = r#"{"repos":{"mill":{"orch":["crates/apps/orch"],"remind":["crates/apps/remind"]},"other":{"plugin":["plugin"]}}}"#;
    fs::write(&file, registry).expect("projects");
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_rusty_mill_board"))
            .arg("--export-projects")
            .arg("--projects")
            .arg(&file)
            .arg("--store")
            .arg(dir.join("must-not-exist/board.mmap"))
            .arg("--settings-dir")
            .arg(dir.join("must-not-exist"))
            .output()
            .expect("export process")
    };
    let output = invoke();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual =
        Projects::parse(std::str::from_utf8(&output.stdout).expect("utf8")).expect("registry");
    assert_eq!(actual, Projects::parse(registry).expect("expected"));
    assert!(!dir.join("must-not-exist").exists());
    fs::write(&file, r#"{"bad":["../escape"]}"#).expect("invalid projects");
    let invalid = invoke();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    fs::remove_dir_all(dir).expect("cleanup");
}
