//! The tool catalog: tools as data (pack D7), read through a [`TomlReader`].

use crate::model::{Error as IdError, Tool};
use crate::toml::{ParseError, Table, TomlReader};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

/// The built-in catalog, `data/tools.toml`.
pub const BUILTIN: &str = include_str!("../data/tools.toml");

/// Why a catalog was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    Parse(ParseError),
    /// A table other than `[[tool]]`.
    UnknownTable {
        line: usize,
        name: String,
    },
    /// A key other than `id` and `skills_dir`: a typo must not pass silently.
    UnknownKey {
        line: usize,
        key: String,
    },
    MissingKey {
        line: usize,
        key: &'static str,
    },
    BadId {
        line: usize,
        source: IdError,
    },
    DuplicateId {
        line: usize,
        id: String,
    },
    /// `skills_dir` must be absolute or start with `~/`.
    BadPath {
        line: usize,
        path: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => e.fmt(f),
            Self::UnknownTable { line, name } => write!(f, "line {line}: unknown table [[{name}]]"),
            Self::UnknownKey { line, key } => write!(f, "line {line}: unknown key {key:?}"),
            Self::MissingKey { line, key } => write!(f, "line {line}: missing key {key:?}"),
            Self::BadId { line, source } => write!(f, "line {line}: {source}"),
            Self::DuplicateId { line, id } => write!(f, "line {line}: duplicate tool id {id:?}"),
            Self::BadPath { line, path } => {
                write!(
                    f,
                    "line {line}: skills_dir {path:?} must be absolute or start with ~/"
                )
            }
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<ParseError> for CatalogError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

fn expand(path: &str, home: &Path, line: usize) -> Result<PathBuf, CatalogError> {
    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return Ok(p);
    }
    Err(CatalogError::BadPath {
        line,
        path: path.to_owned(),
    })
}

fn tool(t: &Table, home: &Path) -> Result<Tool, CatalogError> {
    let line = t.line;
    if t.name != "tool" {
        return Err(CatalogError::UnknownTable {
            line,
            name: t.name.clone(),
        });
    }
    if let Some(key) = t
        .fields
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "skills_dir"))
    {
        return Err(CatalogError::UnknownKey {
            line,
            key: key.clone(),
        });
    }
    let get = |key: &'static str| {
        t.fields
            .get(key)
            .ok_or(CatalogError::MissingKey { line, key })
    };
    Ok(Tool {
        id: get("id")?
            .parse()
            .map_err(|source| CatalogError::BadId { line, source })?,
        skills_dir: expand(get("skills_dir")?, home, line)?,
    })
}

/// Reads `text` into tools, expanding `~/` against `home`. Fails on the first
/// bad entry; ids must be unique.
pub fn parse_catalog(
    reader: &impl TomlReader,
    text: &str,
    home: &Path,
) -> Result<Vec<Tool>, CatalogError> {
    let mut seen = BTreeSet::new();
    let mut tools = Vec::new();
    for table in reader.read(text)? {
        let t = tool(&table, home)?;
        if !seen.insert(t.id.clone()) {
            return Err(CatalogError::DuplicateId {
                line: table.line,
                id: t.id.to_string(),
            });
        }
        tools.push(t);
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toml::Restricted;

    fn parse(t: &str) -> Result<Vec<Tool>, CatalogError> {
        parse_catalog(&Restricted, t, Path::new("/home/u"))
    }

    #[test]
    fn builtin_catalog_parses_and_expands_home() {
        let tools = parse(BUILTIN).unwrap();
        let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["claude-code", "codex"]);
        assert_eq!(tools[0].skills_dir, Path::new("/home/u/.claude/skills"));
        assert_eq!(tools[1].skills_dir, Path::new("/home/u/.agents/skills"));
    }

    #[test]
    fn absolute_paths_pass_through() {
        // `/opt/skills` is not absolute on Windows (no drive), so pick per OS.
        let abs = if cfg!(windows) {
            "C:\\opt\\skills"
        } else {
            "/opt/skills"
        };
        let t = parse(&format!("[[tool]]\nid = \"x\"\nskills_dir = '{abs}'\n")).unwrap();
        assert_eq!(t[0].skills_dir, Path::new(abs));
    }

    #[test]
    fn rejects_bad_catalogs() {
        let cases = [
            "[[tool]]\nskills_dir = \"/a\"\n",                           // missing id
            "[[tool]]\nid = \"x\"\n",                                    // missing path
            "[[tool]]\nid = \"X\"\nskills_dir = \"/a\"\n",               // bad id
            "[[tool]]\nid = \"x\"\nskills_dir = \"rel/a\"\n",            // relative path
            "[[tool]]\nid = \"x\"\nskills_dir = \"/a\"\nextra = \"1\"\n", // typo'd key
            "[[mod]]\nid = \"x\"\nskills_dir = \"/a\"\n",                // unknown table
            "[[tool]]\nid = \"x\"\nskills_dir = \"/a\"\n[[tool]]\nid = \"x\"\nskills_dir = \"/b\"\n",
            "[[tool]]\nid = 1\n",                                        // parse error
        ];
        for text in cases {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn errors_carry_the_line_of_the_entry() {
        let e = parse("\n\n[[tool]]\nid = \"x\"\n").unwrap_err();
        assert_eq!(
            e,
            CatalogError::MissingKey {
                line: 3,
                key: "skills_dir"
            }
        );
    }
}
