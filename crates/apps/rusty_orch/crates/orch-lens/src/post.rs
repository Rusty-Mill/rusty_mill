//! The reply protocol: one JSON object `{"posts": [...]}`, checked strictly.
//!
//! Every adapter runs [`parse`] on the model's text, so one rule holds for
//! all providers: unknown fields, kinds or directions are refused, support
//! must carry executed or cited evidence ("agreement costs evidence"), and
//! a job may only post where its [`Allowed`] says.

use std::collections::BTreeSet;

use rusty_bbp::claims::{DOMAIN_CAP, MAX_DEPS, MAX_DOMAINS};
use rusty_bbp::{ArtId, ClaimBody, Direction, EvidenceBody, EvidenceKind, MsgId, Ref, Stakes};
use rusty_json::{Map, Value};

/// Replies with more posts than this are refused whole.
pub const MAX_POSTS: usize = 6;
/// Longest claim text or evidence note, in characters.
pub const MAX_TEXT: usize = 500;

/// One board post a lens wants made. The runner posts it with the lens's
/// credential; BBP scores and settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Post {
    Claim {
        body: ClaimBody,
        refs: Vec<Ref>,
    },
    Evidence {
        body: EvidenceBody,
        evidence: Vec<Ref>,
    },
}

/// What one job may post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowed {
    /// Evidence on exactly this claim, in these directions.
    pub evidence: Option<EvidenceRule>,
    /// New claims; `answers`, when set, is the question each must answer.
    pub claims: Option<ClaimRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRule {
    pub claim: MsgId,
    pub directions: BTreeSet<DirectionKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRule {
    pub answers: Option<MsgId>,
}

/// `Direction` with an order, for sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirectionKey {
    Support,
    Counter,
}

impl DirectionKey {
    fn of(d: Direction) -> Self {
        match d {
            Direction::Support => Self::Support,
            Direction::Counter => Self::Counter,
        }
    }
}

/// The reply protocol in words, for every lens's prefix. Built from the
/// same limits [`parse`] checks; its example reply parses.
pub fn protocol() -> String {
    format!(
        "Reply with exactly one JSON object and nothing else: {{\"posts\": [...]}}, at most {MAX_POSTS} posts. \
Reply {{\"posts\": []}} when you have nothing to add.\n\
A claim: {{\"kind\": \"claim\", \"text\": one sentence, \"domains\": [1 to {MAX_DOMAINS} names of 1 to {DOMAIN_CAP} characters from a-z, 0-9, _ and -], \
\"stakes\": \"low\", \"medium\" or \"high\", \"reversible\": true or false, and optionally \"deps\": [up to {MAX_DEPS} claim ids], \
\"answers\": a question's id, \"supersedes\": a claim's id, \"refs\": [references]}}.\n\
Evidence: {{\"kind\": \"evidence\", \"claim\": the claim's id, \"direction\": \"support\" or \"counter\", \
\"evidence_kind\": \"executed\", \"cited\" or \"opinion\", \"strength\": 1 to 3 (executed is always 1), \
\"evidence\": [references], and optionally \"note\": why}}.\n\
A reference is \"msg:N\" for a message or \"art:N\" for an artifact. \
Claim and question ids (\"claim\", \"deps\", \"answers\", \"supersedes\") are bare numbers: 7 for msg:7. Support must be executed or cited, never opinion. \
Executed and cited evidence cite at least one reference. Text and notes are at most {MAX_TEXT} characters. No other fields.\n\
Example: {EXAMPLE}\n"
    )
}

/// The example reply in [`protocol`].
const EXAMPLE: &str = r#"{"posts": [{"kind": "evidence", "claim": 7, "direction": "counter", "evidence_kind": "cited", "strength": 2, "evidence": ["msg:5"], "note": "Fails on empty files."}]}"#;

/// What one job may post, in words.
pub fn allowance(a: &Allowed) -> String {
    let mut out = String::from("What you may post:");
    if let Some(e) = &a.evidence {
        let dirs: Vec<&str> = e
            .directions
            .iter()
            .map(|d| match d {
                DirectionKey::Support => "support",
                DirectionKey::Counter => "counter",
            })
            .collect();
        out.push_str(&format!(
            " evidence on claim {} ({}).",
            e.claim,
            dirs.join(" or ")
        ));
    }
    match a.claims.map(|c| c.answers) {
        Some(Some(q)) => out.push_str(&format!(
            " claims that answer {q} (set \"answers\": {}).",
            q.0
        )),
        Some(None) => out.push_str(" new claims."),
        None => {}
    }
    if a.evidence.is_none() && a.claims.is_none() {
        out.push_str(" nothing; reply {\"posts\": []}.");
    }
    out
}

/// Parse and check one reply. Tolerates one ``` fence around the object.
/// The error names the first offending post and field.
pub fn parse(text: &str, allowed: &Allowed) -> Result<Vec<Post>, String> {
    let v = Value::parse(orch_cli::unfence(text)).map_err(|e| format!("reply is not JSON: {e}"))?;
    let top = object(&v, "reply")?;
    only_keys(top, &["posts"], "reply")?;
    let posts = top
        .get("posts")
        .and_then(Value::as_array)
        .ok_or("reply has no \"posts\" array")?;
    if posts.len() > MAX_POSTS {
        return Err(format!(
            "{} posts exceeds the cap of {MAX_POSTS}",
            posts.len()
        ));
    }
    posts
        .iter()
        .enumerate()
        .map(|(i, p)| post(p, allowed).map_err(|m| format!("post {i}: {m}")))
        .collect()
}

fn post(v: &Value, allowed: &Allowed) -> Result<Post, String> {
    let m = object(v, "post")?;
    match string(m, "kind")? {
        "claim" => claim(
            m,
            allowed
                .claims
                .as_ref()
                .ok_or("claims are not allowed here")?,
        ),
        "evidence" => evidence(
            m,
            allowed
                .evidence
                .as_ref()
                .ok_or("evidence is not allowed here")?,
        ),
        other => Err(format!("kind {other:?} is not claim or evidence")),
    }
}

/// The BBP host's domain rule, checked here so a bad reply is refused at
/// the protocol boundary rather than at posting.
fn domain_ok(d: &str) -> bool {
    (1..=DOMAIN_CAP).contains(&d.len())
        && d.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

const CLAIM_KEYS: &[&str] = &[
    "kind",
    "text",
    "domains",
    "stakes",
    "reversible",
    "deps",
    "answers",
    "supersedes",
    "refs",
];

fn claim(m: &Map, rule: &ClaimRule) -> Result<Post, String> {
    only_keys(m, CLAIM_KEYS, "claim")?;
    let answers = optional_msg(m, "answers")?;
    if let Some(question) = rule.answers {
        if answers != Some(question) {
            return Err(format!("a proposal must answer {question}"));
        }
    }
    let domains = strings(m, "domains")?;
    if domains.is_empty() || domains.len() > MAX_DOMAINS {
        return Err(format!("domains: 1 to {MAX_DOMAINS} required"));
    }
    if let Some(d) = domains.iter().find(|d| !domain_ok(d)) {
        return Err(format!(
            "domain {d:?} must be 1 to {DOMAIN_CAP} bytes of [a-z0-9_-]"
        ));
    }
    let deps = msgs(m, "deps")?;
    if deps.len() > MAX_DEPS {
        return Err(format!("deps: at most {MAX_DEPS}"));
    }
    let body = ClaimBody {
        text: text(m, "text")?,
        domains,
        deps,
        answers,
        supersedes: optional_msg(m, "supersedes")?,
        stakes: stakes(string(m, "stakes")?)?,
        reversible: m
            .get("reversible")
            .and_then(Value::as_bool)
            .ok_or("reversible: true or false required")?,
    };
    Ok(Post::Claim {
        body,
        refs: refs(m, "refs")?,
    })
}

const EVIDENCE_KEYS: &[&str] = &[
    "kind",
    "claim",
    "direction",
    "evidence_kind",
    "strength",
    "note",
    "evidence",
];

fn evidence(m: &Map, rule: &EvidenceRule) -> Result<Post, String> {
    only_keys(m, EVIDENCE_KEYS, "evidence")?;
    let claim = msg(m, "claim")?;
    if claim != rule.claim {
        return Err(format!("evidence must be on {}, not {claim}", rule.claim));
    }
    let direction = direction(string(m, "direction")?)?;
    if !rule.directions.contains(&DirectionKey::of(direction)) {
        return Err(format!("direction {direction:?} is not allowed here"));
    }
    let kind = evidence_kind(string(m, "evidence_kind")?)?;
    if direction == Direction::Support && kind == EvidenceKind::Opinion {
        return Err("support must be executed or cited, not opinion".to_owned());
    }
    let cites = refs(m, "evidence")?;
    if kind != EvidenceKind::Opinion && cites.is_empty() {
        return Err("executed or cited evidence must cite an artifact or message".to_owned());
    }
    let strength = m
        .get("strength")
        .and_then(Value::as_u64)
        .ok_or("strength: 1 to 3 required")?;
    let strength = match (kind, strength) {
        (EvidenceKind::Executed, 1) => 1,
        (EvidenceKind::Executed, _) => return Err("executed evidence has strength 1".to_owned()),
        (_, 1..=3) => strength as u8,
        _ => return Err("strength: 1 to 3 required".to_owned()),
    };
    let note = match m.get("note") {
        None => String::new(),
        Some(_) => text(m, "note")?,
    };
    Ok(Post::Evidence {
        body: EvidenceBody {
            claim,
            direction,
            kind,
            strength,
            note,
        },
        evidence: cites,
    })
}

fn object<'a>(v: &'a Value, what: &str) -> Result<&'a Map, String> {
    v.as_object()
        .ok_or_else(|| format!("{what} is not an object"))
}

fn only_keys(m: &Map, keys: &[&str], what: &str) -> Result<(), String> {
    match m.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{what} has unknown field {k:?}")),
        None => Ok(()),
    }
}

fn string<'a>(m: &'a Map, k: &str) -> Result<&'a str, String> {
    m.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{k}: string required"))
}

fn text(m: &Map, k: &str) -> Result<String, String> {
    let s = string(m, k)?.trim();
    if s.is_empty() || s.chars().count() > MAX_TEXT {
        return Err(format!("{k}: 1 to {MAX_TEXT} characters required"));
    }
    Ok(s.to_owned())
}

fn strings(m: &Map, k: &str) -> Result<Vec<String>, String> {
    array(m, k)?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{k}: strings required"))
        })
        .collect()
}

fn array<'a>(m: &'a Map, k: &str) -> Result<&'a [Value], String> {
    match m.get(k) {
        None => Ok(&[]),
        Some(v) => v
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{k}: array required")),
    }
}

fn msg(m: &Map, k: &str) -> Result<MsgId, String> {
    m.get(k)
        .and_then(Value::as_u64)
        .map(MsgId)
        .ok_or_else(|| format!("{k}: message id required"))
}

fn optional_msg(m: &Map, k: &str) -> Result<Option<MsgId>, String> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => msg(m, k).map(Some),
    }
}

fn msgs(m: &Map, k: &str) -> Result<Vec<MsgId>, String> {
    array(m, k)?
        .iter()
        .map(|v| {
            v.as_u64()
                .map(MsgId)
                .ok_or_else(|| format!("{k}: message ids required"))
        })
        .collect()
}

fn refs(m: &Map, k: &str) -> Result<Vec<Ref>, String> {
    array(m, k)?
        .iter()
        .map(|v| v.as_str().and_then(reference))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("{k}: references like \"art:3\" or \"msg:7\" required"))
}

/// `art:N` or `msg:N`.
fn reference(s: &str) -> Option<Ref> {
    let (prefix, n) = s.split_once(':')?;
    let n: u64 = n.parse().ok()?;
    match prefix {
        "art" => Some(Ref::art(ArtId(n))),
        "msg" => Some(Ref::Msg(MsgId(n))),
        _ => None,
    }
}

fn stakes(s: &str) -> Result<Stakes, String> {
    match s {
        "low" => Ok(Stakes::Low),
        "medium" => Ok(Stakes::Medium),
        "high" => Ok(Stakes::High),
        _ => Err(format!("stakes {s:?} is not low, medium or high")),
    }
}

fn direction(s: &str) -> Result<Direction, String> {
    match s {
        "support" => Ok(Direction::Support),
        "counter" => Ok(Direction::Counter),
        _ => Err(format!("direction {s:?} is not support or counter")),
    }
}

fn evidence_kind(s: &str) -> Result<EvidenceKind, String> {
    match s {
        "executed" => Ok(EvidenceKind::Executed),
        "cited" => Ok(EvidenceKind::Cited),
        "opinion" => Ok(EvidenceKind::Opinion),
        _ => Err(format!(
            "evidence_kind {s:?} is not executed, cited or opinion"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attack(claim: u64) -> Allowed {
        Allowed {
            evidence: Some(EvidenceRule {
                claim: MsgId(claim),
                directions: [DirectionKey::Counter, DirectionKey::Support].into(),
            }),
            claims: None,
        }
    }

    fn propose(question: u64) -> Allowed {
        Allowed {
            evidence: None,
            claims: Some(ClaimRule {
                answers: Some(MsgId(question)),
            }),
        }
    }

    #[test]
    fn the_protocol_example_parses_and_names_the_limits() {
        let posts = parse(EXAMPLE, &attack(7)).expect("example parses");
        assert_eq!(posts.len(), 1);
        let p = protocol();
        assert!(p.contains(EXAMPLE));
        for limit in [MAX_POSTS, MAX_DOMAINS, DOMAIN_CAP, MAX_DEPS, MAX_TEXT] {
            assert!(p.contains(&limit.to_string()), "{limit}");
        }
    }

    #[test]
    fn the_allowance_says_what_a_job_may_post() {
        assert_eq!(
            allowance(&attack(7)),
            "What you may post: evidence on claim msg:7 (support or counter)."
        );
        assert_eq!(
            allowance(&propose(3)),
            r#"What you may post: claims that answer msg:3 (set "answers": 3)."#
        );
        let both = Allowed {
            claims: Some(ClaimRule { answers: None }),
            ..attack(7)
        };
        assert!(allowance(&both).ends_with(" new claims."));
        let none = Allowed {
            evidence: None,
            claims: None,
        };
        assert_eq!(
            allowance(&none),
            r#"What you may post: nothing; reply {"posts": []}."#
        );
    }

    const COUNTER: &str = r#"{"posts":[{"kind":"evidence","claim":7,"direction":"counter",
        "evidence_kind":"opinion","strength":2,"note":"misses hard links"}]}"#;

    #[test]
    fn counter_opinion_on_the_target_parses() {
        let posts = parse(COUNTER, &attack(7)).expect("valid");
        let [Post::Evidence { body, evidence }] = posts.as_slice() else {
            panic!("one evidence post: {posts:?}");
        };
        assert_eq!(body.claim, MsgId(7));
        assert_eq!(body.direction, Direction::Counter);
        assert_eq!(body.strength, 2);
        assert!(evidence.is_empty());
    }

    #[test]
    fn a_fenced_reply_and_an_empty_one_parse() {
        let fenced = format!("```json\n{COUNTER}\n```");
        assert_eq!(parse(&fenced, &attack(7)).expect("fenced").len(), 1);
        assert!(parse(r#"{"posts":[]}"#, &attack(7))
            .expect("empty")
            .is_empty());
    }

    #[test]
    fn a_proposal_answering_the_question_parses() {
        let reply = r#"{"posts":[{"kind":"claim","text":"Hash by size, then by content",
            "domains":["dedup"],"stakes":"medium","reversible":true,"answers":3,"refs":["msg:3"]}]}"#;
        let posts = parse(reply, &propose(3)).expect("valid");
        let [Post::Claim { body, refs }] = posts.as_slice() else {
            panic!("one claim: {posts:?}");
        };
        assert_eq!(body.answers, Some(MsgId(3)));
        assert_eq!(body.stakes, Stakes::Medium);
        assert_eq!(refs, &[Ref::Msg(MsgId(3))]);
    }

    #[test]
    fn refusals() {
        let ev = |fields: &str| {
            format!(
                r#"{{"posts":[{{"kind":"evidence","claim":7,"direction":"counter",{fields}}}]}}"#
            )
        };
        let cases: Vec<(String, Allowed, &str)> = vec![
            ("not json".into(), attack(7), "not JSON"),
            (r#"{"posts":[],"extra":1}"#.into(), attack(7), "unknown field \"extra\""),
            (r#"{"entries":[]}"#.into(), attack(7), "unknown field"),
            (
                ev(r#""evidence_kind":"opinion","strength":1,"why":"x""#),
                attack(7),
                "unknown field \"why\"",
            ),
            (
                COUNTER.replace("\"claim\":7", "\"claim\":8"),
                attack(7),
                "must be on msg:7",
            ),
            (
                COUNTER.replace("counter", "support"),
                attack(7),
                "support must be executed or cited",
            ),
            (
                ev(r#""evidence_kind":"cited","strength":1"#),
                attack(7),
                "must cite",
            ),
            (
                ev(r#""evidence_kind":"executed","strength":2,"evidence":["art:4"]"#),
                attack(7),
                "strength 1",
            ),
            (
                ev(r#""evidence_kind":"opinion","strength":4"#),
                attack(7),
                "1 to 3",
            ),
            (
                ev(r#""evidence_kind":"guess","strength":1"#),
                attack(7),
                "not executed, cited or opinion",
            ),
            (
                ev(r#""evidence_kind":"cited","strength":1,"evidence":["file:4"]"#),
                attack(7),
                "references like",
            ),
            (COUNTER.into(), propose(3), "evidence is not allowed"),
            (
                r#"{"posts":[{"kind":"claim","text":"x","domains":["d"],"stakes":"low","reversible":true}]}"#
                    .into(),
                propose(3),
                "must answer msg:3",
            ),
            (
                r#"{"posts":[{"kind":"claim","text":"x","domains":[],"stakes":"low","reversible":true,"answers":3}]}"#
                    .into(),
                propose(3),
                "domains",
            ),
            (
                r#"{"posts":[{"kind":"claim","text":"x","domains":["d"],"stakes":"low","reversible":true}]}"#
                    .into(),
                attack(7),
                "claims are not allowed",
            ),
            (
                r#"{"posts":[{"kind":"review"}]}"#.into(),
                attack(7),
                "not claim or evidence",
            ),
        ];
        for (reply, allowed, expected) in cases {
            let e = parse(&reply, &allowed).expect_err(&reply);
            assert!(
                e.contains(expected),
                "{expected:?} not in {e:?} for {reply}"
            );
        }
    }

    #[test]
    fn domains_follow_the_host_rule() {
        let claim = |d: &str| {
            format!(
                r#"{{"posts":[{{"kind":"claim","text":"x","domains":["{d}"],"stakes":"low","reversible":true,"answers":3}}]}}"#
            )
        };
        let long = "a".repeat(DOMAIN_CAP + 1);
        for bad in [
            "Real Data",
            "real data",
            "UPPER",
            "a.b",
            "a/b",
            "",
            long.as_str(),
        ] {
            let e = parse(&claim(bad), &propose(3)).expect_err(bad);
            assert!(e.contains("[a-z0-9_-]"), "{bad:?}: {e}");
        }
        for good in ["real_data", "dedup-lite", "x9"] {
            assert!(parse(&claim(good), &propose(3)).is_ok(), "{good}");
        }
    }

    #[test]
    fn a_direction_outside_the_rule_is_refused() {
        let mut only_counter = attack(7);
        if let Some(rule) = only_counter.evidence.as_mut() {
            rule.directions = [DirectionKey::Counter].into();
        }
        let support = r#"{"posts":[{"kind":"evidence","claim":7,"direction":"support",
            "evidence_kind":"cited","strength":1,"evidence":["art:2"]}]}"#;
        let e = parse(support, &only_counter).expect_err("support refused");
        assert!(e.contains("not allowed"), "{e}");
    }

    #[test]
    fn too_many_posts_are_refused_whole() {
        let one = r#"{"kind":"evidence","claim":7,"direction":"counter","evidence_kind":"opinion","strength":1}"#;
        let reply = format!(r#"{{"posts":[{}]}}"#, [one; MAX_POSTS + 1].join(","));
        assert!(parse(&reply, &attack(7)).expect_err("cap").contains("cap"));
    }
}
