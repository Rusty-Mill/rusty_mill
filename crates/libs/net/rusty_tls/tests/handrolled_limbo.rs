//! The certificate path validator against x509-limbo, next to webpki.
//!
//! x509-limbo (<https://github.com/C2SP/x509-limbo>) is a corpus of roughly ten
//! thousand certificate-validation cases written by people who were not
//! writing this validator, which is the property no hand-written test here can
//! have. Each case is a trust store, some intermediates, a leaf, a time and a
//! name, plus the verdict limbo says a conforming validator gives.
//!
//! Three things are compared per case:
//!
//! * **ours** -- `handrolled::path::verify_peer_certificate`;
//! * **theirs** -- rustls' webpki (`verify_server_name` plus
//!   `verify_server_cert_signed_by_trust_anchor`), the engine this one would
//!   replace;
//! * **limbo's expectation**.
//!
//! The question the evidence bar asks is only about ours against theirs, and
//! in one direction: a case ours *accepts* and webpki *rejects* ("looser") is a
//! potential forged-certificate acceptance. A case ours rejects and webpki
//! accepts ("stricter") is a compatibility cost, not a vulnerability. The
//! report lists both, by limbo id, and `tests/data/limbo_divergences.txt`
//! records the ones the owner has looked at.
//!
//! Run by hand or in CI; `#[ignore]`d because it needs the corpus:
//!
//! ```text
//! LIMBO_JSON=/path/to/limbo.json RUSTFLAGS='--cfg rusty_tls_handrolled' \
//!   cargo test -p rusty_tls --features handrolled-engine \
//!   --test handrolled_limbo -- --ignored --nocapture
//! ```

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use rustls::client::verify_server_cert_signed_by_trust_anchor;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::RootCertStore;
use rusty_json::Value;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{verify_peer_certificate, PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::{oid, Certificate};

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2024-01-01T00:00:00+00:00` (limbo always writes UTC) to Unix seconds.
fn parse_time(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let clock = rest.get(..8)?;
    if !rest[8..].starts_with("+00:00") && !rest[8..].starts_with('Z') {
        return None;
    }
    let mut c = clock.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
    );
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + sec)
}

fn pems(value: Option<&Value>) -> Vec<Vec<u8>> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|pem| CertificateDer::from_pem_slice(pem.as_bytes()).ok())
                .map(|der| der.as_ref().to_vec())
                .collect()
        })
        .unwrap_or_default()
}

fn strings(value: Option<&Value>) -> Vec<&str> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// One verdict. `Skipped` carries why the case was not comparable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    Accept,
    Reject,
}

enum Outcome {
    Compared { ours: Verdict, theirs: Verdict },
    Skipped(&'static str),
}

fn run_case(case: &Value) -> (Outcome, bool) {
    let expected_ok = case.get("expected_result").and_then(Value::as_str) == Some("SUCCESS");
    if case.get("validation_kind").and_then(Value::as_str) != Some("SERVER") {
        return (Outcome::Skipped("client validation"), expected_ok);
    }
    if !strings(case.get("extended_key_usage")).is_empty()
        || !strings(case.get("key_usage")).is_empty()
    {
        return (
            Outcome::Skipped("custom key usage requirement"),
            expected_ok,
        );
    }
    if case.get("max_chain_depth").is_some_and(|v| !v.is_null()) {
        return (Outcome::Skipped("max chain depth configured"), expected_ok);
    }
    let name = case.get("expected_peer_name");
    let kind = name.and_then(|n| n.get("kind")).and_then(Value::as_str);
    let value = name.and_then(|n| n.get("value")).and_then(Value::as_str);
    let (Some(kind), Some(value)) = (kind, value) else {
        return (Outcome::Skipped("no peer name"), expected_ok);
    };
    // A case with no `validation_time` leaves the instant to the validator.
    // Both engines are given the same one, so the differential stays
    // meaningful; it is picked after parsing, below.
    let stated_time = match case.get("validation_time") {
        Some(Value::String(s)) => match parse_time(s) {
            Some(t) => Some(t),
            None => return (Outcome::Skipped("unparsable validation time"), expected_ok),
        },
        _ => None,
    };

    let roots = pems(case.get("trusted_certs"));
    let intermediates = pems(case.get("untrusted_intermediates"));
    let leaf = pems(Some(&Value::Array(
        case.get("peer_certificate").cloned().into_iter().collect(),
    )));
    let Some(leaf) = leaf.into_iter().next() else {
        return (Outcome::Skipped("peer certificate is not PEM"), expected_ok);
    };

    // With no stated time: one second after the newest `notBefore` of the
    // peer's own certificates, which is the earliest instant they could all be
    // in force. If a case is built to be expired it states its time.
    let time = stated_time.unwrap_or_else(|| {
        std::iter::once(&leaf)
            .chain(intermediates.iter())
            .filter_map(|der| Certificate::parse(der).ok())
            .map(|c| c.validity().not_before)
            .max()
            .map_or(0, |t| t + 1)
    });

    // ---- ours ----
    let ours = (|| {
        let end_entity = Certificate::parse(&leaf).ok()?;
        let mut parsed_roots = Vec::new();
        for der in &roots {
            parsed_roots.push(Certificate::parse(der).ok()?);
        }
        let anchors: Vec<TrustAnchor<'_>> = parsed_roots
            .iter()
            .map(TrustAnchor::from_certificate)
            .collect();
        let mut parsed_intermediates = Vec::new();
        for der in &intermediates {
            // An unparsable intermediate is simply not usable; it must not
            // veto a chain that does not need it.
            if let Ok(c) = Certificate::parse(der) {
                parsed_intermediates.push(c);
            }
        }
        let ip;
        let server_name = match kind {
            "DNS" => ServerName::Dns(value),
            "IP" => {
                ip = value.parse::<IpAddr>().ok()?;
                ServerName::Ip(ip)
            }
            _ => return None,
        };
        let options = PathOptions {
            time,
            required_eku: Some(oid::EKU_SERVER_AUTH),
            ..PathOptions::default()
        };
        verify_peer_certificate(
            &end_entity,
            &parsed_intermediates,
            &anchors,
            &server_name,
            &options,
        )
        .ok()
    })()
    .map_or(Verdict::Reject, |_| Verdict::Accept);
    if kind != "DNS" && kind != "IP" {
        return (Outcome::Skipped("peer name kind not modelled"), expected_ok);
    }

    // ---- theirs ----
    let theirs = (|| {
        let mut store = RootCertStore::empty();
        for der in &roots {
            let _ = store.add(CertificateDer::from(der.clone()));
        }
        let end_entity = CertificateDer::from(leaf.clone());
        let parsed = rustls::server::ParsedCertificate::try_from(&end_entity).ok()?;
        let name = match kind {
            "DNS" => rustls::pki_types::ServerName::try_from(value.to_string()).ok()?,
            _ => rustls::pki_types::ServerName::IpAddress(value.parse::<IpAddr>().ok()?.into()),
        };
        let inters: Vec<CertificateDer<'_>> = intermediates
            .iter()
            .map(|d| CertificateDer::from(d.clone()))
            .collect();
        rustls::client::verify_server_name(&parsed, &name).ok()?;
        verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &store,
            &inters,
            UnixTime::since_unix_epoch(Duration::from_secs(time.max(0) as u64)),
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .all,
        )
        .ok()
    })()
    .map_or(Verdict::Reject, |_| Verdict::Accept);

    (Outcome::Compared { ours, theirs }, expected_ok)
}

/// One case's result, as the harness saw it.
struct CaseResult {
    id: String,
    outcome: Outcome,
    /// What limbo says a conforming validator does.
    expected_ok: bool,
}

fn word(accept: bool) -> &'static str {
    if accept {
        "accept"
    } else {
        "reject"
    }
}

/// Everything the golden file pins, as sorted lines.
///
/// * how many cases were compared and how many were skipped, and why -- so
///   losing coverage (a parse change that skips thousands of cases) is a diff,
///   not a quiet pass;
/// * how many cases fell in each (limbo, ours, webpki) outcome -- so cases
///   that agreed and flip together are counted;
/// * a digest of every case's id and outcome -- so a flip that keeps every
///   count unchanged is still caught;
/// * every case where this validator and webpki differ, or both accept what
///   limbo says to refuse (agreeing with webpki is not the same as being
///   right).
fn summarize(results: &[CaseResult]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    let mut outcomes: BTreeMap<(bool, bool, bool), usize> = BTreeMap::new();
    let mut digest_input: Vec<String> = Vec::new();
    let mut compared = 0usize;

    for r in results {
        match &r.outcome {
            Outcome::Skipped(why) => {
                *skipped.entry(why).or_default() += 1;
                digest_input.push(format!("{} skipped {why}", r.id));
            }
            Outcome::Compared { ours, theirs } => {
                let (ours, theirs) = (*ours == Verdict::Accept, *theirs == Verdict::Accept);
                compared += 1;
                *outcomes.entry((r.expected_ok, ours, theirs)).or_default() += 1;
                digest_input.push(format!(
                    "{} limbo-{} ours-{} webpki-{}",
                    r.id,
                    if r.expected_ok { "ok" } else { "fail" },
                    word(ours),
                    word(theirs)
                ));
                if ours != theirs || (ours && !r.expected_ok) {
                    let class = match (ours, theirs) {
                        (true, true) => "both-accept",
                        (true, false) => "looser",
                        _ => "stricter",
                    };
                    lines.push(format!(
                        "{class} limbo-expects-{} {}",
                        if r.expected_ok { "ok" } else { "fail" },
                        r.id
                    ));
                }
            }
        }
    }

    lines.sort();
    digest_input.sort();
    let digest = ring::digest::digest(&ring::digest::SHA256, digest_input.join("\n").as_bytes());
    let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();

    let mut out = vec![
        format!("compared {compared}"),
        format!("outcomes-sha256 {hex}"),
    ];
    out.extend(skipped.iter().map(|(why, n)| format!("skipped {n} {why}")));
    out.extend(outcomes.iter().map(|((e, o, t), n)| {
        format!(
            "outcome limbo-{} ours-{} webpki-{} {n}",
            if *e { "ok" } else { "fail" },
            word(*o),
            word(*t)
        )
    }));
    out.extend(lines);
    out
}

/// Compare the summary with the golden file's lines (comments and blanks
/// ignored). `Err` carries a readable diff.
fn check_golden(rendered: &[String], golden: &str) -> Result<(), String> {
    let golden: Vec<&str> = golden
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    let new: Vec<&String> = rendered
        .iter()
        .filter(|l| !golden.contains(&l.as_str()))
        .collect();
    let gone: Vec<&&str> = golden
        .iter()
        .filter(|l| !rendered.iter().any(|r| r == **l))
        .collect();
    if new.is_empty() && gone.is_empty() {
        return Ok(());
    }
    Err(format!(
        "limbo results changed.\nnow (look at any `looser` first: a possible forged-certificate \
         acceptance; a lower `compared` or new `skipped` line is lost coverage):\n{}\nwere \
         pinned and are gone:\n{}\nRerun with LIMBO_BLESS=1 once they are understood.",
        new.iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
        gone.iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

#[test]
#[ignore = "needs the x509-limbo corpus: set LIMBO_JSON to limbo.json"]
fn ours_against_webpki_on_x509_limbo() {
    let path = std::env::var("LIMBO_JSON").expect("LIMBO_JSON must point at limbo.json");
    let text = std::fs::read_to_string(&path).expect("limbo.json is readable");
    let doc = Value::parse(&text).expect("limbo.json is JSON");
    let cases = doc
        .get("testcases")
        .and_then(Value::as_array)
        .expect("testcases array");

    let results: Vec<CaseResult> = cases
        .iter()
        .map(|case| {
            let id = case
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let (outcome, expected_ok) = run_case(case);
            CaseResult {
                id,
                outcome,
                expected_ok,
            }
        })
        .collect();
    let rendered = summarize(&results);
    println!("{}", rendered.join("\n"));

    let golden_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/limbo_divergences.txt"
    );
    if std::env::var_os("LIMBO_BLESS").is_some() {
        let header = std::fs::read_to_string(golden_path)
            .map(|t| {
                t.lines()
                    .take_while(|l| l.starts_with('#'))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        std::fs::write(golden_path, format!("{header}\n{}\n", rendered.join("\n")))
            .expect("golden is writable");
        println!("blessed {} lines", rendered.len());
        return;
    }
    let golden =
        std::fs::read_to_string(golden_path).expect("tests/data/limbo_divergences.txt exists");
    if let Err(diff) = check_golden(&rendered, &golden) {
        panic!("{diff}");
    }
}

// ---------------------------------------------------------------------------
// The accounting itself, tested without the corpus.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod accounting {
    use super::*;

    fn compared(id: &str, expected_ok: bool, ours: Verdict, theirs: Verdict) -> CaseResult {
        CaseResult {
            id: id.into(),
            outcome: Outcome::Compared { ours, theirs },
            expected_ok,
        }
    }

    fn skipped(id: &str, why: &'static str) -> CaseResult {
        CaseResult {
            id: id.into(),
            outcome: Outcome::Skipped(why),
            expected_ok: true,
        }
    }

    fn baseline() -> Vec<CaseResult> {
        use Verdict::{Accept, Reject};
        vec![
            compared("a", true, Accept, Accept),
            compared("b", false, Reject, Reject),
            compared("c", false, Accept, Reject), // looser
            compared("d", false, Accept, Accept), // both accept what limbo refuses
            skipped("e", "client validation"),
        ]
    }

    fn pinned() -> String {
        summarize(&baseline()).join("\n")
    }

    #[test]
    fn identical_results_pass_and_the_summary_lists_what_matters() {
        let lines = summarize(&baseline());
        check_golden(&lines, &pinned()).expect("identical results pass");
        assert!(lines.contains(&"compared 4".to_string()));
        assert!(lines.contains(&"skipped 1 client validation".to_string()));
        assert!(lines.contains(&"looser limbo-expects-fail c".to_string()));
        assert!(lines.contains(&"both-accept limbo-expects-fail d".to_string()));
        // Agreement that is right is not listed individually.
        assert!(!lines.iter().any(|l| l.ends_with(" a") || l.ends_with(" b")));
    }

    #[test]
    fn losing_coverage_is_a_diff() {
        // A case that used to be compared is now skipped: no divergence line
        // changes for `a`, but `compared` and `skipped` do.
        let mut results = baseline();
        results[0] = skipped("a", "no peer name");
        let err = check_golden(&summarize(&results), &pinned()).unwrap_err();
        assert!(err.contains("compared 3"), "{err}");
        assert!(err.contains("skipped 1 no peer name"), "{err}");
    }

    #[test]
    fn a_new_skip_reason_is_a_diff() {
        let mut results = baseline();
        results.push(skipped("f", "unparsable validation time"));
        assert!(check_golden(&summarize(&results), &pinned()).is_err());
    }

    #[test]
    fn an_agreeing_case_that_flips_is_a_diff() {
        use Verdict::{Accept, Reject};
        // `a` (limbo ok) was accept/accept and is now reject/reject: both
        // engines agree, no divergence line appears, but the outcome counts
        // and the digest move.
        let mut results = baseline();
        results[0] = compared("a", true, Reject, Reject);
        assert!(check_golden(&summarize(&results), &pinned()).is_err());
        // Two flips that cancel in every count still change the digest.
        let mut cases = baseline();
        cases.push(compared("x", true, Accept, Accept));
        cases.push(compared("y", true, Reject, Reject));
        let before = summarize(&cases);
        let n = cases.len();
        cases[n - 2] = compared("x", true, Reject, Reject);
        cases[n - 1] = compared("y", true, Accept, Accept);
        let after = summarize(&cases);
        let counts = |v: &[String]| -> Vec<String> {
            v.iter()
                .filter(|l| l.starts_with("outcome "))
                .cloned()
                .collect()
        };
        assert_eq!(
            counts(&before),
            counts(&after),
            "the swap keeps every count"
        );
        assert!(
            check_golden(&after, &before.join("\n")).is_err(),
            "the digest catches it"
        );
    }

    #[test]
    fn a_new_looser_case_is_a_diff() {
        use Verdict::{Accept, Reject};
        let mut results = baseline();
        results.push(compared("g", false, Accept, Reject));
        let err = check_golden(&summarize(&results), &pinned()).unwrap_err();
        assert!(err.contains("looser limbo-expects-fail g"), "{err}");
    }
}
