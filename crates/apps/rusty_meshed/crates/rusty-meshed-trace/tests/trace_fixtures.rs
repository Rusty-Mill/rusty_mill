//! Fixture tests for the reverse trace: every verdict, every edge class,
//! worst-first ordering, what-if, and the TOML/JSON round trips.

use std::time::Duration;

use rusty_meshed_trace::{
    builtin, trace, trace_all, Criticality, Domain, DomainId, EdgeState, Fidelity, Maturity,
    NodeRef, Outcome, OutcomeId, Rating, Requirement, Scenario, ScenarioError, Source, SourceId,
    SourceKind, TraceError,
};

fn domain(id: &str, maturity: Maturity) -> Domain {
    Domain {
        id: id.into(),
        name: format!("Domain {id}"),
        owner: format!("Owner of {id}"),
        maturity,
        sources: vec![SourceId::new(format!("{id}_src"))],
    }
}

fn source(domain: &str) -> Source {
    Source {
        id: SourceId::new(format!("{domain}_src")),
        domain: domain.into(),
        name: format!("Source for {domain}"),
        kind: SourceKind::System,
        structure: Rating::new(2).unwrap(),
        availability: Rating::new(2).unwrap(),
        latency: Duration::from_secs(60),
    }
}

fn req(domain: &str, min: Maturity, criticality: Criticality) -> Requirement {
    Requirement {
        domain: domain.into(),
        min_maturity: min,
        criticality,
    }
}

/// Four domains at L4 / L2 / L3 / L1 with one source each.
fn fixture() -> Scenario {
    Scenario {
        name: "fixture".into(),
        description: String::new(),
        domains: vec![
            domain("a", Maturity::Integrated),
            domain("b", Maturity::Structured),
            domain("c", Maturity::Accessible),
            domain("d", Maturity::Documented),
        ],
        sources: vec![source("a"), source("b"), source("c"), source("d")],
        outcomes: vec![],
    }
}

fn with_outcome(mut s: Scenario, requires: Vec<Requirement>) -> Scenario {
    s.outcomes.push(Outcome {
        id: "o".into(),
        name: "Outcome".into(),
        description: "desc".into(),
        requires,
    });
    s
}

fn states(report: &rusty_meshed_trace::TraceReport) -> Vec<(String, EdgeState)> {
    report
        .edges
        .iter()
        .map(|e| {
            let from = match &e.from {
                NodeRef::Outcome(id) => format!("outcome:{id}"),
                NodeRef::Domain(id) => format!("domain:{id}"),
                NodeRef::Source(id) => format!("source:{id}"),
            };
            (from, e.state)
        })
        .collect()
}

#[test]
fn every_requirement_satisfied_is_full() {
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Accessible, Criticality::Blocking),
            req("c", Maturity::Accessible, Criticality::Degrading),
            req("b", Maturity::Structured, Criticality::Optional),
        ],
    );
    let r = trace(&s, &"o".into()).unwrap();
    assert_eq!(r.achievable, Fidelity::Full);
    assert!(r.bottlenecks.is_empty());
    assert!(r.edges.iter().all(|e| e.state == EdgeState::Satisfied));
    // Outcome edges first, in requirement order, then source edges.
    assert_eq!(
        states(&r),
        vec![
            ("domain:a".into(), EdgeState::Satisfied),
            ("domain:c".into(), EdgeState::Satisfied),
            ("domain:b".into(), EdgeState::Satisfied),
            ("source:a_src".into(), EdgeState::Satisfied),
            ("source:c_src".into(), EdgeState::Satisfied),
            ("source:b_src".into(), EdgeState::Satisfied),
        ]
    );
}

#[test]
fn a_degrading_gap_is_partial_with_a_weighted_fraction() {
    // a: blocking (3.0) satisfied; b: degrading (2.0) gap 1; d: optional
    // (1.0) satisfied at its floor -> 4.0 / 6.0.
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Integrated, Criticality::Blocking),
            req("b", Maturity::Accessible, Criticality::Degrading),
            req("d", Maturity::Documented, Criticality::Optional),
        ],
    );
    let r = trace(&s, &"o".into()).unwrap();
    match r.achievable {
        Fidelity::Partial(f) => assert!((f - 4.0 / 6.0).abs() < 1e-6, "got {f}"),
        other => panic!("expected Partial, got {other:?}"),
    }
    assert_eq!(r.bottlenecks.len(), 1);
    assert_eq!(r.bottlenecks[0].domain, DomainId::new("b"));
    assert_eq!(r.bottlenecks[0].state, EdgeState::Degraded);
    assert_eq!(r.bottlenecks[0].gap, 1);
    assert_eq!(r.bottlenecks[0].current, Maturity::Structured);
    assert_eq!(r.bottlenecks[0].required, Maturity::Accessible);
    assert_eq!(r.bottlenecks[0].sources, vec![SourceId::new("b_src")]);
    assert_eq!(r.domain_state(&"b".into()), Some(EdgeState::Degraded));
    assert_eq!(r.domain_state(&"c".into()), None);
}

#[test]
fn a_blocking_gap_is_not_achievable_even_if_everything_else_is_fine() {
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Accessible, Criticality::Blocking),
            req("d", Maturity::Structured, Criticality::Blocking),
        ],
    );
    let r = trace(&s, &"o".into()).unwrap();
    assert_eq!(r.achievable, Fidelity::NotAchievable);
    assert_eq!(r.bottlenecks.len(), 1);
    assert_eq!(r.bottlenecks[0].state, EdgeState::Blocked);
    assert_eq!(r.bottlenecks[0].criticality, Criticality::Blocking);
}

#[test]
fn an_unmet_optional_alone_is_still_full_but_listed_as_missing() {
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Accessible, Criticality::Blocking),
            req("d", Maturity::Accessible, Criticality::Optional),
        ],
    );
    let r = trace(&s, &"o".into()).unwrap();
    assert_eq!(r.achievable, Fidelity::Full);
    assert_eq!(r.bottlenecks.len(), 1);
    assert_eq!(r.bottlenecks[0].state, EdgeState::Missing);
    assert_eq!(r.domain_state(&"d".into()), Some(EdgeState::Missing));
    // The source edge behind a missing domain is missing too.
    let src = r
        .edges
        .iter()
        .find(|e| matches!(&e.from, NodeRef::Source(id) if id.as_str() == "d_src"))
        .unwrap();
    assert_eq!(src.state, EdgeState::Missing);
}

#[test]
fn every_edge_class_appears_in_one_trace() {
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Accessible, Criticality::Blocking), // satisfied
            req("d", Maturity::Structured, Criticality::Blocking), // blocked
            req("b", Maturity::Accessible, Criticality::Degrading), // degraded
            req("c", Maturity::Integrated, Criticality::Optional), // missing
        ],
    );
    let r = trace(&s, &"o".into()).unwrap();
    assert_eq!(r.achievable, Fidelity::NotAchievable);
    let outcome_states: Vec<_> = r.edges.iter().take(4).map(|e| e.state).collect();
    assert_eq!(
        outcome_states,
        vec![
            EdgeState::Satisfied,
            EdgeState::Blocked,
            EdgeState::Degraded,
            EdgeState::Missing
        ]
    );
}

#[test]
fn bottlenecks_sort_by_criticality_then_gap_then_id() {
    let s = with_outcome(
        fixture(),
        vec![
            req("c", Maturity::Integrated, Criticality::Optional), // opt, gap 1
            req("b", Maturity::Accessible, Criticality::Degrading), // deg, gap 1
            req("d", Maturity::Integrated, Criticality::Degrading), // deg, gap 3
            req("a", Maturity::Integrated, Criticality::Blocking), // satisfied
        ],
    );
    let mut s = s;
    // A second blocking requirement with a wider gap than a same-gap
    // sibling, to exercise the id tiebreak: both "b" and "d" blocked at
    // gap 2 would need equal current levels, so use two fresh domains.
    s.domains.push(domain("y", Maturity::Documented));
    s.domains.push(domain("x", Maturity::Documented));
    s.sources.push(source("y"));
    s.sources.push(source("x"));
    s.outcomes[0]
        .requires
        .push(req("y", Maturity::Accessible, Criticality::Blocking));
    s.outcomes[0]
        .requires
        .push(req("x", Maturity::Accessible, Criticality::Blocking));

    let r = trace(&s, &"o".into()).unwrap();
    let order: Vec<_> = r
        .bottlenecks
        .iter()
        .map(|b| b.domain.as_str().to_string())
        .collect();
    assert_eq!(order, vec!["x", "y", "d", "b", "c"]);
}

#[test]
fn unknown_outcome_and_dangling_domain_are_errors() {
    let s = fixture();
    assert_eq!(
        trace(&s, &"nope".into()),
        Err(TraceError::UnknownOutcome("nope".into()))
    );
    let s = with_outcome(s, vec![req("zzz", Maturity::Tribal, Criticality::Optional)]);
    assert_eq!(
        trace(&s, &"o".into()),
        Err(TraceError::UnknownDomain("o".into(), "zzz".into()))
    );
}

#[test]
fn what_if_raising_a_domain_flips_the_verdict() {
    let s = with_outcome(
        fixture(),
        vec![
            req("a", Maturity::Accessible, Criticality::Blocking),
            req("d", Maturity::Accessible, Criticality::Blocking),
        ],
    );
    assert_eq!(
        trace(&s, &"o".into()).unwrap().achievable,
        Fidelity::NotAchievable
    );

    let mut what_if = s.clone();
    assert!(what_if.set_maturity(&"d".into(), Maturity::Accessible));
    assert!(!what_if.set_maturity(&"nope".into(), Maturity::Accessible));
    assert_eq!(
        trace(&what_if, &"o".into()).unwrap().achievable,
        Fidelity::Full
    );
    // The original is untouched.
    assert_eq!(
        s.domain(&"d".into()).unwrap().maturity,
        Maturity::Documented
    );
}

#[test]
fn maturity_ladder_is_ordinal_and_round_trips() {
    assert!(Maturity::Tribal < Maturity::Documented);
    assert!(Maturity::Accessible < Maturity::Integrated);
    for m in Maturity::ALL {
        assert_eq!(Maturity::from_level(m.level() as i64), Ok(m));
        assert_eq!(Maturity::parse(m.name()), Ok(m));
        assert_eq!(Maturity::parse(&m.level().to_string()), Ok(m));
    }
    assert!(Maturity::from_level(5).is_err());
    assert!(Maturity::from_level(-1).is_err());
    assert!(Rating::new(5).is_err());
    assert_eq!(Maturity::Documented.gap_to(Maturity::Integrated), 3);
    assert_eq!(Maturity::Integrated.gap_to(Maturity::Documented), 0);
    assert_eq!(Maturity::Structured.to_string(), "L2 Structured");
}

// ── The shipped scenario ─────────────────────────────────────────────────

#[test]
fn bundled_scenario_loads_and_has_the_specced_shape() {
    let s = builtin::acquisition_status();
    assert_eq!(s.name, "Acquisition Status Dashboard");
    assert_eq!(s.domains.len(), 10);
    assert_eq!(s.outcomes.len(), 4);
    // `order` keys win over the TOML parser's alphabetical table order.
    let ids: Vec<_> = s.domains.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids[0], "schedule");
    assert_eq!(ids[9], "manpower");
    // Domain -> sources is derived from the sources' own `domain` field.
    let contracts = s.domain(&"contracts".into()).unwrap();
    assert_eq!(
        contracts.sources,
        vec![
            SourceId::new("contract_mods_pdf"),
            SourceId::new("contracting_officer")
        ]
    );
    assert_eq!(
        s.source(&"ims_export".into()).unwrap().latency,
        Duration::from_secs(86400)
    );
    assert_eq!(
        s.source(&"te_program_lead".into()).unwrap().kind,
        SourceKind::Person
    );
    // Every domain has at least one source; every source's domain resolves.
    for d in &s.domains {
        assert!(!d.sources.is_empty(), "{} has no sources", d.id);
    }
}

#[test]
fn bundled_scenario_outcomes_cover_every_verdict() {
    let s = builtin::acquisition_status();
    let reports = trace_all(&s).unwrap();
    let verdicts: Vec<(&str, &str)> = reports
        .iter()
        .map(|r| (r.outcome.as_str(), r.achievable.as_str()))
        .collect();
    assert_eq!(
        verdicts,
        vec![
            ("acq_status_dashboard", "not_achievable"),
            ("program_risk_rollup", "partial"),
            ("cost_schedule_variance", "full"),
            ("readiness_status", "partial"),
        ]
    );
    // The dashboard's worst bottleneck is the blocking contracts gap; the
    // tribal T&E domain is the widest degrading gap.
    let dash = &reports[0];
    assert_eq!(dash.bottlenecks[0].domain, DomainId::new("contracts"));
    assert_eq!(dash.bottlenecks[0].state, EdgeState::Blocked);
    assert_eq!(dash.bottlenecks[1].domain, DomainId::new("test_eval"));
    assert_eq!(dash.bottlenecks[1].gap, 2);
    // Every edge class shows up somewhere in the dashboard trace.
    for st in [
        EdgeState::Satisfied,
        EdgeState::Blocked,
        EdgeState::Degraded,
        EdgeState::Missing,
    ] {
        assert!(dash.edges.iter().any(|e| e.state == st), "no {st:?} edge");
    }
}

// ── TOML ─────────────────────────────────────────────────────────────────

#[test]
fn toml_accepts_level_names_and_rejects_bad_references() {
    let good = r#"
name = "t"
[domains.a]
name = "A"
maturity = "accessible"
[sources.s]
domain = "a"
name = "S"
kind = "file"
structure = 1
availability = 1
[outcomes.o]
name = "O"
requires = [ { domain = "a", min_maturity = "Structured", criticality = "blocking" } ]
"#;
    let s = Scenario::from_toml(good).unwrap();
    assert_eq!(s.domains[0].maturity, Maturity::Accessible);
    assert_eq!(s.outcomes[0].requires[0].min_maturity, Maturity::Structured);
    assert_eq!(s.sources[0].latency, Duration::ZERO);
    assert_eq!(trace(&s, &"o".into()).unwrap().achievable, Fidelity::Full);

    let dangling_source = good.replace(
        "domain = \"a\"\nname = \"S\"",
        "domain = \"zz\"\nname = \"S\"",
    );
    assert_eq!(
        Scenario::from_toml(&dangling_source),
        Err(ScenarioError::DanglingDomain("s".into(), "zz".into()))
    );
    let dangling_req = good.replace("{ domain = \"a\"", "{ domain = \"zz\"");
    assert_eq!(
        Scenario::from_toml(&dangling_req),
        Err(ScenarioError::DanglingRequirement("o".into(), "zz".into()))
    );
    let bad_level = good.replace("maturity = \"accessible\"", "maturity = 7");
    assert!(matches!(
        Scenario::from_toml(&bad_level),
        Err(ScenarioError::InvalidField(path, _)) if path == "domains.a.maturity"
    ));
    let bad_kind = good.replace("kind = \"file\"", "kind = \"cloud\"");
    assert!(matches!(
        Scenario::from_toml(&bad_kind),
        Err(ScenarioError::InvalidField(path, _)) if path == "sources.s.kind"
    ));
    let missing = good.replace("name = \"O\"\n", "");
    assert_eq!(
        Scenario::from_toml(&missing),
        Err(ScenarioError::MissingField("outcomes.o.name".into()))
    );
    assert!(matches!(
        Scenario::from_toml("name = [unterminated"),
        Err(ScenarioError::Toml(_))
    ));
}

// ── JSON ─────────────────────────────────────────────────────────────────

#[test]
fn scenario_round_trips_through_json() {
    let s = builtin::acquisition_status();
    let text = s.to_json_string();
    let back = Scenario::from_json_str(&text).unwrap();
    assert_eq!(back, s);
    // Order survives the trip (JSON arrays are ordered, unlike the TOML
    // tables the original was loaded from).
    assert_eq!(
        back.domains
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        s.domains.iter().map(|d| d.id.as_str()).collect::<Vec<_>>()
    );
    assert!(matches!(
        Scenario::from_json_str("{"),
        Err(ScenarioError::Json(_))
    ));
    assert_eq!(
        Scenario::from_json_str(
            r#"{"name":"x","domains":[],"sources":[],"outcomes":[{"id":"o","name":"O"}]}"#
        ),
        Err(ScenarioError::MissingField("outcomes[0].requires".into()))
    );
}

#[test]
fn report_json_carries_verdict_bottlenecks_and_edges() {
    let s = builtin::acquisition_status();
    let r = trace(&s, &OutcomeId::new("program_risk_rollup")).unwrap();
    let v = r.to_json();
    assert_eq!(v["outcome"].as_str(), Some("program_risk_rollup"));
    assert_eq!(v["achievable"]["kind"].as_str(), Some("partial"));
    let fraction = v["achievable"]["fraction"].as_f64().unwrap();
    assert!(fraction > 0.0 && fraction < 1.0);
    let b = &v["bottlenecks"][0];
    assert_eq!(b["state"].as_str(), Some("degraded"));
    assert_eq!(b["gap"].as_i64(), Some(1));
    let e = &v["edges"][0];
    assert_eq!(e["from"]["kind"].as_str(), Some("domain"));
    assert_eq!(e["to"]["kind"].as_str(), Some("outcome"));
    assert_eq!(e["state"].as_str(), Some("satisfied"));
    // It's valid JSON text too.
    let text = r.to_json_string();
    let reparsed: rusty_json::Value = rusty_json::from_str(&text).unwrap();
    assert_eq!(reparsed, v);
}

// ── Markdown ─────────────────────────────────────────────────────────────

#[test]
fn markdown_gap_summary_lists_bottlenecks_worst_first() {
    let s = builtin::acquisition_status();
    let r = trace(&s, &OutcomeId::new("acq_status_dashboard")).unwrap();
    let md = r.to_markdown(&s);
    assert!(md.starts_with("# Gap summary: Acquisition Status Dashboard\n"));
    assert!(md.contains("**Achievable today:** Not achievable"));
    let contracts = md.find("| 1 | Contracts |").expect("contracts row first");
    let te = md
        .find("| 2 | Test & Eval Status |")
        .expect("T&E row second");
    assert!(contracts < te);
    assert!(md.contains("**Blocking**"));
    assert!(md.contains("PDF contract mods (file)"));
    assert!(md.contains("- Schedule (IMS) -- L4 Integrated (Program Control)"));
    assert!(md.contains("_Maturity ladder: L0 Tribal · L1 Documented"));

    let full = trace(&s, &OutcomeId::new("cost_schedule_variance")).unwrap();
    let md = full.to_markdown(&s);
    assert!(md.contains("**Achievable today:** Full"));
    assert!(md.contains("No gaps to fund."));
}
