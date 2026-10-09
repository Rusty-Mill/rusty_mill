//! Removed server options must fail at argument parsing, before forge bootstrap.

use std::process::{Command, Stdio};

#[test]
fn obsolete_server_flags_are_rejected_before_bootstrap() {
    for args in [
        vec!["--transport", "http"],
        vec!["--transport", "stdio"],
        vec!["--transport=http"],
        vec!["--bind", "127.0.0.1:8080"],
        vec!["--bind=[::1]:8080"],
        vec!["--transport", "http", "--bind", "0.0.0.0:8080"],
    ] {
        let root = tempfile::tempdir().unwrap();
        // If dispatch reaches bootstrap, this path cannot be opened as a forge.
        // It also prevents the old HTTP implementation from starting a server.
        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"unchanged").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
            .current_dir(root.path())
            .arg("--forge-path")
            .arg(blocker.join("forge"))
            .args(["-v", "mcp", "serve"])
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains("unexpected argument"), "{args:?}: {stderr}");
        assert!(
            stderr.contains(args[0].split('=').next().unwrap()),
            "{args:?}: {stderr}"
        );
        assert!(output.stdout.is_empty(), "{args:?}: {:?}", output.stdout);
        assert!(!stderr.contains("failed to build runtime"), "{stderr}");
        assert_eq!(std::fs::read(&blocker).unwrap(), b"unchanged");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn server_help_advertises_stdio_without_transport_or_bind_options() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .current_dir(root.path())
        .arg("--forge-path")
        .arg(root.path().join("uncreated-forge"))
        .args(["mcp", "serve", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("stdio"), "{help}");
    assert!(!help.contains("--transport"), "{help}");
    assert!(!help.contains("--bind"), "{help}");
    assert!(!help.contains("HTTP"), "{help}");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
