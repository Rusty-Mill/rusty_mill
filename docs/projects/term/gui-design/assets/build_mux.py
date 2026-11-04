"""Generate terminal-mux.svg: rusty_term + rusty_mux combined GUI concept.

Model shown: session -> windows -> server-owned split tree of blocks.
Each block has a header (program status, lease holder); shell blocks group
output into runs (one command + its output + result).
"""
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
OUT = Path(__file__).with_name("terminal-mux.svg")

# ── tokens ────────────────────────────────────────────────────────────────
FRAME, BASE, RAISED, SUNK = "#171C1F", "#1C2226", "#262F34", "#13171A"
LINE = "#2F393E"
BONE, SLATE, DIM = "#DCE2E1", "#8A979C", "#6E7B80"
VERD, OXIDE = "#5FB8A5", "#D9773A"
UI, MONO = "UI", "Mono"
W, H = 1440, 900
X0, Y0, WW, WH = 24, 24, 1392, 852
TOP_H, RIB_H, GAP = 52, 34, 8
MONO_PX, LINE_H = 13, 21
CH = MONO_PX * 0.6

FONTS = {
    (UI, 400): FONT_DIR / "InstrumentSans-Regular.ttf",
    (UI, 600): FONT_DIR / "InstrumentSans-Bold.ttf",
    (MONO, 400): FONT_DIR / "JetBrainsMono-Regular.ttf",
    (MONO, 700): FONT_DIR / "JetBrainsMono-Bold.ttf",
}
_metrics: dict[tuple[str, int], tuple[dict[str, int], dict[str, tuple[int, int]], int]] = {}

parts: list[str] = []
used_text: list[str] = []


def measure(s: str, font: str = UI, size: float = 12, weight: int = 400) -> float:
    """Advance width of s in px, from the real font metrics."""
    key = (font, weight)
    if key not in _metrics:
        f = TTFont(FONTS[key])
        _metrics[key] = (f.getBestCmap(), f["hmtx"].metrics, f["head"].unitsPerEm)
    cmap, hmtx, upm = _metrics[key]
    return sum(hmtx[cmap.get(ord(c), ".notdef")][0] for c in s) * size / upm


def add(s: str) -> None:
    parts.append(s)


def text(x: float, y: float, s: str, *, font: str = MONO, size: float = MONO_PX,
         fill: str = BONE, weight: int = 400, anchor: str = "start") -> float:
    """Draw text; return x just past its end (for start-anchored text)."""
    used_text.append(s)
    add(f'<text x="{x:.1f}" y="{y:.1f}" font-family="{font}" font-size="{size}" '
        f'font-weight="{weight}" fill="{fill}" text-anchor="{anchor}">{escape(s)}</text>')
    return x + measure(s, font, size, weight)


def ui(x: float, y: float, s: str, *, fill: str = SLATE, size: float = 12,
       weight: int = 400, anchor: str = "start") -> float:
    return text(x, y, s, font=UI, size=size, fill=fill, weight=weight, anchor=anchor)


def spans(x: float, y: float, segs: tuple[tuple[str, str], ...]) -> None:
    for s, fill in segs:
        if s.strip():
            text(x, y, s, fill=fill)
        x += len(s) * CH


def rect(x: float, y: float, w: float, h: float, *, rx: float = 0, fill: str = "none",
         stroke: str | None = None, extra: str = "") -> None:
    st = f' stroke="{stroke}"' if stroke else ""
    add(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" '
        f'fill="{fill}"{st} {extra}/>')


# ── icons (stroke paths, 16px box) ─────────────────────────────────────────
ICONS = {
    "check": "M3.5 8.5l3 3 6-7",
    "cross": "M4.5 4.5l7 7M11.5 4.5l-7 7",
    "branch": "M5 3v10M5 13a1.8 1.8 0 1 0 0 .1M11 3a1.8 1.8 0 1 0 0 .1M11 4.8c0 4-6 3-6 6.2",
    "chev": "M6 4l4 4-4 4",
    "chevdown": "M4 6l4 4 4-4",
    "prompt": "M4 4l5 4-5 4",
    "copy": "M5.5 5.5h7v7h-7zM3.5 10.5v-7h7",
    "rerun": "M12.5 8a4.5 4.5 0 1 1-1.3-3.2M12.5 3v2.5H10",
    "plus": "M8 3.5v9M3.5 8h9",
    "folder": "M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z",
    "server": "M2.5 3.5h11v4h-11zM2.5 8.5h11v4h-11zM5 5.5h.1M5 10.5h.1",
    "zoom": "M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3",
    "clients": "M6 7.5a2.3 2.3 0 1 0 0-.1M2 13c.4-2.4 2-3.6 4-3.6s3.6 1.2 4 3.6M10.5 4.6a2.2 2.2 0 1 1 .8 4.2M12 9.6c1.3.5 2 1.6 2.2 3.4",
    "agent": "M8 2.5v2M4.5 5.5h7v6h-7zM6.5 8h.1M9.5 8h.1M2.5 8v2M13.5 8v2",
    "eye": "M1.8 8s2.3-4.2 6.2-4.2S14.2 8 14.2 8s-2.3 4.2-6.2 4.2S1.8 8 1.8 8zM8 6.3a1.7 1.7 0 1 0 0 3.4 1.7 1.7 0 0 0 0-3.4z",
}


def icon(kind: str, x: float, y: float, color: str, sw: float = 1.5) -> None:
    add(f'<path transform="translate({x:.1f} {y:.1f})" d="{ICONS[kind]}" fill="none" '
        f'stroke="{color}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>')


def spinner(cx: float, cy: float, r: float = 5) -> None:
    add(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{r}" fill="none" stroke="{LINE}" stroke-width="1.8"/>')
    add(f'<path d="M{cx:.1f} {cy - r:.1f}a{r} {r} 0 0 1 {r} {r}" fill="none" stroke="{VERD}" '
        f'stroke-width="1.8" stroke-linecap="round"><animateTransform attributeName="transform" '
        f'type="rotate" from="0 {cx:.1f} {cy:.1f}" to="360 {cx:.1f} {cy:.1f}" dur="1s" '
        f'repeatCount="indefinite"/></path>')


def keycap(x: float, y: float, label: str, *, right: bool = False) -> float:
    """Key hint at x (left edge, or right edge if right=True); returns the far edge."""
    w = max(18.0, measure(label, UI, 11) + 10)
    left = x - w if right else x
    rect(left, y - 13, w, 18, rx=4, stroke=LINE)
    ui(left + w / 2, y, label, size=11, anchor="middle")
    return left if right else left + w


# ── layout model ───────────────────────────────────────────────────────────
@dataclass(frozen=True)
class Box:
    x: float
    y: float
    w: float
    h: float


def split_h(b: Box, frac: float) -> tuple[Box, Box]:
    lw = round((b.w - GAP) * frac)
    return Box(b.x, b.y, lw, b.h), Box(b.x + lw + GAP, b.y, b.w - lw - GAP, b.h)


def split_v(b: Box, frac: float) -> tuple[Box, Box]:
    th = round((b.h - GAP) * frac)
    return Box(b.x, b.y, b.w, th), Box(b.x, b.y + th + GAP, b.w, b.h - th - GAP)


# ── top strip: session + windows + clients ─────────────────────────────────
def thumb(x: float, y: float, cells: tuple[tuple[float, float, float, float, str], ...],
          active: bool) -> None:
    """Tiny split-tree preview, 26x16. Cells are fractions + a status colour."""
    tw, th = 26, 16
    for fx, fy, fw, fh, color in cells:
        rect(x + fx * tw + .5, y + fy * th + .5, fw * tw - 1.5, fh * th - 1.5, rx=1.5,
             fill=color if color != "none" else ("#3A464C" if active else "#2C363B"))


def top_strip() -> None:
    rect(X0, Y0, WW, WH, rx=12, fill=FRAME, stroke="#323C41", extra='filter="url(#lift)"')
    for i in range(3):
        add(f'<circle cx="{X0 + 22 + i * 18}" cy="{Y0 + 26}" r="5.5" fill="#3A454A"/>')

    # session switcher
    sx, cy = X0 + 92, Y0 + 26
    icon("server", sx + 2, cy - 8, SLATE)
    ex = ui(sx + 26, cy + 4.5, "rusty_mill", fill=BONE, size=13, weight=600)
    ex = ui(ex + 8, cy + 4.5, "on baileyai", size=13)
    icon("chevdown", ex + 4, cy - 8, SLATE)
    add(f'<line x1="{ex + 32:.1f}" y1="{cy - 12}" x2="{ex + 32:.1f}" y2="{cy + 12}" stroke="{LINE}"/>')

    # windows, each with its split-tree thumbnail
    wins = (
        ("build", True, ((0, 0, .6, 1, "none"), (.6, 0, .4, .55, VERD), (.6, .55, .4, .45, "none"))),
        ("agents", False, ((0, 0, .5, 1, VERD), (.5, 0, .5, 1, VERD))),
        ("baileyai", False, ((0, 0, 1, 1, "none"),)),
    )
    x = ex + 48
    for i, (name, active, cells) in enumerate(wins, start=1):
        label = f"{i}  {name}"
        w = 12 + 26 + 10 + measure(label, UI, 13, 600 if active else 400) + 14
        if active:
            rect(x, cy - 17, w, 34, rx=7, fill=BASE, stroke=LINE)
        thumb(x + 12, cy - 8, cells, active)
        ui(x + 48, cy + 4.5, label, fill=BONE if active else SLATE, size=13,
           weight=600 if active else 400)
        x += w + 6
    icon("plus", x + 4, cy - 8, DIM)

    # attached clients
    rx = X0 + WW - 20
    left = ui(rx, cy + 4.5, "2 clients attached", anchor="end", size=12)
    icon("clients", rx - measure("2 clients attached", UI, 12) - 24, cy - 8, SLATE)


# ── blocks ─────────────────────────────────────────────────────────────────
HDR = 38


def block_frame(b: Box, *, focused: bool) -> None:
    rect(b.x, b.y, b.w, b.h, rx=9, fill=BASE,
         stroke=VERD if focused else LINE,
         extra='stroke-width="1.5" stroke-opacity="0.7"' if focused else "")
    add(f'<line x1="{b.x + 1}" y1="{b.y + HDR}" x2="{b.x + b.w - 1}" y2="{b.y + HDR}" stroke="{LINE}"/>')


@dataclass(frozen=True)
class Run:
    cmd: str
    ok: bool
    meta: str
    lines: tuple[tuple[tuple[str, str], ...], ...]
    folded: int = 0
    selected: bool = False


def run_height(r: Run) -> float:
    return 40 + (len(r.lines) + (1 if r.folded else 0)) * LINE_H + 8


def run(r: Run, x: float, y: float, w: float) -> float:
    h = run_height(r)
    rail = VERD if r.ok else OXIDE
    if r.selected:
        rect(x, y, w, h, rx=7, fill=OXIDE, stroke=OXIDE,
             extra='fill-opacity="0.07" stroke-opacity="0.35"')
    rect(x, y + 8, 3, h - 16, rx=1.5, fill=rail)

    hy = y + 25
    text(x + 18, hy, r.cmd, weight=700)
    rx = x + w - 14
    mx = ui(rx, hy, r.meta, anchor="end")
    label = "Passed" if r.ok else "Exit 1"
    lx = rx - measure(r.meta, UI, 12) - 14
    ui(lx, hy, label, fill=rail, weight=600, anchor="end")
    ix = lx - measure(label, UI, 12, 600) - 20
    icon("check" if r.ok else "cross", ix, hy - 12, rail, 1.8)
    if r.selected:
        for k, kind in enumerate(("copy", "rerun")):
            bx = ix - 40 - k * 30
            rect(bx - 4, hy - 16, 24, 24, rx=5, fill=RAISED)
            icon(kind, bx, hy - 12, BONE, 1.4)

    ly = y + 40 + 12
    for i, segs in enumerate(r.lines):
        if r.folded and i == 1:
            icon("chev", x + 16, ly - 12, SLATE, 1.6)
            ui(x + 36, ly, f"{r.folded} lines hidden")
            ly += LINE_H
        spans(x + 18, ly, segs)
        ly += LINE_H
    return y + h


RUNS = (
    Run("cargo clippy -p rusty_term -- -D warnings", True, "14.8 s   09:11",
        ((("    Checking rusty_term v0.4.0 (crates/libs/ui/rusty_term)", SLATE),),
         (("    Finished ", VERD), ("dev profile [unoptimized + debuginfo] in 14.71s", SLATE))),
        folded=212),
    Run("git status -sb", True, "0.1 s   09:13",
        ((("## ", DIM), ("feat/zoom-text-only", VERD), ("...origin/feat/zoom-text-only [behind 1]", SLATE)),
         ((" M ", BONE), ("crates/libs/ui/rusty_term/src/gui/zoom.rs", SLATE)),
         ((" M ", BONE), ("crates/libs/ui/rusty_term/src/gui/chrome.rs", SLATE)))),
    Run("cargo test -p rusty_term", True, "6.2 s   09:14",
        ((("running 48 tests", SLATE),),
         (("test result: ", SLATE), ("ok", VERD), (". 48 passed; 0 failed; finished in 6.18s", SLATE))),
        folded=46),
    Run("git push origin feat/zoom-text-only", False, "0.9 s   09:16",
        ((("To github.com:baileyrd/rusty_mill.git", SLATE),),
         ((" ! [rejected]  ", OXIDE), ("feat/zoom-text-only -> feat/zoom-text-only (fetch first)", BONE)),
         (("error: ", OXIDE), ("failed to push some refs to 'github.com:baileyrd/rusty_mill.git'", BONE)),
         (("hint: Updates were rejected because the remote contains work that you", SLATE),),
         (("hint: do not have locally. Integrate the remote changes before pushing.", SLATE),)),
        selected=True),
)


def shell_block(b: Box) -> None:
    block_frame(b, focused=True)
    hy = b.y + 24
    icon("folder", b.x + 14, hy - 12, SLATE, 1.4)
    ex = ui(b.x + 36, hy, "~/src/rusty_mill", fill=BONE, size=13, weight=600)
    icon("branch", ex + 16, hy - 12, VERD, 1.4)
    ex = ui(ex + 38, hy, "feat/zoom-text-only", fill=BONE, size=13)
    ui(ex + 14, hy, "2 changed, 1 behind", size=13)
    rect(b.x + b.w - 38, hy - 17, 26, 24, rx=5, fill=RAISED)
    icon("zoom", b.x + b.w - 33, hy - 13, SLATE, 1.4)

    # scrollback sits on the input, newest last
    iy = b.y + b.h - 62
    y = iy - 12 - sum(run_height(r) + 8 for r in RUNS) + 8
    assert y >= b.y + HDR + 8, "runs overflow the block"
    for r in RUNS:
        y = run(r, b.x + 12, y, b.w - 24) + 8

    # input
    ix, iw = b.x + 12, b.w - 24
    rect(ix, iy, iw, 50, rx=9, fill=SUNK, stroke=VERD,
         extra='stroke-opacity="0.55" stroke-width="1.5"')
    icon("prompt", ix + 14, iy + 17, VERD, 2)
    typed, ghost = "git pull --re", "base origin feat/zoom-text-only"
    tx = ix + 40
    text(tx, iy + 30, typed, size=14)
    cx = tx + len(typed) * 14 * 0.6
    add(f'<rect x="{cx:.1f}" y="{iy + 16}" width="2" height="19" fill="{BONE}">'
        f'<animate attributeName="opacity" values="1;1;0;0" keyTimes="0;.5;.5;1" dur="1.1s" '
        f'repeatCount="indefinite"/></rect>')
    text(cx + 3, iy + 30, ghost, size=14, fill=DIM)
    left = keycap(ix + iw - 14, iy + 30, "Tab", right=True)
    ui(left - 10, iy + 30, "Accept suggestion", anchor="end")


def agent_block(b: Box) -> None:
    block_frame(b, focused=False)
    hy = b.y + 24
    spinner(b.x + 22, hy - 4)
    ex = ui(b.x + 38, hy, "claude", fill=BONE, size=13, weight=600)
    ui(ex + 10, hy, "Editing zoom.rs", size=13)
    # lease: who holds input
    rx = b.x + b.w - 12
    bw = measure("Take over", UI, 12, 600) + 20
    rect(rx - bw, hy - 16, bw, 24, rx=6, stroke=LINE)
    ui(rx - bw / 2, hy, "Take over", fill=BONE, weight=600, anchor="middle")
    lx = ui(rx - bw - 12, hy, "Agent has input", anchor="end")
    icon("eye", rx - bw - 12 - measure("Agent has input", UI, 12) - 22, hy - 12, SLATE, 1.3)

    rows: tuple[tuple[tuple[str, str], ...], ...] = (
        (("> ", VERD), ("Read ", BONE), ("src/gui/zoom.rs", SLATE)),
        (("> ", VERD), ("Read ", BONE), ("src/gui/chrome.rs", SLATE)),
        (("> ", VERD), ("Edit ", BONE), ("src/gui/zoom.rs  ", SLATE), ("+12", VERD), (" -3", OXIDE)),
        (("  ", SLATE), ("ui_px now follows DPI on reset (Ctrl+0)", SLATE)),
        (("> ", VERD), ("Run  ", BONE), ("cargo check -p rusty_term", SLATE)),
        (("  ", SLATE), ("Checking rusty_term v0.4.0", SLATE)),
    )
    y = b.y + HDR + 26
    for segs in rows:
        spans(b.x + 16, y, segs)
        y += LINE_H


def watch_block(b: Box) -> None:
    block_frame(b, focused=False)
    hy = b.y + 24
    icon("check", b.x + 13, hy - 12, VERD, 1.8)
    ex = ui(b.x + 38, hy, "cargo watch -x check", fill=BONE, size=13, weight=600)
    ui(ex + 10, hy, "Idle", size=13)
    rows: tuple[tuple[tuple[str, str], ...], ...] = (
        (("[Running 'cargo check']", SLATE),),
        (("    Finished ", VERD), ("dev profile in 3.84s", SLATE)),
        (("[Finished running. Exit status: 0]", SLATE),),
    )
    y = b.y + HDR + 26
    for segs in rows:
        spans(b.x + 16, y, segs)
        y += LINE_H


# ── status ribbon ──────────────────────────────────────────────────────────
def ribbon() -> None:
    y = Y0 + WH - RIB_H
    add(f'<path d="M{X0} {y}h{WW}v{RIB_H - 12}a12 12 0 0 1-12 12h-{WW - 24}a12 12 0 0 1-12-12z" fill="{SUNK}"/>')
    add(f'<line x1="{X0}" y1="{y}" x2="{X0 + WW}" y2="{y}" stroke="{LINE}"/>')
    ty = y + 22
    add(f'<circle cx="{X0 + 24}" cy="{ty - 4}" r="3.5" fill="{VERD}"/>')
    x = ui(X0 + 34, ty, "Connected to baileyai") + 22
    for seg in ("3 blocks", "zsh", "UTF-8"):
        x = ui(x, ty, seg) + 22

    keys = (("|", "Split right"), ("-", "Split down"), ("z", "Zoom"), ("d", "Detach"))
    # measure right-to-left so the group ends at the margin
    widths = [max(18.0, measure(k, UI, 11) + 10) + 6 + measure(l, UI, 12) for k, l in keys]
    total = sum(widths) + 18 * (len(keys) - 1)
    x = X0 + WW - 20 - total
    pre = "Ctrl-]  then"
    px = x - 18 - measure("then", UI, 12)
    ui(px, ty, "then")
    keycap(px - 8 - (measure("Ctrl-]", UI, 11) + 10), ty, "Ctrl-]")
    for (k, l), w in zip(keys, widths):
        e = keycap(x, ty, k)
        ui(e + 6, ty, l)
        x += w + 18
    del pre


# ── fonts + assembly ───────────────────────────────────────────────────────
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
    top_strip()
    body = Box(X0 + GAP, Y0 + TOP_H, WW - 2 * GAP, WH - TOP_H - RIB_H - GAP)
    left, right = split_h(body, 0.6)
    rtop, rbot = split_v(right, 0.6)
    shell_block(left)
    agent_block(rtop)
    watch_block(rbot)
    ribbon()

    chars = "".join(used_text) + " "
    css = "".join(embed(p, fam, wt, chars) for (fam, wt), p in FONTS.items())
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" '
           f'role="img" aria-label="rusty_term with rusty_mux: session switcher, windows with layout '
           f'previews, three blocks with status and input lease, runs, and prefix key hints">'
           f'<defs><style>{css}text{{white-space:pre}}</style>'
           f'<filter id="lift" x="-5%" y="-5%" width="110%" height="115%">'
           f'<feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000" flood-opacity=".4"/>'
           f'</filter></defs>' + "".join(parts) + "</svg>")
    OUT.write_text(svg)
    print(f"{OUT} {len(svg) // 1024} KB")


if __name__ == "__main__":
    main()
