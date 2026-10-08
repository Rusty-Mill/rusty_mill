//! Real-replay guards on decoded identity and geometry: the bundled samples follow
//! the coordinate convention the position metrics assume, and carry unique,
//! stable platform ids.
use replay_analyzer::analyze::coords::coordinate_report;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;

fn canonical(name: &str) -> replay_analyzer::CanonicalMatch {
    let path = format!(
        "{}/../../rleval/assets/replays/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let decoded = BoxcarsParser::new()
        .parse(&bytes)
        .unwrap_or_else(|e| panic!("decode {name}: {e}"));
    replay_analyzer::build_canonical(&decoded, name)
}

#[test]
fn sample_replays_follow_the_standard_convention() {
    for name in ["42f2.replay", "419a.replay"] {
        let r = coordinate_report(&canonical(name));
        assert!(r.kickoffs_checked >= 1, "{name}: no usable kickoff");
        assert_eq!(r.blue_defends_negative_y(), Some(true), "{name}: {r:?}");
        assert!(r.issues(true).is_empty(), "{name}: {:?}", r.issues(true));
    }
}

#[test]
fn sample_replays_carry_stable_platform_ids() {
    for name in ["42f2.replay", "419a.replay"] {
        let m = canonical(name);
        let ids: Vec<_> = m
            .players
            .iter()
            .filter_map(|p| p.platform_id.as_deref())
            .collect();
        assert!(!ids.is_empty(), "{name}: no platform ids decoded");
        assert!(ids.iter().all(|i| i.contains(':')), "{name}: {ids:?}");
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            ids.len(),
            "{name}: ids must be unique per player"
        );
    }
}
