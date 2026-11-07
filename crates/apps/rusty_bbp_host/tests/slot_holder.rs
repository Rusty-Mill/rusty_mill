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

#[test]
fn a_zombie_main_thread_does_not_release_a_live_workers_slot() {
    let dir = std::env::temp_dir().join(format!("bbp_slot_threads_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture directory");
    let source = dir.join("worker.c");
    let binary = dir.join("worker");
    std::fs::write(
        &source,
        r#"
#include <pthread.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
static void *work(void *arg) {
    char **paths = arg;
    FILE *heartbeat = fopen(paths[1], "a");
    if (!heartbeat) return NULL;
    while (access(paths[0], F_OK) != 0) {
        fprintf(heartbeat, "%ld\n", syscall(SYS_gettid));
        fflush(heartbeat);
        usleep(10000);
    }
    fclose(heartbeat);
    return NULL;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    pthread_t worker;
    if (pthread_create(&worker, NULL, work, argv + 1) != 0) return 3;
    pthread_exit(NULL);
}
"#,
    )
    .expect("native thread fixture");
    let compiled = Command::new("cc")
        .arg("-pthread")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile pthread fixture");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let slots = Slots::open(
        &dir.join("slots"),
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
    let stop = dir.join("stop");
    let heartbeat = dir.join("heartbeat");
    let worker = member(
        holder.0.id(),
        binary.to_str().expect("binary path"),
        &[
            stop.to_str().expect("stop path"),
            heartbeat.to_str().expect("heartbeat path"),
        ],
    );
    zombie(&worker.0);
    let leader_stat = std::fs::read_to_string(format!("/proc/{}/stat", worker.0.id()))
        .expect("zombie leader stat");
    assert_eq!(
        leader_stat
            .rsplit_once(") ")
            .expect("leader stat")
            .1
            .split_whitespace()
            .nth(17),
        Some("2"),
        "zombie leader and live worker must both remain in the thread group"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&heartbeat)
        .unwrap_or_default()
        .is_empty()
    {
        assert!(Instant::now() < deadline, "worker did not emit heartbeat");
        std::thread::sleep(Duration::from_millis(10));
    }
    let first = std::fs::read_to_string(&heartbeat).expect("heartbeat");
    let tid = first.lines().next().expect("worker tid");
    let live_stat = std::fs::read_to_string(format!("/proc/{}/task/{tid}/stat", worker.0.id()))
        .expect("live worker stat");
    assert!(!live_stat
        .rsplit_once(") ")
        .expect("thread stat")
        .1
        .starts_with("Z "));
    assert_eq!(
        live_stat
            .rsplit_once(')')
            .expect("thread stat")
            .1
            .split_whitespace()
            .nth(2)
            .expect("pgrp"),
        holder.0.id().to_string()
    );
    drop(slot);
    drop(holder.0.stdin.take());
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        std::fs::read_to_string(&heartbeat)
            .expect("heartbeat")
            .len()
            > first.len(),
        "worker stopped while its leader was a zombie"
    );
    assert!(
        holder.0.try_wait().expect("holder status").is_none(),
        "zombie leader released the live worker's holder"
    );
    assert!(slots
        .try_acquire(Class::Agent)
        .expect("held slot")
        .is_none());
    std::fs::write(stop, "stop").expect("stop worker normally");
    let deadline = Instant::now() + Duration::from_secs(3);
    while holder.0.try_wait().expect("holder status").is_none() {
        assert!(
            Instant::now() < deadline,
            "slot did not release after the worker exited"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(slots
        .try_acquire(Class::Agent)
        .expect("released slot")
        .is_some());
    drop(worker);
    drop(holder);
    std::fs::remove_dir_all(dir).expect("remove fixture");
}
