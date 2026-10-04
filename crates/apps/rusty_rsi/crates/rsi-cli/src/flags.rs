//! A small `--flag value` parser for the outer-loop commands.

use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

/// Parsed flags: each valued flag at most once, plus bare switches.
#[derive(Debug, Default)]
pub struct Flags {
    values: Vec<(String, OsString)>,
    switches: Vec<String>,
}

impl Flags {
    /// Parses `args`, accepting only `valued` flags (followed by a value)
    /// and bare `switches`.
    ///
    /// # Errors
    /// An unknown or repeated flag, or a valued flag without its value.
    pub fn parse(args: &[OsString], valued: &[&str], switches: &[&str]) -> Result<Self, String> {
        let mut flags = Self::default();
        let mut iter = args.iter();
        while let Some(flag) = iter.next() {
            let name = flag.to_string_lossy().into_owned();
            let known = |list: &[&str]| list.contains(&name.as_str());
            let repeated =
                flags.values.iter().any(|(n, _)| *n == name) || flags.switches.contains(&name);
            if repeated {
                return Err(format!("{name} given twice"));
            }
            if known(switches) {
                flags.switches.push(name);
            } else if known(valued) {
                let value = iter.next().ok_or_else(|| format!("{name} needs a value"))?;
                flags.values.push((name, value.clone()));
            } else {
                return Err(format!("unknown flag {name}"));
            }
        }
        Ok(flags)
    }

    fn get(&self, name: &str) -> Option<&OsString> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A path flag, if given.
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.get(name).map(PathBuf::from)
    }

    /// A path flag that must be given.
    ///
    /// # Errors
    /// When it is missing.
    pub fn required_path(&self, name: &str) -> Result<PathBuf, String> {
        self.path(name).ok_or_else(|| format!("{name} is required"))
    }

    /// A text flag, if given.
    pub fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(|v| v.to_string_lossy().into_owned())
    }

    /// A number flag, or `default` when absent.
    ///
    /// # Errors
    /// When the value does not parse.
    pub fn number<T: FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.get(name) {
            None => Ok(default),
            Some(value) => value
                .to_str()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{name} needs a number")),
        }
    }

    /// A number flag that must be given.
    ///
    /// # Errors
    /// When it is missing or does not parse.
    pub fn required_number<T: FromStr>(&self, name: &str) -> Result<T, String> {
        let value = self
            .get(name)
            .ok_or_else(|| format!("{name} is required"))?;
        value
            .to_str()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{name} needs a number"))
    }

    /// Whether a switch was given.
    pub fn switch(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_values_and_switches() {
        let flags = Flags::parse(
            &os(&["--steps", "10", "--replay", "--dir", "a b"]),
            &["--steps", "--dir"],
            &["--replay"],
        )
        .expect("valid");
        assert_eq!(flags.required_number::<u32>("--steps"), Ok(10));
        assert_eq!(flags.number::<u32>("--seeds", 1), Ok(1));
        assert_eq!(flags.path("--dir"), Some(PathBuf::from("a b")));
        assert!(flags.switch("--replay"));
        assert!(flags.required_path("--repo").is_err());
    }

    #[test]
    fn rejects_bad_flags() {
        let valued = ["--steps"];
        for bad in [
            &["--bogus"][..],
            &["--steps"],
            &["--steps", "1", "--steps", "2"],
        ] {
            assert!(Flags::parse(&os(bad), &valued, &[]).is_err(), "{bad:?}");
        }
        let flags = Flags::parse(&os(&["--steps", "x"]), &valued, &[]).expect("parses");
        assert!(flags.required_number::<u32>("--steps").is_err());
    }
}
