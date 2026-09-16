//! RFC 0009 row 5 — crash-recovery contract for in-flight editor edits.
//!
//! `nexus_forge` carried a per-buffer write-ahead log so that a hard
//! crash lost at most one keystroke. The RFC's rule for porting that
//! design was "write the unsaved-edits-after-crash test first; port
//! only if it fails". This file is that test, driven through the same
//! kernel IPC path the shell uses (`open` → `apply_transaction` →
//! process death → `open` again).
//!
//! # Harness
//!
//! The crash is a real one. Each scenario re-executes this test binary
//! as a child process (`--exact crash_child`) with the forge root in an
//! environment variable; the child boots a runtime, opens the note,
//! applies an edit, optionally saves, and then calls
//! `std::process::abort()`. No `close`, no `save`, no shutdown hooks —
//! the kernel releases the `flock` forge lock because the process is
//! gone, exactly as after a real crash. (Dropping a `Runtime` in-process
//! does *not* release the lock: the storage watcher-bridge threads keep
//! the engine alive, so an in-process "crash" cannot be modelled.)
//!
//! * [`saved_edits_survive_a_hard_crash`] is the control: it proves the
//!   harness models a crash and pins the baseline that `save`d content
//!   is durable.
//! * [`unsaved_edits_survive_a_hard_crash`] is the RFC question and the
//!   acceptance test for the crash journal (`nexus-editor/src/journal.rs`):
//!   an applied-but-unsaved edit comes back on the next `open`, flagged
//!   `recovered_unsaved_edits`, and the file on disk stays untouched
//!   until the user saves.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use nexus_bootstrap::{build_cli_runtime, Runtime};
use nexus_editor::{EditorSnapshot, Operation, Transaction, TransactionMetadata, EDITOR_PLUGIN_ID};
use nexus_kernel::{Ipc as _, IpcError};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const NOTE: &str = "note.md";
const ENV_ROOT: &str = "NEXUS_CRASH_TEST_ROOT";
const ENV_SAVE: &str = "NEXUS_CRASH_TEST_SAVE";

fn scratch_forge() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    nexus_storage::StorageEngine::init(dir.path()).expect("init scratch forge");
    dir
}

fn write_note(root: &Path, relpath: &str, body: &str) {
    let abs = root.join(relpath);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(abs, body).unwrap();
}

async fn call(
    runtime: &Runtime,
    command: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    runtime
        .context
        .ipc_call(EDITOR_PLUGIN_ID, command, args, CALL_TIMEOUT)
        .await
}

async fn markdown(runtime: &Runtime) -> String {
    let v = call(
        runtime,
        "get_markdown",
        serde_json::json!({ "relpath": NOTE }),
    )
    .await
    .expect("get_markdown ok");
    v.as_str().expect("markdown string").to_string()
}

/// Open `NOTE`, append `text` to its first paragraph, and return the
/// session's current markdown. Deliberately does **not** save.
async fn open_and_edit(runtime: &Runtime, text: &str) -> String {
    let snap: EditorSnapshot = serde_json::from_value(
        call(runtime, "open", serde_json::json!({ "relpath": NOTE }))
            .await
            .expect("open ok"),
    )
    .unwrap();
    let para = snap.tree.root_blocks[0];
    let pos = snap.tree.blocks[&para].content.len();
    let tx = Transaction::new(
        vec![Operation::InsertText {
            block_id: para,
            pos,
            text: text.into(),
            pre_annotations: Vec::new(),
        }],
        TransactionMetadata::default(),
    );
    call(
        runtime,
        "apply_transaction",
        serde_json::json!({ "relpath": NOTE, "transaction": serde_json::to_value(&tx).unwrap() }),
    )
    .await
    .expect("apply_transaction ok");
    markdown(runtime).await
}

/// The child half. Only does anything when spawned by a parent scenario
/// (env var set); a plain `cargo test` run returns immediately.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_child() {
    let Ok(root) = std::env::var(ENV_ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let runtime = build_cli_runtime(root.clone()).expect("child: build runtime");
    assert_eq!(open_and_edit(&runtime, " world").await, "Hello world\n");
    if std::env::var_os(ENV_SAVE).is_some() {
        call(&runtime, "save", serde_json::json!({ "relpath": NOTE }))
            .await
            .expect("child: save ok");
    }
    // Hard crash: no close, no save, no shutdown hooks, no Drop.
    std::process::abort();
}

/// Spawn the child scenario against `root` and wait for it to die.
fn run_child_and_crash(root: &Path, save_first: bool) {
    let mut cmd = Command::new(std::env::current_exe().expect("current exe"));
    cmd.args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if save_first {
        cmd.env(ENV_SAVE, "1");
    }
    let status = cmd.status().expect("spawn child test process");
    // An abort is reported as a signal on Unix (`code() == None`); the
    // harness would otherwise exit 0 (test passed) or 101 (test failed).
    // Windows has no signal-based termination — `std::process::abort()`
    // there raises a fail-fast exception and the process exits with the
    // fixed code `STATUS_STACK_BUFFER_OVERRUN` (0xC0000409), which
    // `Command::status()` reports as a normal (non-`None`) exit code.
    #[cfg(unix)]
    assert_eq!(
        status.code(),
        None,
        "child must die by abort, not finish the test harness normally: {status}"
    );
    #[cfg(windows)]
    assert_eq!(
        status.code(),
        Some(0xC0000409_u32 as i32),
        "child must die by abort (fail-fast), not finish the test harness normally: {status}"
    );
}

/// Reopen on a fresh runtime; returns `(markdown, recovered_unsaved_edits)`.
async fn reopen(root: &Path) -> (String, bool) {
    let runtime = build_cli_runtime(root.to_path_buf()).expect("rebuild runtime after crash");
    let snap: EditorSnapshot = serde_json::from_value(
        call(&runtime, "open", serde_json::json!({ "relpath": NOTE }))
            .await
            .expect("open after crash ok"),
    )
    .unwrap();
    (markdown(&runtime).await, snap.recovered_unsaved_edits)
}

fn journal_dir_has_entries(root: &Path) -> bool {
    fs::read_dir(root.join(nexus_editor::journal::JOURNAL_DIR_REL))
        .map(|d| {
            d.filter_map(Result::ok)
                .any(|e| e.path().extension().is_some_and(|x| x == "json"))
        })
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_edits_survive_a_hard_crash() {
    let forge = scratch_forge();
    let root = forge.path().to_path_buf();
    write_note(&root, NOTE, "Hello\n");

    run_child_and_crash(&root, true);

    assert_eq!(
        fs::read_to_string(root.join(NOTE)).unwrap(),
        "Hello world\n"
    );
    assert!(
        !journal_dir_has_entries(&root),
        "save must clear the journal so nothing is replayed on reopen"
    );
    let (markdown, recovered) = reopen(&root).await;
    assert_eq!(markdown, "Hello world\n");
    assert!(!recovered);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsaved_edits_survive_a_hard_crash() {
    let forge = scratch_forge();
    let root = forge.path().to_path_buf();
    write_note(&root, NOTE, "Hello\n");

    run_child_and_crash(&root, false);

    // The edit was applied in the kernel session but never saved: disk
    // still holds the pre-edit bytes, and the journal holds the edit.
    assert_eq!(fs::read_to_string(root.join(NOTE)).unwrap(), "Hello\n");
    assert!(
        journal_dir_has_entries(&root),
        "crash left a journal behind"
    );

    let (markdown, recovered) = reopen(&root).await;
    assert_eq!(
        markdown, "Hello world\n",
        "an applied-but-unsaved edit must be recoverable after a hard crash"
    );
    assert!(
        recovered,
        "open must flag the recovery so the tab opens dirty"
    );
    // File-as-truth: recovery lives in the session until the user saves.
    assert_eq!(fs::read_to_string(root.join(NOTE)).unwrap(), "Hello\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn journal_is_ignored_when_the_file_changed_on_disk() {
    let forge = scratch_forge();
    let root = forge.path().to_path_buf();
    write_note(&root, NOTE, "Hello\n");

    run_child_and_crash(&root, false);
    assert!(journal_dir_has_entries(&root));

    // An external editor rewrote the note between crash and relaunch.
    write_note(&root, NOTE, "Rewritten elsewhere\n");

    let (markdown, recovered) = reopen(&root).await;
    assert_eq!(
        markdown, "Rewritten elsewhere\n",
        "disk wins over a stale journal"
    );
    assert!(!recovered);
    assert!(
        !journal_dir_has_entries(&root),
        "stale journal is discarded"
    );
}
