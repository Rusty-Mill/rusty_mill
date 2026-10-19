//! Running the board as a service: one owner per store, backups, a clean stop.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rusty_mill_board::api::Shared;
use rusty_mill_board::backup::Backups;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::mmdb::MmdbStore;
use rusty_mill_board::store::MemStore;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_service_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn board_at(path: &Path) -> Shared {
    Board::new(
        Box::new(MmdbStore::open(path).expect("open")),
        Roles::default(),
    )
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
    let settings = dir.join("identities.json");
    std::fs::write(&settings, "[]").expect("settings");
    let board = board_at(&path);
    add_cards(&board, 3);
    let backups = Backups::new(out.clone(), 2, vec![settings, dir.join("absent.json")]);

    let mut taken = Vec::new();
    for _ in 0..3 {
        taken.push(backups.run(&board).expect("backup"));
        std::thread::sleep(Duration::from_millis(3));
    }
    let names = backups_in(&out);
    assert_eq!(names.len(), 2, "pruned to the newest two: {names:?}");
    assert!(!taken[0].exists() && taken[1].exists() && taken[2].exists());

    let copy = taken[2].join("board.mmap");
    assert!(taken[2].join("identities.json").is_file());
    assert!(!taken[2].join("absent.json").exists());
    let restored = board_at(&copy);
    assert_eq!(restored.list(None).expect("list").len(), 3);
    assert_eq!(
        restored.events(None, 100).expect("events"),
        board.events(None, 100).expect("events")
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
    Backups::new(dir.join("backups"), 1, vec![])
        .run(&board)
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
    let board: Shared = Board::new(Box::new(MemStore::default()), Roles::default());
    let why = Backups::new(dir.clone(), 3, vec![])
        .run(&board)
        .expect_err("memory board");
    assert!(why.contains("not durable"), "{why}");
    assert!(backups_in(&dir).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

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

#[cfg(unix)]
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
