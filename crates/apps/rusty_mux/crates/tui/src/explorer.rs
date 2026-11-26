//! The workspace tree in the terminal client (card #1573, D337): the model
//! lives in [`rusty_mux_client::workspace`], shared with the GUI, and is
//! re-exported here. This module takes keys and draws text rows; it sends
//! nothing. A caller fetches the path [`Tree::enter`] names or
//! [`Explorer::wants`] asks for and hands the reply to the model.
//!
//! Rendering clips by terminal cell width (`char_width`); combining marks and
//! grapheme clusters are not joined, which the server's escaping makes rare.

use rusty_term_core::char_width;

pub use rusty_mux_client::workspace::*;

/// `line` cut to `width` terminal cells, never splitting a wide character.
fn clip(line: &str, width: usize) -> String {
    let mut used = 0;
    line.chars()
        .take_while(|c| {
            used += char_width(*c);
            used <= width
        })
        .collect()
}

/// The rows of `tree` to draw in `height` lines of `width` columns, scrolled
/// to keep the cursor in view. The cursor row starts with `>`.
#[must_use]
pub fn render_tree(tree: &Tree, width: usize, height: usize) -> Vec<String> {
    let rows = tree.rows();
    // The cursor's place among all rows, to scroll by.
    let mut seen = 0;
    let mut here = 0;
    let marked: Vec<bool> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let on = !r.summary && seen == tree.cursor();
            if !r.summary {
                seen += 1;
            }
            if on {
                here = i;
            }
            on
        })
        .collect();
    // Keep the cut summary right under the cursor in view, so a long
    // listing never looks complete.
    let end = here + 1 + rows[here + 1..].iter().take_while(|r| r.summary).count();
    // The selected row first, then as many summaries as fit.
    let top = end.saturating_sub(height.max(1)).min(here);
    rows.iter()
        .zip(marked)
        .skip(top)
        .take(height)
        .map(|(r, on)| {
            let mark = if on { '>' } else { ' ' };
            clip(&format!("{mark}{}{}", "  ".repeat(r.depth), r.text), width)
        })
        .collect()
}

/// What a key batch did to the open [`Explorer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Still open; draw it again.
    Open,
    /// Closed.
    Leave,
    /// Still open; list this path (see [`Want::Fetch`]) and pass the reply to
    /// [`Explorer::load`], or the refusal to [`Explorer::refuse`].
    Fetch(String),
    /// Still open; put this path (relative to the workspace, real names,
    /// `.` for the workspace itself) on the clipboard.
    Copy(String),
}

/// Acts on the keys in `bytes`: `j`/`k` or the arrows move, Enter, `l` or
/// right opens, `h` or left goes up, `y` copies the row's path, `q` or a lone
/// Esc leaves. Stops at the first key that asks for a listing or a copy.
pub fn press(explorer: &mut Explorer, bytes: &[u8]) -> Step {
    if bytes == b"\x1b" {
        return Step::Leave;
    }
    let mut i = 0;
    while i < bytes.len() {
        let key = match bytes[i..] {
            [0x1b, b'[' | b'O', last, ..] => {
                i += 2;
                last
            }
            [b, ..] => b,
            [] => break,
        };
        i += 1;
        match key {
            b'q' => return Step::Leave,
            b'j' | b'B' => explorer.tree_mut().into_iter().for_each(|t| t.step(1)),
            b'k' | b'A' => explorer.tree_mut().into_iter().for_each(|t| t.step(-1)),
            b'h' | b'D' => explorer.tree_mut().into_iter().for_each(Tree::leave),
            b'y' => {
                let path = explorer.tree().map(|t| match t.selected_path() {
                    p if p.is_empty() => ".".to_owned(),
                    p => p,
                });
                if let Some(path) = path {
                    explorer.clear_note();
                    return Step::Copy(path);
                }
            }
            b'l' | b'C' | b'\r' | b'\n' if !explorer.waiting() => {
                if let Some(Want::Fetch(path)) = explorer.tree_mut().map(Tree::enter) {
                    explorer.set_waiting(true);
                    return Step::Fetch(path);
                }
            }
            _ => {}
        }
    }
    Step::Open
}

/// `height` lines of `width` cells: a title, the tree and, last, the
/// refusal or a wait notice.
#[must_use]
pub fn render(explorer: &Explorer, width: usize, height: usize) -> Vec<String> {
    let mut lines = vec![clip(
        "workspace: j/k move, Enter open, h up, y copy path, q leave",
        width,
    )];
    let status = match (explorer.note(), explorer.waiting()) {
        (Some(note), _) => Some(note),
        (None, true) => Some("reading..."),
        (None, false) => None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stat = explorer.stat_line(now);
    let foot = usize::from(status.is_some()) + usize::from(stat.is_some());
    let room = height.saturating_sub(1 + foot);
    if let Some(tree) = explorer.tree() {
        lines.extend(render_tree(tree, width, room));
    }
    lines.truncate(height.saturating_sub(foot));
    lines.extend(stat.map(|s| clip(&s, width)));
    lines.extend(status.map(|s| clip(s, width)));
    lines.truncate(height);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(l: &Listing) -> Tree {
        Tree::new(l).unwrap()
    }

    fn root() -> Listing {
        parse(".\nd\tsrc\nf\tREADME.md\nl\tlink\no\tsock").unwrap()
    }

    #[test]
    fn draws_kinds_and_the_cursor() {
        let t = tree(&root());
        assert_eq!(
            render_tree(&t, 40, 10),
            [
                ">v ./",
                "   + src/",
                "     README.md",
                "   @ link",
                "   ? sock"
            ]
        );
    }

    #[test]
    fn only_a_directory_opens_and_it_asks_for_its_listing_once() {
        let mut t = tree(&root());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("src".into()));
        t.load(&parse("src\nf\tlib.rs").unwrap());
        assert_eq!(render_tree(&t, 40, 10)[2], "       lib.rs");
        // Loaded: Enter now toggles, with no second fetch.
        assert_eq!(t.enter(), Want::Nothing);
        assert_eq!(render_tree(&t, 40, 10).len(), 5);
        t.step(1);
        t.step(1);
        assert_eq!(t.enter(), Want::Nothing, "a file does nothing");
        t.step(1);
        assert_eq!(t.enter(), Want::Nothing, "a link is never followed");
    }

    #[test]
    fn escaped_names_attach_their_replies() {
        // Exactly as the server writes a tab and a backslash in a name.
        let mut t = tree(&parse(".\nd\ta\\tb\nd\tc\\\\d").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a\tb".into()));
        t.load(&parse("a\\tb\nf\tin").unwrap());
        assert_eq!(render_tree(&t, 40, 10)[2], "       in");
        t.step(2);
        assert_eq!(t.enter(), Want::Fetch("c\\d".into()));
        t.load(&parse("c\\\\d\nf\tx").unwrap());
        assert_eq!(render_tree(&t, 40, 10).len(), 5);
    }

    #[test]
    fn a_late_reply_does_not_move_the_selection() {
        let mut t = tree(&parse(".\nd\ta\nd\tb").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a".into()));
        t.step(1);
        t.load(&parse("a\nf\tx\nf\ty").unwrap());
        assert_eq!(t.selected_path(), "b");
        assert!(
            render_tree(&t, 40, 10)
                .iter()
                .any(|r| r.starts_with(">  + b/"))
        );
    }

    #[test]
    fn the_cut_row_is_never_under_the_cursor() {
        let mut t = tree(&parse(".\nd\ta\n+\t2").unwrap());
        t.step(5);
        assert_eq!(t.selected_path(), "a");
        let rows = render_tree(&t, 40, 10);
        assert!(rows[2].starts_with(' ') && rows[2].contains("not shown"));
        assert_eq!(t.enter(), Want::Fetch("a".into()), "not the root toggle");
    }

    #[test]
    fn wide_characters_are_clipped_by_cells() {
        let t = tree(&parse(".\nf\t\u{4e2d}\u{6587}").unwrap());
        // Five columns of mark, indent and sign, then two 2-cell characters:
        // 8 cells fit one, and the second is not split.
        assert_eq!(render_tree(&t, 8, 10)[1], "     \u{4e2d}");
    }

    #[test]
    fn leave_closes_then_goes_to_the_parent() {
        let mut t = tree(&root());
        t.step(1);
        t.enter();
        t.load(&parse("src\nf\tlib.rs").unwrap());
        t.step(1);
        t.leave();
        assert_eq!(t.selected_path(), "src");
        t.leave();
        assert_eq!(render_tree(&t, 40, 10).len(), 5, "src is closed");
    }

    #[test]
    fn a_cut_listing_says_so() {
        let t = tree(&parse(".\nf\ta\n+\t2\n!\t1").unwrap());
        assert_eq!(
            render_tree(&t, 40, 10).last().unwrap(),
            "   ... 2 not shown, 1 unreadable"
        );
    }

    #[test]
    fn a_stale_reply_is_ignored_and_the_cursor_is_held() {
        let mut t = tree(&root());
        t.load(&parse("gone\nf\tx").unwrap());
        assert_eq!(render_tree(&t, 40, 10).len(), 5);
        t.step(100);
        assert_eq!(t.selected_path(), "sock");
        t.step(-100);
        assert_eq!(t.selected_path(), "");
    }

    #[test]
    fn long_trees_scroll_to_the_cursor_and_lines_are_clipped() {
        let mut t = tree(&parse(".\nf\t1\nf\t2\nf\t3\nf\t4").unwrap());
        t.step(4);
        let rows = render_tree(&t, 5, 3);
        assert_eq!(rows.len(), 3);
        assert!(rows[2].starts_with('>'));
        assert!(rows.iter().all(|r| r.chars().count() <= 5));
    }

    #[test]
    fn a_repeat_reply_keeps_what_was_read() {
        let mut t = tree(&parse(".\nd\ta").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a\nd\tb").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a/b\nf\tc").unwrap());
        t.load(&parse("a\nd\tb").unwrap());
        assert_eq!(render_tree(&t, 40, 10).len(), 4, "b stays open with c");
    }

    #[test]
    fn the_more_trailer_is_not_called_unread() {
        let t = tree(&parse(".\nf\ta\n>\t19001").unwrap());
        assert_eq!(
            render_tree(&t, 40, 10).last().unwrap(),
            "   ... 19001+ not shown"
        );
    }

    #[test]
    fn the_cut_summary_is_reachable_in_a_short_view() {
        let mut t = tree(&parse(".\nf\ta\nf\tb\nf\tc\n+\t2").unwrap());
        t.step(10);
        let rows = render_tree(&t, 40, 3);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].starts_with('>') && rows[2].contains("2 not shown"));
    }

    #[test]
    fn the_selection_stays_in_view_before_summaries() {
        let mut t = tree(&parse(".\nf\ta\nf\tb\n+\t2").unwrap());
        t.step(10);
        assert_eq!(render_tree(&t, 40, 1), [">    b"]);
        // Adjacent summaries of nested cut listings: two of them under `x`.
        let mut t = tree(&parse(".\nd\ta\n+\t1").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a\nf\tx\n+\t3").unwrap());
        t.step(1);
        assert_eq!(t.selected_path(), "a/x");
        let two = render_tree(&t, 40, 2);
        assert!(two[0].starts_with('>') && two[1].contains("3 not shown"));
        let three = render_tree(&t, 40, 3);
        assert!(three[0].starts_with('>') && three[2].contains("1 not shown"));
        assert_eq!(render_tree(&t, 40, 1).len(), 1);
        assert!(render_tree(&t, 40, 1)[0].starts_with('>'));
    }

    #[test]
    fn the_explorer_reads_the_root_then_asks_for_a_directory_once() {
        let mut e = Explorer::opened(true);
        assert!(e.waiting());
        e.load(".\nd\tsrc\nf\ta");
        assert!(!e.waiting());
        assert_eq!(press(&mut e, b"j"), Step::Open);
        assert_eq!(press(&mut e, b"\r"), Step::Fetch("src".into()));
        // Waiting: a second Enter asks for nothing.
        assert_eq!(press(&mut e, b"\r"), Step::Open);
        e.load("src\nf\tlib.rs");
        assert!(render(&e, 40, 10).iter().any(|r| r.ends_with("lib.rs")));
    }

    #[test]
    fn keys_move_leave_and_read_arrows() {
        let mut e = Explorer::opened(true);
        e.load(".\nf\ta\nf\tb");
        assert_eq!(press(&mut e, b"\x1b[B\x1b[B"), Step::Open);
        assert!(render(&e, 40, 10).iter().any(|r| r.starts_with(">    b")));
        assert_eq!(press(&mut e, b"k"), Step::Open);
        assert_eq!(press(&mut e, b"\x1b"), Step::Leave);
        assert_eq!(press(&mut e, b"q"), Step::Leave);
    }

    #[test]
    fn a_refusal_is_one_clean_line_and_frees_the_next_request() {
        let mut e = Explorer::opened(true);
        e.refuse("not listed\u{1b}[2J\n");
        assert!(!e.waiting());
        let lines = render(&e, 40, 4);
        assert_eq!(lines.last().unwrap().trim_end(), "not listed [2J");
        assert!(lines.iter().all(|l| !l.chars().any(char::is_control)));
    }

    #[test]
    fn a_bad_first_reply_leaves_no_tree_and_says_why() {
        let mut e = Explorer::opened(true);
        e.load("src\nf\ta");
        assert!(render(&e, 100, 3)[1].contains("not the root"));
        e.load("garbage");
        assert!(render(&e, 60, 3).len() <= 3);
    }

    #[test]
    fn render_fits_its_height() {
        let mut e = Explorer::opened(true);
        e.load(".\nf\ta\nf\tb\nf\tc");
        for h in 0..6 {
            assert!(render(&e, 30, h).len() <= h);
        }
    }

    #[test]
    fn marks_touched_files_and_the_directories_above_them() {
        let mut t = tree(&parse(".\nd\tsrc\nd\tdocs\nf\tnew").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("src".into()));
        t.load(&parse("src\nf\tlib.rs\nf\tok.rs").unwrap());
        t.set_touched(parse_touched(".\n.M\tsrc/lib.rs\n??\tnew\nM.\tdocs/").unwrap());
        assert_eq!(
            render_tree(&t, 40, 10),
            [
                " v ./  *",
                ">  v src/  *",
                "       lib.rs  M",
                "       ok.rs",
                "   + docs/  M",
                "     new  ?"
            ]
        );
    }

    #[test]
    fn wants_marks_once_then_each_cursor_rows_stat_once() {
        let mut e = Explorer::opened(true);
        assert_eq!(e.wants(), None, "no tree yet");
        e.load(".\nd\tsrc\nf\ta");
        assert_eq!(e.wants(), Some(Call::Touched));
        assert_eq!(e.wants(), Some(Call::Stat(".".into())));
        assert_eq!(e.wants(), None, "one stat in flight");
        e.load_stat(Some("d\t0\t60"));
        assert_eq!(e.wants(), None, "the root's is read");
        assert_eq!(e.stat_line(180).as_deref(), Some("0 B, 2 min ago"));
        press(&mut e, b"j");
        assert_eq!(e.stat_line(180), None);
        assert_eq!(e.wants(), Some(Call::Stat("src".into())));
        e.load_stat(None);
        assert_eq!(e.wants(), None, "a refusal is not asked again");
        assert!(!Explorer::opened(false).wants().is_some());
    }

    #[test]
    fn a_touched_reply_that_cannot_be_read_shows_a_note_and_no_marks() {
        let mut e = Explorer::opened(true);
        e.load(".\nf\ta");
        e.load_touched("not a reply");
        let screen = render(&e, 60, 5);
        assert!(
            screen.iter().any(|l| l.contains("touched reply")),
            "{screen:?}"
        );
        assert!(!screen.iter().any(|l| l.contains("  M")));
    }

    #[test]
    fn y_asks_to_copy_the_row_path_with_real_names() {
        let mut e = Explorer::opened(false);
        assert_eq!(
            press(&mut e, b"y"),
            Step::Open,
            "nothing to copy before the tree"
        );
        e.load(".\nd\ta\\tb");
        assert_eq!(
            press(&mut e, b"y"),
            Step::Copy(".".into()),
            "the workspace itself"
        );
        press(&mut e, b"j");
        assert_eq!(press(&mut e, b"y"), Step::Copy("a\tb".into()));
        e.say("path copied");
        assert!(render(&e, 40, 4).iter().any(|l| l == "path copied"));
        press(&mut e, b"y");
        assert!(!render(&e, 40, 4).iter().any(|l| l == "path copied"));
    }
}
