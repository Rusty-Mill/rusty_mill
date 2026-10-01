//! Probes for the gaps in Rusty-Mill/rusty_mill#382. Each test names the gap.

use rusty_tick::{Status, Task, TaskStore};
use std::time::Instant;
use uuid::Uuid;

fn task(list: Uuid, title: &str, due: Option<i64>, order: i64) -> Task {
    let mut t = Task::new(Uuid::now_v7(), list, title, order, 0);
    t.due_ms = due;
    t
}

#[test]
fn persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let list = Uuid::now_v7();
    let t = task(list, "write spike", Some(10), 1);
    let id = t.id;
    {
        let mut s = TaskStore::open(dir.path()).unwrap();
        s.insert(t.clone()).unwrap();
    }
    let s = TaskStore::open(dir.path()).unwrap();
    assert_eq!(s.get(id), Some(t));
    assert_eq!(s.in_list(list).len(), 1);
}

#[test]
fn gap2_due_range_per_list_via_composite_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = TaskStore::open(dir.path()).unwrap();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    for (l, d) in [(a, 5), (a, 15), (a, 25), (b, 15)] {
        s.insert(task(l, "t", Some(d), 0)).unwrap();
    }
    s.insert(task(a, "no due", None, 0)).unwrap();
    let hits: Vec<i64> = s
        .due_between(a, 10, 30)
        .iter()
        .map(|t| t.due_ms.unwrap())
        .collect();
    assert_eq!(
        hits,
        vec![15, 25],
        "per-list, ascending, excludes other lists and undated"
    );
}

#[test]
fn gap5_reorder_moves_one_task_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let list = Uuid::now_v7();
    let (first, second, third);
    {
        let mut s = TaskStore::open(dir.path()).unwrap();
        let ts: Vec<Task> = (0..3).map(|i| task(list, "t", None, i * 1000)).collect();
        (first, second, third) = (ts[0].id, ts[1].id, ts[2].id);
        for t in ts {
            s.insert(t).unwrap();
        }
        // Drag `third` between `first` and `second`: one slot write, gap-based.
        s.reorder(third, 500).unwrap();
        let order: Vec<Uuid> = s.in_manual_order(list).iter().map(|t| t.id).collect();
        assert_eq!(order, vec![first, third, second]);
    }
    let s = TaskStore::open(dir.path()).unwrap();
    let order: Vec<Uuid> = s.in_manual_order(list).iter().map(|t| t.id).collect();
    assert_eq!(order, vec![first, third, second], "reorder is durable");
}

#[test]
fn gap3_fulltext_phrase_and_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = TaskStore::open(dir.path()).unwrap();
    let list = Uuid::now_v7();
    let t = task(list, "Buy groceries for dinner", None, 0);
    let id = t.id;
    s.insert(t).unwrap();
    assert_eq!(s.search(&["groceries"]), vec![id]);
    assert_eq!(s.search(&["buy groceries"]), vec![id]);
    // Type-ahead: the last word of a phrase matches as a prefix (engine `any_of_prefix`).
    assert_eq!(s.search(&["grocer"]), vec![id]);
    assert_eq!(s.search(&["buy groc"]), vec![id]);
    assert!(
        s.search(&["roceries"]).is_empty(),
        "a prefix, not a substring"
    );
}

#[test]
fn tags_and_replace_keep_derived_indexes_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = TaskStore::open(dir.path()).unwrap();
    let list = Uuid::now_v7();
    let mut t = task(list, "a", None, 0);
    t.tags = vec!["home".into()];
    let id = t.id;
    s.insert(t.clone()).unwrap();
    assert_eq!(s.with_tag("home"), vec![id]);
    t.tags = vec!["work".into()];
    t.title = "renamed".into();
    s.replace(t).unwrap();
    assert!(s.with_tag("home").is_empty());
    assert_eq!(s.with_tag("work"), vec![id]);
    assert_eq!(s.search(&["renamed"]), vec![id]);
    s.delete(id).unwrap();
    assert!(s.with_tag("work").is_empty() && s.search(&["renamed"]).is_empty());
}

#[test]
fn gap1_status_filter_needs_a_scan_within_the_list() {
    // Only `list_id` is indexed; status/priority are filtered from `in_list`.
    let dir = tempfile::tempdir().unwrap();
    let mut s = TaskStore::open(dir.path()).unwrap();
    let list = Uuid::now_v7();
    for i in 0..10 {
        let mut t = task(list, "t", None, i);
        t.status = if i % 2 == 0 {
            Status::Done
        } else {
            Status::Open
        };
        s.insert(t).unwrap();
    }
    let open = s
        .in_list(list)
        .into_iter()
        .filter(|t| t.status == Status::Open)
        .count();
    assert_eq!(open, 5);
}

/// Gap 6 / scale probe. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn scale_open_and_query_time() {
    let dir = tempfile::tempdir().unwrap();
    let lists: Vec<Uuid> = (0..50).map(|_| Uuid::now_v7()).collect();
    let n = 100_000;
    let t0 = Instant::now();
    {
        let mut s = TaskStore::open(dir.path()).unwrap();
        for i in 0..n {
            let mut t = task(
                lists[i % 50],
                &format!("task number {i} about widgets"),
                Some(i as i64),
                i as i64,
            );
            t.tags = vec![format!("tag{}", i % 20)];
            s.insert(t).unwrap();
        }
    }
    eprintln!("insert {n}: {:?}", t0.elapsed());
    let t1 = Instant::now();
    let s = TaskStore::open(dir.path()).unwrap();
    eprintln!("reopen {n}: {:?}", t1.elapsed());
    let t2 = Instant::now();
    let due = s.due_between(lists[0], 0, 1000);
    let list = s.in_list(lists[0]);
    let found = s.search(&["widgets"]);
    eprintln!(
        "due={} list={} search={} in {:?}",
        due.len(),
        list.len(),
        found.len(),
        t2.elapsed()
    );
}
