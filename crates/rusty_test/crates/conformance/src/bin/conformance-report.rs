//! Emits and merges conformance reports.
//!
//! ```text
//! conformance-report probe                       > report-linux.tsv
//! conformance-report matrix CONTRACT.md \
//!     Windows=report-windows.tsv \
//!     Linux=report-linux.tsv \
//!     macOS=report-macos.tsv            # prints the regenerated document
//! conformance-report check  <same args>          # exits 1 on drift
//! conformance-report write  <same args>          # rewrites CONTRACT.md
//! ```

use std::path::Path;
use std::process::ExitCode;

use conformance::{parse_tsv, render_matrix, render_tsv, run_all, splice_matrix, ProbeResult};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");

    let result = match command {
        "probe" => probe(),
        "matrix" | "check" | "write" => merge(command, &args[1..]),
        other => Err(format!(
            "unknown command {other:?}; expected probe|matrix|check|write"
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("conformance-report: {message}");
            ExitCode::FAILURE
        }
    }
}

fn probe() -> Result<(), String> {
    let results = run_all();
    print!("{}", render_tsv(&results));

    // A probe that could not run makes every other row untrustworthy, so the
    // reporting step fails rather than emitting a report with a hole in it.
    let errored: Vec<&ProbeResult> = results
        .iter()
        .filter(|r| r.verdict == conformance::Verdict::Errored)
        .collect();
    if errored.is_empty() {
        return Ok(());
    }
    let detail = errored
        .iter()
        .map(|r| format!("{}: {}", r.id, r.detail))
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!("{} probe(s) errored -> {detail}", errored.len()))
}

/// Parses `Host=path.tsv` pairs, preserving the order given on the command
/// line so the generated column order is the caller's choice, not a map's.
fn load_hosts(specs: &[String]) -> Result<Vec<(String, Vec<ProbeResult>)>, String> {
    if specs.is_empty() {
        return Err("expected at least one Host=report.tsv argument".to_string());
    }
    let mut hosts = Vec::new();
    for spec in specs {
        let (host, path) = spec
            .split_once('=')
            .ok_or_else(|| format!("expected Host=path.tsv, got {spec:?}"))?;
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        hosts.push((host.to_string(), parse_tsv(&text)?));
    }
    Ok(hosts)
}

fn merge(command: &str, args: &[String]) -> Result<(), String> {
    let document_path = args
        .first()
        .ok_or_else(|| "expected a CONTRACT.md path".to_string())?;
    let hosts = load_hosts(&args[1..])?;
    let document = std::fs::read_to_string(document_path)
        .map_err(|e| format!("reading {document_path}: {e}"))?;
    let regenerated = splice_matrix(&document, &render_matrix(&hosts))?;

    match command {
        "matrix" => {
            print!("{regenerated}");
            Ok(())
        }
        "write" => std::fs::write(Path::new(document_path), regenerated)
            .map_err(|e| format!("writing {document_path}: {e}")),
        "check" if regenerated == document => {
            println!("behavior matrix is up to date with the probe reports");
            Ok(())
        }
        "check" => Err(format!(
            "{document_path} is stale. The committed behavior matrix does not match what the \
             probes reported on this run. Regenerate with `conformance-report write` and commit \
             the result."
        )),
        other => Err(format!("unknown merge command {other:?}")),
    }
}
