//! Runs the full probe suite as part of `cargo test --workspace`, so the
//! behavior matrix is exercised on every host in the CI matrix rather than
//! only when someone remembers to run the report binary.

use conformance::{run_all, Verdict, PROBES};

#[test]
fn every_probe_runs_without_erroring() {
    let results = run_all();

    assert_eq!(
        results.len(),
        PROBES.len(),
        "run_all() dropped probes: got {} of {}",
        results.len(),
        PROBES.len()
    );

    let errored: Vec<String> = results
        .iter()
        .filter(|r| r.verdict == Verdict::Errored)
        .map(|r| format!("{}: {}", r.id, r.detail))
        .collect();

    assert!(
        errored.is_empty(),
        "probes errored on this host:\n  {}",
        errored.join("\n  ")
    );

    // Print the evidence so a CI log shows what the host actually did, not
    // just that the assertions held.
    for result in &results {
        println!(
            "{:<20} {:<12} {}",
            result.id,
            result.verdict.as_str(),
            result.detail
        );
    }
}

#[test]
fn probe_ids_are_unique() {
    // Duplicate ids would make matrix rows collide silently, since rendering
    // looks a row up by id.
    let mut ids: Vec<&str> = PROBES.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    let count = ids.len();
    ids.dedup();
    assert_eq!(count, ids.len(), "duplicate probe id in PROBES");
}
