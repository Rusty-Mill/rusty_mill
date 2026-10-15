//! `bbp grade`: blind rubric grading for the 2b run. `pack` writes what the
//! grader sees (spec text and rubric items under neutral ids, in hash order,
//! nothing about arm, pair or log) in two files, `pack.json` (the approved
//! specs: exactly two per task) and `unused.json` (superseded versions, which
//! only a treatment run has, so they never share a pack with the approved
//! ones), and a key kept apart; `join` opens the key
//! only once the scores cover every id, then reports each run's `rubric_ok`
//! and the pair's `lost_rubric_items`. Who scores (a person or a model) is not
//! this tool's business; the scorer is named in the scores file and carried to
//! the output. Blindness is mechanical, not absolute: a spec that names its
//! own author or arm in prose is not scrubbed.

use crate::pairs::{Manifest, RunRef};
use rusty_bbp::spec_metrics::{spec_attribution, Attribution};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// A task's frozen rubric: committed before the run.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Rubric {
    pub items: Vec<String>,
    /// Items that must be met for the spec to pass.
    pub pass_mark: usize,
}

impl Rubric {
    fn check(&self) -> Result<(), String> {
        if self.items.is_empty() || self.pass_mark == 0 || self.pass_mark > self.items.len() {
            return Err("rubric needs items and 1 <= pass_mark <= items".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arm {
    #[rusty_serde(rename = "control")]
    Control,
    #[rusty_serde(rename = "treatment")]
    Treatment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Which {
    #[rusty_serde(rename = "approved")]
    Approved,
    #[rusty_serde(rename = "unused")]
    Unused,
}

/// A spec to grade, with everything the grader must not see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub pair: String,
    pub arm: Arm,
    pub run: String,
    pub artifact: u64,
    pub which: Which,
    pub spec: String,
    pub rubric: Rubric,
}

/// What the grader is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PackEntry {
    pub id: String,
    pub spec: String,
    pub rubric: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEntry {
    pub id: String,
    pub pair: String,
    pub arm: Arm,
    pub run: String,
    pub artifact: u64,
    pub which: Which,
    pub pass_mark: usize,
    pub items: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub seed: String,
    pub entries: Vec<KeyEntry>,
}

fn neutral_id(seed: &str, s: &Source) -> String {
    let h = Sha256::of(format!("{seed}|{}|{}|{}", s.pair, s.run, s.artifact).as_bytes());
    h.hex()[..16].to_owned()
}

/// What `pack` produces: the two packs the grader sees and the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packs {
    pub approved: Vec<PackEntry>,
    pub unused: Vec<PackEntry>,
    pub key: Key,
}

/// Neutral ids and an order that depends only on the seed and the ids' hashes,
/// so neither arm nor pair is recoverable from position. Approved and unused
/// specs go to separate packs: an entry count per task would name the arm.
pub fn pack(sources: &[Source], seed: &str) -> Packs {
    let mut keyed: Vec<(String, &Source)> =
        sources.iter().map(|s| (neutral_id(seed, s), s)).collect();
    keyed.sort_by(|a, b| {
        Sha256::of(format!("{seed}|{}", a.0).as_bytes())
            .0
            .cmp(&Sha256::of(format!("{seed}|{}", b.0).as_bytes()).0)
    });
    let of = |which: Which| -> Vec<PackEntry> {
        keyed
            .iter()
            .filter(|(_, s)| s.which == which)
            .map(|(id, s)| PackEntry {
                id: id.clone(),
                spec: s.spec.clone(),
                rubric: s.rubric.items.clone(),
            })
            .collect()
    };
    let entries = keyed
        .iter()
        .map(|(id, s)| KeyEntry {
            id: id.clone(),
            pair: s.pair.clone(),
            arm: s.arm,
            run: s.run.clone(),
            artifact: s.artifact,
            which: s.which,
            pass_mark: s.rubric.pass_mark,
            items: s.rubric.items.len(),
        })
        .collect();
    Packs {
        approved: of(Which::Approved),
        unused: of(Which::Unused),
        key: Key {
            seed: seed.to_owned(),
            entries,
        },
    }
}

/// The scores file: the scorer and, per neutral id, which rubric items were met.
#[derive(Clone, Debug, Deserialize)]
pub struct Scores {
    pub grader: String,
    pub met: BTreeMap<String, Vec<bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunGrade {
    pub pair: String,
    pub arm: Arm,
    pub run: String,
    pub met: usize,
    pub items: usize,
    pub rubric_ok: bool,
    /// Treatment only: items an unused spec version met and the approved one
    /// did not.
    pub lost_rubric_items: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Joined {
    pub grader: String,
    pub runs: Vec<RunGrade>,
}

/// Open the key. Refused unless `scores` cover every id with one verdict per
/// rubric item (the key stays sealed until then), and no id is unknown.
pub fn join(key: &Key, scores: &Scores) -> Result<Joined, String> {
    let ids: BTreeSet<&str> = key.entries.iter().map(|e| e.id.as_str()).collect();
    if let Some(extra) = scores.met.keys().find(|k| !ids.contains(k.as_str())) {
        return Err(format!(
            "scores name an id that is not in the pack: {extra}"
        ));
    }
    for e in &key.entries {
        let got = scores
            .met
            .get(&e.id)
            .ok_or_else(|| format!("no score for {}: the key stays sealed", e.id))?;
        if got.len() != e.items {
            return Err(format!(
                "{} has {} verdicts for {} rubric items",
                e.id,
                got.len(),
                e.items
            ));
        }
    }
    let mut runs: Vec<RunGrade> = Vec::new();
    for e in key.entries.iter().filter(|e| e.which == Which::Approved) {
        let met_items = &scores.met[&e.id];
        let met = met_items.iter().filter(|m| **m).count();
        let lost = (e.arm == Arm::Treatment).then(|| {
            let unused: Vec<&KeyEntry> = key
                .entries
                .iter()
                .filter(|u| u.which == Which::Unused && u.pair == e.pair && u.run == e.run)
                .collect();
            (0..e.items)
                .filter(|i| !met_items[*i] && unused.iter().any(|u| scores.met[&u.id][*i]))
                .count() as u64
        });
        runs.push(RunGrade {
            pair: e.pair.clone(),
            arm: e.arm,
            run: e.run.clone(),
            met,
            items: e.items,
            rubric_ok: met >= e.pass_mark,
            lost_rubric_items: lost,
        });
    }
    runs.sort_by(|a, b| (a.pair.as_str(), a.arm as u8).cmp(&(b.pair.as_str(), b.arm as u8)));
    Ok(Joined {
        grader: scores.grader.clone(),
        runs,
    })
}

// ---------------------------------------------------------------- loading

fn rubric(dir: &Path, pair: &str) -> Result<Rubric, String> {
    let file = dir.join(format!("{pair}.json"));
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let r: Rubric =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    r.check().map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(r)
}

/// The approved spec of the run and each of its unused versions, as sources.
fn run_sources(r: &RunRef, pair: &str, arm: Arm, rubric: &Rubric) -> Result<Vec<Source>, String> {
    let task = TaskId(r.task.clone());
    let d = crate::open_driver(Path::new(&r.dir), &task)?;
    let events = d.store.events(&task, 0).map_err(|e| format!("{e:?}"))?;
    let blobs: HashMap<ArtId, BlobRef> = events
        .iter()
        .filter_map(|e| match e {
            Event::ArtifactStored(a) if a.kind == ArtifactKind::Spec => Some((a.id, a.blob)),
            _ => None,
        })
        .collect();
    let read = |b: &BlobRef| d.store.blob_get(&b.sha).ok();
    let attr = spec_attribution(&events, &read);
    let approved = attr
        .approved
        .ok_or_else(|| format!("{pair}/{}: no approved spec", r.task))?;
    let mut ids = vec![(approved, Which::Approved)];
    if arm == Arm::Treatment {
        ids.extend(
            attr.versions
                .iter()
                .filter(|v| v.outcome == Attribution::Unused)
                .map(|v| (v.version, Which::Unused)),
        );
    }
    ids.into_iter()
        .map(|(id, which)| {
            let blob = blobs
                .get(&id)
                .ok_or_else(|| format!("spec {} not stored", id.0))?;
            let bytes = read(blob).ok_or_else(|| format!("spec {} blob missing", id.0))?;
            let spec = spec_markdown(&bytes).map_err(|e| format!("spec {}: {e:?}", id.0))?;
            Ok(Source {
                pair: pair.to_owned(),
                arm,
                run: format!("{}:{}", r.dir, r.task),
                artifact: id.0,
                which,
                spec,
                rubric: rubric.clone(),
            })
        })
        .collect()
}

/// Everything to grade for `manifest`: both arms' approved specs and the
/// treatment arm's unused versions. Null pairs are not graded: they measure
/// time only.
pub fn sources(manifest: &Path, rubrics: &Path) -> Result<Vec<Source>, String> {
    let text =
        std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let m: Manifest =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = Vec::new();
    for p in m
        .pairs
        .iter()
        .filter(|p| p.kind != crate::pairs::Kind::Null)
    {
        let rb = rubric(rubrics, &p.id)?;
        out.extend(run_sources(&p.control, &p.id, Arm::Control, &rb)?);
        out.extend(run_sources(&p.treatment, &p.id, Arm::Treatment, &rb)?);
    }
    Ok(out)
}

/// The hash of the committed rubric files, recorded in the run output so a run
/// graded against different rubrics is visible.
pub fn rubrics_sha256(dir: &Path) -> Result<String, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut all = Vec::new();
    for f in files {
        let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        all.extend(
            f.file_name()
                .map(|n| n.as_encoded_bytes().to_vec())
                .unwrap_or_default(),
        );
        all.push(0);
        all.extend(bytes);
        all.push(0);
    }
    Ok(Sha256::of(&all).hex())
}

/// `bbp grade pack`: write `pack.json` and `unused.json` (for the grader) and
/// `key.json` (not).
pub fn write_pack(
    manifest: &Path,
    rubrics: &Path,
    seed: &str,
    out: &Path,
) -> Result<String, String> {
    let Packs {
        approved,
        unused,
        key,
    } = pack(&sources(manifest, rubrics)?, seed);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let w = |name: &str, text: String| {
        std::fs::write(out.join(name), text).map_err(|e| format!("{name}: {e}"))
    };
    w(
        "pack.json",
        rusty_serde::json::to_string(&approved).map_err(|e| e.to_string())?,
    )?;
    // Held back: released only once the approved scores are sealed.
    w(
        "unused.held.json",
        rusty_serde::json::to_string(&unused).map_err(|e| e.to_string())?,
    )?;
    w(
        "key.json",
        rusty_serde::json::to_string(&key).map_err(|e| e.to_string())?,
    )?;
    Ok(format!(
        "{{\"approved\":{},\"unused\":{},\"rubrics_sha256\":\"{}\"}}",
        approved.len(),
        unused.len(),
        rubrics_sha256(rubrics)?
    ))
}

/// `bbp grade release`: check the approved scores are complete, record their
/// hash in `release.json` and only then write `unused.json` for the grader.
pub fn release_files(key: &Path, held: &Path, scores: &Path, out: &Path) -> Result<String, String> {
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let key: Key = rusty_serde::json::from_str(&read(key)?).map_err(|e| e.to_string())?;
    let sha = seal(&key, &read(scores)?)?;
    let unused = read(held)?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let record = format!("{{\"approved_scores_sha256\":\"{sha}\"}}");
    // The record first: an `unused.json` never exists without one.
    std::fs::write(out.join("release.json"), &record).map_err(|e| format!("release.json: {e}"))?;
    std::fs::write(out.join("unused.json"), unused).map_err(|e| format!("unused.json: {e}"))?;
    Ok(record)
}

#[derive(Deserialize)]
struct Release {
    approved_scores_sha256: String,
}

/// `bbp grade join`.
pub fn join_files(
    key: &Path,
    scores: &Path,
    unused_scores: &Path,
    release: &Path,
    rubrics: &Path,
) -> Result<String, String> {
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let key: Key = rusty_serde::json::from_str(&read(key)?).map_err(|e| e.to_string())?;
    let rel: Release = rusty_serde::json::from_str(&read(release)?)
        .map_err(|e| format!("{}: {e}", release.display()))?;
    let joined = join_sealed(
        &key,
        &read(scores)?,
        &read(unused_scores)?,
        &rel.approved_scores_sha256,
    )?;
    Ok(format!(
        "{{\"rubrics_sha256\":\"{}\",\"approved_scores_sha256\":\"{}\",\"graded\":{}}}",
        rubrics_sha256(rubrics)?,
        rel.approved_scores_sha256,
        rusty_serde::json::to_string(&joined).map_err(|e| e.to_string())?
    ))
}

/// Score text, parsed.
fn parse_scores(text: &str) -> Result<Scores, String> {
    rusty_serde::json::from_str(text).map_err(|e| format!("scores: {e}"))
}

fn ids_of(key: &Key, which: Which) -> BTreeSet<&str> {
    key.entries
        .iter()
        .filter(|e| e.which == which)
        .map(|e| e.id.as_str())
        .collect()
}

/// `scores` cover exactly the `which` ids of `key`, one verdict per rubric item,
/// and name no other id.
fn covers(key: &Key, scores: &Scores, which: Which, what: &str) -> Result<(), String> {
    let want = ids_of(key, which);
    if let Some(extra) = scores.met.keys().find(|k| !want.contains(k.as_str())) {
        let kind = if ids_of(key, Which::Unused).contains(extra.as_str()) {
            "an unused version, scored before the release"
        } else {
            "an id that is not one of the approved specs"
        };
        return Err(format!("the {what} scores name {kind}: {extra}"));
    }
    for e in key.entries.iter().filter(|e| e.which == which) {
        let got = scores
            .met
            .get(&e.id)
            .ok_or_else(|| format!("no {what} score for {}: nothing is released", e.id))?;
        if got.len() != e.items {
            return Err(format!(
                "{} has {} verdicts for {} rubric items",
                e.id,
                got.len(),
                e.items
            ));
        }
    }
    Ok(())
}

/// Seal the approved scores: they cover exactly the approved ids, with one
/// verdict per rubric item, and name no unused id (that would be scoring the
/// unused versions with the approved ones in view). Returns the sha256 of the
/// scores text, which the release records.
pub fn seal(key: &Key, scores_text: &str) -> Result<String, String> {
    covers(
        key,
        &parse_scores(scores_text)?,
        Which::Approved,
        "approved",
    )?;
    Ok(Sha256::of(scores_text.as_bytes()).hex())
}

/// [`join`] over the sealed approved scores and the unused scores, once the
/// approved scores still hash to what the release recorded.
pub fn join_sealed(
    key: &Key,
    approved_text: &str,
    unused_text: &str,
    recorded_sha256: &str,
) -> Result<Joined, String> {
    if Sha256::of(approved_text.as_bytes()).hex() != recorded_sha256 {
        return Err("the approved scores changed after the release recorded their hash".into());
    }
    let approved = parse_scores(approved_text)?;
    let unused = parse_scores(unused_text)?;
    covers(key, &approved, Which::Approved, "approved")?;
    covers(key, &unused, Which::Unused, "unused")?;
    let mut met = approved.met;
    met.extend(unused.met);
    join(
        key,
        &Scores {
            grader: approved.grader,
            met,
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rubric(n: usize, pass: usize) -> Rubric {
        Rubric {
            items: (0..n).map(|i| format!("item {i}")).collect(),
            pass_mark: pass,
        }
    }

    fn src(pair: &str, arm: Arm, art: u64, which: Which, spec: &str) -> Source {
        Source {
            pair: pair.into(),
            arm,
            run: format!("/runs/{pair}-{arm:?}:T"),
            artifact: art,
            which,
            spec: spec.into(),
            rubric: rubric(3, 2),
        }
    }

    fn sources() -> Vec<Source> {
        vec![
            src("auth", Arm::Control, 2, Which::Approved, "spec a"),
            src("auth", Arm::Treatment, 3, Which::Approved, "spec b"),
            src("auth", Arm::Treatment, 2, Which::Unused, "spec c"),
            src("cache", Arm::Control, 2, Which::Approved, "spec d"),
            src("cache", Arm::Treatment, 4, Which::Approved, "spec e"),
        ]
    }

    #[test]
    fn the_pack_carries_nothing_that_names_an_arm_a_pair_or_a_run() {
        let p = pack(&sources(), "s1");
        let text = rusty_serde::json::to_string(&[p.approved, p.unused].concat()).unwrap();
        for leak in [
            "auth",
            "cache",
            "control",
            "treatment",
            "approved",
            "unused",
            "/runs",
        ] {
            assert!(!text.contains(leak), "pack leaks {leak}: {text}");
        }
    }

    #[test]
    fn order_and_ids_do_not_depend_on_the_input_order() {
        let mut rev = sources();
        rev.reverse();
        assert_eq!(pack(&sources(), "s1"), pack(&rev, "s1"));
    }

    #[test]
    fn the_seed_changes_ids_and_order() {
        let (a, b) = (pack(&sources(), "s1"), pack(&sources(), "s2"));
        assert_ne!(
            a.approved.iter().map(|e| &e.id).collect::<Vec<_>>(),
            b.approved.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn identical_specs_in_two_arms_get_different_ids() {
        let s = vec![
            src("auth", Arm::Control, 2, Which::Approved, "same"),
            src("auth", Arm::Treatment, 2, Which::Approved, "same"),
        ];
        let p = pack(&s, "s").approved;
        assert_ne!(p[0].id, p[1].id);
    }

    fn scores(key: &Key, f: impl Fn(&KeyEntry) -> Vec<bool>) -> Scores {
        Scores {
            grader: "g".into(),
            met: key.entries.iter().map(|e| (e.id.clone(), f(e))).collect(),
        }
    }

    fn entry<'a>(k: &'a Key, pair: &str, arm: Arm, which: Which) -> &'a KeyEntry {
        k.entries
            .iter()
            .find(|e| e.pair == pair && e.arm == arm && e.which == which)
            .unwrap()
    }

    #[test]
    fn the_key_stays_sealed_until_every_id_is_scored() {
        let key = pack(&sources(), "s").key;
        let mut s = scores(&key, |_| vec![true; 3]);
        let drop = key.entries[0].id.clone();
        s.met.remove(&drop);
        let err = join(&key, &s).unwrap_err();
        assert!(err.contains("sealed"), "{err}");
    }

    #[test]
    fn a_wrong_verdict_count_or_an_unknown_id_is_refused() {
        let key = pack(&sources(), "s").key;
        let short = scores(&key, |_| vec![true; 2]);
        assert!(join(&key, &short).unwrap_err().contains("verdicts"));
        let mut extra = scores(&key, |_| vec![true; 3]);
        extra.met.insert("zz".into(), vec![true; 3]);
        assert!(join(&key, &extra).unwrap_err().contains("not in the pack"));
    }

    #[test]
    fn pass_mark_is_inclusive_and_lost_items_are_unused_met_and_approved_not() {
        let key = pack(&sources(), "s").key;
        let auth_t = entry(&key, "auth", Arm::Treatment, Which::Approved)
            .id
            .clone();
        let auth_u = entry(&key, "auth", Arm::Treatment, Which::Unused)
            .id
            .clone();
        let s = scores(&key, |e| {
            if e.id == auth_t {
                vec![true, true, false] // exactly the pass mark of 2
            } else if e.id == auth_u {
                vec![false, true, true] // item 2 is lost; item 1 is kept
            } else {
                vec![true, false, false]
            }
        });
        let j = join(&key, &s).unwrap();
        let t = j
            .runs
            .iter()
            .find(|r| r.pair == "auth" && r.arm == Arm::Treatment)
            .unwrap();
        assert_eq!(
            (t.met, t.rubric_ok, t.lost_rubric_items),
            (2, true, Some(1))
        );
        let c = j
            .runs
            .iter()
            .find(|r| r.pair == "auth" && r.arm == Arm::Control)
            .unwrap();
        assert_eq!((c.met, c.rubric_ok, c.lost_rubric_items), (1, false, None));
        // The cache treatment has no unused version: nothing lost, zero not None.
        let cache = j
            .runs
            .iter()
            .find(|r| r.pair == "cache" && r.arm == Arm::Treatment)
            .unwrap();
        assert_eq!(cache.lost_rubric_items, Some(0));
        assert_eq!(j.grader, "g");
    }

    #[test]
    fn a_rubric_needs_items_and_a_reachable_pass_mark() {
        assert!(rubric(3, 2).check().is_ok());
        assert!(rubric(0, 1).check().is_err());
        assert!(rubric(3, 0).check().is_err());
        assert!(rubric(3, 4).check().is_err());
    }

    #[test]
    fn the_approved_pack_has_two_entries_per_task_and_unused_versions_stay_apart() {
        // "auth" has an unused version, "cache" has none.
        let p = pack(&sources(), "s");
        assert_eq!(
            p.approved.len(),
            4,
            "two approved specs for each of two tasks"
        );
        assert_eq!(p.unused.len(), 1);
        let ids: BTreeSet<&str> = p.approved.iter().map(|e| e.id.as_str()).collect();
        assert!(p.unused.iter().all(|e| !ids.contains(e.id.as_str())));
        let mut per_task: BTreeMap<&str, usize> = BTreeMap::new();
        for e in p.key.entries.iter().filter(|e| ids.contains(e.id.as_str())) {
            *per_task.entry(e.pair.as_str()).or_default() += 1;
        }
        assert!(per_task.values().all(|n| *n == 2), "{per_task:?}");
        // The key still covers every id, so join sees the unused version.
        assert_eq!(p.key.entries.len(), 5);
    }

    // ---- the unused versions are released only after the approved scores are sealed

    fn scores_text(key: &Key, which: Which, f: impl Fn(&KeyEntry) -> Vec<bool>) -> String {
        let met: Vec<String> = key
            .entries
            .iter()
            .filter(|e| e.which == which)
            .map(|e| format!("\"{}\":{:?}", e.id, f(e)))
            .collect();
        format!(r#"{{"grader":"g","met":{{{}}}}}"#, met.join(","))
    }

    #[test]
    fn sealing_needs_every_approved_score_and_no_unused_one() {
        let key = pack(&sources(), "s").key;
        let ok = scores_text(&key, Which::Approved, |_| vec![true; 3]);
        let sha = seal(&key, &ok).unwrap();
        assert_eq!(sha, Sha256::of(ok.as_bytes()).hex());
        // A missing approved id.
        let first = key
            .entries
            .iter()
            .find(|e| e.which == Which::Approved)
            .unwrap();
        let partial = format!(
            r#"{{"grader":"g","met":{{"{}":[true,true,true]}}}}"#,
            first.id
        );
        assert!(seal(&key, &partial).unwrap_err().contains("approved"));
        // The wrong number of verdicts.
        let short = scores_text(&key, Which::Approved, |_| vec![true; 2]);
        assert!(seal(&key, &short).unwrap_err().contains("verdicts"));
        // An unused id scored before the release.
        let unused = key
            .entries
            .iter()
            .find(|e| e.which == Which::Unused)
            .unwrap();
        let peek = ok.replacen(
            r#""met":{"#,
            &format!(r#""met":{{"{}":[true,true,true],"#, unused.id),
            1,
        );
        assert!(seal(&key, &peek).unwrap_err().contains("unused"));
    }

    #[test]
    fn joining_needs_the_approved_scores_to_be_the_sealed_ones() {
        let key = pack(&sources(), "s").key;
        let approved = scores_text(&key, Which::Approved, |_| vec![true, true, false]);
        let sha = seal(&key, &approved).unwrap();
        let unused = scores_text(&key, Which::Unused, |_| vec![false, true, true]);
        let joined = join_sealed(&key, &approved, &unused, &sha).unwrap();
        assert!(joined.runs.iter().all(|r| r.rubric_ok));
        // Approved scores edited after the release: refused.
        let edited = scores_text(&key, Which::Approved, |_| vec![true, true, true]);
        let err = join_sealed(&key, &edited, &unused, &sha).unwrap_err();
        assert!(err.contains("after the release"), "{err}");
        // Unused scores that miss an id, or that re-score an approved one.
        let none = r#"{"grader":"g","met":{}}"#;
        assert!(join_sealed(&key, &approved, none, &sha).is_err());
        let both = scores_text(&key, Which::Approved, |_| vec![true; 3]);
        assert!(join_sealed(&key, &approved, &both, &sha).is_err());
    }
}
