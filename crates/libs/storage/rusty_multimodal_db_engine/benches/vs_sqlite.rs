//! Design review Tranche 5: the engine against SQLite, the workspace's two
//! storage stacks, at matched durability.
//!
//! `rusty_tick` and `remind_me` store records in [`GenericMmapStore`];
//! nexus, meshed, inventory and the rest use SQLite (bundled, WAL mode).
//! Each runs the same task-shaped workload: `n` inserts, `n` whole-record
//! replaces, `n` gets by id, then two reopens. Three durability levels, set
//! so both stores make the same promise when a write returns:
//!
//! | Level     | Promise                  | Engine                           | SQLite (WAL)                      |
//! |-----------|--------------------------|----------------------------------|-----------------------------------|
//! | `sync`    | on disk                  | default: log fsync per write     | `synchronous=FULL`, autocommit    |
//! | `group`   | on disk every `GROUP`    | `defer_sync`, `commit` per group | `FULL`, one transaction per group |
//! | `no-sync` | survives a process crash | `defer_sync`, never committed    | `synchronous=OFF`, autocommit     |
//!
//! The engine keeps every record in memory and indexes by list; SQLite
//! reads pages on demand, with an index on `list_id` to match. Prints
//! microseconds per operation, two reopen times (the first folds the
//! writes since the last open) and bytes on disk, at two sizes.
//!
//! `cargo bench -p rusty_multimodal_db_engine --bench vs_sqlite`

use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::{params, Connection};
use rusty_multimodal_db_engine::generic::query::GetById;
use rusty_multimodal_db_engine::generic::store::GroupCommit;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::GenericMmapStore;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SIZES: [u64; 2] = [10_000, 100_000];
const GROUP: u64 = 100;
const LISTS: u64 = 20;

/// A `rusty_tick`-shaped task: about 150 bytes encoded.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Task {
    id: Uuid,
    list_id: Uuid,
    title: String,
    notes: String,
    sort_order: i64,
    due_ms: i64,
}

impl Record for Task {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for Task {
    const SCHEMA_TAG: &'static str = "bench::vs_sqlite::Task@1";
}

struct ByList;
impl IndexedField<ByList> for Task {
    type IndexValue = Uuid;
    fn indexed_value(&self) -> &Uuid {
        &self.list_id
    }
}

struct SortOrder;
impl ScannableField<SortOrder> for Task {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.sort_order
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.sort_order = value;
    }
}

type Store = GenericMmapStore<Task, ByList, SortOrder>;

fn task(i: u64, version: u64) -> Task {
    Task {
        id: id(i),
        list_id: Uuid::from_u128(u128::from(i % LISTS) + 1_000_000),
        title: format!("task {i} revision {version}: buy groceries"),
        notes: format!("{version}: milk, eggs, bread, coffee, apples and a newspaper"),
        sort_order: (i * 1_000 + version) as i64,
        due_ms: 1_790_000_000_000 + (i * 60_000) as i64,
    }
}

fn id(i: u64) -> Uuid {
    Uuid::from_u128(u128::from(i) + 1)
}

/// Visits `0..n` in a scattered but deterministic order: the stride is
/// prime and coprime to every size here.
fn scattered(i: u64, n: u64) -> u64 {
    (i * 7_919) % n
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Sync,
    Group,
    NoSync,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Sync => "sync",
            Level::Group => "group",
            Level::NoSync => "no-sync",
        }
    }
}

/// One store's run of the workload at one level and size.
type Run = fn(&Path, Level, u64) -> Row;

struct Row {
    insert_us: f64,
    replace_us: f64,
    get_us: f64,
    /// The first reopen, which folds the writes since the last open.
    reopen_ms: f64,
    /// A second reopen, with nothing left to fold.
    again_ms: f64,
    bytes: u64,
}

fn per_op(start: Instant, n: u64) -> f64 {
    start.elapsed().as_secs_f64() * 1e6 / n as f64
}

fn millis(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

fn engine(dir: &Path, level: Level, n: u64) -> Row {
    let path = dir.join("tasks");
    let mut store = Store::create(Vec::new(), &path).expect("create");
    let start = Instant::now();
    engine_writes(&mut store, level, n, |store, i| {
        store.insert(task(i, 0)).expect("insert")
    });
    let insert_us = per_op(start, n);
    let start = Instant::now();
    engine_writes(&mut store, level, n, |store, i| {
        store.replace(task(scattered(i, n), 1)).expect("replace")
    });
    let replace_us = per_op(start, n);
    let start = Instant::now();
    for i in 0..n {
        std::hint::black_box(store.get(id(scattered(i, n))).expect("present"));
    }
    let get_us = per_op(start, n);
    drop(store);
    let reopen = || {
        let start = Instant::now();
        let store = Store::open_portable(&path).expect("reopen");
        std::hint::black_box(store.get(id(0)).expect("present"));
        millis(start)
    };
    let reopen_ms = reopen();
    let again_ms = reopen();
    Row {
        insert_us,
        replace_us,
        get_us,
        reopen_ms,
        again_ms,
        bytes: dir_bytes(dir),
    }
}

/// Runs `n` writes under `level`'s sync policy. A `no-sync` store is never
/// committed: dropping it leaves its writes to the OS, as SQLite's
/// `synchronous=OFF` does.
fn engine_writes(store: &mut Store, level: Level, n: u64, mut write: impl FnMut(&mut Store, u64)) {
    if level != Level::Sync {
        store.defer_sync();
    }
    for i in 0..n {
        write(store, i);
        if level == Level::Group && (i + 1) % GROUP == 0 {
            store.commit().expect("commit");
            store.defer_sync();
        }
    }
    if level == Level::Group {
        store.commit().expect("commit");
    }
}

const GET: &str = "SELECT * FROM tasks WHERE id = ?1";

fn sqlite(dir: &Path, level: Level, n: u64) -> Row {
    let path = dir.join("tasks.db");
    let conn = open_sqlite(&path, level);
    conn.execute_batch(
        "CREATE TABLE tasks (id BLOB PRIMARY KEY, list_id BLOB NOT NULL, title TEXT NOT NULL,
             notes TEXT NOT NULL, sort_order INTEGER NOT NULL, due_ms INTEGER NOT NULL);
         CREATE INDEX tasks_by_list ON tasks (list_id);",
    )
    .expect("schema");
    let insert = "INSERT INTO tasks VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
    let start = Instant::now();
    sqlite_writes(&conn, level, n, insert, |i| task(i, 0));
    let insert_us = per_op(start, n);
    let replace = "UPDATE tasks SET list_id = ?2, title = ?3, notes = ?4, sort_order = ?5, \
                   due_ms = ?6 WHERE id = ?1";
    let start = Instant::now();
    sqlite_writes(&conn, level, n, replace, |i| task(scattered(i, n), 1));
    let replace_us = per_op(start, n);
    let start = Instant::now();
    {
        let mut get = conn.prepare_cached(GET).expect("prepare");
        for i in 0..n {
            let row = get.query_row([id(scattered(i, n)).as_bytes()], read_task);
            std::hint::black_box(row.expect("present"));
        }
    }
    let get_us = per_op(start, n);
    drop(conn);
    let reopen = || {
        let start = Instant::now();
        let conn = open_sqlite(&path, level);
        let row = conn.query_row(GET, [id(0).as_bytes()], read_task);
        std::hint::black_box(row.expect("present"));
        millis(start)
    };
    let reopen_ms = reopen();
    let again_ms = reopen();
    Row {
        insert_us,
        replace_us,
        get_us,
        reopen_ms,
        again_ms,
        bytes: dir_bytes(dir),
    }
}

fn open_sqlite(path: &Path, level: Level) -> Connection {
    let conn = Connection::open(path).expect("open");
    let synchronous = if level == Level::NoSync {
        "OFF"
    } else {
        "FULL"
    };
    let pragmas = format!("PRAGMA journal_mode = WAL; PRAGMA synchronous = {synchronous};");
    conn.execute_batch(&pragmas).expect("pragmas");
    conn
}

/// Runs `n` writes of `sql` under `level`: autocommit, or one transaction
/// per `GROUP`.
fn sqlite_writes(conn: &Connection, level: Level, n: u64, sql: &str, record: impl Fn(u64) -> Task) {
    let mut statement = conn.prepare_cached(sql).expect("prepare");
    for i in 0..n {
        if level == Level::Group && i % GROUP == 0 {
            conn.execute_batch("BEGIN").expect("begin");
        }
        let t = record(i);
        let values = params![
            t.id.as_bytes(),
            t.list_id.as_bytes(),
            t.title,
            t.notes,
            t.sort_order,
            t.due_ms
        ];
        statement.execute(values).expect("write");
        if level == Level::Group && ((i + 1) % GROUP == 0 || i + 1 == n) {
            conn.execute_batch("COMMIT").expect("commit");
        }
    }
}

fn read_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let uuid = |bytes: Vec<u8>| Uuid::from_slice(&bytes).unwrap_or_default();
    Ok(Task {
        id: uuid(row.get(0)?),
        list_id: uuid(row.get(1)?),
        title: row.get(2)?,
        notes: row.get(3)?,
        sort_order: row.get(4)?,
        due_ms: row.get(5)?,
    })
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.metadata().ok())
                .map(|metadata| metadata.len())
                .sum()
        })
        .unwrap_or(0)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vs_sqlite_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn main() {
    println!("task records over {LISTS} lists, groups of {GROUP}; µs per operation");
    println!(
        "{:>7} {:<8} {:<7} {:>9} {:>9} {:>7} {:>10} {:>9} {:>9}",
        "records",
        "level",
        "store",
        "insert",
        "replace",
        "get",
        "reopen ms",
        "again ms",
        "disk KiB"
    );
    let stores: [(&str, Run); 2] = [("engine", engine), ("sqlite", sqlite)];
    for n in SIZES {
        for level in [Level::Sync, Level::Group, Level::NoSync] {
            for (name, run) in stores {
                let dir = scratch(&format!("{name}_{}_{n}", level.name()));
                let row = run(&dir, level, n);
                println!(
                    "{:>7} {:<8} {:<7} {:>9.2} {:>9.2} {:>7.2} {:>10.2} {:>9.2} {:>9}",
                    n,
                    level.name(),
                    name,
                    row.insert_us,
                    row.replace_us,
                    row.get_us,
                    row.reopen_ms,
                    row.again_ms,
                    row.bytes / 1024
                );
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
}
