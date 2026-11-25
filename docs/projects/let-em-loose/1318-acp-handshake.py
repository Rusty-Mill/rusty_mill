#!/usr/bin/python3 -I
"""#1318 toy run, step 3: a no-cost check that `claude-code-acp` starts under
`lel-jail` with the builder's read and write roots and answers `initialize`
and `session/new`. It never sends `session/prompt`, so no model is asked.

Run inside a delegated user scope (1318-toy-run.sh does it):
  systemd-run --user --scope -p Delegate=yes python3 -I 1318-acp-handshake.py \
      LEL_JAIL WORKDIR [READ_ROOT]...
It makes a child cgroup below its own, a scratch worktree in WORKDIR and a
jail.json shaped like orch's (`builder/live.rs` `jail_file`), then kills the
jail and removes both. Exit 0 and "handshake ok" on success.
"""
import json
import os
import select
import socket
import subprocess
import sys
import time

WAIT = 120  # seconds for each answer
SYSTEM = ["/usr", "/bin", "/lib", "/lib64", "/etc"]


def own_cgroup() -> str:
    line = open("/proc/self/cgroup").read().strip()
    return "/sys/fs/cgroup" + line.split("::", 1)[1]


def jail_spec(cgroup: str, status: str, wt: str, extra: list) -> dict:
    home = os.environ["HOME"]
    read = [p for p in SYSTEM if os.path.exists(p)] + [os.path.join(home, ".claude")] + extra
    env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG", "TERM") if k in os.environ}
    return {
        "cgroup": cgroup, "status": status, "cwd": wt, "read": read, "write": [wt],
        "limits": {"cpu_secs": 600, "memory_bytes": 16 << 30, "file_bytes": 4 << 30,
                   "open_files": 4096, "processes": 512},
        "env": env, "token": None, "program": "/bin/sh",
        "args": ["-c", "exec claude-code-acp <&4 >&4 4>&-"],
    }


def answer(sock: socket.socket, want: int, buf: bytearray) -> dict:
    """The response with id `want`; other lines (notifications) are skipped."""
    until = time.monotonic() + WAIT
    while True:
        while b"\n" in buf:
            line, _, rest = bytes(buf).partition(b"\n")
            buf[:] = rest
            msg = json.loads(line)
            if msg.get("id") == want and "method" not in msg:
                return msg
        left = until - time.monotonic()
        if left <= 0 or not select.select([sock], [], [], left)[0]:
            raise SystemExit(f"STOP: no answer to request {want} within {WAIT}s")
        chunk = sock.recv(65536)
        if not chunk:
            raise SystemExit(f"STOP: the agent closed the socket before answering request {want}")
        buf.extend(chunk)


def call(sock: socket.socket, mid: int, method: str, params: dict, buf: bytearray) -> dict:
    sock.sendall((json.dumps({"jsonrpc": "2.0", "id": mid, "method": method, "params": params}) + "\n").encode())
    msg = answer(sock, mid, buf)
    if "error" in msg:
        raise SystemExit(f"STOP: {method} failed: {json.dumps(msg['error'])}")
    return msg["result"]


def main() -> None:
    jail, work, extra = sys.argv[1], sys.argv[2], sys.argv[3:]
    wt, status, spec_path = (os.path.join(work, n) for n in ("wt", "status", "jail.json"))
    os.makedirs(wt, exist_ok=True)
    open(status, "w").close()
    cgroup = os.path.join(own_cgroup(), "acp-check")
    os.mkdir(cgroup)
    with open(spec_path, "w") as f:
        json.dump(jail_spec(cgroup, status, wt, extra), f)
    ours, theirs = socket.socketpair()
    # High copies first, so placing them at 3 and 4 can't clobber either.
    slot = os.dup2(os.open(os.devnull, os.O_RDONLY), 40)  # stands in for the slot lock
    acp = os.dup2(theirs.fileno(), 41)

    def fds() -> None:
        os.dup2(slot, 3)
        os.dup2(acp, 4)

    proc = subprocess.Popen([jail, spec_path], preexec_fn=fds, pass_fds=(3, 4),
                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.PIPE)
    theirs.close()
    os.close(acp)
    try:
        buf = bytearray()
        init = call(ours, 1, "initialize", {"protocolVersion": 1, "clientCapabilities": {
            "fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}}, buf)
        print("initialize:", json.dumps(init)[:400])
        new = call(ours, 2, "session/new", {"cwd": wt, "mcpServers": []}, buf)
        if not new.get("sessionId"):
            raise SystemExit("STOP: session/new answered no sessionId")
        print("session/new: sessionId", new["sessionId"])
        print("handshake ok")
    finally:
        with open(os.path.join(cgroup, "cgroup.kill"), "w") as f:
            f.write("1")
        proc.wait(timeout=30)
        err = proc.stderr.read().decode(errors="replace").strip()
        if err:
            print("lel-jail stderr:", err[-2000:])
        jail_status = open(status).read().strip()
        if jail_status:
            print("jail status:", jail_status)
        for _ in range(50):
            try:
                os.rmdir(cgroup)
                break
            except OSError:
                time.sleep(0.1)


if __name__ == "__main__":
    main()
