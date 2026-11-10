# C36: upgrade without losing sessions

C36 ([CAPABILITIES](CAPABILITIES.md) row; gap check G24) is about upgrading
rusty_mux while sessions run. At the Rex level, a running server keeps its own
version and its sessions until it stops, and new connections get whatever the
server is. What a user needs on top of that is to be told when the server
lacks what their client can do, and a clean way to stop the old server.

The plan is in the rusty_term handoff pack: D141 to D144 cover the arc 3 order,
and D144 gives the shape of slice (a). The behaviour comes from term ROADMAP
question 9.

| Slice | What | State |
|---|---|---|
| (a) | Hello and Welcome carry each side's version; a client names what an older server lacks | this PR |
| (b) | A clean stop for upgrades (`Shutdown`, `rusty-mux-ctl server stop`, a clean stop on SIGTERM) | built (pack D160, D162, D163); see (b) below |

## (a) Version notice

**Wire.** Hello and Welcome gain field 57, `version`: the sender's
`rusty_mux_proto` package version as text (`rusty_mux_proto::VERSION`). Older
decoders skip the field, and an older peer sends none, so the field changes
nothing for either side. No Features bit is added, because nothing new is
asked for. A received version that is empty, longer than 64 bytes or not
printable ASCII decodes as none, so a peer can't put escape sequences on a
user's terminal and a bad version never fails the handshake.

**The notice.** A client compares the features it asked for with those the
Welcome granted. If any are missing, it tells the user once per connection.
The notice gives the server's version, or "unknown" when the server sent none,
and names what the missing features do (`Features::names`,
`rusty_mux_proto::older`). `DATAGRAM` is not counted, because a current server
grants it only with `--udp`, so its absence says nothing about the server's
version.

- `rusty-mux-gui` shows the notice in the info banner (gui-design D26 and D63).
  It sits under the strip in the connection banner's place, with a slate alert
  icon, and stays until the user clicks Dismiss. A link banner takes its place
  while it shows, and the notice comes back after it. Attaching anew to a
  server, perhaps an upgraded one, counts as a new connection; taking a client
  over on the same server does not.
- `rusty-mux` prints it on stderr when it leaves the screen, because the
  terminal client has no place to show a message while it runs.
- `rusty-mux-ctl` prints it on stderr, so scripts reading stdout are not
  affected; `rusty-mux-ctl own` prints it on each connection, since a
  reconnect may reach an upgraded server. A command that needs a missing feature still fails with its own
  error.

Example: `the server runs an older rusty_mux (0.1.0; this is 0.2.0). It lacks
layout editing, shared focus. Some actions are unavailable.`

**No floor yet.** D144 lets a server refuse clients below a version floor, but
the floor starts at none. The server reads the client's version and refuses
no one. The check is added with the first break that needs it, because a check
that can never fire would only be code to maintain.

**Known limit.** Every crate is still at 0.1.0, so the two versions can match
while the features differ. The notice then says the server runs "another build
of rusty_mux 0.1.0", and the feature names tell the user what is missing.
Raising the version is a release question, not a C36 question. A server test
checks that every feature it offers has a name, so a new bit can't drop out of
the notice.

## (b) Clean stop

Nano chose the recommended shape (pack D160). One stop path serves every trigger: the
store is saved, owned blocks are hung up rather than killed (D87), the socket name is
removed while it is still this server's (Linux), and connections close last.

- **Signals.** On Linux, SIGTERM and SIGINT are clean stops, as Ctrl+C and Ctrl+Break are on
  Windows. Before, a Linux signal killed the server without saving.
- **Wire.** Client message `Shutdown { req }`, type 0x24, behind `Features::SHUTDOWN`
  (bit 38). Only a client of the server's own account may send it, since a stop ends every
  session; anyone else is refused, as is a second request. The server answers `Done`, then
  stops (pack D162). It is not a catalog method, so agents' `api` and MCP tools don't offer it.
- **ctl.** `rusty-mux-ctl server stop` sends it and waits for the connection to close, which
  happens only after the save and the name release. It exits 0 once stopped, 4 when no server
  answers, 1 when refused or when the server predates `SHUTDOWN`, and 124 if the stop takes
  over 15 s (pack D163). A second `server stop` right after exits 4.
- **Upgrade recipe.** `rusty-mux-ctl server stop`, install the new binaries, then connect: the
  first client auto-starts the new server (C69), which restores the saved sessions (C45).
