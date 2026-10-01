//! A dropped `Window` releases its native resources (design review,
//! Tranche 4: rusty_gui X11 `Drop`).
//!
//! Needs an X server (`DISPLAY`); run under `xvfb-run` where there is none.
//! Without one the test reports that it skipped and passes.
#![cfg(target_os = "linux")]

use rusty_gui::WindowBuilder;

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("procfs is mounted")
        .count()
}

#[test]
fn dropping_a_window_closes_its_display_connection() {
    // The first window also loads Xlib's process-wide state, which may hold
    // descriptors of its own; count from after it.
    let Ok(first) = WindowBuilder::new().build() else {
        eprintln!("skipped: no X display (set DISPLAY or run under xvfb-run)");
        return;
    };
    drop(first);
    let before = open_fds();

    for _ in 0..3 {
        let window = WindowBuilder::new()
            .build()
            .expect("the display was reachable a moment ago");
        drop(window);
    }

    assert_eq!(
        open_fds(),
        before,
        "each dropped window leaked a descriptor"
    );
}
