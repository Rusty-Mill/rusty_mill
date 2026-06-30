//! `rleval` — the unified RLEvalSystem application.
//!
//! One binary that replaces juggling the separate `replay-analyzer`,
//! `replay-scoring`, `replay-skills`, `replay-value` and `replay-viewer` CLIs:
//! it serves a web UI where you drop a `.replay` and get the 3D viewer, the
//! scoring report, the skills table and the value-impact analysis side by side,
//! all computed in-process by the [`pipeline`].
//!
//! ```text
//! rleval serve [--host <h>] [--port <p>] [--replays <dir>]
//! rleval analyze <file.replay> [--out <bundle.html>]   # one-shot, no server
//! ```

use std::error::Error;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rleval_app::server::{self, Request, Response};
use rleval_app::{admin, pipeline, ui};

const USAGE: &str = "\
usage:
  rleval serve [--host <host>] [--port <port>] [--replays <dir>] [--corpus <dir>] [--enable-admin-run]
  rleval analyze <file.replay> [--out <bundle.html>]

  serve    start the web UI (default http://127.0.0.1:8080; /admin shows model config)
           --enable-admin-run lets /admin trigger retrain/recalibrate (localhost only)
  analyze  run the pipeline on one replay and write a self-contained HTML bundle";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") | None => serve(args.collect()),
        Some("analyze") => analyze_oneshot(args.collect()),
        Some("-h") | Some("--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}\n{USAGE}").into()),
    }
}

// ---- serve ----

fn serve(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8080;
    let mut replays = PathBuf::from("assets/replays");
    let mut corpus = PathBuf::from("assets/corpus");
    let mut allow_run = false;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => host = it.next().ok_or("--host needs a value")?,
            "--port" => port = it.next().ok_or("--port needs a value")?.parse()?,
            "--replays" => replays = PathBuf::from(it.next().ok_or("--replays needs a dir")?),
            "--corpus" => corpus = PathBuf::from(it.next().ok_or("--corpus needs a dir")?),
            "--enable-admin-run" => allow_run = true,
            other => return Err(format!("unknown flag {other}\n{USAGE}").into()),
        }
    }

    eprintln!(
        "RLEval unified app serving on http://{host}:{port}  (sample dir: {})",
        replays.display()
    );
    eprintln!("Open the URL in a browser, then drop a .replay file. Ctrl-C to stop.");
    if allow_run {
        eprintln!("admin maintenance endpoint ENABLED (/api/admin/run) — localhost only.");
    }
    server::serve(&host, port, move |req| {
        route(req, &replays, &corpus, allow_run)
    })?;
    Ok(())
}

fn route(req: &Request, replays: &Path, corpus: &Path, allow_run: bool) -> Response {
    match (req.method.as_str(), req.route()) {
        ("GET", "/") => Response::html(ui::INDEX_HTML),
        ("GET", "/admin") => Response::html(ui::ADMIN_HTML),
        ("GET", "/healthz") => Response::text(200, "ok"),
        ("GET", "/api/samples") => samples_response(replays),
        ("GET", "/api/config") => match serde_json::to_vec(&admin::config_report(corpus, allow_run)) {
            Ok(json) => Response::json(json),
            Err(e) => Response::text(500, format!("serialize error: {e}")),
        },
        ("POST", "/api/admin/run") => {
            if !allow_run {
                return Response::text(403, "admin run disabled — start with --enable-admin-run");
            }
            let action = query_param(&req.path, "action").unwrap_or_default();
            match admin::run_action(&action, corpus) {
                Some(r) => Response::json(serde_json::to_vec(&r).unwrap_or_default()),
                None => Response::text(400, "unknown action"),
            }
        }
        ("POST", "/api/analyze") => {
            let name = query_param(&req.path, "name").unwrap_or_else(|| "upload".to_string());
            // Optional explicit rank bracket override (?rank=diamond); absent ⇒
            // the lobby's level is inferred.
            let rank = query_param(&req.path, "rank");
            analyze_response(&req.body, &stem(&name), corpus, rank.as_deref())
        }
        ("GET", path) if path.starts_with("/api/analyze/sample/") => {
            let name = path.trim_start_matches("/api/analyze/sample/");
            let name = percent_decode(name);
            // Guard against path traversal — sample names are flat filenames.
            if name.contains('/') || name.contains("..") {
                return Response::text(400, "invalid sample name");
            }
            let rank = query_param(&req.path, "rank");
            match std::fs::read(replays.join(&name)) {
                Ok(bytes) => analyze_response(&bytes, &stem(&name), corpus, rank.as_deref()),
                Err(e) => Response::text(404, format!("sample not found: {e}")),
            }
        }
        _ => Response::not_found(),
    }
}

/// Run the pipeline and return the JSON bundle, isolating decoder panics. Loads
/// the rank-relative norms from `corpus` (if present) so scores are graded
/// against their bracket; `override_bracket` pins an explicit rank when known.
fn analyze_response(
    bytes: &[u8],
    replay_id: &str,
    corpus: &Path,
    override_bracket: Option<&str>,
) -> Response {
    if bytes.is_empty() {
        return Response::text(400, "empty request body — no replay bytes");
    }
    let norms = pipeline::load_rank_norms(corpus);
    let result = catch_unwind(AssertUnwindSafe(|| {
        pipeline::analyze(bytes, replay_id, norms.as_ref(), override_bracket)
    }));
    match result {
        Ok(Ok(analysis)) => match serde_json::to_vec(&analysis) {
            Ok(json) => Response::json(json),
            Err(e) => Response::text(500, format!("serialize error: {e}")),
        },
        Ok(Err(e)) => Response::text(400, format!("could not analyze replay: {e}")),
        Err(_) => Response::text(
            400,
            "could not decode replay (the file may be corrupt or unsupported)",
        ),
    }
}

fn samples_response(replays: &Path) -> Response {
    let mut names: Vec<String> = std::fs::read_dir(replays)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|s| s.to_str()) == Some("replay"))
                .then(|| p.file_name()?.to_str().map(String::from))
                .flatten()
        })
        .collect();
    names.sort();
    let body = serde_json::json!({ "samples": names });
    Response::json(serde_json::to_vec(&body).unwrap_or_default())
}

// ---- analyze (one-shot, no server) ----

fn analyze_oneshot(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let mut replay: Option<String> = None;
    let mut out: Option<String> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(it.next().ok_or("--out needs a path")?),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}\n{USAGE}").into())
            }
            other if replay.is_none() => replay = Some(other.to_string()),
            other => return Err(format!("unexpected arg {other}").into()),
        }
    }
    let replay = replay.ok_or(USAGE)?;
    let bytes = std::fs::read(&replay)?;
    let id = stem(&replay);
    // Use the default corpus norms if present, so the static bundle also carries
    // the rank-relative layer; absent ⇒ purely absolute.
    let norms = pipeline::load_rank_norms(Path::new("assets/corpus"));
    let analysis = pipeline::analyze(&bytes, &id, norms.as_ref(), None)?;

    let out = out.unwrap_or_else(|| format!("{id}.html"));
    std::fs::write(&out, bundle_html(&analysis))?;
    eprintln!(
        "analyzed {id}: {} players, {} skills, {} value rows -> {out}",
        analysis.scores.len(),
        analysis.skill_profiles.len(),
        analysis.impact.players.len(),
    );
    Ok(())
}

/// A static, self-contained HTML bundle (no server): the index shell with the
/// analysis JSON inlined so it renders immediately when opened in a browser.
fn bundle_html(analysis: &pipeline::Analysis) -> String {
    let json = serde_json::to_string(analysis).unwrap_or_else(|_| "null".into());
    // Splice the data in just before the closing script: define a global the UI
    // picks up, and skip the network fetch.
    let inject = format!(
        "<script>window.__RLEVAL_DATA__ = {json};</script>\n",
        json = json.replace("</script>", "<\\/script>"),
    );
    let mut doc = ui::INDEX_HTML.to_string();
    // Drop the bundled data right before </body>, plus a tiny bootstrap.
    let bootstrap = "<script>if(window.__RLEVAL_DATA__){DATA=window.__RLEVAL_DATA__;render();\
        document.getElementById('loader').style.display='none';}</script>";
    doc = doc.replace("</body>", &format!("{inject}{bootstrap}</body>"));
    doc
}

// ---- small helpers ----

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string()
}

/// Pull a query parameter value out of a raw request path.
fn query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Minimal percent-decoding (enough for filenames and query values).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_strips_dir_and_extension() {
        assert_eq!(stem("assets/replays/42f2.replay"), "42f2");
        assert_eq!(stem("upload"), "upload");
    }

    #[test]
    fn query_param_extracts_and_decodes() {
        assert_eq!(
            query_param("/api/analyze?name=my%20game.replay", "name").as_deref(),
            Some("my game.replay")
        );
        assert_eq!(query_param("/api/analyze", "name"), None);
        assert_eq!(query_param("/x?a=1&b=2", "b").as_deref(), Some("2"));
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
