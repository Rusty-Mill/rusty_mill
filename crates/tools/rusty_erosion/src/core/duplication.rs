//! Duplication: the share of tokens covered by a repeated window of
//! [`WINDOW`] normalized tokens.
//!
//! A window is repeated when the same token sequence occurs at two places that
//! do not overlap: in two files, or at least [`WINDOW`] tokens apart in one.
//! Matches are confirmed by comparing tokens, not only hashes. Files only
//! match files of the same language. Any run of 50 or more repeated tokens
//! is covered; 49 is not.

use std::collections::HashMap;

use super::{FileMeasure, Lang, NormTok, Ratio};

/// Tokens in one window.
pub const WINDOW: usize = 50;

/// A window's place: file index and token offset.
type Pos = (usize, usize);

/// Covered tokens over all tokens of `files`.
pub fn share(files: &[FileMeasure]) -> Ratio {
    let mut covered = vec![Vec::new(); files.len()];
    for (i, f) in files.iter().enumerate() {
        covered[i] = vec![false; f.tokens.len()];
    }
    for group in candidate_groups(files) {
        mark_repeats(files, &group, &mut covered);
    }
    Ratio {
        num: covered.iter().flatten().filter(|c| **c).count() as u64,
        den: files.iter().map(|f| f.tokens.len() as u64).sum(),
    }
}

/// Window positions grouped by language and window hash, with fewer than two
/// members dropped. Hash equality is only a candidate match.
fn candidate_groups(files: &[FileMeasure]) -> Vec<Vec<Pos>> {
    let mut by_hash: HashMap<(Lang, u64), Vec<Pos>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        for (start, h) in window_hashes(&f.tokens).into_iter().enumerate() {
            by_hash.entry((f.lang, h)).or_default().push((i, start));
        }
    }
    by_hash.into_values().filter(|g| g.len() > 1).collect()
}

/// Splits a hash group into exact-equal sequences and marks every sequence
/// that occurs at two non-overlapping places.
fn mark_repeats(files: &[FileMeasure], group: &[Pos], covered: &mut [Vec<bool>]) {
    let window = |(i, s): Pos| &files[i].tokens[s..s + WINDOW];
    let mut exact: Vec<Vec<Pos>> = Vec::new();
    for &pos in group {
        match exact.iter_mut().find(|e| window(e[0]) == window(pos)) {
            Some(e) => e.push(pos),
            None => exact.push(vec![pos]),
        }
    }
    for places in exact.iter().filter(|e| repeats(e)) {
        for &(i, s) in places {
            covered[i][s..s + WINDOW].iter_mut().for_each(|c| *c = true);
        }
    }
}

/// Whether sorted places hold two that do not overlap.
fn repeats(places: &[Pos]) -> bool {
    let (Some(first), Some(last)) = (places.first(), places.last()) else {
        return false;
    };
    first.0 != last.0 || last.1 - first.1 >= WINDOW
}

/// A polynomial hash of each window, rolled across the stream.
fn window_hashes(tokens: &[NormTok]) -> Vec<u64> {
    if tokens.len() < WINDOW {
        return Vec::new();
    }
    const B: u64 = 0x100_0000_01b3;
    let top = (1..WINDOW).fold(1u64, |p, _| p.wrapping_mul(B));
    let th: Vec<u64> = tokens.iter().map(|t| fnv(t)).collect();
    let mut h = th[..WINDOW]
        .iter()
        .fold(0u64, |h, t| h.wrapping_mul(B).wrapping_add(*t));
    let mut out = vec![h];
    for k in WINDOW..th.len() {
        h = h
            .wrapping_sub(th[k - WINDOW].wrapping_mul(top))
            .wrapping_mul(B)
            .wrapping_add(th[k]);
        out.push(h);
    }
    out
}

/// FNV-1a: fixed, so hashes are stable across runs and platforms.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` distinct tokens, so no window repeats.
    fn unique(n: usize, seed: usize) -> Vec<NormTok> {
        (0..n)
            .map(|k| &*Box::leak(format!("t{seed}_{k}").into_boxed_str()))
            .collect()
    }

    fn file(path: &str, tokens: Vec<NormTok>) -> FileMeasure {
        FileMeasure {
            path: path.into(),
            lang: Lang::Rust,
            functions: vec![],
            tokens,
            loc: 0,
            parse_error: false,
        }
    }

    #[test]
    fn rolling_hash_matches_direct_hash() {
        let t = unique(120, 1);
        let rolled = window_hashes(&t);
        for (s, h) in rolled.iter().enumerate() {
            assert_eq!(*h, window_hashes(&t[s..s + WINDOW])[0]);
        }
    }

    #[test]
    fn short_streams_have_no_windows() {
        assert!(window_hashes(&unique(49, 0)).is_empty());
        assert_eq!(
            share(&[file("a.rs", unique(49, 0))]),
            Ratio { num: 0, den: 49 }
        );
    }

    #[test]
    fn overlapping_self_match_is_not_a_repeat() {
        assert!(!repeats(&[(0, 0), (0, 49)]));
        assert!(repeats(&[(0, 0), (0, 50)]));
        assert!(repeats(&[(0, 3), (1, 3)]));
    }

    /// Two files sharing a run of `n` tokens between unique tokens.
    fn shared_run(n: usize) -> Ratio {
        let run = unique(n, 9);
        let wrap = |seed| [unique(7, seed), run.clone(), unique(7, seed + 1)].concat();
        share(&[file("a.rs", wrap(1)), file("b.rs", wrap(3))])
    }

    #[test]
    fn a_49_token_run_is_not_duplication_but_50_is() {
        assert_eq!(shared_run(49).num, 0);
        assert_eq!(shared_run(50).num, 100);
        assert_eq!(shared_run(60).num, 120);
    }

    #[test]
    fn languages_and_hash_buckets_confirm_by_tokens() {
        let a = file("a.rs", unique(60, 2));
        assert_eq!(share(&[a.clone(), a.clone()]).num, 120);
        assert_eq!(share(&[a.clone(), file("b.rs", unique(60, 4))]).num, 0);
    }
}
