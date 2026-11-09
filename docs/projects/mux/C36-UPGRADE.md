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
| (b) | A clean stop for upgrades (`Shutdown`, `rusty-mux-ctl server stop`, a clean stop on SIGTERM) | needs Nano's call between three options (pack D144) |

## (a) Version notice

**Wire.** Hello and Welcome gain field 57, `version`: the sender's
`rusty_mux_proto` package version as text (`rusty_mux_proto::VERSION`). Older
decoders skip the field, and an older peer sends none, so the field changes
nothing for either side. No Features bit is added, because nothing new is
asked for.

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
  affected. A command that needs a missing feature still fails with its own
  error.

Example: `the server runs an older rusty_mux (0.1.0; this is 0.2.0). It lacks
layout editing, shared focus. Some actions are unavailable.`

**No floor yet.** D144 lets a server refuse clients below a version floor, but
the floor starts at none. The server reads the client's version and refuses
no one. The check is added with the first break that needs it, because a check
that can never fire would only be code to maintain.

**Known limit.** Every crate is still at 0.1.0, so the two versions can match
while the features differ. The feature names are what tell the user what is
missing. Raising the version is a release question, not a C36 question.
