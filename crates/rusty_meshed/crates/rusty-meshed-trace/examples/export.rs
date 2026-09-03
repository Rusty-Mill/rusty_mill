//! Dumps the bundled scenario and every outcome's trace as JSON (the
//! shape `data-mesh-monitor`'s reverse-trace view consumes), or as
//! Markdown gap summaries.
//!
//! ```text
//! cargo run -p rusty-meshed-trace --example export            # JSON
//! cargo run -p rusty-meshed-trace --example export -- --markdown
//! cargo run -p rusty-meshed-trace --example export -- path/to/scenario.toml
//! ```

use std::process::ExitCode;

use rusty_json::json;
use rusty_meshed_trace::{builtin, trace_all, Scenario};

fn main() -> ExitCode {
    let mut markdown = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        if arg == "--markdown" {
            markdown = true;
        } else {
            path = Some(arg);
        }
    }

    let scenario = match path {
        None => builtin::acquisition_status(),
        Some(p) => {
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("cannot read {p}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match Scenario::from_toml(&text) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{p}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let reports = match trace_all(&scenario) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("trace failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    if markdown {
        for report in &reports {
            println!("{}", report.to_markdown(&scenario));
        }
    } else {
        let doc = json!({
            "scenario": scenario.to_json(),
            "reports": reports.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            rusty_json::to_string_pretty(&doc).expect("a JSON Value always serialises")
        );
    }
    ExitCode::SUCCESS
}
