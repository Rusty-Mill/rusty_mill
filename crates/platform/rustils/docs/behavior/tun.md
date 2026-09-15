# Behavior Spec — tun (Tun, TunDevice)

RFC v2 R5+, decision D14. Single named consumer: rusty_tail's `ts-tun`
(`ts-tun/src/{lib,sys}.rs`), which hand-rolls `/dev/net/tun` +
`TUNSETIFF` + the `SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFMTU`/
`SIOCGIFFLAGS`/`SIOCSIFFLAGS` configuration dance directly against
`libc`. `Tun::create(name, ipv4, prefix_len, mtu)` mirrors that exact
shape — creation, addressing, MTU, and bring-up bundled into one call
— because that is the whole of what the named consumer does; there is
no finer-grained decomposition (bare create without addressing,
address-only reconfiguration of an already-created device) anyone has
asked for.

Live-verified on Linux (`crates/platform-linux/tests/tun_parity.rs`):
this sandboxed environment genuinely has `/dev/net/tun` and
`CAP_NET_ADMIN`, so the parity suite exercises real kernel interaction
— an actually-created interface, a real installed route, a real
kernel-routed outbound packet, and a hand-crafted (independently
checksummed) inbound packet actually delivered to a bound `UdpSocket`
— not merely "the ioctls returned `Ok`". Each test claims its own
`/24` under CGNAT space (100.64.0.0/10, the same convention `ts-tun`'s
own doc comment uses) rather than sharing one, because `cargo test`
runs test functions on parallel threads by default and two tests
racing for the same subnet's route is a real bug (a blocking `read()`
on the "losing" interface hangs forever waiting for a packet the
kernel delivered to the other test's device instead), not a
theoretical one — it was hit and fixed during development.

CI's hosted `ubuntu-latest` runner executes the `test` job as an
unprivileged user, which lacks `CAP_NET_ADMIN` — `tun_parity.rs`'s
tests skip gracefully (`eprintln!` + return, mirroring
`security_sandbox.rs`'s `NotEnforced` degrade path) rather than fail
when `Tun::create` reports `PermissionDenied`, so a contributor running
`cargo test --workspace` locally without elevated privilege still gets
an honest pass, not a hard requirement to run as root. A dedicated,
`sudo`-elevated CI step re-runs just this suite so CI itself still gets
genuine live coverage rather than only ever exercising the skip path.

With that privileged step actually exercising real kernel behavior, it
also surfaced a genuine device-level ordering gotcha: a freshly-up'd
interface can carry the kernel's own spontaneous traffic (IPv6 router
solicitation/neighbor discovery, most commonly) ahead of whatever a
caller just sent — there is no guarantee the first packet `read()`
returns is the one a test (or any consumer) is waiting for. The
outbound-packet test now loops, skipping anything that doesn't match
the expected shape, rather than asserting on the very first read.

## Specified

- `Tun::create` opens a new TUN (not TAP — no consumer needs an
  Ethernet-framed link) device, assigns `ipv4`/`prefix_len` to it,
  which on Linux also installs the connected route for that subnet (no
  explicit route command needed), sets the interface MTU to `mtu`,
  and brings the interface administratively up before returning.
- The returned `TunDevice::name()` is the OS-assigned interface name —
  ordinarily equal to the requested `name`, but callers must not assume
  equality: a platform's own interface-name length limit could force
  truncation.
- `TunDevice::read` blocks until the kernel has routed an outbound IP
  packet into the tunnel, then returns it verbatim (the whole IP
  packet, header included — this is a layer-3 device, not a
  socket-level payload API).
- `TunDevice::write` injects `buf` as an inbound IP packet: the local
  network stack processes it exactly as if it had arrived over a real
  wire, including delivering UDP/TCP payloads to a socket bound to the
  packet's destination address and port.
- `create` requires elevated privilege (`CAP_NET_ADMIN` on Linux) and
  is a real `Err` (not a panic) when the caller lacks it.
- The concrete Linux device type (`platform_linux::LinuxTunDevice`)
  additionally provides `AsFd`/`AsRawFd`/`set_nonblocking` on the
  concrete (non-boxed) type — the same raw-fd escape hatch
  rustils#41/#42 established for `Net`, for the same reason: `ts-tun`,
  like `ts-magicsock` before it, needs to register the device's fd
  with tokio's own reactor and drive I/O directly, not through this
  trait's blocking calls.

## Deliberately unspecified

- Windows (`wintun`): no backend exists.
  `platform_windows::WindowsTun::create` always returns
  `ErrorKind::Unsupported` — explicitly, rather than the module being
  absent — because `ts-tun` (the only named consumer for this
  surface) is `#![cfg(target_os = "linux")]` only, so there is no
  donor evidence for what a Windows shape should even look like. This
  is the same judgment call `Sandbox` already makes for macOS/Windows
  confinement: no invented design without a consumer to validate it
  against.
- macOS/BSD: no backend at all (not even an `Unsupported` stub) — the
  same "no consumer, no speculative surface" call; nothing currently
  imports `platform-bsd`'s crate expecting a `Tun` impl to exist.
- TAP (Ethernet-framed) devices, multi-queue TUN, `IFF_NO_PI` toggling
  as a caller-visible option (this backend always requests
  `IFF_NO_PI`, matching `ts-tun`), IPv6 addressing, and any routing
  beyond the single connected route `ipv4`/`prefix_len` installs —
  none of these have a named consumer.
- `platform-mock`'s `MockTun`/`MockTunDevice`: does not simulate
  kernel routing at all — there is no "other side" within one process
  the way `MockUdpSocket`/`MockTcpStream` simulate a peer socket, since
  a TUN device's real counterpart is the kernel's own routing table,
  not another endpoint this crate could stand up. Instead it is
  scriptable: a test queues raw bytes for a future `read()` via
  `MockTunDevice::queue_inbound`, and every `write()` call is recorded
  for `written_packets()` to assert against. It also does not block —
  `read()` on an empty queue returns `Ok(0)` immediately rather than
  waiting, the same "no real mechanism to block on" tradeoff
  `MockCsprng` makes for randomness quality.
