//! Renders measured rows as one Markdown table.

use std::fmt::Write as _;
use std::time::Duration;

use crate::cargo::{BuildTimes, Closure};
use crate::measure::Run;
use crate::sys::Ended;

/// Everything measured for one product. Each stage fails on its own, so a
/// build failure still reports the dependency closure. `build` is `Ok(None)`
/// when the binary was prebuilt rather than timed here.
#[derive(Debug)]
pub enum Row {
    Measured {
        bin: String,
        closure: Result<Closure, String>,
        build: Result<Option<BuildTimes>, String>,
        size: Option<u64>,
        run: Result<Run, String>,
    },
    Unsupported {
        bin: String,
        current: String,
        allowed: String,
    },
}

const HEADER: &str = "| Product | Binary | Deps (workspace + external) | Clean build | Incremental | Startup | Idle RSS | Peak RSS | Notes |\n|---|--:|--:|--:|--:|--:|--:|--:|---|\n";

/// The table, one row per product, in the order given. An exited
/// product's peak at or below `floor` (see `sys::floor`) shows as `≤ floor`.
pub fn render(rows: &[Row], floor: Option<u64>) -> String {
    let mut out = String::from(HEADER);
    for row in rows {
        // Writing to a `String` cannot fail.
        let _ = writeln!(out, "{}", render_row(row, floor));
    }
    out
}

fn render_row(row: &Row, floor: Option<u64>) -> String {
    if let Row::Unsupported {
        bin,
        current,
        allowed,
    } = row
    {
        return format!(
            "| `{bin}` | — | — | — | — | — | — | — | unsupported on {current} (allowed: {allowed}); skipped |"
        );
    }
    let Row::Measured {
        bin,
        closure,
        build,
        size,
        run,
    } = row
    else {
        unreachable!("unsupported rows returned above")
    };
    let mut notes = Vec::new();
    let deps = match closure {
        Ok(closure) => format!("{} + {}", closure.workspace, closure.external),
        Err(error) => note(&mut notes, "deps", error),
    };
    let (clean, incremental) = match build {
        Ok(Some(times)) => (seconds(times.clean), seconds(times.incremental)),
        Ok(None) => (DASH.to_owned(), DASH.to_owned()),
        Err(error) => (note(&mut notes, "build", error), DASH.to_owned()),
    };
    let (startup, idle, peak) = match run {
        Ok(Run::Exit {
            startup,
            peak_rss,
            ended,
        }) => {
            if *ended != Ended::Exited(0) {
                notes.push(format!("ended {ended:?}"));
            }
            let peak = match (peak_rss, floor) {
                (Some(peak), Some(floor)) if peak <= &floor => format!("≤ {}", bytes(Some(floor))),
                _ => bytes(*peak_rss),
            };
            (millis(*startup), DASH.to_owned(), peak)
        }
        Ok(Run::Idle { idle_rss, peak_rss }) => {
            (DASH.to_owned(), bytes(*idle_rss), bytes(*peak_rss))
        }
        Err(error) if build.is_ok() => (
            note(&mut notes, "run", error),
            DASH.to_owned(),
            DASH.to_owned(),
        ),
        Err(_) => (DASH.to_owned(), DASH.to_owned(), DASH.to_owned()),
    };
    format!(
        "| `{}` | {} | {deps} | {clean} | {incremental} | {startup} | {idle} | {peak} | {} |",
        bin,
        bytes(*size),
        notes.join("; ").replace('|', "\\|"),
    )
}

const DASH: &str = "—";

fn note(notes: &mut Vec<String>, stage: &str, error: &str) -> String {
    notes.push(format!("{stage}: {error}"));
    "failed".to_owned()
}

fn seconds(duration: Duration) -> String {
    format!("{:.1} s", duration.as_secs_f64())
}

fn millis(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

/// Bytes in MiB to one decimal, or a dash when unmeasured.
fn bytes(value: Option<u64>) -> String {
    value.map_or_else(
        || DASH.to_owned(),
        |bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> Result<Option<BuildTimes>, String> {
        Ok(Some(BuildTimes {
            clean: Duration::from_millis(61_250),
            incremental: Duration::from_millis(2_040),
        }))
    }

    #[test]
    fn renders_an_exit_row_and_flags_a_nonzero_exit() {
        let row = Row::Measured {
            bin: "rush".into(),
            closure: Ok(Closure {
                workspace: 12,
                external: 3,
            }),
            build: built(),
            size: Some(3 * 1_048_576),
            run: Ok(Run::Exit {
                startup: Duration::from_micros(1_500),
                peak_rss: Some(4 * 1_048_576),
                ended: Ended::Exited(2),
            }),
        };
        assert_eq!(
            render_row(&row, None),
            "| `rush` | 3.0 MiB | 12 + 3 | 61.2 s | 2.0 s | 1.5 ms | — | 4.0 MiB | ended Exited(2) |"
        );
        assert!(render_row(&row, Some(4 * 1_048_576)).contains("| ≤ 4.0 MiB |"));
        assert!(render_row(&row, Some(3 * 1_048_576)).contains("| 4.0 MiB |"));
    }

    #[test]
    fn a_failed_build_is_noted_once_and_dashes_the_rest() {
        let row = Row::Measured {
            bin: "hub".into(),
            closure: Ok(Closure {
                workspace: 1,
                external: 0,
            }),
            build: Err("no | linker".into()),
            size: None,
            run: Err("not built".into()),
        };
        assert_eq!(
            render_row(&row, None),
            "| `hub` | — | 1 + 0 | failed | — | — | — | — | build: no \\| linker |"
        );
    }

    #[test]
    fn renders_a_prebuilt_idle_row() {
        let row = Row::Measured {
            bin: "ts-daemon".into(),
            closure: Ok(Closure {
                workspace: 4,
                external: 0,
            }),
            build: Ok(None),
            size: Some(1_048_576),
            run: Ok(Run::Idle {
                idle_rss: Some(2 * 1_048_576),
                peak_rss: None,
            }),
        };
        assert_eq!(
            render_row(&row, None),
            "| `ts-daemon` | 1.0 MiB | 4 + 0 | — | — | — | 2.0 MiB | — |  |"
        );
    }

    #[test]
    fn renders_an_explicit_unsupported_row() {
        let row = Row::Unsupported {
            bin: "fedora".into(),
            current: "windows".into(),
            allowed: "linux".into(),
        };
        assert_eq!(
            render_row(&row, None),
            "| `fedora` | — | — | — | — | — | — | — | unsupported on windows (allowed: linux); skipped |"
        );
    }
}
