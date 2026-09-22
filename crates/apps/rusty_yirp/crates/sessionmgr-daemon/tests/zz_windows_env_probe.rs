//! TEMPORARY diagnostic (not for merge): why `cmd /C ping` and
//! `cmd /C sort` report "not recognized" on the windows-latest runner
//! under nextest. Fails deliberately so nextest prints its output.

#![cfg(windows)]

use std::process::Command;

fn run(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(o) => format!(
            "{program} {args:?} -> status={:?}\n  stdout={:?}\n  stderr={:?}",
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => format!("{program} {args:?} -> spawn error: {e}"),
    }
}

#[test]
fn probe_the_child_process_environment() {
    let mut report = String::new();
    for var in ["PATH", "PATHEXT", "SystemRoot", "ComSpec", "windir", "OS"] {
        report.push_str(&format!("{var}={:?}\n", std::env::var(var)));
    }
    report.push_str(&run("cmd", &["/C", "echo from-cmd & set PATH & set PATHEXT & set COMSPEC"]));
    report.push('\n');
    report.push_str(&run("cmd", &["/C", "where ping"]));
    report.push('\n');
    report.push_str(&run("cmd", &["/C", "ping -n 1 127.0.0.1"]));
    report.push('\n');
    report.push_str(&run("cmd.exe", &["/C", "ping -n 1 127.0.0.1 >NUL"]));
    report.push('\n');
    report.push_str(&run(r"C:\Windows\System32\ping.exe", &["-n", "1", "127.0.0.1"]));
    report.push('\n');
    report.push_str(&run(r"C:\Windows\System32\cmd.exe", &["/C", r"C:\Windows\System32\ping.exe -n 1 127.0.0.1"]));
    report.push('\n');
    report.push_str(&run("cmd", &["/C", "dir C:\\Windows\\System32\\ping.exe C:\\Windows\\System32\\sort.exe"]));
    report.push('\n');
    report.push_str(&run("powershell", &["-NoProfile", "-Command", "Get-Command ping,sort | Format-List Name,Source; $env:PATH"]));
    panic!("WINDOWS ENV PROBE\n{report}");
}
