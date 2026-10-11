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

    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    // (expected, ours, theirs) -> ids
    let mut table: BTreeMap<(bool, bool, bool), Vec<String>> = BTreeMap::new();
    for case in cases {
        let id = case
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        match run_case(case) {
            (Outcome::Skipped(why), _) => *skipped.entry(why).or_default() += 1,
            (Outcome::Compared { ours, theirs }, expected) => table
                .entry((expected, ours == Verdict::Accept, theirs == Verdict::Accept))
                .or_default()
                .push(id),
        }
    }

    let compared: usize = table.values().map(Vec::len).sum();
    println!("limbo cases: {}, compared: {compared}", cases.len());
    for (why, n) in &skipped {
        println!("  skipped {n:5}  {why}");
    }
    println!("  expected ours  theirs  count");
    for ((expected, ours, theirs), ids) in &table {
        let e = if *expected { "ok  " } else { "fail" };
        let o = if *ours { "accept" } else { "reject" };
        let t = if *theirs { "accept" } else { "reject" };
        println!("  {e}     {o} {t}  {:5}", ids.len());
    }
    // Every case where this validator and webpki differ, and every case where
    // both accept something limbo says to refuse, as sorted
    // `class limbo-expects-X id` lines. "Both accept" is listed because
    // agreeing with webpki is not the same as being right.
    let mut lines: Vec<String> = Vec::new();
    for ((expected, ours, theirs), ids) in &table {
        if ours == theirs && (*expected || !*ours) {
            continue;
        }
        let class = match (*ours, *theirs) {
            (true, true) => "both-accept",
            (true, false) => "looser",
            _ => "stricter",
        };
        let expects = if *expected { "ok" } else { "fail" };
        lines.extend(
            ids.iter()
                .map(|id| format!("{class} limbo-expects-{expects} {id}")),
        );
    }
    lines.sort();

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
        std::fs::write(golden_path, format!("{header}\n{}\n", lines.join("\n")))
            .expect("golden is writable");
        println!("blessed {} lines", lines.len());
        return;
    }
    let golden: Vec<String> = std::fs::read_to_string(golden_path)
        .expect("tests/data/limbo_divergences.txt exists")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    let new: Vec<&String> = lines.iter().filter(|l| !golden.contains(l)).collect();
    let gone: Vec<&String> = golden.iter().filter(|l| !lines.contains(l)).collect();
    assert!(
        new.is_empty() && gone.is_empty(),
        "limbo results changed.\nnew (look at these first; any `looser` is a possible \
         forged-certificate acceptance):\n{}\nno longer present (improvements, or a changed \
         corpus):\n{}\nrerun with LIMBO_BLESS=1 once they are understood",
        new.iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
        gone.iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert!(compared > 5000, "the corpus compared only {compared} cases");
}
