//! The shape the tree is drawn from: the built-in default, or one derived from
//! the aggregate JSON of `1319-drive-stats.ps1` (`lel-drive-stats/2`).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Largest log2 size bucket drawn: bucket `b` holds sizes in `[2^(b-1), 2^b)`.
pub const MAX_BUCKET: u32 = 34;
/// Largest file the generator writes (8 GiB).
pub const MAX_FILE_BYTES: u64 = 8 << 30;

/// Image extensions counted as images when reading drive stats.
const IMAGE_EXTS: [&str; 14] = [
    "jpg", "jpeg", "png", "heic", "gif", "webp", "tif", "tiff", "bmp", "raw", "cr2", "nef", "arw",
    "dng",
];

/// Generation profile. Rates are shares of *units*: one unit is an original
/// with its copies, a decoy pair, a same-prefix pair, an image group or a
/// single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Where the profile came from: `default` or `stats:<sha256 of the stats file>`.
    pub source: String,
    /// Relative file counts per log2 size bucket (1..=`MAX_BUCKET`).
    pub size_log2_files: BTreeMap<u32, u64>,
    /// Relative counts per directory depth below the share root (1..=30).
    pub depth: BTreeMap<u32, u64>,
    /// Distinct directory names per level.
    pub dir_width: u32,
    /// Share of entries placed under `nas/`.
    pub nas_share: f64,
    /// Share of single units that are images.
    pub image_share: f64,
    /// Share of units that are byte-identical groups (an original plus 1 to 4 copies).
    pub identical_rate: f64,
    /// Share of image units that are same-pixels groups of 2 or 3.
    pub pixel_rate: f64,
    /// Share of units that are decoy pairs: same name and size, different bytes.
    pub decoy_rate: f64,
    /// Share of units that are pairs sharing the first 64 KiB, different tail.
    pub prefix_rate: f64,
    /// Relative weights of extensions for opaque (non-image) files.
    pub opaque_exts: BTreeMap<String, u64>,
}

/// Why a profile or stats file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError(pub String);

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "profile: {}", self.0)
    }
}

impl std::error::Error for ProfileError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ProfileError> {
    Err(ProfileError(msg.into()))
}

impl Default for Profile {
    /// Log-normal-ish sizes, median about 256 KiB, long tail to 8 GiB; the
    /// rates of the approved design (`1319-synthetic-tree-design.md` §2).
    fn default() -> Self {
        // A log2-normal curve, mean bucket 19, sigma 3.5, in parts per million.
        // Kept as integers: `f64::exp` may differ by an ulp across platforms,
        // and a seed must give the same tree everywhere.
        const SIZE_PPM: [u64; MAX_BUCKET as usize] = [
            1, 7, 28, 102, 335, 1_009, 2_801, 7_163, 16_879, 36_658, 73_369, 135_335, 230_066,
            360_447, 520_450, 692_569, 849_365, 960_005, 1_000_000, 960_005, 849_365, 692_569,
            520_450, 360_447, 230_066, 135_335, 73_369, 36_658, 16_879, 7_163, 2_801, 1_009, 335,
            102,
        ];
        let size_log2_files = (1..=MAX_BUCKET).zip(SIZE_PPM).collect();
        let depth = (1..=12u32)
            .map(|d| (d, u64::from(13 - d.abs_diff(5).min(12))))
            .collect();
        let opaque_exts = [
            ("bin", 10),
            ("mp4", 8),
            ("mov", 4),
            ("pdf", 6),
            ("zip", 4),
            ("docx", 3),
            ("txt", 5),
            ("mkv", 2),
        ]
        .into_iter()
        .map(|(e, w)| (e.to_owned(), w))
        .collect();
        Self {
            source: "default".to_owned(),
            size_log2_files,
            depth,
            dir_width: 16,
            nas_share: 0.25,
            image_share: 0.30,
            identical_rate: 0.15,
            pixel_rate: 0.03,
            decoy_rate: 0.01,
            prefix_rate: 0.005,
            opaque_exts,
        }
    }
}

impl Profile {
    /// Checks every field, so the planner can rely on it.
    ///
    /// # Errors
    /// A rate outside `[0, 1]`, unit rates summing above 1, an empty or
    /// all-zero histogram, a bucket or depth out of range, or a zero width.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let rates = [
            ("nas_share", self.nas_share),
            ("image_share", self.image_share),
            ("identical_rate", self.identical_rate),
            ("pixel_rate", self.pixel_rate),
            ("decoy_rate", self.decoy_rate),
            ("prefix_rate", self.prefix_rate),
        ];
        if let Some((name, r)) = rates.iter().find(|(_, r)| !(0.0..=1.0).contains(r)) {
            return err(format!("{name} = {r} is outside [0, 1]"));
        }
        if self.identical_rate + self.decoy_rate + self.prefix_rate > 1.0 {
            return err("identical_rate + decoy_rate + prefix_rate exceed 1");
        }
        check_hist("size_log2_files", &self.size_log2_files, 1, MAX_BUCKET)?;
        check_hist("depth", &self.depth, 1, 30)?;
        if self.dir_width == 0 {
            return err("dir_width must be at least 1");
        }
        if self.opaque_exts.values().all(|&w| w == 0) {
            return err("opaque_exts needs a positive weight");
        }
        if let Some(e) = self.opaque_exts.keys().find(|e| !is_plain_ext(e)) {
            return err(format!(
                "extension {e:?} must be 1-8 lowercase ASCII letters or digits"
            ));
        }
        Ok(())
    }

    /// sha256 of the canonical JSON of the profile, recorded in the manifest.
    ///
    /// # Errors
    /// Only if serialization fails, which a validated profile never does.
    pub fn digest(&self) -> Result<String, ProfileError> {
        let json = serde_json::to_vec(self).map_err(|e| ProfileError(e.to_string()))?;
        Ok(hex(&Sha256::digest(json)))
    }

    /// Derives a profile from a `lel-drive-stats/2` report: size, depth and
    /// fan-out histograms, the NAS byte share and the image share. Duplicate
    /// rates stay at the defaults: the stats pass cannot measure them by kind.
    ///
    /// # Errors
    /// Not JSON, the wrong schema, a missing section, or a derived profile that
    /// fails [`Profile::validate`].
    pub fn from_stats(json: &str) -> Result<Self, ProfileError> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ProfileError(format!("stats JSON: {e}")))?;
        if v["schema"] != "lel-drive-stats/2" {
            return err("stats schema must be lel-drive-stats/2");
        }
        let mut p = Self {
            source: format!("stats:{}", hex(&Sha256::digest(json.as_bytes()))),
            ..Self::default()
        };
        p.size_log2_files = hist_of(&v["size_log2_hist"], |e| e["files"].as_u64())?
            .into_iter()
            .filter(|&(b, _)| (1..=MAX_BUCKET).contains(&b))
            .collect();
        p.depth = hist_of(&v["depth_hist"], serde_json::Value::as_u64)?
            .into_iter()
            .filter(|&(d, _)| (1..=30).contains(&d))
            .collect();
        p.dir_width = median_fanout(&hist_of(
            &v["entries_per_dir_log2_hist"],
            serde_json::Value::as_u64,
        )?);
        p.nas_share = nas_share(&v)?;
        p.image_share = image_share(&v["extensions"]);
        p.validate()?;
        Ok(p)
    }
}

fn is_plain_ext(e: &str) -> bool {
    (1..=8).contains(&e.len())
        && e.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn check_hist(name: &str, h: &BTreeMap<u32, u64>, lo: u32, hi: u32) -> Result<(), ProfileError> {
    if h.values().all(|&w| w == 0) {
        return err(format!("{name} needs a positive weight"));
    }
    match h.keys().find(|k| !(lo..=hi).contains(*k)) {
        Some(k) => err(format!("{name} key {k} is outside {lo}..={hi}")),
        None => Ok(()),
    }
}

/// A stats histogram object (`{"3": ...}`) as key -> count.
fn hist_of(
    v: &serde_json::Value,
    count: impl Fn(&serde_json::Value) -> Option<u64>,
) -> Result<BTreeMap<u32, u64>, ProfileError> {
    let Some(obj) = v.as_object() else {
        return err("stats histogram missing or not an object");
    };
    obj.iter()
        .map(|(k, e)| {
            let key = k
                .parse::<u32>()
                .map_err(|_| ProfileError(format!("histogram key {k:?}")))?;
            let n = count(e).ok_or_else(|| ProfileError(format!("histogram entry {k:?}")))?;
            Ok((key, n))
        })
        .collect()
}

/// Width from the median entries-per-directory bucket: bucket `b` -> `2^(b-1)`.
fn median_fanout(h: &BTreeMap<u32, u64>) -> u32 {
    let total: u64 = h.values().sum();
    let mut seen = 0;
    for (&b, &n) in h {
        seen += n;
        if seen * 2 >= total && b > 0 {
            return 1u32 << (b - 1).min(16);
        }
    }
    16
}

fn nas_share(v: &serde_json::Value) -> Result<f64, ProfileError> {
    let Some(roots) = v["roots"].as_array() else {
        return err("stats roots missing");
    };
    let bytes = |kind: Option<&str>| -> u64 {
        roots
            .iter()
            .filter(|r| kind.is_none_or(|k| r["kind"] == k))
            .filter_map(|r| r["bytes"].as_u64())
            .sum()
    };
    let total = bytes(None);
    if total == 0 {
        return Ok(0.0);
    }
    // Byte totals far below 2^53 in practice; a ratio needs no more precision.
    #[allow(clippy::cast_precision_loss)]
    Ok(bytes(Some("network")) as f64 / total as f64)
}

fn image_share(exts: &serde_json::Value) -> f64 {
    let Some(obj) = exts.as_object() else {
        return Profile::default().image_share;
    };
    let total: u64 = obj.values().filter_map(serde_json::Value::as_u64).sum();
    let images: u64 = obj
        .iter()
        .filter(|(k, _)| IMAGE_EXTS.contains(&k.as_str()))
        .filter_map(|(_, n)| n.as_u64())
        .sum();
    if total == 0 {
        return Profile::default().image_share;
    }
    #[allow(clippy::cast_precision_loss)]
    let share = images as f64 / total as f64;
    share
}

/// Lowercase hex of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATS: &str = r#"{
        "schema": "lel-drive-stats/2",
        "roots": [{"label":"root1","kind":"local","bytes":300},{"label":"root2","kind":"network","bytes":100}],
        "size_log2_hist": {"0": {"files": 5, "bytes": 0}, "10": {"files": 40, "bytes": 1}, "20": {"files": 60, "bytes": 2}},
        "depth_hist": {"1": 3, "2": 7},
        "entries_per_dir_log2_hist": {"3": 2, "5": 6, "7": 1},
        "extensions": {"jpg": 30, "png": 10, "pdf": 40, "other": 20}
    }"#;

    #[test]
    fn default_is_valid_and_has_the_design_rates() {
        let p = Profile::default();
        assert_eq!(p.validate(), Ok(()));
        assert_eq!(
            (p.identical_rate, p.decoy_rate, p.prefix_rate),
            (0.15, 0.01, 0.005)
        );
        let peak = p
            .size_log2_files
            .iter()
            .max_by_key(|&(_, w)| *w)
            .map(|(b, _)| *b);
        assert_eq!(peak, Some(19), "median near 256 KiB");
        assert!(
            p.size_log2_files.contains_key(&MAX_BUCKET),
            "tail reaches the 8 GiB bucket"
        );
    }

    #[test]
    fn digest_is_stable_and_tracks_changes() {
        let p = Profile::default();
        assert_eq!(p.digest(), p.clone().digest());
        let q = Profile {
            nas_share: 0.5,
            ..p.clone()
        };
        assert_ne!(p.digest(), q.digest());
    }

    #[test]
    fn stats_report_maps_to_a_profile() {
        let p = Profile::from_stats(STATS).expect("valid stats");
        assert!(p.source.starts_with("stats:"));
        assert_eq!(
            p.size_log2_files,
            BTreeMap::from([(10, 40), (20, 60)]),
            "zero-byte bucket dropped"
        );
        assert_eq!(p.depth, BTreeMap::from([(1, 3), (2, 7)]));
        assert_eq!(p.dir_width, 16, "median bucket 5 -> 2^4");
        assert!((p.nas_share - 0.25).abs() < 1e-12);
        assert!((p.image_share - 0.40).abs() < 1e-12);
        assert_eq!(p.identical_rate, Profile::default().identical_rate);
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(Profile::from_stats("not json").is_err());
        assert!(Profile::from_stats(r#"{"schema":"other"}"#).is_err());
        let no_sizes = STATS.replace(
            r#""10": {"files": 40, "bytes": 1}, "20": {"files": 60, "bytes": 2}"#,
            "",
        );
        assert!(
            Profile::from_stats(&no_sizes).is_err(),
            "only the zero bucket left"
        );
        let p = Profile {
            identical_rate: 0.9,
            decoy_rate: 0.2,
            ..Profile::default()
        };
        assert!(p.validate().is_err());
        let p = Profile {
            nas_share: 1.5,
            ..Profile::default()
        };
        assert!(p.validate().is_err());
        let p = Profile {
            opaque_exts: BTreeMap::from([("../x".to_owned(), 1)]),
            ..Profile::default()
        };
        assert!(p.validate().is_err(), "extensions cannot carry path parts");
    }
}
