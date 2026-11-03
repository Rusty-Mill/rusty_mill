# rusty_mux UDP manual check, 2026-10-31

Build step 4 of [UDP-TRANSPORT.md](UDP-TRANSPORT.md): `rusty-mux --ssh`
through a real OpenSSH server, on Linux, at `main` after #1283.

## Setup

- OpenSSH `sshd` on 127.0.0.1:2222 with a throwaway host key and one
  authorized key, `AllowStreamLocalForwarding yes`. The client's
  `~/.ssh/config` names it `box`.
- `rusty-mux-server --udp 127.0.0.1 /tmp/mcheck/s.sock`, which listens for UDP
  on the default port 60001.
- A small Python driver runs `rusty-mux --ssh box /tmp/mcheck/s.sock` in a
  pty. It waits 5 s, lists the client's `ssh` processes and UDP sockets (from
  `/proc`), types `echo mark-$((6*7))` and looks for `mark-42`, then sends
  Ctrl-] `d` and checks that the client exited.

## Results

| Case | `ssh` after 5 s | Client UDP socket | Command ran | Detach exits |
|---|---|---|---|---|
| Server with `--udp` | none: the forward stopped | one, bound to 0.0.0.0 | yes | yes |
| `RUSTY_MUX_UDP=off` | `ssh -N -T … -L …` | none | yes | yes |
| `--udp-host 127.0.0.2`: nothing on the port, so the kernel refuses it | `ssh -N -T … -L …` | none | yes | yes |
| `--udp-host 127.0.0.3`: a socket that reads nothing, so datagrams vanish | `ssh -N -T … -L …` | none | yes | yes |
| Server without `--udp` | `ssh -N -T … -L …` | none | yes | yes |

In every case the session worked. With UDP offered and reachable, the client
moved onto it and stopped SSH. In every other case it stayed on SSH.

## Not covered here

- **Roaming.** The client binds to 0.0.0.0 and this host has one route, so
  its source address cannot change under a real sshd. The roam is tested on
  loopback, 127.0.0.2 to 127.0.0.3, in `tui/tests/udp.rs`
  (`a_moved_connection_keeps_its_client_through_a_roam`). This is the
  Phase 3 exit test.
- **Other platforms.** Windows, macOS and a real network path were not run.
- **The unbuilt pieces** listed in the design's status.
