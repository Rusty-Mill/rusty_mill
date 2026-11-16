//! Theme files: `themes/<name>.toml` beside the config file (C61 T1, gui-design
//! D73, handoff D297, D299).
//!
//! A theme file is the config's `[colors]` section on its own:
//!
//! ```toml
//! [colors]
//! foreground = "#d8d8d8"
//! background = "#1d1f21"
//! cursor     = "#aeafad"   # also the chrome accent (D34)
//! color0     = "#282a2e"   # color0 .. color15
//! ```
//!
//! Keys left out inherit the default theme. The name is the file stem, used
//! exactly as written in `theme = "name"`, `theme_dark`, `theme_light` and
//! `[profile.x] theme`. A user file shadows a built-in preset of that name.
//! Any bad line rejects the whole file: nothing half-applies.

use std::path::{Path, PathBuf};

use super::{PRESETS, parse_value, preset, set_color};
use crate::core::Theme;

/// The theme directory for the config file at `config`: `themes/` beside it.
pub fn dir_of(config: &Path) -> Option<PathBuf> {
    Some(config.parent()?.join("themes"))
}

/// Resolve `name` to a theme: the user file `<dir>/<name>.toml` if there is
/// one, else the built-in preset. Errors are one line, ready to show.
pub fn resolve(name: &str, dir: Option<&Path>) -> Result<Theme, String> {
    if let Some(path) = dir
        .filter(|_| is_stem(name))
        .map(|d| d.join(format!("{name}.toml")))
        && path.is_file()
    {
        let text = read_capped(&path).map_err(|e| format!("theme {name}: {e}"))?;
        return parse_file(&text).map_err(|e| format!("theme file {}: {e}", path.display()));
    }
    preset(name).ok_or_else(|| format!("unknown theme `{name}` (try {})", PRESETS.join(", ")))
}

/// Largest theme file read (a real one is under 1 KiB).
const MAX_BYTES: u64 = 64 * 1024;

/// Read `path` as text, refusing more than [`MAX_BYTES`] so a symlink to a huge
/// file cannot stall a reload.
fn read_capped(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("file too large (over 64 KiB)"));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("not valid UTF-8"))
}

/// A usable file stem: no path separators, no leading dot.
fn is_stem(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':'])
}

/// Parse a theme file. Strict: the first bad line is the error.
pub fn parse_file(text: &str) -> Result<Theme, String> {
    let mut theme = Theme::default();
    let mut in_colors = false;
    let mut keys = 0;
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |e: String| format!("line {}: {e}", idx + 1);
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest
                .split_once(']')
                .filter(|(_, tail)| tail.trim().is_empty() || tail.trim().starts_with('#'))
                .map(|(n, _)| n.trim().to_ascii_lowercase());
            if name.as_deref() != Some("colors") {
                return Err(at("only a [colors] section is allowed".into()));
            }
            in_colors = true;
            continue;
        }
        let (key, rest) = line
            .split_once('=')
            .ok_or_else(|| at("expected `key = value`".into()))?;
        if !in_colors {
            return Err(at("keys belong under [colors]".into()));
        }
        let key = key.trim().to_ascii_lowercase().replace('-', "_");
        let value = parse_value(rest.trim()).map_err(|e| at(format!("{e} ({key})")))?;
        set_color(&mut theme, &key, value).map_err(at)?;
        keys += 1;
    }
    if keys == 0 {
        return Err("no colours set under [colors]".into());
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# mine\n[colors]\nforeground = \"#112233\"\nbackground = \"445566\"\ncolor1 = \"#ff0000\"\n";

    #[test]
    fn a_file_sets_named_keys_and_inherits_the_rest() {
        let t = parse_file(GOOD).unwrap();
        assert_eq!((t.fg, t.bg, t.palette16[1]), (0x112233, 0x445566, 0xff0000));
        assert_eq!(t.cursor, Theme::default().cursor);
        assert_eq!(t.palette16[2], Theme::default().palette16[2]);
    }

    #[test]
    fn any_bad_line_rejects_the_whole_file() {
        for bad in [
            "[colors]\nforeground = \"#12\"\n",
            "[colors]\ncolor16 = \"#000000\"\n",
            "[colors]\nforeground = 5\n",
            "[colors]\nshell = \"x\"\n",
            "[window]\nopacity = 1\n",
            "foreground = \"#000000\"\n",
            "[colors]\nforeground\n",
            "[colors]junk\nforeground = \"#000000\"\n",
            "# nothing\n",
        ] {
            assert!(parse_file(bad).is_err(), "accepted: {bad:?}");
        }
        let e =
            parse_file("[colors]\nforeground = \"#000000\"\ncolor99 = \"#000000\"\n").unwrap_err();
        assert!(e.starts_with("line 3:"), "{e}");
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_user_file_shadows_a_preset_and_new_names_resolve() {
        let d = scratch("shadow");
        std::fs::write(d.join("nord.toml"), GOOD).unwrap();
        std::fs::write(d.join("mine.toml"), GOOD).unwrap();
        assert_eq!(resolve("nord", Some(&d)).unwrap().fg, 0x112233);
        assert_eq!(resolve("mine", Some(&d)).unwrap().bg, 0x445566);
        assert_eq!(
            resolve("dracula", Some(&d)).unwrap(),
            preset("dracula").unwrap()
        );
        assert!(resolve("mine", None).unwrap_err().contains("unknown theme"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_bad_user_file_is_an_error_not_a_fallback_to_the_preset() {
        let d = scratch("bad");
        std::fs::write(d.join("nord.toml"), "[colors]\nforeground = \"oops\"\n").unwrap();
        let e = resolve("nord", Some(&d)).unwrap_err();
        assert!(e.contains("nord.toml") && e.contains("line 2"), "{e}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_oversized_file_is_refused() {
        let d = scratch("big");
        let big = format!("[colors]\n{}", "# pad\n".repeat(20_000));
        std::fs::write(d.join("big.toml"), big).unwrap();
        let e = resolve("big", Some(&d)).unwrap_err();
        assert!(e.contains("too large"), "{e}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn names_cannot_escape_the_theme_directory() {
        let d = scratch("escape");
        let evil = d
            .parent()
            .unwrap()
            .join(format!("evil-{}.toml", std::process::id()));
        std::fs::write(&evil, GOOD).unwrap();
        let rel = format!("../{}", evil.file_stem().unwrap().to_string_lossy());
        for name in [rel.as_str(), "..\\evil", ".hidden", "a/b", ""] {
            assert!(resolve(name, Some(&d)).is_err(), "{name:?}");
        }
        let _ = std::fs::remove_file(evil);
        let _ = std::fs::remove_dir_all(d);
    }
}
