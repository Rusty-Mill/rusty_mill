//! Parser side: classifies files and turns measured ones into the core's
//! [`FileMeasure`](crate::core::FileMeasure). v0 measures Rust only (D45).

pub mod rust;

use crate::core::Report;

/// How a file takes part in the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// Rust production code.
    Rust,
    /// Code in a language v0 does not measure. Reported, never failed.
    Unmeasured,
    /// Not code, test code, or build output.
    Ignored,
}

/// Extensions of code v0 cannot measure. Anything else that is not Rust is
/// ignored: docs, config, data and unknown files.
const UNMEASURED: &[&str] = &[
    "c", "cc", "cpp", "cs", "cxx", "go", "h", "hpp", "java", "js", "jsx", "kt", "lua", "mjs",
    "php", "ps1", "py", "rb", "sh", "sql", "swift", "ts", "tsx", "zig",
];

/// Classifies a `/`-separated repository path. Paths under a `tests` or
/// `target` directory, and `tests.rs` files (the usual out-of-line
/// `#[cfg(test)] mod tests;`), are ignored.
pub fn classify(path: &str) -> FileClass {
    let mut parts = path.split('/');
    let name = parts.next_back().unwrap_or_default();
    if parts.any(|d| d == "tests" || d == "target") || name == "tests.rs" {
        return FileClass::Ignored;
    }
    match name.rsplit_once('.') {
        Some((_, "rs")) => FileClass::Rust,
        Some((_, ext)) if UNMEASURED.contains(&ext) => FileClass::Unmeasured,
        _ => FileClass::Ignored,
    }
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
            Error::Language(e) => write!(f, "tree-sitter-rust grammar: {e}"),
            Error::NoTree { path } => write!(f, "tree-sitter returned no tree for {path}"),
        }
    }
}

impl std::error::Error for Error {}

/// Measures `(path, source)` pairs into one [`Report`]. Paths are
/// `/`-separated and relative to the repository root.
pub fn measure<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Report, Error> {
    let mut parser = rust::Parser::new()?;
    let mut measured = Vec::new();
    let mut unmeasured = Vec::new();
    for (path, source) in files {
        match classify(path) {
            FileClass::Rust => measured.push(parser.measure(path, source)?),
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
        assert_eq!(classify("src/lib.rs"), FileClass::Rust);
        assert_eq!(classify("lib.rs"), FileClass::Rust);
        assert_eq!(classify("src/attests.rs"), FileClass::Rust);
        assert_eq!(classify("crate/tests/it.rs"), FileClass::Ignored);
        assert_eq!(classify("src/tests.rs"), FileClass::Ignored);
        assert_eq!(classify("target/debug/build/x.rs"), FileClass::Ignored);
        assert_eq!(classify("tools/gen.py"), FileClass::Unmeasured);
        assert_eq!(classify("Cargo.toml"), FileClass::Ignored);
        assert_eq!(classify("README.md"), FileClass::Ignored);
        assert_eq!(classify("Makefile"), FileClass::Ignored);
    }
}
