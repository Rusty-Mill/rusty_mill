//! Exercises `AsyncSpawner::is_zombie` against a real child — running,
//! killed-but-unreaped, then reaped — mirroring rustils' own
//! `linux_is_zombie_detects_an_unreaped_child`.

use std::time::Duration;

use platform::process::{Command, Signal};
use platform_async::process::AsyncSpawner;
use platform_async_linux::AsyncLinuxSpawner;

#[test]
fn is_zombie_detects_an_unreaped_child() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let mut child = spawner
        .spawn(&Command::new("/bin/sleep", "/").arg("30"))
        .expect("spawn");
    let pid = child.id();

    assert!(
        !spawner.is_zombie(pid).expect("probe"),
        "a running process is not a zombie"
    );

    child.kill_single(Signal::Kill).expect("kill_single");
    // The kernel needs a moment to transition the killed process to `Z`
    // before this, its real parent, reaps it — the probe above this
    // sleep must run before `try_wait` below does the reaping.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        spawner.is_zombie(pid).expect("probe"),
        "an unreaped, terminated child is a zombie"
    );

    child.try_wait().expect("try_wait reaps it");
    assert!(
        !spawner.is_zombie(pid).expect("probe"),
        "a reaped pid no longer exists"
    );
}
