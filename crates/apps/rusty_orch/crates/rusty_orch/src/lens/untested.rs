//! `rusty_orch lens untested`: the open-time check for Windows-only intent
//! lines (#600, card #1405; D178). Reads the frozen `INTENT.md` and the
//! task's Windows mapping file and prints one JSON line; the check itself
//! is `orch_lens::untested`. Read-only.

use std::path::PathBuf;

use orch_lens::untested::{self, Report};
use rusty_serde::Serialize;

use crate::args::ArgsError;

pub const USAGE: &str = "usage: rusty_orch lens untested <INTENT.md> --map <windows-lines.json>\n\nCheck the frozen intent's Windows-only lines at task open. Every line the mapping file lists\nmust be a line of the intent; a listed line with no golden case or shadow run is untested for\nthe Linux runs. Prints one JSON line: {\"untested\":[...],\"covered\":[{\"line\",\"by\"}]}.\nFails, naming the entry, when the map is malformed or names a line the intent lacks.";

/// What to check.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub intent: PathBuf,
    pub map: PathBuf,
}

/// Parse `untested <INTENT.md> --map <file>`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    if args.next().as_deref() != Some("untested") {
        return Err(ArgsError::Help);
    }
    let (mut intent, mut map) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--map" if map.is_some() => return Err(ArgsError::Usage("duplicate --map".into())),
            "--map" => {
                let value = args
                    .next()
                    .filter(|v| !v.is_empty() && !v.starts_with('-'))
                    .ok_or_else(|| ArgsError::Usage("--map needs a path".into()))?;
                map = Some(PathBuf::from(value));
            }
            _ if !arg.starts_with('-') && intent.is_none() => intent = Some(PathBuf::from(arg)),
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        }
    }
    Ok(Options {
        intent: intent.ok_or_else(|| ArgsError::Usage("the intent file is required".into()))?,
        map: map.ok_or_else(|| ArgsError::Usage("--map is required".into()))?,
    })
}

/// One covered line in the output.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Covered {
    line: String,
    by: String,
}

/// The output's one line.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Out {
    untested: Vec<String>,
    covered: Vec<Covered>,
}

/// Read both files and check them.
pub fn run(o: &Options) -> Result<String, String> {
    let read = |p: &PathBuf| {
        std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
    };
    let map =
        untested::parse_map(&read(&o.map)?).map_err(|e| format!("{}: {e}", o.map.display()))?;
    let report = untested::check(&read(&o.intent)?, &map)
        .map_err(|e| format!("{}: {e}", o.map.display()))?;
    render(report)
}

fn render(r: Report) -> Result<String, String> {
    let out = Out {
        untested: r.untested,
        covered: r
            .covered
            .into_iter()
            .map(|(line, c)| Covered { line, by: c.word() })
            .collect(),
    };
    let json = rusty_serde::json::to_string(&out).map_err(|e| e.to_string())?;
    Ok(format!("{json}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_need_the_intent_and_the_map() {
        let o = options(argv("untested INTENT.md --map w.json")).expect("parses");
        assert_eq!(o.intent, PathBuf::from("INTENT.md"));
        assert_eq!(o.map, PathBuf::from("w.json"));
        let usage = |s: &str| matches!(options(argv(s)), Err(ArgsError::Usage(_)));
        assert!(usage("untested INTENT.md"));
        assert!(usage("untested --map w.json"));
        assert!(usage("untested INTENT.md --map"));
        assert!(usage("untested INTENT.md --map a --map b"));
        assert!(usage("untested INTENT.md other.md --map a"));
        assert!(matches!(options(argv("untested -h")), Err(ArgsError::Help)));
    }

    #[test]
    fn run_prints_one_json_line_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("orch-untested-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let intent = dir.join("INTENT.md");
        std::fs::write(
            &intent,
            "platform: Windows first\nalways:\n  - a desktop GUI\n",
        )
        .expect("write intent");
        let map = dir.join("w.json");
        std::fs::write(
            &map,
            r#"{"lines": [{"line": "platform: Windows first", "covered_by": "shadow"},
                          {"line": "a desktop GUI", "covered_by": null}]}"#,
        )
        .expect("write map");
        let o = Options {
            intent: intent.clone(),
            map: map.clone(),
        };
        assert_eq!(
            run(&o).expect("runs"),
            "{\"untested\":[\"a desktop GUI\"],\"covered\":[{\"line\":\"platform: Windows first\",\"by\":\"shadow\"}]}\n"
        );
        std::fs::write(&map, r#"{"lines": [{"line": "gone", "covered_by": null}]}"#)
            .expect("rewrite map");
        assert!(run(&o)
            .expect_err("stale map")
            .contains("not in the intent"));
        let missing = Options {
            intent: dir.join("nope.md"),
            map,
        };
        assert!(run(&missing).expect_err("missing").contains("cannot read"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
