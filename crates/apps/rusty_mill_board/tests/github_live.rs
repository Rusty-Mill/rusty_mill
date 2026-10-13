//! Read-only check against the real GitHub API. Ignored by default:
//!
//! ```text
//! MILL_BOARD_LIVE_REPO=owner/name GITHUB_TOKEN=... \
//!     cargo test -p rusty_mill_board --test github_live -- --ignored --nocapture
//! ```
//! Only lists issues; it never writes.

use rusty_mill_board::github::{GithubConfig, GithubForge};
use rusty_mill_board::sync::Forge;

#[test]
#[ignore = "talks to the real GitHub API"]
fn lists_real_issues() {
    let repo = std::env::var("MILL_BOARD_LIVE_REPO").expect("MILL_BOARD_LIVE_REPO");
    let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN");
    let forge = GithubForge::new(
        GithubConfig::new("https://api.github.com", &repo, &token).expect("config"),
    )
    .expect("forge");
    let issues = forge.list_issues().expect("list issues");
    let open = issues.iter().filter(|i| !i.closed).count();
    println!("{repo}: {} issues ({open} open)", issues.len());
    for i in issues.iter().take(3) {
        println!(
            "  #{} [{}] {:?} labels={:?} updated_ms={}",
            i.number,
            if i.closed { "closed" } else { "open" },
            i.title,
            i.labels,
            i.updated_ms
        );
    }
    assert!(
        issues
            .iter()
            .all(|i| i.number > 0 && i.updated_ms > 1_500_000_000_000)
    );
}
