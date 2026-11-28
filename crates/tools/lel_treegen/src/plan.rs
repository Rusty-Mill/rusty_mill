//! The pure planner: `(seed, scale, profile) -> Manifest`, with no I/O.
//!
//! The tree is a sequence of *units*. Unit `i` is drawn only from the keyed
//! stream `(seed, UNITS, i)`, and units are emitted until the planned bytes
//! reach `scale * FULL_TREE_BYTES`. So a smaller scale is an exact prefix of a
//! larger one, and the fixed edge-case units come first in every slice.
//!
//! Layout under the output directory: `local/` and `nas/` are the scanned
//! roots; `outside/` is never scanned and holds the targets of the links that
//! leave the roots.

use serde::{Deserialize, Serialize};

use crate::profile::{MAX_FILE_BYTES, Profile, ProfileError};
use crate::rng::Rng;

/// Bytes of the full-scale tree (500 GB, decimal).
pub const FULL_TREE_BYTES: u64 = 500_000_000_000;
/// The block a decoy or same-prefix pair is built around (64 KiB).
pub const BLOCK: u64 = 64 << 10;
/// Metadata bytes per richness step of an image variant.
pub const META_STEP: u64 = 64;

const STREAM_UNITS: u64 = 1;
const SYMLINKS: u64 = 50;
const HARDLINK_PAIRS: u64 = 20;
const ZERO_BYTE_FILES: u64 = 5;
const SPECIAL_FILE_BYTES: u64 = 4096;

/// What a file's bytes are; the writer derives them from this alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    /// Zero bytes.
    Empty,
    /// Pseudo-random bytes from content id `id`.
    Opaque { id: u64 },
    /// The bytes of `id` with each byte at the `flips` offsets inverted.
    Variant { id: u64, flips: [u64; 2] },
    /// The first `prefix_len` bytes from `prefix`, the rest from `tail`.
    Prefix {
        prefix: u64,
        tail: u64,
        prefix_len: u64,
    },
    /// An image; same `pixels` means identical pixel data.
    Image(ImageSpec),
}

/// Image container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    /// RGB PNG, stored (uncompressed) deflate; metadata in `tEXt`/`eXIf`.
    Png,
    /// Grayscale baseline JPEG, DC only; metadata in APP1, padding in COM.
    Jpeg,
}

/// One image. The writer pads to exactly [`ImageSpec::size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSpec {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// Pixel content id.
    pub pixels: u64,
    /// Metadata content id.
    pub meta: u64,
    /// 0..=3; the group member with the highest is the suggested keeper.
    pub richness: u8,
}

impl ImageSpec {
    /// The file size the writer must produce: an upper bound on the encoded
    /// image plus `META_STEP * richness`, padded exactly.
    #[must_use]
    pub fn size(&self) -> u64 {
        let (w, h) = (u64::from(self.width), u64::from(self.height));
        let base = match self.format {
            ImageFormat::Png => {
                let raw = h * (1 + 3 * w);
                let zlib = 2 + raw + 5 * raw.div_ceil(65_535) + 4;
                // Signature, IHDR, IDAT, one padding chunk, IEND.
                8 + 25 + (12 + zlib) + 12 + 12
            }
            // Fixed header budget plus a worst case of 8 bytes per 8x8 block.
            ImageFormat::Jpeg => 1024 + (w / 8) * (h / 8) * 8,
        };
        base + META_STEP * u64::from(self.richness)
    }
}

/// What kind of filesystem entry a record is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// A regular file.
    File,
    /// A regular file the writer makes unreadable.
    Unreadable,
    /// A directory the writer makes unreadable (its contents are listed separately).
    UnreadableDir,
    /// A symbolic link to `target`, relative to the link's directory.
    Symlink { target: String },
    /// A second name for the file at `target` (a path from the output root).
    Hardlink { target: String },
}

/// One entry of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Path from the output root, `/`-separated.
    pub path: String,
    /// Bytes of the file (0 for links and directories).
    pub size: u64,
    pub kind: Kind,
    pub content: Content,
    /// The expected group this entry belongs to, if any.
    pub group: Option<u64>,
}

/// How the members of a group match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Byte-identical files.
    Identical,
    /// Identical pixel data, different metadata.
    SamePixels,
}

/// One expected group, the answer a candidate is checked against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The unit index that made the group.
    pub id: u64,
    pub match_kind: MatchKind,
    pub members: Vec<String>,
    /// The suggested keeper (same-pixels groups: the richest metadata).
    pub keeper: Option<String>,
}

/// The pure plan: every entry and every expected group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub seed: u64,
    pub scale: f64,
    pub entries: Vec<Entry>,
    pub groups: Vec<Group>,
}

impl Plan {
    /// Total bytes of regular files.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

/// What one unit adds.
#[derive(Debug, Default)]
struct Unit {
    entries: Vec<Entry>,
    group: Option<Group>,
}

/// Plans the tree.
///
/// # Errors
/// An invalid profile, or a scale that is not finite and in `(0, 4]`.
pub fn plan(seed: u64, scale: f64, profile: &Profile) -> Result<Plan, ProfileError> {
    profile.validate()?;
    if !(scale.is_finite() && scale > 0.0 && scale <= 4.0) {
        return Err(ProfileError(format!("scale {scale} must be in (0, 4]")));
    }
    // The product is far below 2^53; truncating to whole bytes is intended.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let target = (scale * FULL_TREE_BYTES as f64) as u64;
    let tables = Tables::new(profile);
    let mut out = Plan {
        seed,
        scale,
        entries: Vec::new(),
        groups: Vec::new(),
    };
    let mut bytes = 0u64;
    let mut index = 0u64;
    while bytes < target {
        let unit = make_unit(seed, index, profile, &tables);
        bytes += unit.entries.iter().map(|e| e.size).sum::<u64>();
        out.entries.extend(unit.entries);
        out.groups.extend(unit.group);
        index += 1;
    }
    Ok(out)
}

/// Cumulative tables for weighted draws, built once per plan.
struct Tables {
    sizes: Weighted<u32>,
    depth: Weighted<u32>,
    exts: Weighted<String>,
}

impl Tables {
    fn new(p: &Profile) -> Self {
        Self {
            sizes: Weighted::new(p.size_log2_files.iter().map(|(&k, &w)| (k, w))),
            depth: Weighted::new(p.depth.iter().map(|(&k, &w)| (k, w))),
            exts: Weighted::new(p.opaque_exts.iter().map(|(k, &w)| (k.clone(), w))),
        }
    }
}

struct Weighted<T> {
    items: Vec<(T, u64)>,
    total: u64,
}

impl<T: Clone> Weighted<T> {
    fn new(pairs: impl Iterator<Item = (T, u64)>) -> Self {
        let mut total = 0;
        let items = pairs
            .filter(|(_, w)| *w > 0)
            .map(|(k, w)| {
                total += w;
                (k, total)
            })
            .collect();
        Self { items, total }
    }

    fn draw(&self, rng: &mut Rng) -> T {
        let x = rng.below(self.total);
        let i = self.items.partition_point(|(_, cum)| *cum <= x);
        self.items[i].0.clone()
    }
}

/// Number of fixed edge-case units at the start of every tree.
const SPECIAL_UNITS: u64 = SYMLINKS + HARDLINK_PAIRS + 1;

fn make_unit(seed: u64, index: u64, p: &Profile, t: &Tables) -> Unit {
    let mut rng = Rng::keyed(seed, STREAM_UNITS, index);
    if index < SYMLINKS {
        return symlink_unit(index, &mut rng);
    }
    if index < SYMLINKS + HARDLINK_PAIRS {
        return hardlink_unit(index, &mut rng);
    }
    if index < SPECIAL_UNITS {
        return edge_unit(&mut rng);
    }
    random_unit(index, p, t, &mut rng)
}

fn special(path: String, rng: &mut Rng) -> Entry {
    Entry {
        path,
        size: SPECIAL_FILE_BYTES,
        kind: Kind::File,
        content: Content::Opaque { id: rng.next_u64() },
        group: None,
    }
}

/// An in-tree unique file, its copy in `outside/`, and a link to the copy.
/// Following the link would invent a group the manifest does not have.
fn symlink_unit(index: u64, rng: &mut Rng) -> Unit {
    let original = special(format!("local/links/orig{index}.bin"), rng);
    let copy = Entry {
        path: format!("outside/copy{index}.bin"),
        ..original.clone()
    };
    let link = Entry {
        path: format!("local/links/link{index}.bin"),
        size: 0,
        kind: Kind::Symlink {
            target: format!("../../outside/copy{index}.bin"),
        },
        content: Content::Empty,
        group: None,
    };
    Unit {
        entries: vec![original, copy, link],
        group: None,
    }
}

/// One inode with two names: never a reclaimable duplicate, so no group.
fn hardlink_unit(index: u64, rng: &mut Rng) -> Unit {
    let first = special(format!("local/hard/a/h{index}.bin"), rng);
    let second = Entry {
        path: format!("local/hard/b/h{index}.bin"),
        size: 0,
        kind: Kind::Hardlink {
            target: first.path.clone(),
        },
        content: Content::Empty,
        group: None,
    };
    Unit {
        entries: vec![first, second],
        group: None,
    }
}

/// Zero-byte files, unreadable entries, Unicode and 255-byte names, a path
/// over 260 characters and depth 40.
fn edge_unit(rng: &mut Rng) -> Unit {
    let mut entries: Vec<Entry> = (0..ZERO_BYTE_FILES)
        .map(|i| Entry {
            path: format!("local/edge/zero/z{i}.txt"),
            size: 0,
            kind: Kind::File,
            content: Content::Empty,
            group: None,
        })
        .collect();
    let mut add = |path: String, kind: Kind, rng: &mut Rng| {
        let mut e = special(path, rng);
        if kind == Kind::UnreadableDir {
            e.size = 0;
            e.content = Content::Empty;
        }
        e.kind = kind;
        entries.push(e);
    };
    add("local/edge/locked.bin".into(), Kind::Unreadable, rng);
    add("local/edge/lockeddir".into(), Kind::UnreadableDir, rng);
    add("local/edge/lockeddir/inside.bin".into(), Kind::File, rng);
    add(
        "local/edge/unicode/caf\u{e9} \u{65e5}\u{672c} \u{1f600}.bin".into(),
        Kind::File,
        rng,
    );
    add(format!("local/edge/{}", "n".repeat(255)), Kind::File, rng);
    let long_dir = (0..6)
        .map(|i| format!("{i}{}", "l".repeat(48)))
        .collect::<Vec<_>>()
        .join("/");
    add(
        format!("local/edge/long/{long_dir}/file.bin"),
        Kind::File,
        rng,
    );
    add(
        format!("local/edge/deep/{}/file.bin", vec!["d"; 40].join("/")),
        Kind::File,
        rng,
    );
    Unit {
        entries,
        group: None,
    }
}

/// Root and directory for one entry, drawn per entry so copies spread across
/// folders and across the local/NAS split.
fn place(index: u64, p: &Profile, t: &Tables, rng: &mut Rng) -> String {
    let root = if rng.chance(p.nas_share) {
        "nas"
    } else {
        "local"
    };
    let depth = t.depth.draw(rng);
    let dirs: Vec<String> = (0..depth)
        .map(|_| format!("d{}", rng.below(u64::from(p.dir_width))))
        .collect();
    format!("{root}/{}/u{index}", dirs.join("/"))
}

fn draw_size(t: &Tables, rng: &mut Rng) -> u64 {
    let b = t.sizes.draw(rng);
    let lo = 1u64 << (b - 1);
    rng.range(lo, (2 * lo - 1).min(MAX_FILE_BYTES))
}

fn file(path: String, size: u64, content: Content, group: Option<u64>) -> Entry {
    Entry {
        path,
        size,
        kind: Kind::File,
        content,
        group,
    }
}

fn random_unit(index: u64, p: &Profile, t: &Tables, rng: &mut Rng) -> Unit {
    let pick = rng.unit();
    let ext = t.exts.draw(rng);
    if pick < p.identical_rate {
        return identical_unit(index, &ext, p, t, rng);
    }
    if pick < p.identical_rate + p.decoy_rate {
        return pair_unit(index, &ext, p, t, rng, true);
    }
    if pick < p.identical_rate + p.decoy_rate + p.prefix_rate {
        return pair_unit(index, &ext, p, t, rng, false);
    }
    if rng.chance(p.image_share) {
        return image_unit(index, p, t, rng);
    }
    let size = draw_size(t, rng);
    let path = format!("{}/f.{ext}", place(index, p, t, rng));
    Unit {
        entries: vec![file(
            path,
            size,
            Content::Opaque { id: rng.next_u64() },
            None,
        )],
        group: None,
    }
}

/// An original and 1 to 4 byte-identical copies.
fn identical_unit(index: u64, ext: &str, p: &Profile, t: &Tables, rng: &mut Rng) -> Unit {
    let size = draw_size(t, rng);
    let id = rng.next_u64();
    let copies = rng.range(1, 4);
    let entries: Vec<Entry> = (0..=copies)
        .map(|m| {
            file(
                format!("{}/c{m}.{ext}", place(index, p, t, rng)),
                size,
                Content::Opaque { id },
                Some(index),
            )
        })
        .collect();
    let members = entries.iter().map(|e| e.path.clone()).collect();
    let group = Group {
        id: index,
        match_kind: MatchKind::Identical,
        members,
        keeper: None,
    };
    Unit {
        entries,
        group: Some(group),
    }
}

/// Two files of the same size and leaf name in different folders: a decoy
/// (one byte flipped mid-file and one in the last block) or a pair sharing the
/// first 64 KiB. Neither is a group.
fn pair_unit(index: u64, ext: &str, p: &Profile, t: &Tables, rng: &mut Rng, decoy: bool) -> Unit {
    let size = draw_size(t, rng).max(2 * BLOCK);
    let (a, b) = if decoy {
        let id = rng.next_u64();
        let last = size - 1 - rng.below(BLOCK.min(size / 2));
        (
            Content::Opaque { id },
            Content::Variant {
                id,
                flips: [size / 2, last],
            },
        )
    } else {
        let prefix = rng.next_u64();
        let pair = |tail: u64| Content::Prefix {
            prefix,
            tail,
            prefix_len: BLOCK,
        };
        (pair(rng.next_u64()), pair(rng.next_u64()))
    };
    let name = if decoy {
        format!("same.{ext}")
    } else {
        format!("head.{ext}")
    };
    let mut paths = [place(index, p, t, rng), place(index, p, t, rng)];
    if paths[0] == paths[1] {
        paths[1].push('b');
    }
    let [pa, pb] = paths.map(|d| format!("{d}/{name}"));
    Unit {
        entries: vec![file(pa, size, a, None), file(pb, size, b, None)],
        group: None,
    }
}

/// A single image, or a same-pixels group of 2 or 3 differing only in metadata.
fn image_unit(index: u64, p: &Profile, t: &Tables, rng: &mut Rng) -> Unit {
    let format = if rng.chance(0.5) {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };
    // Dimension draws are at most 192, so the casts cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    let (width, height) = (8 * rng.range(8, 192) as u32, 8 * rng.range(8, 192) as u32);
    let pixels = rng.next_u64();
    let ext = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
    };
    let members = if rng.chance(p.pixel_rate) {
        rng.range(2, 3)
    } else {
        1
    };
    // Distinct richness per member, shuffled, so exactly one keeper is richest.
    let mut richness: Vec<u8> = vec![0, 1, 2, 3];
    for i in (1..richness.len()).rev() {
        // `i` is below 4, so the cast cannot truncate.
        #[allow(clippy::cast_possible_truncation)]
        let j = rng.below(i as u64 + 1) as usize;
        richness.swap(i, j);
    }
    let grouped = members > 1;
    let entries: Vec<Entry> = (0..members)
        .map(|m| {
            // `m` is below 3.
            #[allow(clippy::cast_possible_truncation)]
            let r = richness[m as usize];
            let spec = ImageSpec {
                format,
                width,
                height,
                pixels,
                meta: rng.next_u64(),
                richness: r,
            };
            let path = format!("{}/img{m}.{ext}", place(index, p, t, rng));
            file(
                path,
                spec.size(),
                Content::Image(spec),
                grouped.then_some(index),
            )
        })
        .collect();
    if !grouped {
        return Unit {
            entries,
            group: None,
        };
    }
    let keeper = entries
        .iter()
        .max_by_key(|e| match &e.content {
            Content::Image(s) => s.richness,
            _ => 0,
        })
        .map(|e| e.path.clone());
    let members = entries.iter().map(|e| e.path.clone()).collect();
    let group = Group {
        id: index,
        match_kind: MatchKind::SamePixels,
        members,
        keeper,
    };
    Unit {
        entries,
        group: Some(group),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    fn small(seed: u64, scale: f64) -> Plan {
        plan(seed, scale, &Profile::default()).expect("valid plan")
    }

    #[test]
    fn same_seed_same_plan_other_seed_other_plan() {
        assert_eq!(small(1, 0.001), small(1, 0.001));
        assert_ne!(small(1, 0.001).entries, small(2, 0.001).entries);
    }

    #[test]
    fn smaller_scale_is_an_exact_prefix() {
        let big = small(9, 0.004);
        let little = small(9, 0.001);
        assert!(little.entries.len() < big.entries.len());
        assert_eq!(big.entries[..little.entries.len()], little.entries[..]);
        assert_eq!(big.groups[..little.groups.len()], little.groups[..]);
    }

    #[test]
    fn bytes_reach_the_target_and_paths_are_unique() {
        let p = small(3, 0.002);
        assert!(p.bytes() >= 1_000_000_000);
        let mut seen = HashSet::new();
        assert!(
            p.entries.iter().all(|e| seen.insert(e.path.as_str())),
            "duplicate path"
        );
    }

    #[test]
    fn fixed_edge_cases_are_in_every_slice() {
        let p = small(4, 0.0001);
        let count = |f: &dyn Fn(&Entry) -> bool| p.entries.iter().filter(|e| f(e)).count();
        assert_eq!(count(&|e| matches!(e.kind, Kind::Symlink { .. })), 50);
        assert_eq!(count(&|e| matches!(e.kind, Kind::Hardlink { .. })), 20);
        assert_eq!(count(&|e| e.path.starts_with("outside/")), 50);
        assert_eq!(count(&|e| e.kind == Kind::Unreadable), 1);
        assert_eq!(count(&|e| e.kind == Kind::UnreadableDir), 1);
        assert!(p.entries.iter().any(|e| e.path.len() > 260));
        assert!(p.entries.iter().any(|e| e.path.split('/').count() > 40));
        assert!(
            p.entries
                .iter()
                .all(|e| e.path.split('/').all(|c| c.len() <= 255))
        );
        assert!(
            p.entries
                .iter()
                .filter(|e| e.path.starts_with("outside/"))
                .all(|e| e.group.is_none())
        );
    }

    #[test]
    fn rates_hold_within_tolerance() {
        let prof = Profile::default();
        let p = small(5, 0.1);
        let units = p
            .entries
            .iter()
            .filter_map(|e| e.path.split("/u").nth(1))
            .filter_map(|s| s.split('/').next());
        let n = units.collect::<HashSet<_>>().len() as f64;
        assert!(n > 5_000.0, "enough units for the check: {n}");
        let identical = p
            .groups
            .iter()
            .filter(|g| g.match_kind == MatchKind::Identical)
            .count() as f64;
        assert!(
            (identical / n - prof.identical_rate).abs() < 0.02,
            "identical {}",
            identical / n
        );
        let decoys = p
            .entries
            .iter()
            .filter(|e| matches!(e.content, Content::Variant { .. }))
            .count() as f64;
        assert!(
            (decoys / n - prof.decoy_rate).abs() < 0.005,
            "decoys {}",
            decoys / n
        );
        let nas: u64 = p
            .entries
            .iter()
            .filter(|e| e.path.starts_with("nas/"))
            .map(|e| e.size)
            .sum();
        let nas_share = nas as f64 / p.bytes() as f64;
        assert!((nas_share - 0.25).abs() < 0.1, "nas byte share {nas_share}");
    }

    #[test]
    fn decoys_and_prefix_pairs_never_share_a_group() {
        let p = small(6, 0.05);
        let by_path: HashMap<&str, &Entry> =
            p.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        let pairs: Vec<&Entry> = p
            .entries
            .iter()
            .filter(|e| matches!(e.content, Content::Variant { .. } | Content::Prefix { .. }))
            .collect();
        assert!(!pairs.is_empty());
        assert!(pairs.iter().all(|e| e.group.is_none()));
        for g in &p.groups {
            let contents: HashSet<String> = g
                .members
                .iter()
                .map(|m| format!("{:?}", by_path[m.as_str()].content))
                .collect();
            match g.match_kind {
                MatchKind::Identical => assert_eq!(contents.len(), 1, "identical group differs"),
                MatchKind::SamePixels => assert!(g.keeper.is_some()),
            }
        }
    }

    #[test]
    fn decoy_flips_land_mid_file_and_in_the_last_block() {
        let p = small(7, 0.05);
        for e in &p.entries {
            if let Content::Variant {
                flips: [mid, last], ..
            } = e.content
            {
                assert_eq!(mid, e.size / 2);
                assert!(last < e.size && last >= e.size - BLOCK);
            }
        }
    }

    #[test]
    fn same_pixels_groups_share_pixels_and_name_the_richest_keeper() {
        let p = small(8, 0.1);
        let by_path: HashMap<&str, &Entry> =
            p.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        let groups: Vec<&Group> = p
            .groups
            .iter()
            .filter(|g| g.match_kind == MatchKind::SamePixels)
            .collect();
        assert!(!groups.is_empty());
        for g in groups {
            let specs: Vec<ImageSpec> = g
                .members
                .iter()
                .map(|m| match by_path[m.as_str()].content {
                    Content::Image(s) => s,
                    ref c => panic!("not an image: {c:?}"),
                })
                .collect();
            assert!(
                specs
                    .windows(2)
                    .all(|w| w[0].pixels == w[1].pixels && w[0].meta != w[1].meta)
            );
            let best = specs.iter().map(|s| s.richness).max();
            let keeper = g.keeper.as_deref().map(|k| by_path[k]);
            assert!(
                matches!(keeper.map(|e| &e.content), Some(Content::Image(s)) if Some(s.richness) == best)
            );
        }
    }

    #[test]
    fn bad_scale_is_refused() {
        for s in [0.0, -1.0, f64::NAN, 5.0] {
            assert!(plan(1, s, &Profile::default()).is_err(), "scale {s}");
        }
    }

    #[test]
    fn image_sizes_follow_the_format_bound() {
        let png = ImageSpec {
            format: ImageFormat::Png,
            width: 64,
            height: 64,
            pixels: 1,
            meta: 2,
            richness: 0,
        };
        let raw = 64 * (1 + 3 * 64);
        assert_eq!(png.size(), 8 + 25 + 12 + (2 + raw + 5 + 4) + 12 + 12);
        let jpg = ImageSpec {
            format: ImageFormat::Jpeg,
            richness: 2,
            ..png
        };
        assert_eq!(jpg.size(), 1024 + 64 * 8 + 2 * META_STEP);
    }
}
