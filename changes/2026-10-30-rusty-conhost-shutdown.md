---
category: Added
changelog: rusty_conhost closes a console the way Windows does: hang-up or child exit sends CTRL_CLOSE to every client, and the server closes on the last disconnect or after a fixed 2 s grace.
---
## 2026-10-30 - rusty_conhost: CTRL_CLOSE and the grace deadline

- **Added:** host PR 8 of the console-host plan
  (`docs/projects/term/conhost/design.md` §5.8, §7.3). `hang_up`, the
  last `OwnPtyMaster` dropping, or the direct child exiting now sends
  CTRL_CLOSE to every attached client through the ctrl worker, instead of
  closing the server at once. The console keeps serving clients while
  their handlers run, and closes when the last one disconnects or at an
  absolute 2 s deadline, whichever comes first. A client that connects
  during the grace gets CTRL_CLOSE too. A console with no client closes
  at once.
- **Added:** dropping the master detaches it: queued and later output is
  discarded, so a handler's writes never stall on a reader that is gone.
  `write` fails with `BrokenPipe` from the first `hang_up` on.
- **Changed:** a CTRL_CLOSE supersedes Ctrl+C and Ctrl+Break still waiting
  in the ctrl queue, so every client's CTRL_CLOSE fits, and the ctrl
  worker still delivers queued CTRL_CLOSE after the server has closed.
- **Fixed:** the direct child exiting no longer closes the server under
  clients still attached, which cut off a grandchild's last output
  (spike S1, design §9.1).
- **Known limitations:** the host kills no client. Windows ends a client
  that got CTRL_CLOSE within about 5 s; one that ignores the closed
  console's errors is left to the caller (the mux's own kill, rusty_term's
  Drop).
