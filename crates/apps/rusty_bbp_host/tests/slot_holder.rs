//! Real slot holders must outlive live members, but not unreaped zombies.
#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use rusty_bbp_host::slots::{Capacity, Class, Slots};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn member(group: u32, program: &str, args: &[&str]) -> Reap {
    Reap(
        Command::new(program)
            .args(args)
            .process_group(group.try_into().expect("group fits pid_t"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("start group member"),
    )
}

fn zombie(child: &Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id()))
            .expect("unreaped child stat");
        if stat
            .rsplit_once(") ")
            .expect("stat fields")
            .1
            .starts_with("Z ")
        {
            return;
        }
        assert!(Instant::now() < deadline, "child did not become a zombie");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn releases_with_zombie(mixed: bool) {
    let dir = std::env::temp_dir().join(format!("bbp_slot_zombie_{}_{mixed}", std::process::id()));
    let slots = Slots::open(
        &dir,
        Capacity {
            agents: 1,
            validators: 1,
        },
    )
    .expect("slots");
    let slot = slots
        .try_acquire(Class::Agent)
        .expect("acquire")
        .expect("slot");
    let mut holder = Reap(
        slot.holder(Path::new(env!("CARGO_BIN_EXE_bbp")))
            .expect("holder"),
    );
    let dead = member(holder.0.id(), "/bin/sh", &["-c", "exit 0"]);
    // Deliberately do not wait: this process owns and later reaps the zombie.
    zombie(&dead.0);
    let live = mixed.then(|| member(holder.0.id(), "sleep", &["30"]));
    drop(slot);
    drop(holder.0.stdin.take());
    if mixed {
        std::thread::sleep(Duration::from_millis(350));
        assert!(holder.0.try_wait().expect("holder status").is_none());
        assert!(slots
            .try_acquire(Class::Agent)
            .expect("held slot")
            .is_none());
    }
    drop(live);
    let deadline = Instant::now() + Duration::from_secs(3);
    while holder.0.try_wait().expect("holder status").is_none() {
        assert!(Instant::now() < deadline, "zombie kept the holder alive");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(slots
        .try_acquire(Class::Agent)
        .expect("released slot")
        .is_some());
    drop(dead);
    drop(holder);
    std::fs::remove_dir_all(dir).expect("remove fixture");
}

#[test]
fn zombie_only_group_releases_its_slot_without_waiting_for_reaping() {
    releases_with_zombie(false);
}

#[test]
fn a_live_member_keeps_the_slot_when_another_member_is_a_zombie() {
    releases_with_zombie(true);
}
