//! Running the board as a service: one owner per store, backups that can be restored, a clean stop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusty_mill_board::api::Identity;
use rusty_mill_board::api::Shared;
use rusty_mill_board::backup::Backups;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Column, ExecutionLink};
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::projects::Projects;
use rusty_mill_board::settings::{Files, Settings, open_credentials};
use rusty_mill_board::store::{MemStore, Store};
use rusty_mill_board::syncer::Syncer;
use serde_json::json;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_service_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn board_at(path: &Path) -> Arc<Shared> {
    Arc::new(Board::new(
        Box::new(MmdbStore::open(path).expect("open")) as Box<dyn Store>,
        Roles::default(),
    ))
}

fn add_cards(board: &Shared, n: usize) {
    let human = board.roles().actor("nano", "human").expect("role");
    for i in 0..n {
        board
            .add(&human, &format!("card {i}"), "", vec![])
            .expect("add");
    }
}

fn backups_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .expect("read")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Wait until a settings change is waiting for the lock the caller holds.
fn wait_for_waiter(settings: &Settings) {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while settings.waiting() == 0 {
        assert!(
            std::time::Instant::now() < end,
            "no settings change reached the lock: it did not wait for the copy"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn no_settings(board: &Arc<Shared>) -> Settings {
    Settings::read_only(Arc::clone(board))
}

#[test]
fn identical_execution_binding_is_concurrent_and_restart_idempotent() {
    let dir = dir("execution_adopt");
    let path = dir.join("board.mmap");
    let board = board_at(&path);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "execute once", "", vec![]).expect("add");
    let link = ExecutionLink {
        execution: "execution-1".into(),
        goal: "goal-1".into(),
        bbp_task: "task-1".into(),
        attempt: "attempt-1".into(),
        assignment: "project-orch".into(),
        generation: 1,
    };
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let (board, barrier, link) = (Arc::clone(&board), Arc::clone(&barrier), link.clone());
        threads.push(std::thread::spawn(move || {
            let builder = board.roles().actor("nano", "human").expect("role");
            barrier.wait();
            board.bind_execution(card.id, link, &builder).expect("bind")
        }));
    }
    for thread in threads {
        let adopted = thread.join().expect("thread");
        assert_eq!(adopted.execution.as_ref(), Some(&link));
        assert_eq!(adopted.rev, 1);
    }
    assert_eq!(board.events(None, 100).expect("events").len(), 2);
    drop(board);

    let reopened = board_at(&path);
    let builder = reopened.roles().actor("nano", "human").expect("role");
    let adopted = reopened
        .bind_execution(card.id, link, &builder)
        .expect("adopt");
    assert_eq!(adopted.rev, 1);
    assert_eq!(reopened.events(None, 100).expect("events").len(), 2);
    drop(reopened);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_store_has_one_owner_until_it_is_dropped() {
    let dir = dir("owner");
    let path = dir.join("board.mmap");
    let first = MmdbStore::open(&path).expect("open");
    let second = MmdbStore::open(&path).err().expect("second open refused");
    assert!(second.to_string().contains("in use"), "{second}");
    drop(first);
    MmdbStore::open(&path).expect("free again");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_backup_opens_as_the_board_stood_and_old_ones_are_pruned() {
    let dir = dir("backup");
    let (path, out) = (dir.join("board.mmap"), dir.join("backups"));
    let board = board_at(&path);
    add_cards(&board, 3);
    let (backups, settings) = (Backups::new(out.clone(), 2), no_settings(&board));

    let mut taken = Vec::new();
    for _ in 0..3 {
        taken.push(backups.run(&board, &settings).expect("backup"));
        std::thread::sleep(Duration::from_millis(3));
    }
    let names = backups_in(&out);
    assert_eq!(names.len(), 2, "pruned to the newest two: {names:?}");
    assert!(!taken[0].exists() && taken[1].exists() && taken[2].exists());

    let restored = board_at(&taken[2].join("store").join("board.mmap"));
    assert_eq!(restored.list(None).expect("list").len(), 3);
    assert_eq!(
        restored.events(None, 100).expect("events"),
        board.events(None, 100).expect("events")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_backup_keeps_settings_that_share_a_file_name_with_each_other_and_with_the_store() {
    let dir = dir("collide");
    let (a, b, c, d) = (dir.join("a"), dir.join("b"), dir.join("c"), dir.join("d"));
    for d in [&a, &b, &c, &d] {
        std::fs::create_dir_all(d).expect("dir");
    }
    let board = board_at(&c.join("board.mmap"));
    add_cards(&board, 2);
    // Identities and roles are both `config.json`; the projects file is `board.mmap`, like the store.
    let roles = Roles::default();
    std::fs::write(
        b.join("config.json"),
        serde_json::to_string(&roles).expect("roles"),
    )
    .expect("roles file");
    std::fs::write(d.join("board.mmap"), "{}").expect("projects file");
    let (creds, _) =
        open_credentials(&a.join("config.json"), &roles, &Projects::default()).expect("creds");
    drop(creds);
    let files = Files {
        identities: Some(a.join("config.json")),
        roles: Some(b.join("config.json")),
        projects: Some(d.join("board.mmap")),
        github: None,
    };
    let settings = Settings::new(board.clone(), files, None, Syncer::new(board.clone()), None);

    let done = Backups::new(dir.join("backups"), 3)
        .run(&board, &settings)
        .expect("backup");

    let kept = |name: &str| std::fs::read(done.join("settings").join(name)).expect(name);
    assert_eq!(
        kept("identities.json"),
        std::fs::read(a.join("config.json")).expect("a")
    );
    assert_eq!(
        kept("roles.json"),
        std::fs::read(b.join("config.json")).expect("b")
    );
    assert_eq!(kept("projects.json"), b"{}");
    assert_eq!(
        backups_in(&done.join("store")),
        ["board.mmap", "board.mmap.inserts", "board.mmap.records"].map(String::from),
        "the store's files are not mixed with the settings"
    );
    assert_eq!(
        board_at(&done.join("store").join("board.mmap"))
            .list(None)
            .expect("list")
            .len(),
        2
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A change to the agents and the roles together must not be seen half-way by a backup: an agent
/// copied with a role the roles copy no longer has would not load on restore.
#[test]
fn a_backup_never_sees_a_settings_change_half_way() {
    let dir = dir("interleave");
    let roles_file = dir.join("roles.json");
    let roles = json!({
        "human": ["triage", "build", "qa", "review", "merge", "admin"],
        "old": ["build"],
        "new": ["qa"],
    });
    std::fs::write(&roles_file, roles.to_string()).expect("roles");
    let roles: Roles = serde_json::from_value(roles).expect("roles");
    let board: Arc<Shared> = Arc::new(Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        roles.clone(),
    ));
    let files = Files::in_dir(&dir);
    let (creds, token) = open_credentials(
        files.identities.as_deref().expect("path"),
        &roles,
        &Projects::default(),
    )
    .expect("creds");
    let creds = creds.shared();
    let admin: Identity = creds
        .read()
        .expect("lock")
        .identify(&token.expect("first admin"))
        .cloned()
        .expect("admin");
    let settings = Arc::new(Settings::new(
        board.clone(),
        files,
        Some(creds),
        Syncer::new(board),
        None,
    ));
    let call = |s: &Settings, op: &str, body| {
        s.call(op, Some(&admin), body)
            .expect("a settings operation")
            .expect("accepted");
    };
    call(
        &settings,
        "settings_agent_add",
        json!({"agent": {"name": "worker", "role": "old"}}),
    );

    let mut change = None;
    let out = dir.join("copy");
    std::fs::create_dir_all(&out).expect("dir");
    settings
        .snapshot_between(&out, &mut || Ok(()), &mut || {
            if change.is_some() {
                return;
            }
            // Mid-copy: the agents are copied; now move the agent to another role and drop its old one.
            let (s, admin, roles_file) = (Arc::clone(&settings), admin.clone(), roles_file.clone());
            change = Some(std::thread::spawn(move || {
                let run = |op: &str, body| {
                    s.call(op, Some(&admin), body)
                        .expect("a settings operation")
                        .expect("accepted");
                };
                run(
                    "settings_agent_update",
                    json!({"agent": {"name": "worker", "role": "new"}}),
                );
                let mut kept = serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(&roles_file).expect("roles"),
                )
                .expect("json");
                kept.as_object_mut().expect("map").remove("old");
                run("settings_roles_set", json!({"roles": kept}));
            }));
            wait_for_waiter(&settings);
            assert!(
                !change.as_ref().is_some_and(|c| c.is_finished()),
                "the change reached the lock and must wait for the copy, not slip in between its files"
            );
        })
        .expect("a consistent set that loads");
    change.expect("the change ran").join().expect("change");

    // The copy kept the old pairing; the live files have the new one; each loads on its own.
    let copied = std::fs::read_to_string(out.join("roles.json")).expect("copied roles");
    assert!(copied.contains("\"old\""), "{copied}");
    let again = dir.join("again");
    std::fs::create_dir_all(&again).expect("dir");
    settings.snapshot(&again).expect("the later set loads too");
    assert!(
        !std::fs::read_to_string(again.join("roles.json"))
            .expect("roles")
            .contains("\"old\"")
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The copy of the cards and the copy of the registry they belong to are one moment: a project
/// emptied and removed between the two copies would leave a card in a project the backup lacks.
#[test]
fn a_project_removed_between_the_two_copies_cannot_leave_a_card_without_its_project() {
    let dir = dir("race");
    let projects_file = dir.join("projects.json");
    std::fs::write(
        &projects_file,
        json!({"alpha": ["crates/apps/alpha"], "beta": ["crates/apps/beta"]}).to_string(),
    )
    .expect("projects");
    let roles = Roles::default();
    let projects = Projects::load(&projects_file).expect("projects");
    let board: Arc<Shared> = Arc::new(
        Board::new(
            Box::new(MmdbStore::open(&dir.join("board.mmap")).expect("open")) as Box<dyn Store>,
            roles.clone(),
        )
        .with_projects(projects.clone()),
    );
    let files = Files::in_dir(&dir);
    let (creds, token) = open_credentials(
        files.identities.as_deref().expect("path"),
        &roles,
        &projects,
    )
    .expect("creds");
    let creds = creds.shared();
    let admin: Identity = creds
        .read()
        .expect("lock")
        .identify(&token.expect("first admin"))
        .cloned()
        .expect("admin");
    let settings = Arc::new(Settings::new(
        board.clone(),
        files,
        Some(creds),
        Syncer::new(board.clone()),
        None,
    ));
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board
        .add_in(&human, "alpha", "t", "", vec![])
        .expect("card");

    let mut removal = None;
    let done = Backups::new(dir.join("backups"), 3)
        .run_with(&board, &settings, &mut || {
            // The store is copied (the card is in alpha). Now move the card out and try to drop alpha.
            board.assign(card.id, "beta", &human).expect("assign");
            let (s, admin) = (Arc::clone(&settings), admin.clone());
            removal = Some(std::thread::spawn(move || {
                s.call(
                    "settings_projects_set",
                    Some(&admin),
                    json!({"repos": {"default": {"beta": ["crates/apps/beta"]}}}),
                )
                .expect("a settings operation")
            }));
            wait_for_waiter(&settings);
        })
        .expect("the backup holds together");
    removal
        .expect("the removal ran")
        .join()
        .expect("removal")
        .expect("it is allowed once the backup is done");

    // The backup is the board as it stood: the card in alpha, with alpha in the registry.
    let copied =
        std::fs::read_to_string(done.join("settings").join("projects.json")).expect("projects");
    assert!(copied.contains("alpha"), "{copied}");
    let restored = board_at(&done.join("store").join("board.mmap"));
    assert_eq!(restored.get(card.id).expect("card").project, "alpha");
    // Live, the removal took effect after the backup.
    assert_eq!(board.get(card.id).expect("card").project, "beta");
    assert!(
        !std::fs::read_to_string(&projects_file)
            .expect("live")
            .contains("alpha")
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A backup is checked by opening it: cards against the copied registry.
#[test]
fn a_backup_whose_cards_do_not_fit_its_settings_is_refused_and_not_published() {
    let dir = dir("misfit");
    let projects_file = dir.join("projects.json");
    // The board serves alpha and beta; the file on disk (the copy's source) knows only beta.
    std::fs::write(
        &projects_file,
        json!({"beta": ["crates/apps/beta"]}).to_string(),
    )
    .expect("file");
    let served = Projects::parse(
        &json!({"alpha": ["crates/apps/alpha"], "beta": ["crates/apps/beta"]}).to_string(),
    )
    .expect("projects");
    let board: Arc<Shared> = Arc::new(
        Board::new(
            Box::new(MmdbStore::open(&dir.join("board.mmap")).expect("open")) as Box<dyn Store>,
            Roles::default(),
        )
        .with_projects(served),
    );
    let human = board.roles().actor("nano", "human").expect("role");
    board
        .add_in(&human, "alpha", "t", "", vec![])
        .expect("card");
    let files = Files {
        projects: Some(projects_file),
        ..Files::default()
    };
    let settings = Settings::new(board.clone(), files, None, Syncer::new(board.clone()), None);

    let out = dir.join("backups");
    let why = Backups::new(out.clone(), 3)
        .run(&board, &settings)
        .expect_err("a card without its project");
    assert!(why.contains("which the copied settings lack"), "{why}");
    assert!(
        backups_in(&out).is_empty(),
        "nothing published, no leftovers"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `--store` may be named with either companion suffix; the backup must still find the main file.
#[test]
fn a_store_named_with_a_companion_suffix_backs_up_and_restores() {
    for name in ["board.records", "board.inserts"] {
        let dir = dir(&format!("suffix_{}", name.replace('.', "_")));
        let board = board_at(&dir.join(name));
        add_cards(&board, 2);
        let done = Backups::new(dir.join("backups"), 1)
            .run(&board, &no_settings(&board))
            .unwrap_or_else(|why| panic!("{name}: {why}"));
        let mut copied = backups_in(&done.join("store"));
        copied.sort();
        assert_eq!(
            copied,
            [name, &format!("{name}.inserts"), &format!("{name}.records")].map(String::from),
            "{name}"
        );
        let restored = board_at(&done.join("store").join(name));
        assert_eq!(restored.list(None).expect("list").len(), 2, "{name}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// A store that has lost its main file must not become an empty backup that retires the last good one.
#[cfg(unix)]
#[test]
fn a_store_whose_main_file_is_gone_is_not_backed_up_and_the_last_good_copy_stays() {
    let dir = dir("gone");
    let (path, out) = (dir.join("board.mmap"), dir.join("backups"));
    let board = board_at(&path);
    add_cards(&board, 2);
    let settings = no_settings(&board);
    let backups = Backups::new(out.clone(), 1);
    let good = backups
        .run(&board, &settings)
        .expect("backup while healthy");
    let kept = [good
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned()];

    // The mapping outlives the file: the live store keeps answering with its main file removed.
    std::fs::remove_file(&path).expect("remove the live main file");
    assert_eq!(board.list(None).expect("still readable").len(), 2);
    let why = backups
        .run(&board, &settings)
        .expect_err("a store without its main file");
    assert!(why.contains("copy"), "{why}");
    assert_eq!(backups_in(&out), kept, "the previous backup stays");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_copy_that_loses_its_main_file_before_it_is_checked_is_not_published() {
    let dir = dir("lost");
    let (path, out) = (dir.join("board.mmap"), dir.join("backups"));
    let board = board_at(&path);
    add_cards(&board, 2);
    let settings = no_settings(&board);
    let backups = Backups::new(out.clone(), 1);
    let good = backups
        .run(&board, &settings)
        .expect("backup while healthy");
    let kept = [good
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned()];
    std::thread::sleep(Duration::from_millis(3));

    let why = backups
        .run_with(&board, &settings, &mut || {
            // After the store was copied, before the copy is opened: its main file goes missing.
            for entry in std::fs::read_dir(&out).expect("backups") {
                let entry = entry.expect("entry");
                if entry.file_name().to_string_lossy().starts_with(".board-") {
                    std::fs::remove_file(entry.path().join("store").join("board.mmap"))
                        .expect("remove the copied main file");
                }
            }
        })
        .expect_err("an empty store must not pass for a backup");
    assert!(why.contains("lacks the store's main file"), "{why}");
    assert_eq!(
        backups_in(&out),
        kept,
        "no empty backup, and the good one is not retired"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_backup_that_died_part_way_is_swept_and_never_counted_as_finished() {
    let dir = dir("sweep");
    let out = dir.join("backups");
    std::fs::create_dir_all(out.join(".board-0000000000001").join("store")).expect("leftover");
    let board = board_at(&dir.join("board.mmap"));
    let done = Backups::new(out.clone(), 1)
        .run(&board, &no_settings(&board))
        .expect("backup");
    assert_eq!(
        backups_in(&out),
        [done
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned()]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_store_serves_the_same_board_after_a_backup_compacts_it() {
    let dir = dir("compact");
    let path = dir.join("board.mmap");
    let board = board_at(&path);
    add_cards(&board, 4);
    let before = board.events(None, 100).expect("events");
    Backups::new(dir.join("backups"), 1)
        .run(&board, &no_settings(&board))
        .expect("backup");
    add_cards(&board, 1);
    drop(board);

    let reopened = board_at(&path);
    assert_eq!(reopened.list(None).expect("list").len(), 5);
    assert_eq!(reopened.events(None, 100).expect("events")[..4], before[..]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_board_that_cannot_back_up_leaves_no_partial_backup() {
    let dir = dir("memory");
    let board: Arc<Shared> = Arc::new(Board::new(
        Box::new(MemStore::default()) as Box<dyn Store>,
        Roles::default(),
    ));
    let why = Backups::new(dir.join("backups"), 3)
        .run(&board, &no_settings(&board))
        .expect_err("memory board");
    assert!(why.contains("not durable"), "{why}");
    assert!(backups_in(&dir.join("backups")).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// After an append of unknown outcome the log holds an event the state lacks. Neither a backup, its
/// rotation, nor a compaction may run before a reopen recovers it.
#[test]
fn nothing_is_rotated_or_compacted_away_after_an_append_of_unknown_outcome() {
    use rusty_multimodal_db_engine::test_support::{Fault, arm_fault};

    let dir = dir("unsettled");
    let (path, out) = (dir.join("board.mmap"), dir.join("backups"));
    let board = board_at(&path);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let (backups, settings) = (Backups::new(out.clone(), 1), no_settings(&board));
    let before = backups
        .run(&board, &settings)
        .expect("backup while healthy");

    arm_fault(Fault::InsertAfterLog);
    assert!(board.move_card(card.id, Column::Ready, &human).is_err());

    let refused = backups.run(&board, &settings).expect_err("unsettled");
    assert!(refused.contains("not taking a backup"), "{refused}");
    assert!(board.compact().is_err(), "compaction refused as well");
    assert_eq!(
        backups_in(&out),
        [before
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned()],
        "the copy from before the failure is still the only one: nothing rotated it out"
    );
    drop((board, settings));

    let reopened = board_at(&path);
    assert_eq!(
        reopened.get(card.id).expect("get").column,
        Column::Ready,
        "the durable event is recovered by the reopen"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The real binary as a child process: signals and file modes are Unix, so it is Unix-only.
#[cfg(unix)]
mod process {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::dir;

    fn get(port: u16, path: &str) -> Option<String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut out = String::new();
        stream.read_to_string(&mut out).ok()?;
        Some(out)
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port()
    }

    fn spawn(store: &Path, settings: &Path, port: u16) -> Child {
        Command::new(env!("CARGO_BIN_EXE_rusty_mill_board"))
            .arg("--store")
            .arg(store)
            .arg("--settings-dir")
            .arg(settings)
            .arg("--http")
            .arg(format!("127.0.0.1:{port}"))
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
    }

    fn wait_exit(child: &mut Child, within: Duration) -> Option<std::process::ExitStatus> {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if let Some(status) = child.try_wait().expect("wait") {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    #[test]
    fn the_server_reports_health_keeps_its_first_credential_off_stderr_and_stops_on_sigterm() {
        use std::os::unix::fs::PermissionsExt;

        let dir = dir("process");
        let (store, settings, port) = (dir.join("board.mmap"), dir.join("settings"), free_port());
        let mut server = spawn(&store, &settings, port);

        let up = Instant::now() + Duration::from_secs(20);
        let health = loop {
            match get(port, "/health") {
                Some(r) if r.starts_with("HTTP/1.1 200") => break r,
                _ if Instant::now() > up => panic!("never became healthy"),
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        assert!(health.contains(r#""ok":true"#), "{health}");

        // A second process on the same store is refused, and says why.
        let mut second = spawn(&store, &dir.join("settings2"), free_port());
        let status = wait_exit(&mut second, Duration::from_secs(20)).expect("second exits");
        let mut stderr = String::new();
        second
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut stderr)
            .expect("read");
        assert!(!status.success() && stderr.contains("in use"), "{stderr}");

        // The first admin's credential is in an owner-only file, not in the log.
        let file = settings.join("first-admin.token");
        let mode = std::fs::metadata(&file)
            .expect("credential file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        let killed = Command::new("kill")
            .args(["-TERM", &server.id().to_string()])
            .status()
            .expect("kill");
        assert!(killed.success());
        let status = wait_exit(&mut server, Duration::from_secs(10)).expect("exits on SIGTERM");
        assert!(status.success(), "{status:?}");

        let mut log = String::new();
        server
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut log)
            .expect("read");
        let secret = std::fs::read_to_string(&file).expect("read credential");
        assert!(
            !secret.trim().is_empty() && !log.contains(secret.trim()),
            "{log}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
