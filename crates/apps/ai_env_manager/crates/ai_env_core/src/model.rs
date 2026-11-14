//! Domain types. Illegal states are unrepresentable: ids are validated once at
//! the boundary and everything downstream can join them onto a path safely.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Why a value was rejected at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An id breaks the rules in [`validate_id`], so it could not be used
    /// safely as a folder name on every supported OS.
    InvalidId(String),
    /// An unknown entry kind name.
    UnknownKind(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidId(s) => write!(
                f,
                "invalid id {s:?}: use lower-case [a-z0-9._-], not starting or ending with '.', \
                 and not a Windows device name (con, nul, com1, ...)"
            ),
            Error::UnknownKind(s) => write!(f, "unknown entry kind {s:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Windows device names, reserved with or without an extension.
fn is_windows_device(stem: &str) -> bool {
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
        })
}

/// An id is a folder name that must be the same path on Windows, macOS and
/// Linux: lower-case only (so `Foo` and `foo` can never be two entries that
/// share one path), no separators, `:` or traversal, no leading or trailing
/// dot (Windows drops a trailing one, so `foo.` would land on `foo`), and not a
/// Windows device name (`NUL`, `COM1`, `nul.txt`, ...).
fn validate_id(s: &str) -> Result<(), Error> {
    let stem = s.split('.').next().unwrap_or_default();
    let ok = !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !is_windows_device(stem);
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidId(s.to_owned()))
    }
}

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// The validated id as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(s: &str) -> Result<Self, Error> {
                validate_id(s)?;
                Ok(Self(s.to_owned()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Id of a catalog entry; also its folder name in a tool's skills folder.
    EntryId
);
id_type!(
    /// Id of a target AI tool (for example `claude-code`).
    ToolId
);

/// What an entry is. The first five are `library/README.md`'s kinds; `Agent` is
/// new, required by pack D2 (see D15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Skill,
    Prompt,
    /// A Claude Code plugin; "plugin" is its display name.
    Mod,
    Agent,
    ClaudeMd,
    AgentsMd,
}

impl FromStr for Kind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Ok(match s {
            "skill" => Kind::Skill,
            "prompt" => Kind::Prompt,
            "mod" => Kind::Mod,
            "agent" => Kind::Agent,
            "claude-md" => Kind::ClaudeMd,
            "agents-md" => Kind::AgentsMd,
            other => return Err(Error::UnknownKind(other.to_owned())),
        })
    }
}

/// A catalog entry, linked in place from where it lives (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: EntryId,
    pub kind: Kind,
    /// Where the entry lives: the link target.
    pub source: PathBuf,
}

/// A target AI tool and the folder its entries of this kind are linked into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub id: ToolId,
    pub skills_dir: PathBuf,
}

impl Tool {
    /// Where `entry` appears inside this tool's folder.
    pub fn target(&self, entry: &Entry) -> PathBuf {
        self.skills_dir.join(entry.id.as_str())
    }
}

/// How a link is made (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMethod {
    Symlink,
    /// Windows directory junction: needs no admin rights.
    Junction,
    /// A copy carrying our marker; works everywhere.
    Copy,
}

/// Host family, as far as link support goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Unix,
    Windows,
}

impl LinkMethod {
    /// The method to try first on `os`: the head of its fallback chain.
    pub fn preferred(os: Os) -> LinkMethod {
        Self::fallback_chain(os)
            .first()
            .copied()
            .unwrap_or(LinkMethod::Copy)
    }

    /// Methods to try, best first. Symlinks need Developer Mode on Windows;
    /// junctions do not, so they come second there.
    pub fn fallback_chain(os: Os) -> &'static [LinkMethod] {
        match os {
            Os::Unix => &[LinkMethod::Symlink, LinkMethod::Copy],
            Os::Windows => &[LinkMethod::Symlink, LinkMethod::Junction, LinkMethod::Copy],
        }
    }
}
