//! The v0 lens config and prompts (card #1123) load and read as designed.
//! `lens check` never reads the prompt files, so this test does.

use std::path::PathBuf;

use orch_lens::LensConfig;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lens")
}

fn config() -> LensConfig {
    let text = std::fs::read_to_string(dir().join("lens.json")).expect("read lens.json");
    LensConfig::parse(&text).expect("lens.json parses")
}

#[test]
fn the_config_has_the_four_v0_lenses() {
    let ids: Vec<String> = config().lenses.iter().map(|l| l.id.clone()).collect();
    assert_eq!(ids, ["architect", "research", "adversary", "operator"]);
}

#[test]
fn every_prompt_file_exists_and_is_plain_text() {
    for lens in config().lenses {
        let text = std::fs::read_to_string(dir().join(&lens.prompt))
            .unwrap_or_else(|e| panic!("{}: {e}", lens.prompt));
        assert!(!text.trim().is_empty(), "{} is empty", lens.prompt);
        // orch sends the file verbatim under its own `# Your lens` heading.
        assert!(!text.starts_with("---"), "{} has front matter", lens.prompt);
        assert!(
            !text.starts_with('#'),
            "{} opens with a heading",
            lens.prompt
        );
    }
}

#[test]
fn every_prompt_carries_the_shared_rules_and_forbids_executed_evidence() {
    for lens in config().lenses {
        let text = std::fs::read_to_string(dir().join(&lens.prompt)).expect("read prompt");
        assert!(text.contains("Rules for every post:"), "{}", lens.prompt);
        assert!(
            text.contains("Do not post executed evidence"),
            "{} lacks the executed-evidence rule",
            lens.prompt
        );
    }
}
