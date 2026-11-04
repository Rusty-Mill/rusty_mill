use super::*;

fn started(shell: bool) -> Started {
    Started {
        shell,
        program: "cargo".into(),
        args: vec!["build".into(), "\x1b]0;x\x07".into()],
        cwd: "/".into(),
    }
}

#[test]
fn the_marker_names_a_command_not_run_again_without_its_controls() {
    let shell = marker(Some(&started(true)));
    assert!(shell.contains("[restored after a server restart]"));
    let command = marker(Some(&started(false)));
    assert!(command.contains("`cargo build ]0;x` was not run again"), "{command:?}");
    assert_eq!(marker(None), shell);
}

#[test]
fn the_marker_leaves_the_alternate_screen_below_the_saved_text() {
    let (mut grid, mut parser) = (Grid::new(40, 4), AnsiParser::new());
    parser.advance(&mut grid, b"kept\x1b[?1049hfull screen app\x1b[");
    parser.advance(&mut grid, marker(None).as_bytes());
    let text: Vec<_> = (0..).map_while(|n| grid.line_text(n)).collect();
    assert_eq!(text[0], "kept");
    assert!(text.iter().any(|l| l.contains("restored after")), "{text:?}");
    assert!(!text.iter().any(|l| l.contains("full screen app")), "{text:?}");
}
