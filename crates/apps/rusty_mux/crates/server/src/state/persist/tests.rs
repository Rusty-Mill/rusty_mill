use super::*;

fn started(shell: bool) -> Started {
    Started {
        shell,
        read_only: false,
        program: "cargo".into(),
        args: vec!["build".into(), "\x1b]0;x\x07".into()],
        cwd: "/".into(),
        workspace: String::new(),
    }
}

/// The saved bytes must not change: the records are positional, so a field
/// that is written would make every older record unreadable.
#[cfg(target_os = "linux")]
#[test]
fn the_workspace_is_not_written_to_the_saved_record() {
    let plain = started(true);
    let mut moved = started(true);
    moved.workspace = "/repo".into();
    assert_eq!(
        serde_json::to_string(&plain).unwrap(),
        serde_json::to_string(&moved).unwrap()
    );
    assert!(!serde_json::to_string(&plain).unwrap().contains("workspace"));
}

#[test]
fn the_workspace_is_the_launch_directory_not_the_last_known_one() {
    let mut s = started(true);
    // A record from before the field: the last known directory stands in.
    assert_eq!(s.workspace(), "/");
    // Later saves move `cwd` and leave the workspace where it was launched.
    s.workspace = "/repo".into();
    s.cwd = "/tmp".into();
    assert_eq!(s.workspace(), "/repo");
}

#[test]
fn the_marker_names_a_command_not_run_again_without_its_controls() {
    let shell = marker(Some(&started(true)));
    assert!(shell.contains("[restored after a server restart]"));
    let command = marker(Some(&started(false)));
    assert!(
        command.contains("read-only: `cargo build ]0;x` was not run again"),
        "{command:?}"
    );
    assert_eq!(marker(None), shell);
}

/// Everything `grid` holds, scrollback first.
fn all_text(grid: &Grid) -> Vec<String> {
    let capture = terminal::Capture {
        history: u32::MAX,
        trim: true,
        unwrap: false,
        vt: false,
    };
    let text = terminal::format(grid, capture, usize::MAX).unwrap();
    text.lines().map(str::to_owned).collect()
}

fn marked(saved: &[u8]) -> Vec<String> {
    let (mut grid, mut parser) = (Grid::new(40, 4), AnsiParser::new());
    parser.advance(&mut grid, saved);
    mark(&mut grid, &mut parser, Some(&started(false)));
    assert!(!grid.in_alt_screen());
    all_text(&grid)
}

#[test]
fn an_alternate_screen_is_kept_on_the_main_one_above_the_marker() {
    let text = marked(b"kept\x1b[?1049h\x1b[31mfull screen app\x1b[");
    let at = |needle: &str| text.iter().position(|l| l.contains(needle));
    assert_eq!(at("kept"), Some(0), "{text:?}");
    assert!(at("full screen app") > at("kept"), "{text:?}");
    assert!(at("restored after") > at("full screen app"), "{text:?}");
}

#[test]
fn a_cursor_above_saved_text_overwrites_none_of_it() {
    let text = marked(b"one\r\ntwo\r\nthree\x1b[1;1H");
    assert_eq!(text[..3], ["one", "two", "three"], "{text:?}");
    assert!(
        text[3..].iter().any(|l| l.contains("restored after")),
        "{text:?}"
    );
}
