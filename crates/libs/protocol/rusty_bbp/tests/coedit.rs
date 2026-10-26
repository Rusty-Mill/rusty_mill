#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Step 4: several live planners co-editing one spec as line edits.

mod common;
use common::*;
use proptest::prelude::*;
use rusty_bbp::*;

const EDITORS: [&str; 4] = ["planner", "planner2", "planner3", "planner4"];

/// The four test roles plus three more planners: four concurrent spec editors.
fn editors() -> Vec<Role> {
    let mut roles = four_roles();
    let planner = roles[0].clone();
    for (i, name) in EDITORS.iter().enumerate().skip(1) {
        let mut r = planner.clone();
        r.id = RoleId((*name).into());
        r.turn_order = 10 + i as u16;
        roles.push(r);
    }
    roles
}

fn fx() -> Fx {
    let b = Budget {
        reads_per_turn: 100,
        ..default_budget()
    };
    Fx::concurrent(editors(), 4, b)
}

fn ins(after: Option<LineId>, lines: &[&str]) -> LineOp {
    LineOp::Insert {
        after,
        lines: lines.iter().map(|s| s.to_string()).collect(),
    }
}

fn edit(fx: &mut Fx, role: &str, base: Option<ArtId>, ops: Vec<LineOp>) -> Response {
    let brief = fx.st().brief.unwrap();
    fx.put(
        role,
        ArtifactPayload::SpecEdit(SpecEdit { brief, base, ops }),
        b"",
    )
}

/// The id of the visible line whose text is `text` in version `v`.
fn line(fx: &Fx, v: ArtId, text: &str) -> LineId {
    fx.st()
        .doc(v)
        .unwrap()
        .lines()
        .find(|(_, t)| *t == text)
        .map(|(id, _)| id)
        .unwrap()
}

/// A spec with four sections, one per editor.
fn seed(fx: &mut Fx) -> ArtId {
    let text = ["# A", "a", "# B", "b", "# C", "c", "# D", "d"];
    stored(edit(fx, "planner", None, vec![ins(None, &text)]))
}

#[test]
fn four_planners_hold_turns_at_once() {
    let fx = fx();
    let live: Vec<&str> = fx.st().turns.iter().map(|t| t.role.0.as_str()).collect();
    assert_eq!(live, EDITORS);
}

#[test]
fn edits_in_different_sections_all_land_on_one_base() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let mut head = v1;
    for (role, sec) in EDITORS.iter().zip(["a", "b", "c", "d"]) {
        let after = line(&fx, v1, sec);
        let text = format!("{sec} by {role}");
        head = stored(edit(
            &mut fx,
            role,
            Some(v1),
            vec![ins(Some(after), &[&text])],
        ));
    }
    assert_eq!(
        fx.st().doc(head).unwrap().text(),
        "# A\na\na by planner\n# B\nb\nb by planner2\n# C\nc\nc by planner3\n# D\nd\nd by planner4"
    );
    assert_eq!(
        fx.st().doc(v1).unwrap().lines().count(),
        8,
        "a version never changes"
    );
}

#[test]
fn a_stale_edit_of_a_changed_line_is_refused_and_costs_nothing() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let a = line(&fx, v1, "a");
    stored(edit(
        &mut fx,
        "planner",
        Some(v1),
        vec![ins(Some(a), &["first"])],
    ));
    let spent = fx.st().spent.bytes;
    let refused = edit(
        &mut fx,
        "planner2",
        Some(v1),
        vec![LineOp::Delete { line: a }],
    );
    assert_eq!(rejected(refused), Code::StaleBase);
    assert_eq!(fx.st().spent.bytes, spent, "a refused edit is not charged");
    let log = fx.d.store.events(&TaskId("T1".into()), 0).unwrap();
    assert!(log.iter().any(|e| matches!(
        e,
        Event::Rejected {
            code: Code::StaleBase,
            ..
        }
    )));
    let (head, _) = fx.st().since(v1).unwrap();
    let rebased = stored(edit(
        &mut fx,
        "planner2",
        Some(head),
        vec![LineOp::Delete { line: a }],
    ));
    assert_eq!(
        fx.st().doc(rebased).unwrap().text(),
        "# A\nfirst\n# B\nb\n# C\nc\n# D\nd"
    );
}

#[test]
fn a_stale_edit_of_another_line_in_the_same_section_lands() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let (heading, a) = (line(&fx, v1, "# A"), line(&fx, v1, "a"));
    stored(edit(
        &mut fx,
        "planner",
        Some(v1),
        vec![ins(Some(a), &["after a"])],
    ));
    let v3 = stored(edit(
        &mut fx,
        "planner2",
        Some(v1),
        vec![ins(Some(heading), &["intro to A"])],
    ));
    assert_eq!(
        fx.st().doc(v3).unwrap().text(),
        "# A\nintro to A\na\nafter a\n# B\nb\n# C\nc\n# D\nd"
    );
}

#[test]
fn malformed_edits_and_snapshot_bases_are_refused() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    assert_eq!(
        rejected(edit(&mut fx, "planner", Some(v1), vec![])),
        Code::BadEdit
    );
    let ghost = LineId { art: v1, n: 99 };
    let hidden = vec![LineOp::Delete { line: ghost }];
    assert_eq!(
        rejected(edit(&mut fx, "planner", Some(v1), hidden)),
        Code::RefUnresolved
    );
    let brief = fx.st().brief.unwrap();
    let snap = stored(fx.put("planner", ArtifactPayload::Spec { brief }, b"# S"));
    let on_snap = vec![ins(None, &["x"])];
    assert_eq!(
        rejected(edit(&mut fx, "planner", Some(snap), on_snap)),
        Code::WrongKind
    );
    let a = line(&fx, v1, "a");
    let on_brief = vec![ins(Some(a), &["x"])];
    assert_eq!(
        rejected(edit(&mut fx, "planner", Some(brief), on_brief)),
        Code::WrongKind
    );
}

#[test]
fn the_plan_gate_binds_one_co_edited_version() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let a = line(&fx, v1, "a");
    let v2 = stored(edit(
        &mut fx,
        "planner2",
        Some(v1),
        vec![ins(Some(a), &["more"])],
    ));
    let mut d = Draft::new(MessageKind::RequestDecision, "Please approve.")
        .to(Recipient::Human)
        .refs(vec![Ref::art(v2)]);
    d.gate = true;
    posted(fx.post("planner", d));
    assert_eq!(fx.st().state, State::PlanGate);
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Plan,
        subject: v2,
        run: None,
    }));
    assert_eq!(fx.st().approved_spec, Some(v2));
    assert_eq!(
        fx.st().doc(v2).unwrap().text(),
        "# A\na\nmore\n# B\nb\n# C\nc\n# D\nd"
    );
}

#[test]
fn the_plan_gate_refuses_a_version_that_is_not_the_head() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let a = line(&fx, v1, "a");
    let v2 = stored(edit(
        &mut fx,
        "planner2",
        Some(v1),
        vec![ins(Some(a), &["more"])],
    ));
    let gate_on = |v: ArtId| {
        let mut d = Draft::new(MessageKind::RequestDecision, "Please approve.")
            .to(Recipient::Human)
            .refs(vec![Ref::art(v)]);
        d.gate = true;
        d
    };
    assert_eq!(
        rejected(fx.post("planner", gate_on(v1))),
        Code::StaleSubject,
        "approving v1 would drop v2's line"
    );
    assert_eq!(fx.st().state, State::Planning);
    posted(fx.post("planner", gate_on(v2)));
    assert_eq!(fx.st().state, State::PlanGate);
}

#[test]
fn a_replayed_log_rebuilds_every_version() {
    let mut fx = fx();
    let v1 = seed(&mut fx);
    let b = line(&fx, v1, "b");
    stored(edit(
        &mut fx,
        "planner3",
        Some(v1),
        vec![LineOp::Delete { line: b }],
    ));
    let events = fx.d.store.events(&TaskId("T1".into()), 0).unwrap();
    let replay = TaskState::fold(TaskId("T1".into()), &events);
    assert_eq!(replay.versions, fx.st().versions);
    assert_eq!(replay.heads, fx.st().heads);
}

#[test]
fn a_co_edited_spec_round_trips_through_its_stored_bytes() {
    let payload = ArtifactPayload::SpecEdit(SpecEdit {
        brief: ArtId(1),
        base: Some(ArtId(2)),
        ops: vec![
            ins(None, &["# T"]),
            LineOp::Delete {
                line: LineId {
                    art: ArtId(2),
                    n: 0,
                },
            },
        ],
    });
    let bytes = encode_artifact(&payload, b"ignored").unwrap();
    assert_eq!(
        decode_artifact(ArtifactKind::Spec, &bytes).unwrap(),
        payload
    );
    let snapshot = ArtifactPayload::Spec { brief: ArtId(1) };
    let bytes = encode_artifact(&snapshot, b"# S").unwrap();
    assert_eq!(
        decode_artifact(ArtifactKind::Spec, &bytes).unwrap(),
        snapshot
    );
    let both = br#"{"brief":1,"markdown":"x","edit":{"base":null,"ops":[]}}"#;
    assert!(decode_artifact(ArtifactKind::Spec, both).is_err());
}

// ------------------------------------------------- the fold is RGA's order

/// Textbook array RGA: a line goes after its parent, past every sibling run
/// with a larger id. Correct under any causal delivery order.
#[derive(Default)]
struct Rga(Vec<(LineId, String, bool)>);

impl Rga {
    fn insert(&mut self, parent: Option<LineId>, id: LineId, text: &str) {
        let mut i = match parent {
            None => 0,
            Some(p) => self.0.iter().position(|e| e.0 == p).unwrap() + 1,
        };
        while i < self.0.len() && self.0[i].0 > id {
            i += 1;
        }
        self.0.insert(i, (id, text.to_string(), true));
    }
    fn delete(&mut self, id: LineId) {
        self.0.iter_mut().find(|e| e.0 == id).unwrap().2 = false;
    }
    fn text(&self) -> String {
        let live: Vec<&str> = self
            .0
            .iter()
            .filter(|e| e.2)
            .map(|e| e.1.as_str())
            .collect();
        live.join("\n")
    }
}

/// One edit as RGA elements: each inserted line with its parent, in id order.
struct Elems {
    inserts: Vec<(Option<LineId>, LineId, String)>,
    deletes: Vec<LineId>,
}

fn elems(at: ArtId, ops: &[LineOp]) -> Elems {
    let (mut inserts, mut deletes, mut n) = (Vec::new(), Vec::new(), 0);
    for op in ops {
        match op {
            LineOp::Insert { after, lines } => {
                let mut parent = *after;
                for text in lines {
                    let id = LineId { art: at, n };
                    inserts.push((parent, id, text.clone()));
                    parent = Some(id);
                    n += 1;
                }
            }
            LineOp::Delete { line } => deletes.push(*line),
        }
    }
    Elems { inserts, deletes }
}

/// A small deterministic generator, so a proptest seed fixes a whole history.
struct Gen(u64);

impl Gen {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % below.max(1) as u64) as usize
    }
}

proptest! {
    /// Edits made against random earlier versions, applied in log order by
    /// the fold, give the text a textbook RGA reaches when it receives the
    /// same edits in a random causal order.
    #[test]
    fn the_log_order_fold_is_what_rga_converges_to(seed in any::<u64>()) {
        let mut g = Gen(seed);
        let mut docs = vec![Doc::default()];
        let mut edits: Vec<Elems> = Vec::new();
        for v in 1..=12u64 {
            let base = docs[g.next(docs.len())].clone();
            let seen: Vec<LineId> = base.lines().map(|(id, _)| id).collect();
            let mut ops = Vec::new();
            for k in 0..1 + g.next(3) {
                let anchor = (!seen.is_empty() && g.next(4) > 0).then(|| seen[g.next(seen.len())]);
                if g.next(3) == 0 && anchor.is_some() {
                    ops.push(LineOp::Delete { line: anchor.unwrap() });
                } else {
                    let lines: Vec<String> = (0..1 + g.next(2)).map(|j| format!("{v}.{k}.{j}")).collect();
                    ops.push(LineOp::Insert { after: anchor, lines });
                }
            }
            let at = ArtId(v);
            let head = docs.last().unwrap().apply(at, &ops).unwrap();
            docs.push(head);
            edits.push(elems(at, &ops));
        }
        // Deliver each edit's elements once their parents and targets exist.
        let mut rga = Rga::default();
        let mut pending: Vec<(Option<LineId>, LineId, String)> =
            edits.iter().flat_map(|e| e.inserts.clone()).collect();
        let mut deletes: Vec<LineId> = edits.iter().flat_map(|e| e.deletes.clone()).collect();
        while !pending.is_empty() || !deletes.is_empty() {
            let has = |rga: &Rga, id: LineId| rga.0.iter().any(|e| e.0 == id);
            let ready: Vec<usize> = (0..pending.len())
                .filter(|&i| pending[i].0.is_none_or(|p| has(&rga, p)))
                .collect();
            let dready: Vec<usize> = (0..deletes.len()).filter(|&i| has(&rga, deletes[i])).collect();
            let pick = g.next(ready.len() + dready.len());
            if pick < ready.len() {
                let (p, id, text) = pending.remove(ready[pick]);
                rga.insert(p, id, &text);
            } else {
                rga.delete(deletes.remove(dready[pick - ready.len()]));
            }
        }
        prop_assert_eq!(docs.last().unwrap().text(), rga.text());
    }
}
