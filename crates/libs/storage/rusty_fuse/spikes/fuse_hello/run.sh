#!/bin/bash
# Spike B evidence run. Must be started as root inside the disposable distro.
set -u
bin=/home/spike/rm/crates/libs/storage/rusty_fuse/spikes/fuse_hello/target/release/rusty_fuse_spike_fuse_hello
echo "== environment"
uname -r; grep PRETTY /etc/os-release
ls -l /dev/fuse; command -v fusermount3; ls /usr/lib/x86_64-linux-gnu/libfuse3.so.3*
grep -E '^Seccomp' /proc/self/status
for k in kernel.unprivileged_userns_clone kernel.apparmor_restrict_unprivileged_userns user.max_user_namespaces; do
  printf '%s = %s\n' "$k" "$(sysctl -n $k 2>&1)"
done
echo "LSMs: $(cat /sys/kernel/security/lsm 2>&1)"

echo; echo "== A: root, mount + self-test + umount2"
mkdir -p /root/mnt; "$bin" run /root/mnt; echo "exit=$?"

echo; echo "== B: root, external ls/cat from another process"
"$bin" serve /root/mnt & pid=$!
sleep 1
ls -la /root/mnt; cat /root/mnt/hello.txt; cat /root/mnt/dir/a.txt; stat -f -c 'type=%T' /root/mnt
grep rusty_fuse /proc/mounts
"$bin" umount /root/mnt; echo "umount exit=$?"
wait $pid; echo "serve exit=$?"
grep -c rusty_fuse /proc/mounts

echo; echo "== C (Q5): non-root, plain mount(2) without a user namespace"
su spike -c "mkdir -p /tmp/spike-mnt && $bin run /tmp/spike-mnt"; echo "exit=$?"

echo; echo "== D (Q5): non-root, unshare(CLONE_NEWUSER|CLONE_NEWNS) then mount(2)"
su spike -c "mkdir -p /tmp/spike-mnt && $bin userns /tmp/spike-mnt"; echo "exit=$?"
echo "leftover mounts: $(grep -c rusty_fuse /proc/mounts)"
