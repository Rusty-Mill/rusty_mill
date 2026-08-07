# 005 — The transposition a type cannot catch

Encountered migrating `sys::console` and `sys::handle` to Track W
(platform-windows, D-15's third slice). The mechanical part went the way
slices 1 and 2 did. Two things in it are worth keeping.

## `(u16, u16)` is not a type, it is a coin flip

`rusty_win32::console::window_size` returns the console viewport as
`(cols, rows)`. `sys::console::window_size` returns `(rows, cols)`,
because `platform::term` is a portable surface and its contract follows
`winsize`'s `ws_row`/`ws_col` ordering. Both crates compute the identical
`srWindow` extents from the identical struct; they disagree only about
which one comes first.

Nothing catches this. Both are `(u16, u16)`; both compile; both run.
Every existing test still passes on a square-ish terminal, and the
failure it would otherwise produce — a transposed terminal size — is
silent, cosmetic, and shows up in someone's wrapped output weeks later.
It was caught here only by reading the donor's doc comment closely
enough to notice it said "columns, rows" where the call site expected
the opposite.

The general shape, which the remaining slices should assume rather than
hope about: **when a donor call returns a tuple of same-typed values,
the ordering is a contract that lives only in prose, so read the prose.**
Track P had nothing like this — `read`/`write` return one `usize`, and
a syscall's register conventions leave no room for an author's taste.
Two hand-written crates, each internally consistent, are exactly where
this class of mismatch lives.

Marked with a swap at the boundary and a comment saying why, rather than
"fixed" in either crate: neither ordering is wrong, and changing one to
match the other would just move the trap to that crate's other callers.

## "Very probably inert" is not a reason to migrate

One call in `sys::console` did not migrate: `reopen`, the
`CreateFileW("CONIN$"/"CONOUT$", …)` that backs the `has_console` probe
and the post-acquisition std-handle repoint.

`rusty_win32::fs::open_file` matches it on everything that obviously
matters — `OPEN_EXISTING`, `FILE_SHARE_READ | FILE_SHARE_WRITE`, null
security attributes — and differs in one argument:
`FILE_ATTRIBUTE_NORMAL` where this crate passes `0`. On an
`OPEN_EXISTING` open of a console pseudo-device that difference is very
probably inert, since `dwFlagsAndAttributes`' attribute half applies to
*creation*.

"Very probably" was the whole problem. This work happens on a Linux host
against a cross-compiler; the claim could not be tested where it was
made, only asserted from documentation and then shipped to a Windows CI
run. And the site is a *probe* — `has_console`'s previous incarnation
used `GetConsoleWindow` and shipped a false negative that surfaced only
as a confusing `ERROR_ACCESS_DENIED` two steps downstream, as that
function's own comment records at length.

So the trade on offer was: a verified-correct call, swapped for an
unverified-equivalent one, to gain binding provenance at exactly one
site. Declined, and written down at the call site so the next person
sees a decision rather than an oversight.

This is a different kind of "didn't migrate" from note 004's
`CreateProcessW`, and the difference is worth keeping straight.
`CreateProcessW` is closed permanently — the donor's shape encodes an
architectural decision this repo made differently, and no rev bump
changes that. `reopen` is merely *declined for now*, on a risk/benefit
basis, and would be reconsidered the moment the donor grows an explicit
console-device open. A migration list needs both verbs; collapsing them
into "not done" loses the information.
