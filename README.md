# rusty_text

Real (subset, honestly-documented) `sed` and `awk` engines, both built on
[`rusty_regx`](../rusty_regx). Ships two binaries, `rsed` and `rawk`.

## `rsed` — stream editor

```
rsed [-n] [-e 'script']... ['script'] [file...]
```

**Implemented:**
- Addresses: line number (`N`), last line (`$`), regex (`/re/`), and
  ranges (`addr1,addr2`, inclusive).
- Commands: `s/pattern/replacement/[flags]`, `d` (delete), `p` (print),
  `q` (quit), `=` (print line number).
- `s///` flags: `g` (global), `p` (print if changed), `i`/`I`
  (case-insensitive), and a leading digit `N` (start at the Nth
  occurrence; `Ng` replaces the Nth match and every one after it).
- Backreferences in the replacement: `\1`..`\9` (capture groups), `&` and
  `\0` (whole match).
- Any delimiter after `s` (`s#/usr/bin#/opt/bin#`, not just `/`).
- Multiple `-e` scripts, joined with `;`.
- `-n` (suppress the default auto-print of every line).

**Known gaps:** no hold space (`h`/`H`/`g`/`G`/`x`), no `a`/`i`/`c` text
insertion, no branching (`b`/`t`/`:label`), no multi-line pattern-space
commands (`N`/`D`/`P`).

**Regex dialect note:** patterns are `rusty_regx`'s ERE (unescaped `(...)`
for groups, `+`/`?`/`{}` as metacharacters), **not** POSIX BRE — real
GNU `sed`'s *default* dialect, where grouping is `\(...\)`. `rsed`
therefore always behaves like `sed -E`/`sed -r`, not classic
`\(...\)`-style sed. A script written for default-mode GNU sed with
`\(...\)` groups needs its parentheses un-escaped to work here.

## `rawk` — pattern scanning and processing

```
rawk [-F fs] 'BEGIN{...} pattern{action} END{...}' [file...]
```

**Implemented:**
- `BEGIN`/`END` blocks, plain `/regex/` patterns, and general expression
  patterns (`NR==2`, `$1=="foo"`, `$1 > 10 && $1 < 100`).
- Field variables `$0`, `$1`.. `$NF` (`$(expr)` for a computed field
  number), plus assignment to a field (`$1 = "x"`, which rebuilds `$0`
  from the fields joined by `OFS`).
- Built-in variables `NR`, `NF`, `FS`, `OFS` (readable and assignable).
- Arithmetic (`+ - * / %`), relational (`== != < <= > >=`), logical
  (`&& ||`, `!`), string concatenation (juxtaposition: `$1 $2`), and
  `~`/`!~` regex matching.
- `=`, `+=`, `-=`, `*=`, `/=`, `%=` assignment to variables and fields.
- `print` (comma-separated expression list; no args prints `$0`).
- `if`/`else` and `{ }` blocks.
- `-F` field separator (single char splits on it literally; the default
  `" "` splits on runs of whitespace like real awk).

**Known gaps:** no user-defined functions, no arrays, no `for`/`while`
loops, no `getline`, no `printf` (only `print`), and a multi-character
`-F` is treated as a literal substring separator rather than a real ERE
(that's how GNU awk's *default* single/space `FS` behaves; multi-char
`FS`-as-regex is the gap).

## Testing

```
cargo test
```

44 tests across both engines: parser/lexer unit tests, and end-to-end
script tests (addresses, flags, patterns, arithmetic, field assignment,
etc.). Also manually smoke-tested via the built `rsed`/`rawk` binaries.
