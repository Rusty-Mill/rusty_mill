"""Generate terminal-modern.svg: a rusty_term GUI concept with command blocks."""
from __future__ import annotations

import base64
import glob
import io
from dataclasses import dataclass
from html import escape
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

FONT_DIR = Path(glob.glob("/root/.claude/skills/synced/*/canvas-design/canvas-fonts")[0])
OUT = Path(__file__).with_name("terminal-modern.svg")

# ── tokens ────────────────────────────────────────────────────────────────
BASE, RAISED, SUNK = "#1C2226", "#242C31", "#171C1F"
LINE = "#2F393E"
BONE, SLATE, DIM = "#DCE2E1", "#8A979C", "#6E7B80"
VERD, OXIDE = "#5FB8A5", "#D9773A"
UI, MONO = "UI", "Mono"
W, H = 1200, 716
X0, Y0, WW, WH = 24, 24, 1152, 652
MONO_PX, LINE_H = 13, 21
CH = MONO_PX * 0.6  # JetBrains Mono advance is 600/1000 em

parts: list[str] = []
used_text: list[str] = []


def add(s: str) -> None:
    parts.append(s)


def text(x: float, y: float, s: str, *, font: str = MONO, size: float = MONO_PX,
         fill: str = BONE, weight: int = 400, anchor: str = "start") -> None:
    used_text.append(s)
    add(f'<text x="{x:.1f}" y="{y:.1f}" font-family="{font}" font-size="{size}" '
        f'font-weight="{weight}" fill="{fill}" text-anchor="{anchor}">{escape(s)}</text>')


def spans(x: float, y: float, segs: list[tuple[str, str]]) -> None:
    """Mono line with coloured segments; positions from the fixed advance."""
    for s, fill in segs:
        if s.strip():
            text(x, y, s, fill=fill)
        x += len(s) * CH


# ── icons (stroke paths, 16px box) ─────────────────────────────────────────
def icon(kind: str, x: float, y: float, color: str, sw: float = 1.6) -> None:
    d = {
        "check": "M3.5 8.5l3 3 6-7",
        "cross": "M4.5 4.5l7 7M11.5 4.5l-7 7",
        "branch": "M5 3v10M5 13a1.8 1.8 0 1 0 0 .1M11 3a1.8 1.8 0 1 0 0 .1M11 4.8c0 4-6 3-6 6.2",
        "chev": "M6 4l4 4-4 4",
        "prompt": "M4 4l5 4-5 4",
        "copy": "M5.5 5.5h7v7h-7zM3.5 10.5v-7h7",
        "rerun": "M12.5 8a4.5 4.5 0 1 1-1.3-3.2M12.5 3v2.5H10",
        "plus": "M8 3.5v9M3.5 8h9",
        "folder": "M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z",
    }[kind]
    add(f'<path transform="translate({x:.1f} {y:.1f})" d="{d}" fill="none" stroke="{color}" '
        f'stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>')


def keycap(x: float, y: float, label: str) -> float:
    """Draw a key hint ending at x (right-aligned); return its left edge."""
    w = 9 + len(label) * 6.4
    add(f'<rect x="{x - w:.1f}" y="{y - 13}" width="{w:.1f}" height="18" rx="4" '
        f'fill="none" stroke="{LINE}"/>')
    text(x - w / 2, y, label, font=UI, size=11, fill=SLATE, anchor="middle")
    return x - w


# ── window + tab strip ─────────────────────────────────────────────────────
def chrome() -> None:
    add(f'<rect x="{X0}" y="{Y0}" width="{WW}" height="{WH}" rx="12" fill="{BASE}" '
        f'stroke="#323C41" filter="url(#lift)"/>')
    add(f'<path d="M{X0} {Y0 + 12}a12 12 0 0 1 12-12h{WW - 24}a12 12 0 0 1 12 12v36h-{WW}z" fill="{SUNK}"/>')
    for i in range(3):
        add(f'<circle cx="{X0 + 22 + i * 18}" cy="{Y0 + 24}" r="5.5" fill="#3A454A"/>')

    tabs = [("rusty_mill", "check", True), ("hermes", "spin", False), ("baileyai", "ssh", False)]
    x = X0 + 92
    for title, state, active in tabs:
        w = 30 + len(title) * 7.4 + 26
        if active:
            add(f'<path d="M{x} {Y0 + 48}v-30a6 6 0 0 1 6-6h{w - 12}a6 6 0 0 1 6 6v30z" fill="{BASE}"/>')
        tx = x + 14
        if state == "check":
            icon("check", tx - 2, Y0 + 22, VERD, 1.8)
        elif state == "spin":
            add(f'<circle cx="{tx + 6}" cy="{Y0 + 30}" r="5" fill="none" stroke="{LINE}" stroke-width="1.8"/>')
            add(f'<path d="M{tx + 6} {Y0 + 25}a5 5 0 0 1 5 5" fill="none" stroke="{VERD}" '
                f'stroke-width="1.8" stroke-linecap="round"><animateTransform attributeName="transform" '
                f'type="rotate" from="0 {tx + 6} {Y0 + 30}" to="360 {tx + 6} {Y0 + 30}" dur="1s" '
                f'repeatCount="indefinite"/></path>')
        else:
            add(f'<circle cx="{tx + 6}" cy="{Y0 + 30}" r="3" fill="{DIM}"/>')
        text(tx + 20, Y0 + 34, title, font=UI, size=13, weight=600 if active else 400,
             fill=BONE if active else SLATE)
        x += w + 4
    icon("plus", x + 6, Y0 + 22, DIM)


def context_bar() -> None:
    y = Y0 + 48
    add(f'<line x1="{X0}" y1="{y + 40}" x2="{X0 + WW}" y2="{y + 40}" stroke="{LINE}"/>')
    icon("folder", X0 + 22, y + 12, SLATE, 1.4)
    text(X0 + 44, y + 25, "~/src/rusty_mill", font=UI, size=13, weight=600)
    icon("branch", X0 + 182, y + 12, VERD, 1.4)
    text(X0 + 204, y + 25, "feat/zoom-text-only", font=UI, size=13, fill=BONE)
    text(X0 + 346, y + 25, "2 changed, 1 behind", font=UI, size=13, fill=SLATE)
    left = keycap(X0 + WW - 22, y + 25, "Ctrl Shift F")
    text(left - 10, y + 25, "Search output", font=UI, size=12, fill=SLATE, anchor="end")


# ── command blocks ─────────────────────────────────────────────────────────
@dataclass(frozen=True)
class Block:
    cmd: str
    ok: bool
    meta: str
    lines: tuple[tuple[tuple[str, str], ...], ...]
    folded: int = 0
    selected: bool = False


def block(b: Block, y: float) -> float:
    x, w = X0 + 20, WW - 40
    n = len(b.lines) + (1 if b.folded else 0)
    h = 44 + n * LINE_H + 10
    rail = VERD if b.ok else OXIDE
    if b.selected:
        add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{OXIDE}" fill-opacity="0.07" '
            f'stroke="{OXIDE}" stroke-opacity="0.35"/>')
    add(f'<rect x="{x}" y="{y + 8}" width="3" height="{h - 16}" rx="1.5" fill="{rail}"/>')

    hy = y + 27
    text(x + 22, hy, b.cmd, weight=700)
    rx = x + w - 18
    text(rx, hy, b.meta, font=UI, size=12, fill=SLATE, anchor="end")
    mw = len(b.meta) * 6.3 + 14
    label = "Passed" if b.ok else "Exit 1"
    text(rx - mw, hy, label, font=UI, size=12, weight=600, fill=rail, anchor="end")
    icon("check" if b.ok else "cross", rx - mw - len(label) * 7 - 22, hy - 12, rail, 1.8)
    if b.selected:
        bx = rx - mw - len(label) * 7 - 92
        for k, kind in enumerate(("rerun", "copy")):
            cx = bx + k * 32
            add(f'<rect x="{cx - 4}" y="{hy - 16}" width="24" height="24" rx="5" fill="{RAISED}"/>')
            icon(kind, cx, hy - 12, BONE, 1.4)

    ly = y + 44 + 14
    rows = list(b.lines)
    for i, segs in enumerate(rows):
        if b.folded and i == 1:
            icon("chev", x + 20, ly - 12, SLATE, 1.6)
            text(x + 40, ly, f"{b.folded} lines hidden", font=UI, size=12, fill=SLATE)
            ly += LINE_H
        spans(x + 22, ly, list(segs))
        ly += LINE_H
    return y + h


BLOCKS = (
    Block("cargo test -p rusty_term", True, "6.2 s   09:14",
          ((("running 48 tests", SLATE),),
           (("test result: ", SLATE), ("ok", VERD), (". 48 passed; 0 failed; finished in 6.18s", SLATE))),
          folded=46),
    Block("git status -sb", True, "0.1 s   09:15",
          ((("## ", DIM), ("feat/zoom-text-only", VERD), ("...origin/feat/zoom-text-only [behind 1]", SLATE)),
           ((" M ", BONE), ("crates/libs/ui/rusty_term/src/gui/zoom.rs", SLATE)),
           ((" M ", BONE), ("crates/libs/ui/rusty_term/src/gui/chrome.rs", SLATE)))),
    Block("git push origin feat/zoom-text-only", False, "0.9 s   09:16",
          ((("To github.com:baileyrd/rusty_mill.git", SLATE),),
           ((" ! [rejected]  ", OXIDE), ("feat/zoom-text-only -> feat/zoom-text-only (fetch first)", BONE)),
           (("error: ", OXIDE), ("failed to push some refs to 'github.com:baileyrd/rusty_mill.git'", BONE)),
           (("hint: Updates were rejected because the remote contains work that you do not", SLATE),),
           (("hint: have locally. Integrate the remote changes before pushing again.", SLATE),)),
          selected=True),
)


def input_bar() -> None:
    y = Y0 + WH - 36 - 64
    x, w = X0 + 20, WW - 40
    add(f'<rect x="{x}" y="{y}" width="{w}" height="50" rx="10" fill="{SUNK}" stroke="{VERD}" '
        f'stroke-opacity="0.55" stroke-width="1.5"/>')
    icon("prompt", x + 16, y + 17, VERD, 2)
    typed, ghost = "git pull --re", "base origin feat/zoom-text-only"
    tx = x + 42
    text(tx, y + 30, typed, size=14)
    cx = tx + len(typed) * 14 * 0.6
    add(f'<rect x="{cx:.1f}" y="{y + 16}" width="2" height="19" fill="{BONE}">'
        f'<animate attributeName="opacity" values="1;1;0;0" keyTimes="0;.5;.5;1" dur="1.1s" '
        f'repeatCount="indefinite"/></rect>')
    text(cx + 3, y + 30, ghost, size=14, fill=DIM)
    left = keycap(x + w - 16, y + 30, "Tab")
    text(left - 10, y + 30, "Accept suggestion", font=UI, size=12, fill=SLATE, anchor="end")


def status_ribbon() -> None:
    y = Y0 + WH - 36
    add(f'<path d="M{X0} {y}h{WW}v24a12 12 0 0 1-12 12h-{WW - 24}a12 12 0 0 1-12-12z" fill="{SUNK}"/>')
    add(f'<line x1="{X0}" y1="{y}" x2="{X0 + WW}" y2="{y}" stroke="{LINE}"/>')
    x = X0 + 22
    for seg in ("zsh", "120 × 36", "UTF-8"):
        text(x, y + 23, seg, font=UI, size=12, fill=SLATE)
        x += len(seg) * 6.8 + 24
    add(f'<circle cx="{X0 + WW - 22 - len("1 agent running") * 6.1 - 10:.1f}" cy="{y + 19}" r="3.5" fill="{VERD}"/>')
    text(X0 + WW - 22, y + 23, "1 agent running", font=UI, size=12, fill=SLATE, anchor="end")


# ── fonts ──────────────────────────────────────────────────────────────────
def embed(path: Path, family: str, weight: int, chars: str) -> str:
    font = TTFont(path)
    opts = subset.Options()
    opts.flavor = "woff"
    opts.layout_features = ["kern", "liga", "calt"]
    sub = subset.Subsetter(opts)
    sub.populate(text=chars)
    sub.subset(font)
    buf = io.BytesIO()
    font.flavor = "woff"
    font.save(buf)
    b64 = base64.b64encode(buf.getvalue()).decode()
    return (f"@font-face{{font-family:{family};font-weight:{weight};"
            f"src:url(data:font/woff;base64,{b64}) format('woff')}}")


def main() -> None:
    chrome()
    context_bar()
    y = Y0 + 48 + 40 + 14
    for b in BLOCKS:
        y = block(b, y) + 8
    input_bar()
    status_ribbon()

    chars = "".join(used_text) + " "
    css = "".join([
        embed(FONT_DIR / "InstrumentSans-Regular.ttf", UI, 400, chars),
        embed(FONT_DIR / "InstrumentSans-Bold.ttf", UI, 600, chars),
        embed(FONT_DIR / "JetBrainsMono-Regular.ttf", MONO, 400, chars),
        embed(FONT_DIR / "JetBrainsMono-Bold.ttf", MONO, 700, chars),
    ])
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" '
           f'role="img" aria-label="rusty_term concept: command blocks with status, context bar, '
           f'and suggestion input">'
           f'<defs><style>{css}text{{white-space:pre}}</style>'
           f'<filter id="lift" x="-5%" y="-5%" width="110%" height="115%">'
           f'<feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000" flood-opacity=".4"/>'
           f'</filter></defs>'
           + "".join(parts) + "</svg>")
    OUT.write_text(svg)
    print(f"{OUT} {len(svg) // 1024} KB, content ends y={y:.0f}")


if __name__ == "__main__":
    main()
