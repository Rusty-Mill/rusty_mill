//! [`Proposer`] adapters (ADR-0005 §7, §8).
//!
//! - [`ModelProposer`]: asks a chat model (any [`ChatModel`], such as an
//!   OpenAI-compatible endpoint) for whole-file rewrites and writes them
//!   into the workspace.
//! - [`ScriptedProposer`]: canned edits, for tests and CI.
//!
//! Neither decides what may change: a proposer may write anywhere inside
//! the workspace (never outside it), and the outer loop's allowlist check
//! decides afterwards. That way a model that edits a manifest or a task's
//! data is caught and recorded as a path violation, not silently confined.

use std::cell::{Cell, RefCell};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rsi_core::{ChatModel, CostUsage, Message, ModelId, Precedent, Proposal, Proposer, Role};

use crate::error::RuntimeError;
use crate::git::{HARNESS_DIR, HARNESS_SRC};

/// Writes `content` to `path` (relative to `workspace`) without ever
/// leaving the workspace.
///
/// The path must be plain and relative, with no `.git` component in any
/// letter case. Each directory on the way is checked without following
/// links: an existing symlink (or a non-directory) refuses the write, so a
/// symlinked ancestor checked out from a candidate cannot redirect it, and
/// a missing directory is created one level at a time. A symlink at the
/// leaf is replaced, never written through.
///
/// # Errors
/// [`RuntimeError::Model`] for a refused path; [`RuntimeError::Io`] on
/// write errors.
pub fn write_inside(
    workspace: &Path,
    path: &str,
    content: impl AsRef<[u8]>,
) -> Result<(), RuntimeError> {
    let target = clear_leaf(workspace, path)?;
    std::fs::write(&target, content).map_err(|e| RuntimeError::io(format!("writing {path}"), e))
}

/// Makes `path` a symlink to `link`, under the same rules as
/// [`write_inside`]. The link is created, never followed; the allowlist
/// rejects it afterwards wherever it is.
///
/// # Errors
/// As [`write_inside`].
#[cfg(unix)]
pub fn link_inside(workspace: &Path, path: &str, link: &Path) -> Result<(), RuntimeError> {
    let target = clear_leaf(workspace, path)?;
    std::os::unix::fs::symlink(link, &target)
        .map_err(|e| RuntimeError::io(format!("linking {path}"), e))
}

/// Removes the file or symlink at `path`, under the same rules as
/// [`write_inside`]; a missing one is fine.
///
/// # Errors
/// As [`write_inside`].
pub fn remove_inside(workspace: &Path, path: &str) -> Result<(), RuntimeError> {
    clear_leaf(workspace, path).map(|_| ())
}

/// Checks `path` and its ancestors as [`write_inside`] describes, creating
/// missing directories, and removes whatever file or symlink sits at the
/// leaf. Returns the leaf's full path.
fn clear_leaf(workspace: &Path, path: &str) -> Result<PathBuf, RuntimeError> {
    let refuse = |why: &str| {
        Err(RuntimeError::Model(format!(
            "refusing to write `{path}`: {why}"
        )))
    };
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) if !name.eq_ignore_ascii_case(".git") => parts.push(name),
            _ => return refuse("not a plain relative path"),
        }
    }
    let Some((leaf, ancestors)) = parts.split_last() else {
        return refuse("empty path");
    };
    let mut dir = workspace.to_path_buf();
    for name in ancestors {
        dir.push(name);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return refuse(&format!("{} is a symlink", dir.display()));
            }
            Ok(meta) if !meta.is_dir() => {
                return refuse(&format!("{} is not a directory", dir.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&dir)
                    .map_err(|e| RuntimeError::io(format!("creating {}", dir.display()), e))?;
            }
            Err(e) => return Err(RuntimeError::io(format!("inspecting {}", dir.display()), e)),
        }
    }
    let target = dir.join(leaf);
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_dir() => refuse(&format!("{} is a directory", target.display())),
        Ok(_) => std::fs::remove_file(&target)
            .map(|()| target)
            .map_err(|e| RuntimeError::io(format!("replacing {path}"), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(e) => Err(RuntimeError::io(format!("inspecting {path}"), e)),
    }
}

/// The harness source files in `workspace`, as `(repository path,
/// contents)`, sorted by path.
///
/// # Errors
/// [`RuntimeError::Io`] when the directory cannot be read.
pub fn harness_sources(workspace: &Path) -> Result<Vec<(String, String)>, RuntimeError> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) -> Result<(), RuntimeError> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| RuntimeError::io(format!("listing {}", dir.display()), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| RuntimeError::io("listing the harness", e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry
                .file_type()
                .map_err(|e| RuntimeError::io("reading a file type", e))?;
            let path = format!("{prefix}{name}");
            if kind.is_dir() {
                walk(&entry.path(), &format!("{path}/"), out)?;
            } else if kind.is_file() {
                let text = std::fs::read_to_string(entry.path())
                    .map_err(|e| RuntimeError::io(format!("reading {path}"), e))?;
                out.push((path, text));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(&workspace.join(HARNESS_SRC), HARNESS_SRC, &mut files)?;
    files.sort();
    Ok(files)
}

const OPEN: &str = "<<<FILE ";
const CLOSE: &str = ">>>END";

/// The files in a model reply: blocks of `<<<FILE path` ... `>>>END`.
fn parse_files(reply: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut rest = reply;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(line_end) = after.find('\n') else {
            break;
        };
        let path = after[..line_end].trim().to_owned();
        let body = &after[line_end + 1..];
        let Some(end) = body.find(CLOSE) else {
            break;
        };
        files.push((path, body[..end].to_owned()));
        rest = &body[end + CLOSE.len()..];
    }
    files
}

const SYSTEM: &str = "You improve the inner agent of a self-improvement \
harness. The agent is a Rust program, standard library only, compiled as \
`rustc --edition 2021 -O --crate-type bin src/lib.rs`; `pub fn main` in \
src/lib.rs is its entry point. It runs in a sandbox with `task.md` in its \
working directory and a broker socket on standard input (see the protocol \
in src/broker.rs), and is started as `rsi-harness --seed N`. Through the \
broker it may call a language model, run candidate solutions on public \
data to get a score, and submit its best solution; the latest submission \
counts. It is graded on held-out data it never sees, under a fixed token \
and time budget. Make it find better solutions within that budget.\n\n\
Only files under crates/apps/rusty_rsi/harness/src/ may change; any other \
change rejects the candidate. Reply with a one-line summary, then every \
file you change, in full, as:\n<<<FILE crates/apps/rusty_rsi/harness/src/lib.rs\n\
...entire file...\n>>>END";

/// Rewrites the harness with a chat model.
#[derive(Debug)]
pub struct ModelProposer<'a, M> {
    model: &'a M,
    max_tokens: u64,
    timeout: Duration,
}

impl<'a, M> ModelProposer<'a, M> {
    /// Proposes with `model`, asking for at most `max_tokens` per reply
    /// and waiting at most `timeout`.
    #[must_use]
    pub const fn new(model: &'a M, max_tokens: u64, timeout: Duration) -> Self {
        Self {
            model,
            max_tokens,
            timeout,
        }
    }
}

/// The history as the proposer's prompt shows it.
pub(crate) fn describe(history: &[Precedent]) -> String {
    let mut out = String::from("# Candidates so far\n\n");
    for p in history {
        let grade = p
            .grade
            .map_or_else(|| "-".to_owned(), |g| format!("{:.4}", g.get()));
        let parent = p
            .parent
            .map_or_else(|| "-".to_owned(), |c| c.get().to_string());
        out.push_str(&format!(
            "- candidate {} (parent {parent}): {}, grade {grade}\n",
            p.candidate.get(),
            p.verdict.name()
        ));
        if let Some(note) = &p.note {
            let short: String = note.chars().take(1500).collect();
            out.push_str(&format!("  note: {short}\n"));
        }
        for r in &p.public {
            let score = r
                .public
                .map_or_else(|| "none".to_owned(), |s| format!("{:.4}", s.get()));
            out.push_str(&format!(
                "  public {} seed {}: {score}\n",
                r.task.as_str(),
                r.seed.get()
            ));
        }
    }
    out
}

impl<M> Proposer for ModelProposer<'_, M>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
{
    type Error = RuntimeError;

    fn model(&self) -> Option<&ModelId> {
        Some(self.model.id())
    }

    fn propose(&self, history: &[Precedent], workspace: &Path) -> Result<Proposal, RuntimeError> {
        let mut source = String::from("# Current harness source\n");
        for (path, text) in harness_sources(workspace)? {
            source.push_str(&format!("\n{OPEN}{path}\n{text}\n{CLOSE}\n"));
        }
        let messages = [
            Message {
                role: Role::System,
                content: SYSTEM.to_owned(),
            },
            Message {
                role: Role::User,
                content: format!("{}\n{source}", describe(history)),
            },
        ];
        let completion = self
            .model
            .complete(&messages, self.max_tokens, self.timeout)
            .map_err(|e| RuntimeError::Model(e.to_string()))?;
        for (path, text) in parse_files(&completion.text) {
            write_inside(workspace, &path, &text)?;
        }
        let summary = completion
            .text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with(OPEN))
            .unwrap_or("proposal")
            .chars()
            .take(72)
            .collect();
        Ok(Proposal {
            summary,
            usage: CostUsage {
                prompt_tokens: completion.prompt_tokens,
                completion_tokens: completion.completion_tokens,
                ..CostUsage::default()
            },
        })
    }
}

/// A scripted change to a workspace root.
pub type Apply = dyn Fn(&Path) -> Result<(), RuntimeError>;

/// One scripted step: a summary and the edit it makes to the workspace.
pub struct ScriptedEdit {
    /// The commit message.
    pub summary: String,
    /// Writes the change into the workspace root.
    pub apply: Box<Apply>,
}

impl std::fmt::Debug for ScriptedEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedEdit")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

impl ScriptedEdit {
    /// An edit that writes `files` (repository paths and contents).
    #[must_use]
    pub fn files(summary: &str, files: Vec<(String, String)>) -> Self {
        Self {
            summary: summary.to_owned(),
            apply: Box::new(move |workspace| {
                for (path, text) in &files {
                    write_inside(workspace, path, text)?;
                }
                Ok(())
            }),
        }
    }

    /// An edit that replaces the harness's `src/lib.rs` and nothing else.
    #[must_use]
    pub fn harness(summary: &str, lib: &str) -> Self {
        Self::files(
            summary,
            vec![(format!("{HARNESS_DIR}/src/lib.rs"), lib.to_owned())],
        )
    }
}

/// Canned edits, applied in order and then cycled.
#[derive(Debug)]
pub struct ScriptedProposer {
    edits: Vec<ScriptedEdit>,
    next: Cell<usize>,
    seen: RefCell<Vec<usize>>,
}

impl ScriptedProposer {
    /// A proposer that makes `edits` in turn.
    #[must_use]
    pub fn new(edits: Vec<ScriptedEdit>) -> Self {
        Self {
            edits,
            next: Cell::new(0),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// How many precedents each call received, in order.
    #[must_use]
    pub fn history_lengths(&self) -> Vec<usize> {
        self.seen.borrow().clone()
    }
}

impl Proposer for ScriptedProposer {
    type Error = RuntimeError;

    fn model(&self) -> Option<&ModelId> {
        None
    }

    fn propose(&self, history: &[Precedent], workspace: &Path) -> Result<Proposal, RuntimeError> {
        self.seen.borrow_mut().push(history.len());
        let index = self.next.get();
        let edit = self
            .edits
            .get(index % self.edits.len().max(1))
            .ok_or_else(|| RuntimeError::Model("the script has no edits".into()))?;
        self.next.set(index + 1);
        (edit.apply)(workspace)?;
        Ok(Proposal {
            summary: edit.summary.clone(),
            usage: CostUsage::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use rsi_core::{CandidateId, Completion, Grade, Verdict};

    use std::path::PathBuf;

    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsi-proposer-{name}-{}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir).expect("clear");
            }
            std::fs::create_dir_all(dir.join(HARNESS_SRC)).expect("mkdir");
            std::fs::write(dir.join(HARNESS_SRC).join("lib.rs"), "pub fn main() {}\n")
                .expect("lib");
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn writes_only_plain_relative_paths() {
        let dir = Dir::new("write");
        write_inside(&dir.0, "Cargo.toml", "x").expect("inside the workspace is allowed");
        for bad in [
            "/etc/passwd",
            "../escape",
            "a/../../b",
            ".git/config",
            "a/.git/x",
            ".GIT/config",
            "a/.Git/x",
            "",
        ] {
            assert!(write_inside(&dir.0, bad, "x").is_err(), "{bad}");
        }
        #[cfg(unix)]
        {
            let link = dir.0.join(HARNESS_SRC).join("link.rs");
            let target = dir.0.join("outside.txt");
            std::fs::write(&target, "keep").expect("target");
            std::os::unix::fs::symlink(&target, &link).expect("symlink");
            write_inside(&dir.0, &format!("{HARNESS_SRC}link.rs"), "new").expect("replaces");
            assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");
        }
    }

    /// A symlinked directory checked out from a candidate must not carry a
    /// write outside the workspace, whether into an existing file or a new
    /// directory under it.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_ancestor_cannot_redirect_a_write() {
        let dir = Dir::new("ancestor");
        let outside = dir.0.with_extension("outside");
        if outside.exists() {
            std::fs::remove_dir_all(&outside).expect("clear");
        }
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("sentinel.txt"), "keep").expect("sentinel");
        let workspace = dir.0.join("ws");
        std::fs::create_dir_all(workspace.join("crates")).expect("mkdir");
        std::os::unix::fs::symlink(&outside, workspace.join("crates/link")).expect("symlink");
        for path in [
            "crates/link/sentinel.txt",
            "crates/link/new/deeper/file.rs",
            "crates/link/new.rs",
        ] {
            assert!(write_inside(&workspace, path, "pwned").is_err(), "{path}");
        }
        assert_eq!(
            std::fs::read_to_string(outside.join("sentinel.txt")).expect("read"),
            "keep"
        );
        let entries: Vec<_> = std::fs::read_dir(&outside).expect("list").collect();
        assert_eq!(
            entries.len(),
            1,
            "nothing was created outside the workspace"
        );
        // A file where a directory is needed is refused too.
        std::fs::write(workspace.join("plain"), "x").expect("file");
        assert!(write_inside(&workspace, "plain/child.rs", "x").is_err());
        std::fs::remove_dir_all(&outside).expect("cleanup");
    }

    #[test]
    fn parses_file_blocks() {
        let reply =
            "Faster drafts.\n<<<FILE a/b.rs\nfn a() {}\n>>>END\ntext\n<<<FILE c.rs\nx\n>>>END";
        assert_eq!(
            parse_files(reply),
            vec![
                ("a/b.rs".into(), "fn a() {}\n".into()),
                ("c.rs".into(), "x\n".into())
            ]
        );
        assert!(parse_files("<<<FILE unterminated\nx").is_empty());
    }

    struct Fixed(ModelId, String, std::cell::RefCell<Vec<Message>>);

    impl ChatModel for Fixed {
        type Error = String;

        fn id(&self) -> &ModelId {
            &self.0
        }

        fn complete(
            &self,
            messages: &[Message],
            _: u64,
            _: Duration,
        ) -> Result<Completion, String> {
            *self.2.borrow_mut() = messages.to_vec();
            Ok(Completion {
                text: self.1.clone(),
                prompt_tokens: 100,
                completion_tokens: 20,
            })
        }
    }

    #[test]
    fn the_model_proposer_shows_grades_and_writes_the_reply() {
        let dir = Dir::new("model");
        let reply =
            format!("Use more drafts.\n<<<FILE {HARNESS_SRC}lib.rs\npub fn main() {{ }}\n>>>END\n");
        let model = Fixed(
            ModelId::parse("outer").expect("id"),
            reply,
            Default::default(),
        );
        let proposer = ModelProposer::new(&model, 1000, Duration::from_secs(5));
        let history = vec![Precedent {
            candidate: CandidateId::BASELINE,
            parent: None,
            verdict: Verdict::Baseline,
            grade: Some(Grade::new(0.5).expect("grade")),
            note: None,
            public: vec![],
        }];
        let proposal = proposer.propose(&history, &dir.0).expect("proposes");
        assert_eq!(proposal.summary, "Use more drafts.");
        assert_eq!(proposal.usage.tokens(), 120);
        let lib = std::fs::read_to_string(dir.0.join(HARNESS_SRC).join("lib.rs")).expect("lib");
        assert_eq!(lib, "pub fn main() { }\n");
        let prompt = &model.2.borrow()[1].content;
        assert!(
            prompt.contains("candidate 0 (parent -): baseline, grade 0.5000"),
            "{prompt}"
        );
        assert!(
            prompt.contains("pub fn main() {}"),
            "shows the current source"
        );
        assert_eq!(proposer.model().map(ModelId::as_str), Some("outer"));
    }

    #[test]
    fn scripted_edits_cycle() {
        let dir = Dir::new("scripted");
        let proposer = ScriptedProposer::new(vec![
            ScriptedEdit::harness("one", "pub fn main() { /* 1 */ }\n"),
            ScriptedEdit::files("two", vec![("Cargo.toml".into(), "x".into())]),
        ]);
        let names: Vec<String> = (0..3)
            .map(|_| {
                proposer
                    .propose(&[], &dir.0)
                    .map(|p| p.summary)
                    .expect("proposes")
            })
            .collect();
        assert_eq!(names, vec!["one", "two", "one"]);
        assert!(dir.0.join("Cargo.toml").is_file());
        assert_eq!(proposer.history_lengths(), vec![0, 0, 0]);
    }
}
