use super::*;
use crate::marker::{marker_source, MARKER};
use ai_env_core::{plan_disable, plan_enable, Kind};
use std::sync::atomic::{AtomicU32, Ordering};

/// A scratch folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aienv-fs-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An entry with a two-file source and an empty tool folder.
fn fixture(t: &Tmp) -> (Entry, Tool) {
    let source = t.0.join("src/code-review");
    fs::create_dir_all(source.join("refs")).unwrap();
    fs::write(source.join("SKILL.md"), "hello").unwrap();
    fs::write(source.join("refs/a.md"), "a").unwrap();
    let entry = Entry {
        id: "code-review".parse().unwrap(),
        kind: Kind::Skill,
        source,
    };
    let tool = Tool {
        id: "claude-code".parse().unwrap(),
        skills_dir: t.0.join("home/skills"),
    };
    (entry, tool)
}

fn enable(e: &Entry, t: &Tool, m: LinkMethod) -> Result<(), Error> {
    apply(&plan_enable(e, t, observe(e, t).unwrap(), m))
}

fn disable(e: &Entry, t: &Tool) -> Result<(), Error> {
    apply(&plan_disable(e, t, observe(e, t).unwrap()))
}

#[cfg(unix)]
#[test]
fn symlink_round_trip() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Symlink).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedCurrent);
    assert!(t.target(&e).join("SKILL.md").is_file());
    disable(&e, &t).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::Absent);
    assert!(e.source.join("SKILL.md").is_file(), "source must survive");
}

#[cfg(unix)]
#[test]
fn symlink_elsewhere_is_unmanaged_and_kept() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    fs::create_dir_all(&t.skills_dir).unwrap();
    std::os::unix::fs::symlink(tmp.0.join("src"), t.target(&e)).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::Unmanaged);
    assert!(matches!(disable(&e, &t), Err(Error::Refused(_))));
    assert!(fs::symlink_metadata(t.target(&e)).is_ok());
}

#[test]
fn copy_round_trip_writes_marker() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    assert!(t.target(&e).join(MARKER).is_file());
    assert!(t.target(&e).join("refs/a.md").is_file());
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedCurrent);
    // Enabling again is a no-op.
    assert!(plan_enable(&e, &t, observe(&e, &t).unwrap(), LinkMethod::Copy).is_noop());
    disable(&e, &t).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::Absent);
}

#[test]
fn changed_source_makes_copy_stale_and_enable_replaces_it() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    fs::write(e.source.join("SKILL.md"), "v2").unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedStale);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    assert_eq!(
        fs::read_to_string(t.target(&e).join("SKILL.md")).unwrap(),
        "v2"
    );
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedCurrent);
}

#[test]
fn edited_copy_is_refused_and_edits_survive() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    fs::write(t.target(&e).join("SKILL.md"), "my edit").unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedModified);
    assert!(matches!(disable(&e, &t), Err(Error::Refused(_))));
    assert!(matches!(
        enable(&e, &t, LinkMethod::Copy),
        Err(Error::Refused(_))
    ));
    assert_eq!(
        fs::read_to_string(t.target(&e).join("SKILL.md")).unwrap(),
        "my edit"
    );
}

#[test]
fn unmarked_folder_is_never_replaced_or_removed() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    let target = t.target(&e);
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("mine.md"), "keep").unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::Unmanaged);
    assert!(matches!(
        enable(&e, &t, LinkMethod::Copy),
        Err(Error::Refused(_))
    ));
    assert!(matches!(disable(&e, &t), Err(Error::Refused(_))));
    assert!(matches!(unlink(&target), Err(Error::NotManaged(_))));
    assert_eq!(fs::read_to_string(target.join("mine.md")).unwrap(), "keep");
}

#[test]
fn plain_file_at_target_is_unmanaged() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    fs::create_dir_all(&t.skills_dir).unwrap();
    fs::write(t.target(&e), "x").unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::Unmanaged);
    assert!(matches!(unlink(&t.target(&e)), Err(Error::NotManaged(_))));
}

#[cfg(unix)]
#[test]
fn junction_is_unsupported_off_windows_and_the_chain_still_links() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    assert!(junction(&e.source, &t.target(&e)).is_err());
    // A planned junction is not in the Unix chain: the walk starts at the
    // front, so the link still succeeds.
    enable(&e, &t, LinkMethod::Junction).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedCurrent);
}

#[cfg(unix)]
#[test]
fn symlink_inside_an_entry_fails_the_copy_cleanly() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    std::os::unix::fs::symlink("/etc", e.source.join("escape")).unwrap();
    assert!(enable(&e, &t, LinkMethod::Copy).is_err());
    assert!(!t.target(&e).exists(), "no partial copy left behind");
}

#[test]
fn tree_hash_ignores_marker_and_sees_changes() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let before = tree_hash(&e.source).unwrap();
    fs::write(e.source.join(MARKER), "x").unwrap();
    assert_eq!(tree_hash(&e.source).unwrap(), before);
    fs::write(e.source.join("refs/a.md"), "b").unwrap();
    assert_ne!(tree_hash(&e.source).unwrap(), before);
}

#[test]
fn unlink_refuses_an_edited_copy_even_if_the_plan_said_remove() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    fs::write(t.target(&e).join("SKILL.md"), "my edit").unwrap();
    assert!(matches!(unlink(&t.target(&e)), Err(Error::Modified(_))));
    assert!(t.target(&e).join("SKILL.md").is_file());
}

#[test]
fn source_that_changes_during_copy_reads_stale_not_modified() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    // The marker hashes what was written, so a later source change is Stale.
    fs::write(e.source.join("SKILL.md"), "changed").unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedStale);
}

#[test]
fn source_paths_that_break_the_marker_are_refused() {
    assert!(marker_source(Path::new("a/b")).is_ok());
    assert!(marker_source(Path::new("line\nbreak")).is_err());
    assert!(marker_source(Path::new("cr\rhere")).is_err());
}

/// End to end, so the refusal is proven to happen before any change. Needs a
/// filesystem that allows a line break in a name, which Windows does not.
#[cfg(unix)]
#[test]
fn unrecordable_source_is_refused_before_any_change() {
    let tmp = Tmp::new();
    let (mut e, t) = fixture(&tmp);
    let odd = tmp.0.join("src/line\nbreak");
    fs::rename(&e.source, &odd).unwrap();
    e.source = odd;
    assert!(enable(&e, &t, LinkMethod::Copy).is_err());
    assert!(!t.target(&e).exists());
}

#[test]
fn cmd_metacharacters_are_not_safe_for_mklink() {
    assert!(cmd_safe(Path::new("C:\\Users\\me\\skills\\code-review")));
    for bad in ["a&b", "a|b", "a^b", "a\"b", "%PATH%", "a>b", "a(b)", "a!b"] {
        assert!(!cmd_safe(Path::new(bad)), "{bad}");
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_by_another_spelling_of_the_source_is_still_ours() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    fs::create_dir_all(&t.skills_dir).unwrap();
    // `src/./code-review` is a different path text for the same folder.
    let alias = tmp.0.join("src/./code-review");
    std::os::unix::fs::symlink(&alias, t.target(&e)).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedCurrent);
}

#[test]
fn discovery_finds_skill_folders_and_reports_bad_names() {
    let tmp = Tmp::new();
    let root = tmp.0.join("skills");
    for (name, has_skill) in [
        ("beta", true),
        ("alpha", true),
        ("Bad_Name", true),
        ("notes", false),
    ] {
        fs::create_dir_all(root.join(name)).unwrap();
        if has_skill {
            fs::write(root.join(name).join("SKILL.md"), "x").unwrap();
        }
    }
    fs::write(root.join("README.md"), "x").unwrap();
    let found = discover_skills(&root).unwrap();
    let ids: Vec<_> = found.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "beta"]);
    assert!(found.entries.iter().all(|e| e.source.is_absolute()));
    assert_eq!(found.skipped.len(), 1);
    assert!(found.skipped[0].path.ends_with("Bad_Name"));
    assert!(discover_skills(&tmp.0.join("missing")).is_err());
}

// --- editor (edit.rs) ---

use ai_env_core::{plan_edit, EditPlan, EditRefusal, Ownership, RelPath};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

/// A plan to replace `file` under `e`'s source with `new`, from a fresh read.
fn edit_plan(e: &Entry, file: &str, own: Ownership, new: &str) -> EditPlan {
    let read = read_file(&e.source, &rel(file)).unwrap();
    plan_edit(
        e.id.clone(),
        rel(file),
        own,
        read.hash,
        &read.text,
        new.into(),
    )
    .unwrap()
}

#[test]
fn lists_regular_files_without_the_marker() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    fs::write(e.source.join(MARKER), "x").unwrap();
    let names: Vec<_> = list_files(&e.source).unwrap();
    assert_eq!(names, [rel("SKILL.md"), rel("refs/a.md")]);
}

#[test]
fn read_returns_text_and_a_hash_that_follows_the_bytes() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let a = read_file(&e.source, &rel("SKILL.md")).unwrap();
    assert_eq!(a.text, "hello");
    assert_eq!(a.hash, read_file(&e.source, &rel("SKILL.md")).unwrap().hash);
    fs::write(e.source.join("SKILL.md"), "hellp").unwrap();
    assert_ne!(a.hash, read_file(&e.source, &rel("SKILL.md")).unwrap().hash);
}

#[test]
fn read_refuses_binary_oversize_missing_and_directories() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    fs::write(e.source.join("bin"), [0xff, 0x00]).unwrap();
    fs::write(
        e.source.join("big"),
        vec![b'a'; ai_env_core::MAX_EDIT_BYTES + 1],
    )
    .unwrap();
    assert!(matches!(
        read_file(&e.source, &rel("bin")),
        Err(Error::Edit(EditRefusal::NotText))
    ));
    assert!(matches!(
        read_file(&e.source, &rel("big")),
        Err(Error::Edit(EditRefusal::TooLarge(_)))
    ));
    assert!(matches!(
        read_file(&e.source, &rel("nope")),
        Err(Error::Io(..))
    ));
    assert!(matches!(
        read_file(&e.source, &rel("refs")),
        Err(Error::NotFile(_))
    ));
}

#[test]
fn write_replaces_the_text_and_keeps_the_mode() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let file = e.source.join("SKILL.md");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    }
    write_file(
        &e.source,
        &edit_plan(&e, "SKILL.md", Ownership::Source, "new\n"),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    let leftovers = fs::read_dir(&e.source).unwrap().count();
    assert_eq!(leftovers, 2, "no temp file left behind");
}

#[test]
fn write_refuses_a_file_changed_since_it_was_read() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let plan = edit_plan(&e, "SKILL.md", Ownership::Source, "mine");
    fs::write(e.source.join("SKILL.md"), "theirs").unwrap();
    assert!(matches!(
        write_file(&e.source, &plan),
        Err(Error::Changed(_))
    ));
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "theirs"
    );
}

#[cfg(unix)]
#[test]
fn links_inside_the_entry_are_never_followed() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let outside = tmp.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), "s").unwrap();
    std::os::unix::fs::symlink(outside.join("secret"), e.source.join("leak")).unwrap();
    std::os::unix::fs::symlink(&outside, e.source.join("dir")).unwrap();
    assert!(matches!(
        read_file(&e.source, &rel("leak")),
        Err(Error::Link(_))
    ));
    assert!(matches!(
        read_file(&e.source, &rel("dir/secret")),
        Err(Error::Link(_))
    ));
    assert_eq!(fs::read_to_string(outside.join("secret")).unwrap(), "s");
}

#[cfg(unix)]
#[test]
fn locate_tells_source_copy_and_foreign_apart() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    assert_eq!(locate(&e, &t).unwrap(), None);
    enable(&e, &t, LinkMethod::Symlink).unwrap();
    assert_eq!(
        locate(&e, &t).unwrap(),
        Some((e.source.clone(), Ownership::Source))
    );
    disable(&e, &t).unwrap();
    enable(&e, &t, LinkMethod::Copy).unwrap();
    assert_eq!(
        locate(&e, &t).unwrap(),
        Some((t.target(&e), Ownership::MarkedCopy))
    );
    disable(&e, &t).unwrap();
    fs::create_dir_all(t.target(&e)).unwrap();
    assert_eq!(
        locate(&e, &t).unwrap(),
        Some((t.target(&e), Ownership::Unmanaged))
    );
}

#[test]
fn editing_a_marked_copy_makes_it_edited() {
    let tmp = Tmp::new();
    let (e, t) = fixture(&tmp);
    enable(&e, &t, LinkMethod::Copy).unwrap();
    let root = t.target(&e);
    let read = read_file(&root, &rel("SKILL.md")).unwrap();
    let plan = plan_edit(
        e.id.clone(),
        rel("SKILL.md"),
        Ownership::MarkedCopy,
        read.hash,
        &read.text,
        "x".into(),
    )
    .unwrap();
    write_file(&root, &plan).unwrap();
    assert_eq!(observe(&e, &t).unwrap(), Observed::ManagedModified);
}

#[cfg(unix)]
#[test]
fn staged_text_is_never_more_readable_than_the_original() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let file = e.source.join("SKILL.md");
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    for orig in [0o600, 0o640] {
        fs::set_permissions(&file, fs::Permissions::from_mode(orig)).unwrap();
        let staged =
            edit::stage_for_test(&file, b"secret", fs::metadata(&file).unwrap().permissions())
                .unwrap();
        // Staged, not yet published: original text, and the staged file has no
        // wider mode than the original.
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
        assert_eq!(mode(&staged), orig);
        fs::remove_file(staged).unwrap();
    }
}

#[test]
fn a_failed_stage_leaves_no_temp_file() {
    let tmp = Tmp::new();
    let (e, _) = fixture(&tmp);
    let gone = e.source.join("no-such-dir/x.md");
    assert!(
        edit::stage_for_test(&gone, b"x", fs::metadata(&e.source).unwrap().permissions()).is_err()
    );
    assert_eq!(fs::read_dir(&e.source).unwrap().count(), 2);
}
