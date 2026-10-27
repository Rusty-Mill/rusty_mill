#!/bin/sh
# rt-caps.sh: a guided tour of rusty_term's capabilities.
#
# Run it inside rusty_term (TUI or --gui window). Each section prints what you
# should see; sections that can check themselves (probe, widths, graphemes)
# print PASS/FAIL. Plain POSIX sh plus stty, dd, awk, sed. No dependencies.
#
#   sh rt-caps.sh                  # every non-interactive section
#   sh rt-caps.sh widths sync      # just those sections
#   sh rt-caps.sh list             # section names
#
# Env: RT_NOPAUSE=1 skips "press enter" stops; RT_MB=<n> sets throughput size.
set -u

TTY=/dev/tty
ESC=$(printf '\033')
BEL=$(printf '\007')
SAVED=''
ALT=0
TMPF=''
PASS=0
FAIL=0

ALL='probe colors sgr text widths graphemes cursor screen sync osc images'
EXTRA='input throughput'

# ---------------------------------------------------------------- helpers

usage() {
    cat <<'EOF'
usage: sh rt-caps.sh [section...]     (no section = all)

  probe       terminal identity and mode queries (DA, XTVERSION, DECRQM, ...)
  colors      16 / 256 / truecolor palettes, colon and semicolon SGR forms
  sgr         bold, dim, italic, underline styles + colors, strike, overline...
  text        box drawing, blocks, braille, powerline, bidi, DECDHL/DECDWL, ligatures
  widths      per-code-point cell widths, self-checked with cursor reports
  graphemes   mode 2027 grapheme clustering (ZWJ, flags, VS16, skin tones)
  cursor      CUP/CUF/CUD moves, DECSC/DECRC, DECSCUSR shapes, hide/show, tabs
  screen      alt screen, scroll regions, left/right margins, ICH/DCH/ECH/REP/IL/DL, DECALN
  sync        synchronized output (mode 2026), including its 800 ms timeout
  osc         title, OSC 7 / 8 / 9 / 777 / 52 / 4 / 10-12 / 133 / 7501
  images      Sixel, kitty graphics and iTerm2 inline images (window mode)
  input       interactive: mouse, bracketed paste, focus, kitty keyboard
  throughput  bulk output MB/s for ascii, SGR-heavy and unicode text (RT_MB, default 32)

Launch rusty_term from the repo root, then run this script in its shell:
  cargo run --release -p rusty_term
  cargo run --release -p rusty_term --features gui -- --gui
  cargo run --release -p rusty_term --features gui-gpu -- --gui --gpu
Run it from the shell inside rusty_term: when rusty_term runs the script
itself (-- sh rt-caps.sh), it exits with the child and can drop the last frame.
Windows: run it from WSL (rusty_term -- wsl sh rt-caps.sh) or Git Bash.
EOF
}

hdr() { printf '\n\033[1;7m %s \033[0m \033[1m%s\033[0m\n' "$1" "$2"; }
see() { printf '\033[36m  see: %s\033[0m\n' "$*"; }
note() { printf '\033[2m  note: %s\033[0m\n' "$*"; }
ok() { PASS=$((PASS + 1)); printf '\033[32mPASS\033[0m'; }
bad() { FAIL=$((FAIL + 1)); printf '\033[31mFAIL\033[0m %s' "$1"; }
nap() { sleep "$1" 2>/dev/null || sleep 1; }
lines() { i=0; while [ $i -lt "$1" ]; do printf '\n'; i=$((i + 1)); done; }

pause() {
    [ "${RT_NOPAUSE:-0}" = 1 ] && { nap 1; return; }
    printf '\033[2m  [enter] %s\033[0m' "${1:-}"
    read -r _ <"$TTY"
}

raw_on() { SAVED=$(stty -g <"$TTY"); stty -echo -icanon min 0 time "${1:-3}" <"$TTY"; }
raw_off() { [ -n "$SAVED" ] && stty "$SAVED" <"$TTY"; SAVED=''; }

# Read whatever the terminal sends until it has been quiet for one read timeout.
drain() {
    r=''
    while c=$(dd bs=256 count=1 <"$TTY" 2>/dev/null) && [ -n "$c" ]; do r=$r$c; done
    printf '%s' "$r"
}

# Send a query (printf format) and print the raw reply.
ask() { raw_on 3; printf "$1" >"$TTY"; drain; raw_off; }

# Make control bytes visible.
vis() { sed "s/$ESC/^[/g; s/$BEL/^G/g"; }

# Current cursor column from a CPR (CSI 6n) reply; empty when there is none.
col() { ask '\033[6n' | sed -n 's/.*\[[0-9]*;\([0-9]*\)R.*/\1/p'; }

# DECRQM status of a DEC private mode: 1 set, 2 reset, 0 unknown, '' no reply.
decrqm() { ask "\\033[?$1\$p" | sed -n "s/.*\\[?$1;\\([0-9]\\)\\\$y.*/\\1/p"; }

now_ms() {
    t=$(date +%s%N 2>/dev/null)
    case $t in *N | '') echo $(($(date +%s) * 1000)) ;; *) echo $((t / 1000000)) ;; esac
}

size() { stty size <"$TTY" 2>/dev/null || echo '24 80'; }

need_size() {
    set -- $(size)
    [ "$1" -lt 24 ] || [ "$2" -lt 80 ] && note "window is ${2}x$1; this section is laid out for at least 80x24"
    return 0
}

enter_alt() { ALT=1; printf '\033[?1049h\033[2J\033[H'; }
leave_alt() { printf '\033[r\033[?1049l'; ALT=0; }
at() { printf '\033[%d;%dH' "$1" "$2"; }

cleanup() {
    raw_off
    [ "$ALT" = 1 ] && leave_alt
    printf '\033[0m\033[?25h\033[r\033[?69l\033[0 q'
    printf '\033[?1000l\033[?1002l\033[?1006l\033[?1004l\033[?2004l'
    [ -n "$TMPF" ] && rm -f "$TMPF"
}

# ---------------------------------------------------------------- sections

sec_probe() {
    hdr probe "identity and mode queries"
    note "TERM=${TERM:-unset} COLORTERM=${COLORTERM:-unset}"
    q() { printf '  %-30s %s\n' "$1" "$(ask "$2" | vis)"; }
    q 'DA1        CSI c' '\033[c'
    q 'DA2        CSI > c' '\033[>c'
    q 'XTVERSION  CSI > q' '\033[>q'
    q 'CPR        CSI 6 n' '\033[6n'
    q 'size chars CSI 18 t' '\033[18t'
    q 'size px    CSI 14 t' '\033[14t'
    q 'fg color   OSC 10 ?' '\033]10;?\033\\'
    q 'bg color   OSC 11 ?' '\033]11;?\033\\'
    q 'scheme     CSI ? 996 n' '\033[?996n'
    q 'kitty kbd  CSI ? u' '\033[?u'
    q 'XTGETTCAP  TN' '\033P+q544E\033\\'
    q 'sixel regs CSI ? 1;1;0 S' '\033[?1;1;0S'
    see "XTVERSION reads rusty_term(<version>); every row has a reply"
    printf '  %-30s ' 'DECRQM modes (1 set, 2 reset)'
    for m in 7 25 1049 2004 2026 2027 1006; do printf '%s=%s ' "$m" "$(decrqm $m)"; done
    printf '\n  %-30s ' 'running under rusty_term?'
    case $(ask '\033[>q') in *rusty_term*) ok ;; *) bad 'XTVERSION did not name rusty_term' ;; esac
    printf '\n'
}

sec_colors() {
    hdr colors "16 / 256 / truecolor"
    see "rows: fg normal, fg bright, bg normal, bg bright (8 swatches each)"
    for base in 30 90 40 100; do
        i=0
        while [ $i -lt 8 ]; do printf '\033[%dm %3d \033[0m' $((base + i)) $((base + i)); i=$((i + 1)); done
        printf '\n'
    done
    see "6x6x6 color cube in six blocks, then a 24-step gray ramp"
    awk 'BEGIN {
        for (g = 0; g < 6; g++) {
            printf "  "
            for (r = 0; r < 6; r++) {
                for (b = 0; b < 6; b++) printf "\033[48;5;%dm  ", 16 + r * 36 + g * 6 + b
                printf "\033[0m "
            }
            printf "\n"
        }
        printf "  "; for (i = 232; i < 256; i++) printf "\033[48;5;%dm  ", i; printf "\033[0m\n"
    }'
    see "smooth rainbow and a smooth gray ramp (no banding into 256 steps)"
    awk 'BEGIN {
        n = 72; printf "  "
        for (i = 0; i < n; i++) {
            h = i / n * 6; s = int(h); f = h - s; q = int(255 * (1 - f)); t = int(255 * f)
            if (s == 0) { r = 255; g = t; b = 0 } else if (s == 1) { r = q; g = 255; b = 0 }
            else if (s == 2) { r = 0; g = 255; b = t } else if (s == 3) { r = 0; g = q; b = 255 }
            else if (s == 4) { r = t; g = 0; b = 255 } else { r = 255; g = 0; b = q }
            printf "\033[48;2;%d;%d;%dm ", r, g, b
        }
        printf "\033[0m\n  "
        for (i = 0; i < n; i++) { v = int(i * 255 / (n - 1)); printf "\033[48;2;%d;%d;%dm ", v, v, v }
        printf "\033[0m\n"
    }'
    see "both lines orange: colon form and semicolon form agree"
    printf '  \033[38:2::255:140:0mcolon 38:2::255:140:0\033[0m   \033[38;2;255;140;0msemicolon 38;2;255;140;0\033[0m   \033[38:5:208m38:5:208\033[0m\n'
    printf '  \033[39;49mdefault fg/bg (39/49)\033[0m   \033[7mreverse of defaults\033[0m\n'
}

sec_sgr() {
    hdr sgr "text attributes"
    see "each sample shows the attribute its label names"
    s='The quick brown fox 0123'
    for pair in '1|bold' '2|dim' '3|italic' '1;3|bold italic' '4|underline' \
        '4:2|double underline' '4:3|curly underline' '4:4|dotted underline' \
        '4:5|dashed underline' '21|double underline (SGR 21)' \
        '4:3;58:2::255:0:0|red curly underline' '4;58;5;33|blue underline (58;5)' \
        '5|blink' '7|reverse' '9|strikethrough' '53|overline' \
        '1;4;9;53;38;5;214|stacked'; do
        code=${pair%%|*}
        printf '  %-28s \033[%sm%s\033[0m\n' "${pair#*|}" "$code" "$s"
    done
    printf '  %-28s [\033[8msecret\033[28m]  (blank between brackets)\n' 'hidden (8)'
    printf '  %-28s \033[1;3;4;9;53mXXXX\033[22;23;24;29;55m plain after 22/23/24/29/55\033[0m\n' 'resets'
    printf '  %-28s \033[1;2mbold+dim\033[22m plain after 22\033[0m\n' 'bold and dim share 22'
}

sec_text() {
    hdr text "glyph coverage, bidi, line sizes, ligatures"
    see "box drawing joins into a clean grid with no gaps"
    printf '  ┌──┬──┐ ╔══╦══╗ ╭──╮ ┏━━┳━━┓\n  │  │  │ ║  ║  ║ │  │ ┃  ┃  ┃\n  ├──┼──┤ ╠══╬══╣ ╰──╯ ┣━━╋━━┫\n  └──┴──┘ ╚══╩══╝      ┗━━┻━━┛\n'
    see "blocks fill whole cells; shades step light to dark; braille rises"
    printf '  ░▒▓█ ▀▄▌▐ ▁▂▃▄▅▆▇█  ⠁⠃⠇⡇⣇⣧⣷⣿\n'
    see "powerline arrows (need a patched font; boxes otherwise)"
    printf '  \033[44m main \033[34;42m\033[30m src \033[32;49m\033[0m  \n'
    see "Hebrew and Arabic read right to left; Arabic letters join (window mode)"
    printf '  abc שלום עולם def   مرحبا بالعالم\n'
    see "ligatures in window mode (on by default): -> => != === <= >= ::"
    printf '  -> => != === <= >= :: <!-- --> |> www\n'
    see "next three lines: double-height (two halves), then double-width"
    printf '\033#3  Double height\n\033#4  Double height\n\033#6  Double width\n\033#5'
    see "tabs land on every 8th column under the ruler"
    printf '|-------|-------|-------|-------|\n\ta\tb\tc\td\n'
}

# Print a label, the sample at column 16, then its measured width.
measure() {
    printf '  %-12s\033[16G' "$1"
    printf "$2"
    c=$(col)
    printf '|\033[32G'
    if [ -z "$c" ]; then printf 'no cursor report  '; bad 'CPR'; printf '\n'; return; fi
    w=$((c - 16))
    printf '%2d cells  ' "$w"
    if [ -n "${3:-}" ]; then [ "$w" = "$3" ] && ok || bad "want $3"; fi
    printf '\n'
}

sec_widths() {
    hdr widths "cell widths, measured with cursor position reports"
    see "PASS on every row; the | marks line up if the display agrees with the grid"
    was=$(decrqm 2027)
    printf '\033[?2027l'
    measure ascii 'hello, world' 12
    measure cjk '漢字かなカナ' 12
    measure emoji '😀🚀🎉👍🔥🌈' 12
    measure combining 'cafe\314\201 nai\314\210ve!!' 12
    measure jamo '\341\204\222\341\205\241\341\206\253' 2
    measure ambiguous '±§°' 3
    measure fullwidth 'ＡＢＣ' 6
    measure halfwidth 'ｱｲｳ' 3
    measure zero-width 'a\342\200\213b\342\200\215c' 3
    [ "$was" = 1 ] && printf '\033[?2027h'
    printf '  %d PASS, %d FAIL so far\n' "$PASS" "$FAIL"
}

sec_graphemes() {
    hdr graphemes "mode 2027 grapheme clustering"
    was=$(decrqm 2027)
    printf '  DECRQM 2027 reply: %s  ' "${was:-none}"
    case $was in 1 | 2) ok ;; *) bad 'mode 2027 not recognized' ;; esac
    printf '\n'
    see "OFF: widths per code point (what most apps assume); shown, not judged"
    printf '\033[?2027l'
    set -- family '\360\237\221\250\342\200\215\360\237\221\251\342\200\215\360\237\221\247' \
        flag '\360\237\207\257\360\237\207\265' \
        heart-vs16 '\342\235\244\357\270\217' \
        skin-tone '\360\237\221\215\360\237\217\275' \
        keycap '1\357\270\217\342\203\243' \
        devanagari '\340\244\250\340\244\277'
    for mode in off on; do
        [ $mode = on ] && { see "ON: one grapheme per cluster, emoji clusters two cells wide"; printf '\033[?2027h'; }
        i=1
        while [ $i -lt $# ]; do
            eval "name=\${$i} sample=\${$((i + 1))}"
            want=''
            if [ $mode = on ]; then
                case $name in devanagari) want=1 ;; *) want=2 ;; esac
            fi
            measure "$name" "$sample" "$want"
            i=$((i + 2))
        done
    done
    [ "$was" = 1 ] || printf '\033[?2027l'
    note "server and client must agree on 2027; a mux pane inherits the program's choice"
}

sec_cursor() {
    hdr cursor "movement, save/restore, shapes, visibility"
    see "a closed box with a centered label and an X in its middle-right, drawn with relative moves"
    lines 7
    printf '\033[7A\0337'
    rel() { printf '\0338'; [ "$1" -gt 0 ] && printf '\033[%dB' "$1"; [ "$2" -gt 0 ] && printf '\033[%dC' "$2"; return 0; }
    rel 0 2; printf '+------------------+'
    r=1
    while [ $r -le 3 ]; do rel $r 2; printf '|\033[18C|'; r=$((r + 1)); done
    rel 4 2; printf '+------------------+'
    rel 2 5; printf 'CUF/CUD ok'
    rel 2 21; printf '\033[3DX'
    printf '\0338\033[6B\r'
    see "the cursor cycles six shapes (block, underline, bar; blinking then steady)"
    for p in '1|blinking block' '2|steady block' '3|blinking underline' \
        '4|steady underline' '5|blinking bar' '6|steady bar'; do
        printf '\r\033[K  DECSCUSR %s: %s ' "${p%%|*}" "${p#*|}"
        printf '\033[%s q' "${p%%|*}"
        nap 1.2
    done
    printf '\033[0 q\r\033[K  cursor hidden for 2 s ->'
    printf '\033[?25l'; nap 2; printf '\033[?25h back\n'
    see "CHA and CUB: the digits below read 1-9 left to right"
    printf '  \033[10G9\033[2G1\033[4G3\033[3G2\033[6G5\033[5G4\033[8G7\033[7G6\033[9G8\n'
}

sec_screen() {
    hdr screen "alt screen, scroll regions, margins, editing"
    need_size
    printf '  MAIN SCREEN MARKER: this line must still be here after the tour\n'
    see "the tour opens on the alternate screen; enter steps through it"
    pause 'to enter the alt screen'
    enter_alt
    at 1 1; printf '\033[7m header: fixed, row 1 \033[0m   scroll region rows 3-8, margins rows 12-17'
    at 2 1; printf '%s' '------ region top ------'
    at 9 1; printf '%s' '------ region end ------'
    at 10 1; printf '\033[7m footer: fixed, row 10 \033[0m'
    printf '\033[3;8r'; at 8 1
    i=1
    while [ $i -le 20 ]; do printf '\nscrolled line %02d' $i; i=$((i + 1)); done
    printf '\033[2T\033[r'
    at 4 30; printf 'see: rows 3-4 blank, then 15-18'

    r=12
    while [ $r -le 17 ]; do at $r 9; printf '|'; at $r 41; printf '|'; r=$((r + 1)); done
    printf '\033[12;17r\033[?69h\033[10;40s'; at 12 10
    printf 'Left/right margins: this text wraps inside the bars at columns 9 and 41 and scrolls within rows 12-17 without disturbing anything around it. The quick brown fox jumps over the lazy dog, again and again, until the box has scrolled a few times and the first words are gone.'
    printf '\033[?69l\033[r'
    at 15 44; printf 'see: text stays inside the bars'

    line='0123456789ABCDEFGHIJ'
    at 19 1; printf '%s\033[6G\033[3@' "$line"; at 19 30; printf 'ICH 3: 01234   56789ABCDEFGHIJ'
    at 20 1; printf '%s\033[6G\033[3P' "$line"; at 20 30; printf 'DCH 3: 0123489ABCDEFGHIJ'
    at 21 1; printf '%s\033[6G\033[3X' "$line"; at 21 30; printf 'ECH 3: 01234   89ABCDEFGHIJ'
    at 22 1; printf 'x\033[9b'; at 22 30; printf 'REP 9: xxxxxxxxxx'
    at 23 1; printf '%s\033[11G\033[1K' "$line"; at 23 30; printf 'EL 1:             BCDEFGHIJ'
    at 24 1; pause 'next: IL/DL'

    printf '\033[2J'
    r=1
    while [ $r -le 8 ]; do at $r 1; printf 'line %d' $r; r=$((r + 1)); done
    at 3 1; printf '\033[2L'; at 3 1; printf 'inserted A'; at 4 1; printf 'inserted B'
    at 8 1; printf '\033[1M'
    at 1 30; printf 'see: line 1, line 2, inserted A,'
    at 2 30; printf '     inserted B, line 3, line 4,'
    at 3 30; printf '     line 5, then line 7 and line 8'
    at 12 1; pause 'next: DECALN'

    printf '\033#8'
    at 12 20; printf '\033[7m DECALN: the whole screen fills with E \033[0m'
    at 24 1; pause 'to leave the alt screen'
    leave_alt
    see "the marker line above is intact and the prompt continues below it"
}

sec_sync() {
    hdr sync "synchronized output (mode 2026)"
    printf '  DECRQM 2026 reply: %s\n' "$(decrqm 2026)"
    see "1: no sync, the top half lands first. 2: sync, the frame appears whole."
    see "3: sync held 2 s, it paints anyway after about 0.8 s (the safety timeout)"
    pause 'to start'
    enter_alt
    # frame <bg color> <sync 0|1> <half-way hold, s> <caption>
    frame() {
        printf '\033[2J'; at 22 1; printf '%s' "$4"; nap 1.5
        [ "$2" = 1 ] && printf '\033[?2026h'
        r=1
        while [ $r -le 20 ]; do
            at $r 1; printf '\033[4%dm%60s\033[0m' "$1" "frame row $r "
            [ $r = 10 ] && nap "$3"
            r=$((r + 1))
        done
        [ "$2" = 1 ] && printf '\033[?2026l'
        nap 1.5
    }
    frame 1 0 0.6 'phase 1: no sync. Watch the top half appear first.'
    frame 2 1 0.6 'phase 2: sync for 0.6 s. The frame should appear in one go.'
    frame 4 1 2 'phase 3: sync for 2 s. The top half should appear at ~0.8 s (timeout).'
    leave_alt
}

sec_osc() {
    hdr osc "operating system commands"
    printf '\033[22;0t\033]2;rt-caps: title test ✓\007'
    see "window or tab title reads 'rt-caps: title test ✓' (restored at the end)"
    printf '  OSC 8 links: \033]8;;https://github.com/Rusty-Mill/rusty_mill\033\\rusty_mill repo\033]8;;\033\\'
    printf ' and \033]8;id=rt;https://example.com\033\\example.com\033]8;;\033\\\n'
    see "both link texts are clickable (window mode: hover shows the URL)"
    printf '\033]9;rt-caps: OSC 9 notification\007\033]777;notify;rt-caps;OSC 777 notification\007'
    see "two desktop notifications, OSC 9 and OSC 777 (window mode)"
    printf '\033]52;c;cnVzdHlfdGVybSBPU0MgNTIgb2s=\007'
    see "paste now: the clipboard holds 'rusty_term OSC 52 ok'"
    printf '\033]7;file://%s%s\007' "$(uname -n)" "$PWD"
    see "OSC 7 sent for $PWD: a new tab (window mode) starts in this directory"
    printf '\033]133;A\007$ \033]133;B\007echo marked\n\033]133;C\007marked\n\033]133;D;0\007'
    see "OSC 133 marks the prompt above: prompt jumps / mux blocks see it"
    printf '\033]7501;state=working\007\033]7501;state=done\007'
    note "OSC 7501 status sent (working, done): no visible change; a mux host records it"
    printf '  \033[31mpalette 1 normal\033[0m -> '
    printf '\033]4;1;rgb:ff/88/00\007\033[31mpalette 1 now orange (OSC 4)\033[0m\n'
    printf '\033]10;rgb:ff/ff/aa\007\033]11;rgb:10/20/50\007\033]12;rgb:ff/00/00\007'
    see "colors: fg pale yellow, bg navy, red cursor for 3 s (OSC 10/11/12, window mode)"
    nap 3
    printf '\033]104;1\007\033]110\007\033]111\007\033]112\007'
    printf '  \033[31mpalette 1 restored (OSC 104)\033[0m, fg/bg/cursor reset (OSC 110-112)\n'
    printf '\033[23;0t'
}

sec_images() {
    hdr images "Sixel, kitty graphics, iTerm2 (window mode; the TUI relay draws none)"
    png='iVBORw0KGgoAAAANSUhEUgAAACAAAAAQCAIAAAD4YuoOAAAALElEQVR42mO4Y6RPEmqdyUASYhi1YNSCIWCBWvclktDWTgaS0KgFoxYMAQsA53qOH2iUi58AAAAASUVORK5CYII='
    see "Sixel: four horizontal bands, red green red green, 64 px wide"
    printf '  \033Pq#1;2;90;15;15#2;2;15;70;15#1!64~-#2!64~-#1!64~-#2!64~\033\\\n\n'
    see "kitty: a 32x16 image in red, green, blue and yellow quadrants"
    printf '  \033_Gf=100,a=T;%s\033\\\n\n' "$png"
    see "iTerm2: the same quadrant image"
    printf '  \033]1337;File=inline=1;size=101:%s\007\n\n' "$png"
}

capture() {
    printf '  %s. q or %d s idle stops.\n' "$1" "$2"
    raw_on 10
    idle=0
    while :; do
        c=$(dd bs=64 count=1 <"$TTY" 2>/dev/null)
        if [ -z "$c" ]; then
            idle=$((idle + 1)); [ $idle -ge "$2" ] && break; continue
        fi
        idle=0
        [ "$c" = q ] && break
        printf '    %s\n' "$(printf '%s' "$c" | vis)"
    done
    raw_off
}

sec_input() {
    hdr input "interactive input reporting"
    printf '\033[?1002h\033[?1006h'
    capture 'Click, drag and scroll; expect ^[[<b;x;yM / m reports' 6
    printf '\033[?1002l\033[?1006l\033[?2004h'
    capture 'Paste something; expect it wrapped in ^[[200~ ... ^[[201~' 8
    printf '\033[?2004l\033[?1004h'
    capture 'Switch focus away and back; expect ^[[O then ^[[I' 8
    printf '\033[?1004l\033[>1u'
    capture 'Press Esc, ctrl+i and alt+a; kitty keyboard gives ^[[27u, ^[[105;5u, ^[[97;3u' 6
    printf '\033[<u'
}

gen() {
    awk -v kind="$1" -v mb="$2" 'BEGIN {
        a = ""; for (i = 0; i < 160; i++) a = a sprintf("%c", 33 + i % 94)
        target = mb * 1048576; n = 0
        while (n < target) {
            if (kind == "ascii") line = substr(a, n % 80 + 1, 79)
            else if (kind == "sgr") {
                line = ""
                for (k = 0; k < 8; k++) line = line sprintf("\033[1;38;5;%dmword%d\033[0m ", (n + k) % 256, k)
            } else line = "漢字かなカナ 😀🚀🎉 café ñandú Ωμέγα ┌─┐ " (n % 1000)
            print line; n += length(line) + 1
        }
    }'
}

sec_throughput() {
    hdr throughput "bulk output speed"
    mb=${RT_MB:-32}
    TMPF=$(mktemp "${TMPDIR:-/tmp}/rtcaps.XXXXXX")
    res=''
    pause "to flood ${mb} MB per workload (ascii, sgr, unicode)"
    for kind in ascii sgr unicode; do
        gen $kind "$mb" >"$TMPF"
        bytes=$(wc -c <"$TMPF")
        t0=$(now_ms); cat "$TMPF"; t1=$(now_ms)
        res="$res$kind $bytes $((t1 - t0))
"
    done
    printf '\033[0m\033[2J\033[H'
    hdr throughput "results (wall clock of cat; includes PTY, parser and render backpressure)"
    printf '%s' "$res" | awk '{ ms = $3 < 1 ? 1 : $3; printf "  %-8s %7.1f MB in %6d ms  = %7.1f MB/s\n", $1, $2 / 1048576, $3, $2 / 1048576 / (ms / 1000) }'
    note "coarse 1 s timer where date lacks %N; use crates/libs/ui/rusty_term/bench for real numbers"
    rm -f "$TMPF"; TMPF=''
}

# ---------------------------------------------------------------- main

run() {
    case " $ALL $EXTRA " in
    *" $1 "*) "sec_$1" ;;
    *) printf 'unknown section: %s\n\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
}

main() {
    [ $# -eq 0 ] && set -- all
    case $1 in list | help | -h | --help) usage; exit 0 ;; esac
    if ! [ -t 1 ] || ! : <"$TTY" 2>/dev/null; then
        echo 'rt-caps.sh needs an interactive terminal' >&2; exit 1
    fi
    trap cleanup EXIT
    trap 'exit 130' INT TERM HUP
    for s; do
        if [ "$s" = all ]; then
            for x in $ALL; do run "$x"; done
        else
            run "$s"
        fi
    done
    printf '\n\033[1mdone:\033[0m %d PASS, %d FAIL (self-checked rows); everything else is by eye.\n' "$PASS" "$FAIL"
    nap 0.3 # let the last frame paint before a parent rusty_term exits with us
}

main "$@"
