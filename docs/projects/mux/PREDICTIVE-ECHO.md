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
- **The check.** When `EchoAck { req }` arrives, the client has already
  applied every Output before it to its copy of the block's grid. For each
  prediction of key `<= req`, the cell it predicted must now hold the
  predicted character, and for the newest such key the cursor must be just
  after it. All pass: they are confirmed and dropped (the real screen now
  shows them). Any fails: the epoch's predictions are all dropped and erased,
  and a new tentative epoch starts.
- **Confidence.** Two failed checks within ten keys turn predictions off for
  the block until the next Enter. Predictions are drawn only while the
  smoothed time from `Input` to its `EchoAck` is above 30 ms, and stop below
  20 ms, so a fast path looks as it does today.

A predicted character is drawn underlined until confirmed, so a guess never
passes for the screen. A prediction is never drawn past the right margin
or over a wide character.

## Password suppression

Three layers. The first alone already keeps a password off the screen; the
other two cover what it does not.

1. **Epochs.** Every way into a password prompt goes through Enter or a
   control key (running `sudo`, answering `ssh`), and that starts a
   tentative epoch, whose predictions are never drawn. A prompt that never
   echoes never confirms the epoch, so nothing typed into it is ever drawn.
2. **`Hidden` mode.** While the latest `EchoMode` is `Hidden`, the client
   makes no predictions at all, confirmed epoch or not. This covers a
   program that turns echo off in the middle of a line.
3. **`Unknown` mode.** Without termios (Windows), predictions stay off,
   since layer 2 is missing (owner decision 2).

What stays possible: a program in `Program` mode that draws its own masked
field (`*` per key) mid-line, without a control key first, after an epoch
on the same line was confirmed. The first masked key fails the check, so at
most one character shows, underlined, for one round trip. The owner accepted
this (decision 3).

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
  epochs never draw, a failed check drops the whole epoch, Backspace,
  confidence, and the margin.
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
3. In the masked-field case above, one underlined character for one round
   trip is accepted. `Program` mode gets no extra rule.
4. The user can turn predictions off. The first version has a switch; the
   build PRs name it.

## Build order, once approved

One PR each: (1) `ECHO_ACK`, `EchoAck` and `EchoMode` in `rusty_mux_proto`
and the server, with the server tests; (2) the client's epoch machine and
composed-mode display; (3) passing-mode display and the end-to-end tests.
