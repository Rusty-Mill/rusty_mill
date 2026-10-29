# Predictive echo: design for review

Status: proposed 2026-10-29, not built. The owner answered its open
questions on 2026-10-29; the design awaits review. Phase 3 step 3 (C13). The owner's
order (DECISIONS 2026-10-28) is to design causality, epochs and password
suppression and have them reviewed before any code. Nothing here changes a
wire format yet. [SPIKE-4-ROAMING.md](SPIKE-4-ROAMING.md) states the open
problem this answers.

## Goal and limits

Over a slow path (`rusty-mux --ssh`), a key typed at a shell prompt should
show at once, then be confirmed or taken back when the server's screen
arrives. It is a display aid only: a prediction never reaches the block, the
server or other clients, and a wrong one is erased.

Out of scope: predicting anything but typing on the current line (no
newlines, wraps, scrolling, completion or editor commands), and local
sockets, where the round trip is too short to matter.

## What the server can know

Three instants per keystroke, for input request `r` on one block
connection:

- `W(r)`: the server's write of `r` to the PTY master returned.
- `E(r)`: the program wrote the echo of `r`, if it ever does.
- `D`: a drain of the PTY master returned `EAGAIN`: everything the program
  had written by then has been read.

The server knows `W` and `D` exactly and never knows `E`. It can say one
thing that is exact, and one that is a guess:

1. **Order (exact).** Output frames on a block connection are in read order.
   A message sent after a drain that started after `W(r)` follows every
   byte that reached the master before that drain, including any echo of
   `r` by then.
2. **When the echo comes (a guess).** Nothing ties `E(r)` to `W(r)`.
   - When the program echoes for itself (readline, zsh, editors: `ICANON`
     and `ECHO` off), it echoes once it is scheduled and has read the key,
     which can take any time.
   - When the line discipline echoes (`ICANON` and `ECHO` on: `cat`, a plain
     `read`), the echo does not wait for the program, but it is not
     synchronous either. Linux hands the write to a kernel worker, so a
     read right after the write returns usually finds nothing. A probe on
     this container (Python, `os.openpty`, 2000 keys, 2026-10-29) missed the
     echo on an immediate non-blocking read 1997 times in 2000; waiting for
     it took 0.04 ms at the median, 0.09 ms at p99 and 2.9 ms at worst.

   So the server waits a grace period `G` after `W(r)` in every mode (mosh
   uses 50 ms; we start there), and treats an echo not seen by then as
   absent.

So the ack below means: *every byte that reached the PTY master by at least
`G` after your key is before this message*. It never means *this output is
your echo*. Only the client's screen comparison decides that, and it can
still be wrong (see Residual errors).

## Protocol (when built)

One feature bit, `ECHO_ACK`, asked for on a block's connection. With it:

- **`EchoAck { block, req }`**, server to client, on the block's connection.
  It is sent once the server has drained the PTY master after
  `W(req) + G`, and follows the Output frames of that drain. `req` is the newest of this connection's
  `Input` requests that this holds for; acks only increase. The server
  sends it from a timer too, so a key with no echo at all (a password)
  still gets its ack and the client can stop waiting.
- **`EchoMode { block, mode }`**, server to client, on the block's
  connection, whenever the mode changes and once after attach:
  - `Kernel`: `ICANON` and `ECHO` on.
  - `Hidden`: `ICANON` on, `ECHO` off. This is `getpass`, `sudo`, `ssh`,
    `read -s`.
  - `Program`: `ICANON` off and `ECHO` off. The program draws its own echo.
  - `Unknown`: no termios to read (Windows ConPTY), or the read failed.

  The server reads the slave's termios through the master (`tcgetattr`
  on the master fd; the same probe showed the master reporting the
  slave's `ECHO` off and `ICANON` on after `stty -echo`) after each drain and
  before each `Input` write, and sends `EchoMode` ahead of that drain's
  Output frames when it changed. So a program that turns echo off and then
  prints its prompt has its mode change in the stream before the prompt.

Nothing else changes: `Input` already carries a `req`, and only the client
holding the block's lease can send it, so this client's keys are the only
keys in flight on that block. The server's own replies to terminal queries
go to the program, not to the screen.

## The client: epochs

Predictions are grouped in **epochs**. An epoch is a run of keys whose
effect the client can model: printable characters (including UTF-8) typed
at the cursor, and Backspace over a character predicted in the same epoch.

- **A new epoch starts** on any other key (Enter, Tab, Ctrl-anything,
  Escape and every escape sequence, arrows, paste), and on anything that
  moves the client's model away from the server's: a resize, focus moving
  to another block, the lease lost, a reconnect or takeover (the screen is
  sent again), the alternate screen entered or left, an `EchoMode` change,
  output that moves the cursor off the row being predicted, and a
  prediction that fails its check.
- **An epoch starts tentative.** Its predictions are kept but **not drawn**.
  It becomes confirmed when the first of its predictions passes the check
  below. Only then are its predictions, and later keys of the same epoch,
  drawn ahead of the server.
- **Only the blank tail.** A key is predicted only when the predicted cursor
  cell and every cell to its right on the row are blank on the client's
  copy of the screen, as when typing at the end of a command line. Typing
  in the middle of a line may insert or overwrite depending on the program,
  so a key whose cell or any cell right of it is not blank is not
  predicted, and starts a new tentative epoch.
- **The overlay.** An epoch's predictions are kept as the edit they make to
  that blank tail, not as a list of keys: for each cell, the character the
  epoch expects there and the key that wrote it. A printable key writes its
  character at the predicted cursor and moves it right.
- **Backspace** is predicted only over a character this epoch predicted and
  that is not yet settled (see the check): it removes that cell from the
  overlay, which leaves it blank and unchecked, and moves the predicted
  cursor left. Backspace over anything else (a confirmed or real character,
  or the start of the epoch) is not predicted and starts a new tentative
  epoch, since what erasing does there is the program's business.
- **The check.** An ack is a watermark, not a snapshot of the screen at one
  key: by the time it arrives, the screen may already show the echoes of
  later keys. So when `EchoAck { req }` arrives (after every Output before
  it is applied to the client's copy of the grid), the client checks only
  the **settled** cells, those in the overlay whose key is `<= req`. Each
  must hold the character the overlay expects. Cells written by a later key
  are left for a later ack, whatever they show now. The cursor is not
  compared with a position (later echoes may have moved it on), only
  required to still be on the predicted row, as the epoch rule above
  already demands.
  - Any settled cell fails: the whole epoch is dropped and erased, and a
    new tentative epoch starts.
  - All pass and there was at least one: they leave the overlay, since the
    screen now shows them, and the epoch is confirmed.
  - No settled cells (an ack for keys whose predictions a Backspace already
    removed, or for no predicted key at all): nothing is learned, and the
    epoch stays as it was. A tentative epoch is confirmed only by a
    character actually found on the screen.
- **Confidence.** Two failed checks within ten keys turn predictions off for
  the block until the next Enter.
- **Latency gate.** Predictions are drawn only on a slow path. The time from
  `Input` to `EchoAck` cannot measure that, since it always includes `G`.
  The client measures the path itself: it sends `Sync` on the block's
  connection with the first key after at least a second without a sample,
  and times the answer (`Done`), which the server sends at once. It keeps a
  smoothed round trip (each sample weighted 1/8, as TCP's SRTT), starts
  drawing above 30 ms and stops below 20 ms. A fresh connection starts with
  drawing off until the first sample.

A predicted character is drawn underlined until confirmed, so a guess never
passes for the screen. A prediction is never drawn past the right margin
or over a wide character.

## Password suppression

Three layers, each with what it guarantees and what it does not.

1. **Epochs.** A prompt reached after an epoch break (Enter to run `sudo`,
   Enter after a username) starts in a tentative epoch, whose predictions
   are never drawn. A prompt that never echoes never confirms it, so
   nothing typed there is drawn. This is exact for prompts reached that
   way, which are the common ones. It does not cover a prompt that appears
   with no epoch break in between, for example one a program opens by
   itself while the user is typing on a confirmed line.
2. **`Hidden` mode.** While the latest `EchoMode` the client has received is
   `Hidden`, it makes no predictions and erases any drawn ones. The
   notification travels like any message, so keys typed while it is in
   flight (up to one path round trip) may already have been drawn; this
   layer stops further ones, it cannot take back the first.
3. **`Unknown` mode.** Without termios (Windows), predictions stay off,
   since layer 2 is missing (owner decision 2).

**What stays possible**, then, is one case: on a line whose epoch is
already confirmed, the program, with no epoch-breaking key in between,
starts reading a secret, either drawing its own masked field (`*` per key)
or turning echo off. The client only knows the last mode it received, so
this includes a program that turns echo off while the client still shows
`Kernel` or `Program`: no mode is exempt. Until the first rejecting ack or
the `Hidden` notification arrives, keys typed are drawn, underlined.

How far to bound this is back with the owner (decision 3, reopened): the
earlier answer accepted "one character", which the design did not enforce.
The options, with what each guarantees:

- **A. Lead cap of one, in every mode.** At most one drawn prediction is
  unsettled at any time; the next key's prediction waits, undrawn, until
  the previous one is confirmed. So while a secret is being read, at most
  one typed character is drawn before the first rejecting ack or the
  `Hidden` notification stops the epoch. One exception: a typed character
  equal to the mask character (typing `*` into a field that draws `*`)
  passes its check falsely, and the next key may then be drawn too; each
  such coincidence lets one more character show. The cost: typing faster
  than one key per round trip shows each key late, so on the slow links
  that predictions are for, most keys gain little.
- **B. No cap.** Every key typed within one path round trip plus `G` of the
  secret starting may be drawn, about two at 200 ms and 8 keys a second;
  more on a slower path or with faster typing. Predictions keep their full
  use while typing fast. This is what mosh does.

## Display

- **Composed** (a split window): predictions are one more layer in
  `screen::compose`, drawn over the block's cells. Erasing one is a normal
  redraw.
- **Passing** (one block, raw output): the client draws a prediction with
  an absolute cursor move to its cell, the underline and the character,
  then moves the cursor back and restores the rendition from its copy of
  the grid. It never uses `DECSC`/`DECRC`, whose single slot the program
  may be using. Erasing redraws the touched cells from the grid the same
  way. An echo overwrites its own prediction, since it lands on the same
  cell. If the implementation finds that `rusty_term_core` does not expose
  the current rendition, passing mode shows no predictions in the first
  version.

## Residual errors

The check compares cells, so it can be fooled:

- **False confirmation.** The predicted cell already held the predicted
  character (typing `l` over an `l` left on screen). The prediction is
  dropped as confirmed and the screen shows what the server sent, so the
  user sees the truth once the ack arrives. Nothing is drawn wrongly after
  that.
- **False rejection.** A program slower than `G` echoes after the ack. The
  epoch is dropped and the real echo then draws normally. The cost is a
  flicker and a tentative epoch, not wrong output.

In both cases the screen converges to the server's within one ack, which is
the property the design keeps: a prediction lives at most one round trip
plus `G` past its key.

## Tests (when built)

- **Server, real PTY.** Kernel mode: an ack after `cat` echoes follows the
  echo. Program mode: an ack for a key the program echoes
  after 10 ms follows the echo, and one for a key echoed after 200 ms
  precedes it. `stty -echo` sends `Hidden` before the next output, and
  `read -s` gives `Hidden`. An ack arrives for a key with no echo.
- **Client, pure.** The epoch machine is plain data in, decisions out, with
  no I/O, and is tested on its own: which keys start epochs, tentative
  epochs never draw, a failed check drops the whole epoch, confidence, and
  the margin. Deterministic cases for the check:
  - coalesced typing: `a` (r1) and `b` (r2) both echoed before `EchoAck
    { r1 }`: the ack passes, `b`'s cell is left for `EchoAck { r2 }`, which
    also passes;
  - coalesced Backspace: `a` (r1) then Backspace (r2) before `EchoAck
    { r1 }`. The Backspace removed `a`'s cell from the overlay, so the ack
    settles nothing, and a tentative epoch stays tentative: nothing typed
    after it is drawn until a character is found on the screen;
  - Backspace over a character already confirmed, or over text that was
    there before the epoch: not predicted, and a new tentative epoch;
  - typing with a non-blank cell at or right of the cursor (in the middle
    of a line): not predicted, and a new tentative epoch;
  - a partial watermark: three keys, an ack settling two, the third checked
    by the next ack;
  - a masked field: several keys typed on a confirmed line before the first
    ack, and the program drawing `*` for each. With the chosen option, the
    drawn characters before the rejecting ack are at most one (A) or the
    keys typed so far (B), and none after it; with A, a typed `*` among
    them lets exactly one more show;
  - a `Hidden` change in flight while keys are typed, with the client still
    in `Kernel` mode and in `Program` mode: the same bound as the masked
    field, no prediction drawn after it arrives, and drawn ones erased.
- **Latency gate.** With a stand-in path of about 2 ms and `G` at 50 ms,
  predictions stay off; raising the path delay to 100 ms turns them on
  after the first samples, and lowering it to 10 ms turns them off again.
- **End to end**, with the stand-in ssh delaying every message by 200 ms:
  typing at a shell shows the characters before the echo, underlined, then
  plain. After `read -s` the typed characters never appear in anything
  the client writes to its terminal. Resize, reconnect and focus moves
  clear predictions.

## Decided by the owner

Nano agreed with all four proposed answers on #1245 (2026-10-29 20:51Z):

1. Grace `G` is a fixed 50 ms in the first version. An adaptive grace can
   follow if measurements call for it.
2. `Unknown` mode (Windows): predictions are off.
3. *(Reopened after review.)* The answer accepted one underlined character
   for one round trip in the masked-field case, but the design did not
   enforce that bound: it allowed every key typed within a round trip. See
   options A and B under Password suppression.
4. The user can turn predictions off. The first version has a switch; the
   build PRs name it.

## Build order, once approved

One PR each: (1) `ECHO_ACK`, `EchoAck` and `EchoMode` in `rusty_mux_proto`
and the server, with the server tests; (2) the client's epoch machine and
composed-mode display; (3) passing-mode display and the end-to-end tests.
