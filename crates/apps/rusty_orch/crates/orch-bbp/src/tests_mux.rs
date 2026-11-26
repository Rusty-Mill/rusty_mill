//! #1318 S4: the mux adapter against a fake `rusty-mux-ctl` script: the
//! session and window lookup, exact `own` request lines, answers and
//! events, the passed descriptors, and a lost connection mid-launch.

use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusty_json::Value;

use crate::mux::{self, Ctl, Found, Launch, Line, MuxError, Own};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("orch-mux-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("dir");
    d
}

/// A fake `rusty-mux-ctl`: logs its argv to `calls`, then runs `body`
/// (sh), where `$LOG` is the directory and the args are `$@`.
fn fake(d: &Path, body: &str) -> Ctl {
    let bin = d.join("rusty-mux-ctl");
    let script = format!(
        "#!/bin/sh\nLOG='{}'\necho \"$*\" >> \"$LOG/calls\"\n{body}\n",
        d.display()
    );
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    Ctl {
        bin,
        socket: Some(d.join("mux.sock")),
    }
}

/// `Own::spawn`, retried while the fake script is "text file busy": a
/// fork in a parallel test can hold its just-written descriptor until that
/// child execs (ETXTBSY), which is a test-harness race, not the adapter's.
fn spawn_own(ctl: &Ctl, passed: &[std::os::fd::BorrowedFd<'_>]) -> Own {
    for _ in 0..100 {
        match Own::spawn(ctl, passed) {
            Err(MuxError::Ctl(e)) if e.contains("busy") => {
                std::thread::sleep(Duration::from_millis(20));
            }
            other => return other.expect("spawn"),
        }
    }
    panic!("the fake ctl stayed busy");
}

fn calls(d: &Path) -> Vec<String> {
    std::fs::read_to_string(d.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

const LIST: &str = r#"
case "$*" in
  *" list") cat "$LOG/list" 2>/dev/null || true ;;
  *create-session*) echo '{"method":"create-session","ok":true,"created":"S-new"}' ;;
  *create-window*) echo '{"method":"create-window","ok":true,"created":"W-new"}' ;;
esac
"#;

#[test]
fn the_window_is_found_or_made() {
    let d = dir("window");
    let ctl = fake(&d, LIST);
    let sock = d.join("mux.sock").display().to_string();

    // Nothing yet: a session, then a window in it.
    assert_eq!(ctl.window("lel-dedup", "builders"), Ok("W-new".into()));
    assert_eq!(
        calls(&d),
        [
            format!("--socket {sock} --json list"),
            format!("--socket {sock} --json create-session --name lel-dedup"),
            format!("--socket {sock} --json create-window --session S-new --name builders"),
        ]
    );

    // The session exists: only the window is made, in it.
    std::fs::write(
        d.join("list"),
        concat!(
            r#"{"kind":"session","session":"S1","name":"other"}"#,
            "\n",
            r#"{"kind":"session","session":"S2","name":"lel-dedup"}"#,
            "\n",
            r#"{"kind":"window","window":"W9","session":"S1","name":"builders"}"#,
            "\n",
        ),
    )
    .expect("list");
    std::fs::remove_file(d.join("calls")).expect("reset");
    assert_eq!(ctl.window("lel-dedup", "builders"), Ok("W-new".into()));
    assert_eq!(
        calls(&d)[1],
        format!("--socket {sock} --json create-window --session S2 --name builders")
    );

    // Both exist: nothing is made.
    std::fs::write(
        d.join("list"),
        concat!(
            r#"{"kind":"session","session":"S2","name":"lel-dedup"}"#,
            "\n",
            r#"{"kind":"window","window":"W2","session":"S2","name":"builders"}"#,
            "\n",
        ),
    )
    .expect("list");
    std::fs::remove_file(d.join("calls")).expect("reset");
    assert_eq!(ctl.window("lel-dedup", "builders"), Ok("W2".into()));
    assert_eq!(calls(&d).len(), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failed_or_unreadable_ctl_call_is_an_error() {
    let d = dir("ctl-fail");
    let ctl = fake(&d, "echo 'no server' >&2; exit 3");
    assert!(matches!(ctl.window("s", "w"), Err(MuxError::Ctl(e)) if e.contains("no server")));
    assert_eq!(
        mux::find(&["not json".into()], "s", "w").map_err(|_| ()),
        Err(())
    );
    assert_eq!(mux::find(&[], "s", "w"), Ok(Found::Nothing));
    let _ = std::fs::remove_dir_all(&d);
}

fn builder_launch() -> Launch {
    Launch {
        window: "W2".into(),
        program: "lel-jail".into(),
        args: vec!["/runs/7/1/jail.json".into()],
        cwd: Some("/wt/7/1".into()),
        env: Some(mux::exact_env(|k| match k {
            "PATH" => Some("/usr/bin".into()),
            "HOME" => Some("/home/nano".into()),
            "RUSTY_MUX_SOCKET" => Some("/leak".into()),
            _ => None,
        })),
        fds: 2,
        hold: true,
        owner_only: true,
        meta: Some(["claude-builder".into(), "builder".into(), "C1/7/1".into()]),
    }
}

#[test]
fn the_launch_line_is_exact() {
    let line = Value::parse(&mux::launch_line(1, &builder_launch())).expect("json");
    assert_eq!(
        line,
        Value::parse(
            r#"{"window":"W2","program":"lel-jail","args":["/runs/7/1/jail.json"],
                "cwd":"/wt/7/1","env":{"PATH":"/usr/bin","HOME":"/home/nano"},"fds":2,
                "hold":true,"input":"owner-only","agent":"claude-builder","role":"builder",
                "card":"C1/7/1","id":1,"op":"launch"}"#
        )
        .expect("expected")
    );
    let viewer = Value::parse(&mux::launch_line(
        2,
        &mux::viewer("W2", "/runs/7/1/acp.log"),
    ))
    .expect("json");
    assert_eq!(viewer["program"], Value::from("tail"));
    assert_eq!(viewer["args"][1], Value::from("/runs/7/1/acp.log"));
    assert_eq!(viewer.get("fds"), None, "the viewer gets no descriptors");
    assert_eq!(viewer["hold"], Value::Bool(true));
}

/// A fake `own`: records each request line and its own descriptors, and
/// answers by op. `launch` gets a block; `signal` gets a program-exited
/// event before its answer; `refuse` ops are refused.
const OWN: &str = r#"
ls /proc/$$/fd | tr '\n' ' ' > "$LOG/fds"
for fd in 3 4; do readlink /proc/$$/fd/$fd >> "$LOG/targets"; done
while IFS= read -r line; do
  echo "$line" >> "$LOG/requests"
  id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
  case "$line" in
    *'"op":"launch"'*) echo "{\"id\":$id,\"ok\":true,\"block\":\"B$id\"}" ;;
    *'"op":"signal"'*) echo '{"event":"program-exited","block":"B1","code":0}'
                       echo "{\"id\":$id,\"ok\":true}" ;;
    *'"op":"close"'*) echo "{\"id\":$id,\"ok\":false,\"error\":\"not owned\"}" ;;
    *) echo "{\"id\":$id,\"ok\":true}" ;;
  esac
done
"#;

#[test]
fn own_launches_signals_and_reports_events() {
    let d = dir("own");
    let ctl = fake(&d, OWN);
    let slot = std::fs::File::create(d.join("slot.lock")).expect("slot");
    let acp = std::fs::File::create(d.join("acp-end")).expect("acp");
    let mut own =
        spawn_own(&ctl, &[slot.as_fd(), acp.as_fd()]).answer_wait(Duration::from_secs(10));
    assert_eq!(own.launch(&builder_launch()), Ok("B1".into()));
    assert_eq!(
        own.launch(&mux::viewer("W2", "/x/acp.log")),
        Ok("B2".into())
    );
    assert_eq!(own.signal("B1", "term"), Ok(()));
    // The event that came before the signal's answer is kept, not lost.
    assert!(matches!(
        own.next_event(Duration::from_secs(1)),
        Ok(Some(Line::Event { kind, .. })) if kind == "program-exited"
    ));
    assert_eq!(own.next_event(Duration::from_millis(50)), Ok(None));
    assert_eq!(own.release("B1"), Ok(()));
    assert_eq!(own.close("B1"), Err(MuxError::Refused("not owned".into())));
    own.finish();

    let sock = d.join("mux.sock").display().to_string();
    assert_eq!(calls(&d), [format!("--socket {sock} --json own")]);
    let requests = std::fs::read_to_string(d.join("requests")).expect("requests");
    let ops: Vec<String> = requests
        .lines()
        .map(|l| {
            Value::parse(l).expect("json")["op"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    assert_eq!(ops, ["launch", "launch", "signal", "release", "close"]);
    assert_eq!(
        requests.lines().next().map(str::to_owned),
        Some(mux::launch_line(1, &builder_launch()))
    );
    // own got the slot at 3 and the ACP end at 4, and nothing else above 2
    // but the descriptor `ls` itself opens to list them.
    let targets = std::fs::read_to_string(d.join("targets")).expect("targets");
    let targets: Vec<&str> = targets.lines().collect();
    assert_eq!(
        targets,
        [
            d.join("slot.lock").display().to_string(),
            d.join("acp-end").display().to_string()
        ]
    );
    let fds = std::fs::read_to_string(d.join("fds")).expect("fds");
    let mut fds: Vec<i32> = fds
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    fds.sort_unstable();
    assert_eq!(&fds[..5], [0, 1, 2, 3, 4]);
    assert!(fds.len() <= 6, "{fds:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_lost_connection_mid_launch_is_lost_not_failed() {
    let d = dir("lost");
    let ctl = fake(&d, r#"read -r line; echo '{"event":"lost"}'; exit 4"#);
    let mut own = spawn_own(&ctl, &[]).answer_wait(Duration::from_secs(10));
    assert!(matches!(
        own.launch(&builder_launch()),
        Err(MuxError::Lost(_))
    ));
    own.finish();

    // `own` ending without a word is lost too.
    let ctl = fake(&d, "read -r line; exit 4");
    let mut own = spawn_own(&ctl, &[]).answer_wait(Duration::from_secs(10));
    assert!(matches!(
        own.launch(&builder_launch()),
        Err(MuxError::Lost(_))
    ));
    assert!(matches!(
        own.next_event(Duration::from_millis(10)),
        Err(MuxError::Lost(_))
    ));
    own.finish();

    // An `own` that never answers is lost after the wait.
    let ctl = fake(&d, "read -r line; sleep 30");
    let mut own = spawn_own(&ctl, &[]).answer_wait(Duration::from_millis(300));
    assert!(
        matches!(own.launch(&builder_launch()), Err(MuxError::Lost(e)) if e.contains("no answer"))
    );
    own.finish();
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn own_lines_parse_into_answers_and_events() {
    assert_eq!(
        mux::parse_line(r#"{"id":3,"ok":false,"error":"no window"}"#),
        Ok(Line::Answer {
            id: 3,
            outcome: Err("no window".into())
        })
    );
    assert!(matches!(
        mux::parse_line(r#"{"event":"block-closed","block":"B1"}"#),
        Ok(Line::Event { kind, .. }) if kind == "block-closed"
    ));
    assert!(mux::parse_line("{}").is_err());
    assert!(mux::parse_line("garbage").is_err());
}
