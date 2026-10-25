#![allow(clippy::expect_used)]
use super::*;
use crate::slots::Capacity;

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_admission_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn slots(tag: &str, agents: u32) -> Result<Slots, String> {
    Slots::open(
        &dir(tag),
        Capacity {
            agents,
            validators: 1,
        },
    )
}

/// A canonical fixture directory every test shares, made once.
fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("bbp_admission_{}_fs", std::process::id()));
    std::fs::create_dir_all(&root).expect("fixture root");
    root.canonicalize().expect("canonical fixture root")
}

/// `path` beneath the fixture root, made, as the canonical path a claim
/// needs.
fn abs(path: &str) -> String {
    let path = root().join(path.trim_start_matches('/'));
    std::fs::create_dir_all(&path).expect("fixture path");
    path.canonicalize()
        .expect("canonical fixture path")
        .to_string_lossy()
        .into_owned()
}

fn claim(parent: Option<(&str, u32)>, priority_secs: u64, write: &[&str]) -> Claim {
    Claim {
        parent: parent.map(|(id, agents)| Parent {
            id: id.into(),
            agents,
        }),
        priority_secs,
        write: write.iter().map(|w| abs(w)).collect(),
    }
}

/// One moderator's view: its admission and its ticket per class.
struct Waiter {
    admission: Admission,
    ticket: Option<Ticket>,
}

impl Waiter {
    /// A waiter for `task` of the shared fixture store.
    fn new(slots: &Slots, task: &str, claim: Claim) -> Result<Waiter, String> {
        Self::of(slots, &root(), task, claim)
    }

    fn of(slots: &Slots, store: &Path, task: &str, claim: Claim) -> Result<Waiter, String> {
        Ok(Waiter {
            admission: Admission::new(slots.clone(), store, task.into(), claim)?,
            ticket: None,
        })
    }

    fn agent(&mut self, now_ms: u64) -> Result<Option<Slot>, String> {
        self.admission
            .admit_at(Class::Agent, &mut self.ticket, now_ms)
    }
}

#[test]
fn waiters_are_served_in_arrival_order_and_a_newcomer_waits_its_turn() -> Result<(), String> {
    let slots = slots("fifo", 1)?;
    let mut holder = Waiter::new(&slots, "H", Claim::default())?;
    let mut first = Waiter::new(&slots, "A", Claim::default())?;
    let mut second = Waiter::new(&slots, "B", Claim::default())?;
    let held = holder.agent(1_000)?.ok_or("free slot")?;
    assert!(first.agent(2_000)?.is_none());
    assert!(second.agent(3_000)?.is_none());
    drop(held);
    // The slot is reserved for the earlier ticket, however often B asks.
    assert!(second.agent(4_000)?.is_none());
    let a = first.agent(5_000)?.ok_or("A's turn")?;
    assert!(
        first.ticket.is_none(),
        "the ticket is given up with the slot"
    );
    drop(a);
    second.agent(6_000)?.ok_or("B's turn")?;
    Ok(())
}

#[test]
fn priority_is_precedence_in_seconds_so_low_priority_never_starves() -> Result<(), String> {
    let slots = slots("priority", 1)?;
    let mut holder = Waiter::new(&slots, "H", Claim::default())?;
    let mut low = Waiter::new(&slots, "L", Claim::default())?;
    let mut held = holder.agent(0)?.ok_or("free slot")?;
    assert!(low.agent(100_000)?.is_none());
    // High priority, 60 s of precedence: a ticket taken within 60 s after
    // the low one is served first, one taken later is not.
    let mut t = 100_000;
    for task in ["P1", "P2"] {
        t += 20_000;
        let mut high = Waiter::new(&slots, task, claim(None, 60, &[]))?;
        assert!(high.agent(t)?.is_none());
        drop(held);
        assert!(low.agent(t)?.is_none(), "{task} ranks ahead");
        held = high.agent(t)?.ok_or("high's turn")?;
    }
    let mut late = Waiter::new(&slots, "P3", claim(None, 60, &[]))?;
    assert!(late.agent(170_000)?.is_none());
    drop(held);
    assert!(
        late.agent(170_000)?.is_none(),
        "the low ticket ranks ahead now"
    );
    low.agent(170_000)?.ok_or("low's turn")?;
    Ok(())
}

#[test]
fn a_parent_ceiling_counts_each_slot_once_and_blocks_no_one_else() -> Result<(), String> {
    let slots = slots("ceiling", 3)?;
    let goal = Some(("goal", 2));
    let mut a = Waiter::new(&slots, "A", claim(goal, 0, &[]))?;
    let mut b = Waiter::new(&slots, "B", claim(goal, 0, &[]))?;
    let mut other = Waiter::new(&slots, "O", claim(Some(("other", 2)), 0, &[]))?;
    // Two of the goal's tasks, each wanting two harnesses: two slots in all.
    let a1 = a.agent(1)?.ok_or("A's first")?;
    let _b1 = b.agent(2)?.ok_or("B's first")?;
    assert!(a.agent(3)?.is_none(), "the goal is at its ceiling");
    assert!(b.agent(4)?.is_none());
    // A waiter at its ceiling leaves the free slot to the next.
    let _o = other.agent(5)?.ok_or("another parent's task")?;
    drop(a1);
    a.agent(6)?.ok_or("A again under the ceiling")?;
    Ok(())
}

#[test]
fn overlapping_write_leases_of_two_tasks_never_run_together() -> Result<(), String> {
    let slots = slots("lease", 4)?;
    let mut a = Waiter::new(&slots, "A", claim(None, 0, &["/srv/out"]))?;
    let mut b = Waiter::new(&slots, "B", claim(None, 0, &["/srv/out/b"]))?;
    let mut c = Waiter::new(&slots, "C", claim(None, 0, &["/srv/outside"]))?;
    let mut d = Waiter::new(&slots, "D", claim(None, 0, &["/srv/out"]))?;
    // Harnesses of one task share its writes.
    let a1 = a.agent(1)?.ok_or("A")?;
    let a2 = a.agent(2)?.ok_or("A's second harness")?;
    assert!(b.agent(3)?.is_none(), "B writes beneath A");
    // A sibling spelling is no overlap.
    let _c = c.agent(4)?.ok_or("C")?;
    // B, waiting, now holds back A's next harness as well as D.
    assert!(a.agent(5)?.is_none(), "A cannot pass a waiting B");
    a.ticket = None;
    assert!(d.agent(5)?.is_none());
    drop((a1, a2));
    assert!(d.agent(6)?.is_none(), "B keeps its paths while it waits");
    let b1 = b.agent(7)?.ok_or("B")?;
    assert!(d.agent(8)?.is_none(), "B now holds them");
    drop(b1);
    d.agent(9)?.ok_or("D")?;
    Ok(())
}

#[test]
fn the_same_task_id_in_another_store_is_another_task() -> Result<(), String> {
    let slots = slots("stores", 2)?;
    let (one, two) = (abs("/store-one"), abs("/store-two"));
    let write = claim(None, 0, &["/shared"]);
    let mut a = Waiter::of(&slots, Path::new(&one), "T", write.clone())?;
    let mut b = Waiter::of(&slots, Path::new(&two), "T", write.clone())?;
    let mut again = Waiter::of(&slots, Path::new(&one), "T", write)?;
    let _a = a.agent(1)?.ok_or("A")?;
    assert!(
        b.agent(2)?.is_none(),
        "T of store two writes where T of store one does"
    );
    b.ticket = None;
    again
        .agent(3)?
        .ok_or("T of store one shares its own writes")?;
    Ok(())
}

#[test]
fn tickets_of_gone_waiters_are_dropped_and_validators_queue_alone() -> Result<(), String> {
    let slots = slots("stale", 1)?;
    let mut holder = Waiter::new(&slots, "H", Claim::default())?;
    let held = holder.agent(1)?.ok_or("free slot")?;
    let mut gone = Waiter::new(&slots, "G", Claim::default())?;
    assert!(gone.agent(2)?.is_none());
    let mut next = Waiter::new(&slots, "N", Claim::default())?;
    assert!(next.agent(3)?.is_none());
    // A waiter that died leaves its files with no lock on them.
    let stem = gone.ticket.as_ref().ok_or("ticket")?.stem.clone();
    let json = std::fs::read(with_ext(&stem, "json")).map_err(|e| e.to_string())?;
    drop(gone);
    std::fs::write(with_ext(&stem, "lock"), "").map_err(|e| e.to_string())?;
    std::fs::write(with_ext(&stem, "json"), json).map_err(|e| e.to_string())?;
    drop(held);
    let _n = next.agent(4)?.ok_or("the dead ticket is passed")?;
    assert!(!with_ext(&stem, "lock").exists(), "and removed");

    // Agent claims bound agents only: a validator conflicts with nothing.
    let mut v = Waiter::new(&slots, "V", claim(Some(("p", 1)), 0, &["/srv"]))?;
    let mut ticket = None;
    let run = v.admission.admit_at(Class::Validator, &mut ticket, 5)?;
    assert!(run.is_some());
    assert!(v.agent(6)?.is_none(), "the agent slot is N's");
    Ok(())
}

#[test]
fn claims_are_validated_at_the_boundary() -> Result<(), String> {
    let slots = slots("invalid", 1)?;
    let out = abs("/valid/out");
    abs("/valid/sub");
    let raw = |path: String| Claim {
        write: vec![path],
        ..Claim::default()
    };
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut bad = vec![
        claim(Some(("", 1)), 0, &[]),
        claim(Some(("p", 0)), 0, &[]),
        claim(None, MAX_PRIORITY_SECS + 1, &[]),
        raw("relative".into()),
        // A parent component and a missing path both validate as absolute
        // yet do not name what they spell.
        raw(Path::new(&abs("/valid/sub"))
            .join("..")
            .join("out")
            .to_string_lossy()
            .into_owned()),
        raw(Path::new(&out)
            .join("missing")
            .to_string_lossy()
            .into_owned()),
    ];
    #[cfg(unix)]
    {
        let alias = root().join("valid/alias");
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(&out, &alias).map_err(|e| e.to_string())?;
        bad.push(raw(alias.to_string_lossy().into_owned()));
    }
    for claim in bad {
        assert!(
            Admission::new(slots.clone(), &root(), "T".into(), claim.clone()).is_err(),
            "{claim:?}"
        );
    }
    assert!(Admission::new(slots, &root(), " ".into(), Claim::default()).is_err());
    let file = dir("claim_file").with_extension("json");
    let json = format!(
        r#"{{"parent":{{"id":"g","agents":2}},"write":[{}]}}"#,
        rusty_serde::json::to_string(&abs("/w")).map_err(|e| e.to_string())?
    );
    std::fs::write(&file, json).map_err(|e| e.to_string())?;
    assert_eq!(Claim::read_file(&file)?, claim(Some(("g", 2)), 0, &["/w"]));
    Ok(())
}
