---
category: Fixed
changelog: rusty_mux keeps a reconnecting client for 30 s after its connection ends, and accepts its last confirmed takeover token, so a reconnect that gave up mid-handshake can still resume.
---
## 2026-10-30 - rusty_mux: a reconnect that gives up midway can still resume

- **Fixed:** when `rusty-mux --ssh` gave up on a slow reconnect, the server had often already handed the client to that connection. Closing it dropped the client, and the takeover had replaced the client's token with one in a Welcome the client never received, so the next attempt failed with "the server let this client go". The server now keeps a `TAKEOVER` client for 30 s after its connection ends (attachment and lease included), and a takeover accepts the client's current token or the last one it confirmed, by presenting it in a takeover or by sending a request after the Welcome that carried it.
- **Changed:** `rusty-mux` detaches each block before a clean exit, and each block it stops showing (another client's zoom or layout change), so those clients release their leases at once instead of waiting out the grace.
- **Fixed:** `a_split_window_stays_on_screen_while_the_path_is_down` marked the output to search after the reconnect's login was seen, which a fast redraw could beat; it now marks it before.
- **Known limitations:** a client that dies without detaching (killed, or its terminal closed) keeps its lease for up to 30 s; another client can still end it with `Disconnect`. An idle control client lingers in `list` for the grace after a clean exit.
