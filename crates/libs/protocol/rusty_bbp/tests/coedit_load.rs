#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Step 4 load harness: scripted editors (no model, no spend) co-editing one
//! spec through the real driver, engine and store, in virtual time. The
//! workload and the decision rule are fixed in `docs/bbp-event-based-plan.md`
//! (step 4a). The report is the ignored test:
//!
//! `cargo test -p rusty_bbp --release --test coedit_load -- --ignored --nocapture`
//! (about 13 minutes, nearly all of it the N = 32, 4-section cells).

mod common;
use common::*;
use rusty_bbp::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Lines per section of the seed spec, heading included: 4 x 8 = 32 lines.
const SECTION_LINES: usize = 8;
/// Think time per edit, from the 2b planners' spec times.
const THINK_MS: (u64, u64) = (10_000, 17_000);

#[derive(Clone, Copy, Debug)]
struct Load {
    editors: usize,
    sections: usize,
    /// Percent chance an edit leaves the editor's own section.
    overlap: u32,
    edits: usize,
    seed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Tally {
    accepted: u64,
    refused: u64,
    /// Accepted edits whose base was not the head: merged by id, unseen work kept.
    merged: u64,
    think_ms: u64,
    wasted_ms: u64,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.accepted += o.accepted;
        self.refused += o.refused;
        self.merged += o.merged;
        self.think_ms += o.think_ms;
        self.wasted_ms += o.wasted_ms;
    }
}

/// splitmix64: a seed fixes the whole run.
struct Gen(u64);

impl Gen {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % below.max(1)
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.next(xs.len() as u64) as usize]
    }
}

fn editor(i: usize) -> String {
    if i == 0 {
        "planner".into()
    } else {
        format!("planner{}", i + 1)
    }
}

/// The four test roles plus `n - 1` more planners.
fn roster(n: usize) -> Vec<Role> {
    let mut roles = four_roles();
    let planner = roles[0].clone();
    for i in 1..n {
        let mut r = planner.clone();
        r.id = RoleId(editor(i));
        r.turn_order = 10 + i as u16;
        roles.push(r);
    }
    roles
}

fn budget() -> Budget {
    let big = u64::MAX / 4;
    Budget {
        messages: big,
        bytes: big,
        reads: big,
        reads_per_turn: big,
        turns: big,
        iterations: 4,
    }
}

/// One editor's edit in flight: what it read and what it will submit.
struct Pending {
    base: ArtId,
    ops: Vec<LineOp>,
    think: u64,
}

/// Section `s` of `doc`: its heading and the lines up to the next heading.
fn section(doc: &Doc, s: usize) -> Vec<(LineId, bool)> {
    let mut at = None;
    let mut out = Vec::new();
    for (id, text) in doc.lines() {
        if text.starts_with('#') {
            at = Some(at.map_or(0, |a| a + 1));
        }
        if at == Some(s) {
            out.push((id, text.starts_with('#')));
        }
    }
    out
}

/// One edit to one section: insert 1 to 3 lines after any of its lines, or
/// replace one body line with 1 or 2.
fn plan(g: &mut Gen, doc: &Doc, s: usize, tag: &str) -> Vec<LineOp> {
    let lines = section(doc, s);
    let body: Vec<LineId> = lines.iter().filter(|l| !l.1).map(|l| l.0).collect();
    let all: Vec<LineId> = lines.iter().map(|l| l.0).collect();
    let text = |k: u64| (0..k).map(|j| format!("{tag}.{j}")).collect::<Vec<_>>();
    if body.is_empty() || g.next(2) == 0 {
        let after = Some(g.pick(&all));
        return vec![LineOp::Insert {
            after,
            lines: text(1 + g.next(3)),
        }];
    }
    let line = g.pick(&body);
    vec![
        LineOp::Delete { line },
        LineOp::Insert {
            after: Some(line),
            lines: text(1 + g.next(2)),
        },
    ]
}

fn edit<S: Store>(fx: &mut Fx<S>, role: &str, base: Option<ArtId>, ops: Vec<LineOp>) -> Response {
    let brief = fx.st().brief.unwrap();
    fx.put(
        role,
        ArtifactPayload::SpecEdit(SpecEdit { brief, base, ops }),
        b"",
    )
}

/// The head of the one co-edited spec.
fn head<S: Store>(fx: &Fx<S>) -> ArtId {
    *fx.st().heads.values().next().unwrap()
}

/// Read the head through the engine, as an agent would, and draft an edit.
fn read<S: Store>(fx: &mut Fx<S>, g: &mut Gen, l: &Load, i: usize, n: usize) -> Pending {
    let base = head(fx);
    match fx.agent(RoleId(editor(i)), AgentAction::GetArtifact { id: base }) {
        Response::Artifact(_) => {}
        other => panic!("{} read {base}: {other:?}", editor(i)),
    }
    let own = i % l.sections;
    let s = if g.next(100) < u64::from(l.overlap) {
        g.next(l.sections as u64) as usize
    } else {
        own
    };
    let doc = fx.st().doc(base).unwrap().clone();
    let ops = plan(g, &doc, s, &format!("e{i}.{n}"));
    let think = THINK_MS.0 + g.next(THINK_MS.1 - THINK_MS.0 + 1);
    Pending { base, ops, think }
}

/// Run one load on `store`. Every editor reads, thinks and submits until it
/// has `edits` accepted; a refused edit is re-read and redone in full.
fn run<S: Store>(store: S, l: &Load) -> Tally {
    let mut fx = Fx::open_with(store, roster(l.editors), &[], l.editors as u32, budget());
    let mut g = Gen(l.seed);
    let mut lines = Vec::new();
    for s in 0..l.sections {
        lines.push(format!("# S{s}"));
        lines.extend((1..SECTION_LINES).map(|k| format!("s{s}.{k}")));
    }
    let after = None;
    stored(edit(
        &mut fx,
        "planner",
        None,
        vec![LineOp::Insert { after, lines }],
    ));
    let start = fx.now.0;
    let mut t = Tally::default();
    let mut done = vec![0usize; l.editors];
    let mut tries = vec![0usize; l.editors];
    let mut drafts: Vec<Pending> = (0..l.editors)
        .map(|i| read(&mut fx, &mut g, l, i, 0))
        .collect();
    let mut queue: BinaryHeap<Reverse<(u64, usize)>> = (0..l.editors)
        .map(|i| Reverse((start + drafts[i].think, i)))
        .collect();
    while let Some(Reverse((at, i))) = queue.pop() {
        // The moderator's tick: expired turns end and are granted again, as
        // they would be for a live agent between two of its turns.
        fx.now = Time(at);
        fx.tick();
        let d = &drafts[i];
        let (base, think) = (d.base, d.think);
        t.think_ms += think;
        let stale = !fx.st().since(base).unwrap().1.is_empty();
        match edit(&mut fx, &editor(i), Some(base), d.ops.clone()) {
            Response::Stored(_) => {
                t.accepted += 1;
                t.merged += u64::from(stale);
                done[i] += 1;
            }
            Response::Rejected(r) if r.code == Code::StaleBase => {
                t.refused += 1;
                t.wasted_ms += think;
            }
            other => panic!("{} edit on {base}: {other:?}", editor(i)),
        }
        if done[i] == l.edits {
            continue;
        }
        tries[i] += 1;
        drafts[i] = read(&mut fx, &mut g, l, i, tries[i]);
        queue.push(Reverse((at + drafts[i].think, i)));
    }
    t
}

fn cell(editors: usize, sections: usize, overlap: u32, seeds: u64) -> Tally {
    let mut sum = Tally::default();
    for seed in 0..seeds {
        let l = Load {
            editors,
            sections,
            overlap,
            edits: 20,
            seed: seed + 1,
        };
        sum.add(run(MemStore::new(), &l));
    }
    sum
}

fn ratio(a: u64, b: u64) -> f64 {
    a as f64 / b.max(1) as f64
}

#[test]
fn disjoint_owners_never_clash() {
    let t = cell(4, 4, 0, 2);
    assert_eq!(t.accepted, 2 * 4 * 20);
    assert_eq!((t.refused, t.wasted_ms), (0, 0));
}

#[test]
fn a_shared_section_clashes_and_a_clash_is_never_lost() {
    let l = Load {
        editors: 4,
        sections: 1,
        overlap: 0,
        edits: 5,
        seed: 7,
    };
    let t = run(MemStore::new(), &l);
    assert_eq!(t.accepted, 20, "every editor finishes despite refusals");
    assert!(t.refused > 0, "one section, four editors: {t:?}");
    assert!(
        t.merged > 0,
        "most concurrent edits touch other lines: {t:?}"
    );
    assert!(t.wasted_ms > 0 && t.wasted_ms < t.think_ms);
}

#[test]
fn the_durable_store_gives_the_same_run() {
    let dir = std::env::temp_dir().join(format!("rusty_bbp_coedit_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let l = Load {
        editors: 4,
        sections: 4,
        overlap: 25,
        edits: 3,
        seed: 3,
    };
    let fs = run(FsStore::open(&dir).unwrap(), &l);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(fs, run(MemStore::new(), &l));
}

#[test]
#[ignore = "the report: run with --release --ignored --nocapture"]
fn report() {
    println!("| N | sections | overlap | accepted | refused/edit | wasted work | merged/edit |");
    println!("| ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for n in [2, 4, 8, 16, 32] {
        let shapes = if n == 4 { vec![4] } else { vec![4, n] };
        for sections in shapes {
            for overlap in [0, 25, 100] {
                let t = cell(n, sections, overlap, 5);
                println!(
                    "| {n} | {sections} | {:.2} | {} | {:.2} | {:.0}% | {:.2} |",
                    f64::from(overlap) / 100.0,
                    t.accepted,
                    ratio(t.refused, t.accepted),
                    100.0 * ratio(t.wasted_ms, t.think_ms),
                    ratio(t.merged, t.accepted),
                );
            }
        }
    }
}
