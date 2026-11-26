//! Parser side: classifies files and turns measured ones into the core's
//! [`FileMeasure`](crate::core::FileMeasure). The gate measures Rust only
//! (D45); [`measure_in`] also measures Python for the #705 spike.

pub mod python;
pub mod rust;

use crate::core::{Lang, Report};

/// How a file takes part in the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// Production code in a language being measured.
    Measured(Lang),
    /// Code in a language v0 does not measure. Reported, never failed.
    Unmeasured,
    /// Not code, test code, or build output.
    Ignored,
}

/// Extensions of code v0 cannot measure. Anything else that is not Rust is
/// ignored: docs, config, data and unknown files.
const UNMEASURED: &[&str] = &[
    "c", "cc", "cpp", "cs", "cxx", "go", "h", "hpp", "java", "js", "jsx", "kt", "lua", "mjs",
    "php", "ps1", "rb", "sh", "sql", "swift", "ts", "tsx", "zig",
];

/// Classifies a `/`-separated repository path for the gate, which measures
/// Rust only: `classify_in(path, &[Lang::Rust])`.
pub fn classify(path: &str) -> FileClass {
    classify_in(path, &[Lang::Rust])
}

/// Classifies a path, measuring the languages in `langs`. Code in any other
/// language is unmeasured. Paths under a `tests` or `target` directory are
/// ignored, as are Rust `tests.rs` files (the usual out-of-line
/// `#[cfg(test)] mod tests;`) and Python `test_*.py`, `*_test.py` and
/// `conftest.py` files.
pub fn classify_in(path: &str, langs: &[Lang]) -> FileClass {
    let mut parts = path.split('/');
    let name = parts.next_back().unwrap_or_default();
    if parts.any(|d| d == "tests" || d == "target") || is_test_file(name) {
        return FileClass::Ignored;
    }
    let lang = match name.rsplit_once('.') {
        Some((_, "rs")) => Lang::Rust,
        Some((_, "py")) => Lang::Python,
        Some((_, ext)) if UNMEASURED.contains(&ext) => return FileClass::Unmeasured,
        _ => return FileClass::Ignored,
    };
    match langs.contains(&lang) {
        true => FileClass::Measured(lang),
        false => FileClass::Unmeasured,
    }
}

fn is_test_file(name: &str) -> bool {
    name == "tests.rs"
        || name == "conftest.py"
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name.ends_with("_test.py")
}

/// Why a file set could not be measured.
#[derive(Debug)]
pub enum Error {
    /// The linked grammar does not match the tree-sitter runtime's ABI.
    Language(tree_sitter::LanguageError),
    /// tree-sitter returned no tree for this path.
    NoTree { path: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Language(e) => write!(f, "tree-sitter grammar: {e}"),
            Error::NoTree { path } => write!(f, "tree-sitter returned no tree for {path}"),
        }
    }
}

impl std::error::Error for Error {}

/// Measures `(path, source)` pairs into one [`Report`], Rust only, as the
/// gate does. Paths are `/`-separated and relative to the repository root.
pub fn measure<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Report, Error> {
    measure_in(&[Lang::Rust], files)
}

/// Measures `(path, source)` pairs in the languages `langs` into one
/// [`Report`]; code in other languages is listed as unmeasured.
pub fn measure_in<'a>(
    langs: &[Lang],
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<Report, Error> {
    let mut rust = None;
    let mut python = None;
    let mut measured = Vec::new();
    let mut unmeasured = Vec::new();
    for (path, source) in files {
        match classify_in(path, langs) {
            FileClass::Measured(Lang::Rust) => {
                let parser = match &mut rust {
                    Some(p) => p,
                    None => rust.insert(rust::Parser::new()?),
                };
                measured.push(parser.measure(path, source)?);
            }
            FileClass::Measured(Lang::Python) => {
                let parser = match &mut python {
                    Some(p) => p,
                    None => python.insert(python::Parser::new()?),
                };
                measured.push(parser.measure(path, source)?);
            }
            FileClass::Unmeasured => unmeasured.push(path.to_owned()),
            FileClass::Ignored => {}
        }
    }
    Ok(Report::build(measured, unmeasured))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        let rust = FileClass::Measured(Lang::Rust);
        assert_eq!(classify("src/lib.rs"), rust);
        assert_eq!(classify("lib.rs"), rust);
        assert_eq!(classify("src/attests.rs"), rust);
        assert_eq!(classify("crate/tests/it.rs"), FileClass::Ignored);
        assert_eq!(classify("src/tests.rs"), FileClass::Ignored);
        assert_eq!(classify("target/debug/build/x.rs"), FileClass::Ignored);
        assert_eq!(classify("tools/gen.py"), FileClass::Unmeasured);
        assert_eq!(classify("Cargo.toml"), FileClass::Ignored);
        assert_eq!(classify("README.md"), FileClass::Ignored);
        assert_eq!(classify("Makefile"), FileClass::Ignored);
    }

    #[test]
    fn classifies_python_only_when_asked() {
        let both = [Lang::Rust, Lang::Python];
        let py = FileClass::Measured(Lang::Python);
        assert_eq!(classify("app/main.py"), FileClass::Unmeasured);
        assert_eq!(classify_in("app/main.py", &both), py);
        assert_eq!(classify_in("app/contest.py", &both), py);
        assert_eq!(classify_in("app/test_main.py", &both), FileClass::Ignored);
        assert_eq!(classify_in("app/main_test.py", &both), FileClass::Ignored);
        assert_eq!(classify_in("conftest.py", &both), FileClass::Ignored);
        assert_eq!(classify_in("tests/helpers.py", &both), FileClass::Ignored);
        assert_eq!(
            classify_in("src/lib.rs", &[Lang::Python]),
            FileClass::Unmeasured
        );
        assert_eq!(classify_in("web/app.ts", &both), FileClass::Unmeasured);
    }
}
