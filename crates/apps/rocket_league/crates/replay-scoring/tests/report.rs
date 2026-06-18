//! Tests for report assembly + rendering (lobby table, SVG heatmaps, HTML).

use std::collections::BTreeMap;

use replay_scoring::config::Role;
use replay_scoring::heatmap::{render_svg, Occupancy};
use replay_scoring::lobby::{LobbyReport, MetricRow};
use replay_scoring::render::html;
use replay_scoring::report::{Confidence, MetricBreakdown, Report};

fn mk_report(name: &str, pri: i32, team: i32, composite: f32) -> Report {
    Report {
        replay_id: "r".into(),
        score_config_version: "scfg-test".into(),
        parser_version: "p".into(),
        analyzer_version: "a".into(),
        target_player: name.into(),
        target_pri: pri,
        target_team: Some(team),
        composite,
        first_man: 50.0,
        second_man: 40.0,
        general: 30.0,
        licence: "Silver Licence".into(),
        player_type: "Calm Controller".into(),
        main_leak: "boost_management".into(),
        focus_chapter: "Fundamentals / boost".into(),
        confidence: Confidence::Ok,
        metrics: vec![MetricBreakdown {
            key: "boost_management".into(),
            role: Role::General,
            raw: Some(0.1),
            normalized: composite,
            effective_weight: 0.01,
        }],
    }
}

#[test]
fn render_svg_has_cells_furniture_and_touches() {
    // One hot cell (index 1 = ix=1,iy=0) out of a 2x2 grid.
    let occ = Occupancy {
        nx: 2,
        ny: 2,
        counts: vec![0, 4, 0, 0],
        max: 4,
        samples: 4,
    };
    let svg = render_svg(&occ, &[(0.0, 0.0)]);
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("viewBox=\"0 0 40 40\"")); // nx*20 x ny*20
    assert!(svg.contains("fill-opacity")); // the single hot cell
    assert!(svg.contains("<circle")); // touch marker + centre circle
    assert!(svg.trim_end().ends_with("</svg>"));
    // Empty cells are skipped: exactly one occupancy rect is drawn.
    assert_eq!(svg.matches("fill-opacity").count(), 1);
}

#[test]
fn html_report_has_sections_and_escapes_hostile_names() {
    let players = vec![
        mk_report("Alice", 1, 0, 60.0),
        mk_report("<script>", 2, 1, 40.0),
    ];
    let comparison = vec![MetricRow {
        key: "boost_management".into(),
        normalized: vec![60.0, 40.0],
        leader: Some(0),
    }];
    let lobby = LobbyReport {
        replay_id: "rid".into(),
        map: Some("DFH Stadium".into()),
        duration_s: 312.0,
        score_config_version: "scfg-test".into(),
        team_scores: BTreeMap::from([(0, 3), (1, 2)]),
        players,
        comparison,
    };

    let out = html(&lobby, &[(1, "<svg>HEAT</svg>".to_string())]);
    assert!(out.starts_with("<!doctype html"));
    assert!(out.contains("Lobby comparison"));
    assert!(out.contains("Alice"));
    assert!(out.contains("DFH Stadium"));
    assert!(out.contains("5:12")); // duration mm:ss
    assert!(out.contains("<svg>HEAT</svg>")); // heatmap embedded for pri 1
                                              // A hostile player name is escaped, never injected as a tag.
    assert!(out.contains("&lt;script&gt;"));
    assert!(!out.contains("<script>"));
}

#[test]
fn html_for_player_scopes_to_one_card() {
    let players = vec![mk_report("Alice", 1, 0, 60.0), mk_report("Bob", 2, 1, 40.0)];
    let comparison = vec![MetricRow {
        key: "boost_management".into(),
        normalized: vec![60.0, 40.0],
        leader: Some(0),
    }];
    let lobby = LobbyReport {
        replay_id: "rid".into(),
        map: Some("DFH Stadium".into()),
        duration_s: 312.0,
        score_config_version: "scfg-test".into(),
        team_scores: BTreeMap::from([(0, 3), (1, 2)]),
        players,
        comparison,
    };

    let full = html(&lobby, &[]);
    assert_eq!(
        full.matches("<article class=\"card").count(),
        2,
        "full report renders every player card"
    );

    let scoped = replay_scoring::render::html_for_player(&lobby, &[], 1);
    assert_eq!(
        scoped.matches("<article class=\"card").count(),
        1,
        "scoped report renders only the focus player's card"
    );
    assert!(scoped.contains("Decision-discipline report — Alice")); // focus name in header
    assert!(scoped.contains("<h3>Alice</h3>"));
    assert!(!scoped.contains("<h3>Bob</h3>")); // Bob's card is dropped
    assert!(scoped.contains("Bob")); // ...but he's still in the comparison table
    assert!(scoped.contains("focus")); // the focus column is marked

    // An unknown focus pri falls back to the full report.
    let fallback = replay_scoring::render::html_for_player(&lobby, &[], 999);
    assert_eq!(fallback.matches("<article class=\"card").count(), 2);
}

/// End-to-end smoke test: assemble a real corpus replay if one is present.
#[test]
fn assembles_on_real_replay_if_present() {
    use replay_analyzer::analyze::build_canonical;
    use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
    use replay_analyzer::decode::ReplayParser;
    use replay_scoring::config::ScoreConfig;
    use replay_scoring::heatmap::occupancy;
    use replay_scoring::lobby::assemble;
    use std::path::Path;

    let dir = Path::new("assets/corpus");
    let Some(file) = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "replay"))
    else {
        eprintln!("skip: no corpus .replay present");
        return;
    };

    let data = std::fs::read(&file).unwrap();
    let decoded = BoxcarsParser::new().parse(&data).unwrap();
    let m = build_canonical(&decoded, "test");
    let cfg = ScoreConfig::default();

    let lobby = assemble(&m, &cfg);
    assert!(!lobby.players.is_empty());
    assert_eq!(lobby.comparison.len(), cfg.metrics.len());
    for row in &lobby.comparison {
        assert_eq!(row.normalized.len(), lobby.players.len());
        if let Some(i) = row.leader {
            assert!(i < lobby.players.len());
        }
    }

    let occ = occupancy(&m, lobby.players[0].target_pri, 12, 15);
    assert_eq!(occ.counts.len(), 12 * 15);
    assert!(occ.samples > 0);
}
