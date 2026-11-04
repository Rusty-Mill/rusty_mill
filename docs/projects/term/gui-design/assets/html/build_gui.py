"""Build the rusty_term GUI design pages (states, light theme, alt layouts, settings).

Run: python build_gui.py   (writes *.html next to this file; open index.html)
Standalone replacement for the Paper pages that hit Paper's weekly MCP limit.
"""
from __future__ import annotations

from pathlib import Path

from gui_lib import (AGENT_OUT, ALL_RUNS, RUN_CLIPPY, RUN_PUSH, RUN_STATUS, RUN_TEST, T_AGENTS, T_BUILD, T_ONE,
                     agent_block, agent_header, btn, frame, ic, idle_block, input_box, item,
                     kc, page, ribbon, run, shell_block, shell_header, spinner, strip, tab, thumb)

OUT = Path(__file__).parent
EMPTY_RUNS = '<div class="empty"><b>No runs yet</b><span>Type a command to start.</span></div>'


def blocks(inner: str, dim: bool = False) -> str:
    return f'<div class="blocks" style="{"opacity:.7" if dim else ""}">{inner}</div>'


def lead_alert() -> str:
    return ic("alert", None, 1.6).replace('class="ic"', 'class="ic lead"')


def std(theme="dark", sel=1, shell=None, agent=None, idle=None, rib=None, overlay="", dim=False, detached=0):
    inner = (shell or shell_block()) + f'<div class="col" style="flex:1">{agent or agent_block()}{idle or idle_block()}</div>'
    return frame(strip(sel, detached) + blocks(inner, dim) + (rib or ribbon()), theme, overlay)


def pop_takeover() -> str:
    return (f'<div class="pop pad" style="right:20px;top:98px;width:300px"><h4>Take input from claude?</h4>'
            f'<p>claude is editing zoom.rs. It stops when you type.</p><div class="acts">{btn("Cancel")}{btn("Take over", "p")}</div></div>')


def pop_switcher(up: bool = False) -> str:
    def row(n, s, cur=False):
        if cur:
            return f'<div class="prow cur"><div class="n"><b>{n}</b><span>{s}</span></div>{ic("check", "verd", 1.8)}<span class="ok">Current</span></div>'
        return f'<div class="prow"><div class="n"><b>{n}</b><span>{s}</span></div></div>'
    body = (f'<div class="sw-h">baileyai</div>{row("rusty_mill", "3 windows, 2 clients", True)}{row("rusty_data_os", "2 windows, 1 client")}'
            f'{row("agents-night", "1 window, 0 clients")}<div class="sw-h top">local</div>{row("scratch", "1 window, 1 client")}'
            f'{row("notes", "2 windows, 0 clients")}<div class="pfoot"><span>{kc("Up")}{kc("Down")}Move</span><span>{kc("Enter")}Switch</span><span>{kc("Esc")}Close</span></div>')
    pos = "left:92px;bottom:46px" if up else "left:92px;top:46px"
    return f'<div class="pop" style="{pos};width:360px">{body}</div>'


def banner_reconnect(top: str = "") -> str:
    st = f' style="top:{top}"' if top else ""
    return (f'<div class="banner"{st}>{spinner()}<b>Reconnecting to baileyai, 22 s left</b><div class="bar"><i style="width:73%"></i></div>'
            f'<span class="d">Your blocks keep running on the server.</span>{btn("Retry now", "s", sm=True)}{btn("Stop trying", "g", sm=True)}</div>')


def banner_ok(top: str = "") -> str:
    st = f' style="top:{top}"' if top else ""
    return (f'<div class="banner"{st}>{ic("check", "verd", 1.8)}<b>Reconnected to baileyai</b>'
            f'<span class="d">This client took over your session (token confirmed).</span>{btn("Dismiss", "s", sm=True)}</div>')


def rail_zoom() -> str:
    return f'<div class="rail"><div class="rchip">{spinner()}2</div><div class="rchip">{ic("check", "verd", 1.8)}3</div></div>'


def f1(theme): return std(theme)
def f2(theme): return std(theme, shell=shell_block(badge="1"), agent=agent_block(badge="2"), idle=idle_block(badge="3"), rib=ribbon(active=True))
def f3a(theme): return std(theme, overlay=pop_takeover())
def f3b(theme):
    return std(theme, shell=shell_block(focus=False, input_html=input_box("", "", focus=False)),
               agent=agent_block(focus=True, hdr=agent_header("Paused", "You have input", "Give back", spinning=False)))
def f4(theme): return std(theme, overlay=pop_switcher())
def f5a(theme): return std(theme, dim=True, overlay=banner_reconnect(), rib=ribbon("Reconnecting to baileyai", dot="off"))
def f5b(theme): return std(theme, overlay=banner_ok())
def f6(theme):
    pop = (f'<div class="pop" style="right:176px;top:46px;width:340px"><div class="sw-h">Detached blocks</div><div class="prow" style="min-height:52px;padding-left:14px">'
           f'{ic("check", "verd", 1.8)}<div class="n"><b>cargo watch -x check</b><span>Idle, detached 4 min ago</span></div>{btn("Reattach", "p")}</div></div>')
    return frame(strip(1, 1) + blocks(shell_block() + f'<div class="col" style="flex:1">{agent_block(height=None)}</div>') + ribbon(blocks="2 blocks, 1 detached"), theme, pop)
def f7(theme):
    return frame(strip() + blocks(shell_block(width="1332px", hdr=shell_header(zoomed=True)) + rail_zoom()) + ribbon(), theme)


# ══ states.html ═════════════════════════════════════════════════════════════
def cells(*c): return "".join(f'<div class="cell">{x}</div>' for x in c)


def matrix(rows, heads=("Default", "Hover", "Focus", "Pressed", "Disabled")) -> str:
    h = '<div class="h"></div>' + "".join(f'<div class="h">{x}</div>' for x in heads)
    r = "".join(f'<div class="rn">{n}</div>{cells(*c)}' for n, c in rows)
    return f'<div class="mx">{h}{r}</div>'


def states_page() -> str:
    t = lambda: tab(2, "agents", T_AGENTS)
    tsel = lambda s="": tab(1, "build", T_BUILD, sel=True, state=s)
    foc = lambda x: f'<span class="is-focus" style="display:inline-flex;border-radius:6px">{x}</span>'
    dis = lambda x: f'<span class="is-disabled" style="display:inline-flex">{x}</span>'
    chip = lambda s="": f'<span class="chip {s}">{ic("zoom")}1 detached{ic("chevdown")}</span>'
    controls = matrix([
        ("Keycap", [kc("|"), kc("|", "is-hover"), foc(kc("|")), kc("|", "is-pressed"), dis(kc("|"))]),
        ("Keycap, active choice", [kc("z", act=True), kc("z", "is-hover", True), foc(kc("z", act=True)), kc("z", "is-pressed", True), dis(kc("z", act=True))]),
        ("Button, primary", [btn("Reattach", "p"), btn("Reattach", "p", "is-hover"), foc(btn("Reattach", "p")), btn("Reattach", "p", "is-pressed"), dis(btn("Reattach", "p"))]),
        ("Button, secondary", [btn("Cancel"), btn("Cancel", "s", "is-hover"), foc(btn("Cancel")), btn("Cancel", "s", "is-pressed"), dis(btn("Cancel"))]),
        ("Button, ghost", [btn("Stop trying", "g"), btn("Stop trying", "g", "is-hover"), foc(btn("Stop trying", "g")), btn("Stop trying", "g", "is-pressed"), dis(btn("Stop trying", "g"))]),
        ("Window tab, other", [t(), tab(2, "agents", T_AGENTS, state="is-hover"), tab(2, "agents", T_AGENTS, state="is-focus"), tab(2, "agents", T_AGENTS, state="is-hover"), dis(t())]),
        ("Window tab, selected", [tsel(), tsel("is-hover"), tsel("is-focus"), tsel(), dis(tsel())]),
        ("Chip", [chip(), chip("is-hover"), chip("is-focus"), chip("is-pressed"), dis(chip())]),
    ])
    sheet1 = (f'<div class="sheet" data-theme="dark"><h2>Control states</h2><p class="lbl" style="margin:-10px 0 20px">Hover is one surface step lighter, pressed one step more. '
              f'Focus is a 2 px verdigris ring with a 2 px gap, never removed. Disabled is 45% opacity. No new colours (D26).</p>{controls}</div>')

    inp = lambda **k: f'<div style="width:100%">{input_box(**k)}</div>'
    input_states = (f'<div class="sheet" data-theme="dark"><h2>Input states</h2><div style="display:grid;grid-template-columns:repeat(2,1fr);gap:8px 24px">'
                    f'<div><div class="lbl">Empty</div>{inp(typed="", ghost="", placeholder="Type a command")}</div>'
                    f'<div><div class="lbl">Typing, no suggestion</div>{inp(typed="cargo build -p rusty_ter", ghost="")}</div>'
                    f'<div><div class="lbl">Ghost suggestion (Tab accepts)</div>{inp()}</div>'
                    f'<div><div class="lbl">Disabled, agent has input</div>{inp(typed="", ghost="", focus=False, disabled=True, placeholder="claude has input")}</div>'
                    f'<div><div class="lbl">Keyboard focus ring</div><div class="is-focus" style="border-radius:9px">{input_box(focus=False, typed="git status", ghost="")}</div></div>'
                    f'<div><div class="lbl">Unfocused</div>{inp(typed="git status", ghost="", focus=False)}</div></div></div>')

    def mini(run_html): return f'<div class="surf" style="width:640px">{run_html}</div>'
    unfolded = run("cargo test -p rusty_term", "Passed", "6.2 s   09:14",
                   'running 48 tests\ntest zoom::dpi_reset ... ok\ntest zoom::text_only ... ok\ntest chrome::tabs ... ok\ntest result: <span class="v">ok</span>. 48 passed; 0 failed')
    runs = (f'<div class="sheet" data-theme="dark"><h2>Run states</h2><div style="display:grid;grid-template-columns:repeat(2,1fr);gap:20px 24px">'
            f'<div><div class="lbl">Running (spinner and word)</div>{mini(run("cargo build -p rusty_term", "Running", "3.1 s", "   Compiling rusty_term v0.4.0"))}</div>'
            f'<div><div class="lbl">Passed</div>{mini(RUN_STATUS)}</div>'
            f'<div><div class="lbl">Failed and selected (Copy, Rerun)</div>{mini(RUN_PUSH)}</div>'
            f'<div><div class="lbl">Folded</div>{mini(RUN_CLIPPY)}</div>'
            f'<div><div class="lbl">Unfolded</div>{mini(unfolded)}</div>'
            f'<div><div class="lbl">Hover</div>{mini(run("git status -sb", "Passed", "0.1 s   09:13", "## feat/zoom-text-only", state="is-hover"))}</div>'
            f'<div><div class="lbl">Keyboard focus</div>{mini(run("git status -sb", "Passed", "0.1 s   09:13", "## feat/zoom-text-only", state="is-focus"))}</div></div></div>')

    err_h = (f'<div class="bh"><span class="g">{ic("alert", "oxide", 1.6)}<b>claude</b><span class="bad" style="font-weight:500">stopped: error</span></span>'
             f'<span class="g">{btn("Show log", "s", sm=True)}{btn("Restart", "s", sm=True)}</span></div>')
    idle_h = f'<div class="bh"><span class="g">{ic("check", "verd", 1.8)}<b>cargo watch -x check</b><span class="s">Idle</span></span></div>'
    det_h = (f'<div class="bh"><span class="g">{ic("zoom", "slate", 1.4)}<b>cargo watch -x check</b><span class="s">Detached 4 min ago</span></span>{btn("Reattach", "p", sm=True)}</div>')
    hdr_specs = [("Focused shell", "focus", shell_header()), ("Unfocused shell", "", shell_header()), ("Agent has input", "", agent_header()),
                 ("You have input (focused)", "focus", agent_header("Paused", "You have input", "Give back", spinning=False)),
                 ("Agent errored", "", err_h), ("Idle", "", idle_h), ("Detached (in tray, with Reattach)", "", det_h),
                 ("Zoomed", "focus", shell_header(zoomed=True)), ("Disconnected (dimmed, D21)", "dim", shell_header())]
    hdrs = ('<div class="sheet" data-theme="dark"><h2>Block header states</h2><div style="display:grid;grid-template-columns:repeat(2,1fr);gap:16px 24px">'
            + "".join(f'<div><div class="lbl">{lab}</div><div class="block {c}" style="width:640px">{h}</div></div>' for lab, c, h in hdr_specs) + '</div></div>')

    tabs_s = (f'<div class="sheet" data-theme="dark"><h2>Window tab states</h2><div style="display:flex;gap:28px;align-items:center;flex-wrap:wrap">'
              f'<div><div class="lbl">Selected</div>{tsel()}</div><div><div class="lbl">Other</div>{t()}</div>'
              f'<div><div class="lbl">Activity (cell lights verdigris)</div>{tab(2, "agents", T_AGENTS)}</div>'
              f'<div><div class="lbl">Bell (glyph and dot, not colour alone)</div>{tab(3, "baileyai", T_ONE, bell=True)}</div>'
              f'<div><div class="lbl">Hover</div>{tab(3, "baileyai", T_ONE, state="is-hover")}</div></div></div>')

    def inline_banner(inner): return f'<div style="position:relative;height:44px;overflow:hidden">{inner}</div>'
    info = (f'<div class="banner" style="top:0">{ic("eye", "slate", 1.4)}<b>2 clients attached</b>'
            f'<span class="d">The other client is on this session too. Last to type has input.</span>{btn("Dismiss", "s", sm=True)}</div>')
    err = (f'<div class="banner err" style="top:0">{lead_alert()}<b>Can\'t reach baileyai</b>'
           f'<span class="d">Check the network and that rusty_mux is running there.</span>{btn("Retry", "s", sm=True)}{btn("Edit connection", "g", sm=True)}</div>')
    bans = ('<div class="sheet" data-theme="dark"><h2>Banner states</h2><div style="display:flex;flex-direction:column;gap:12px;width:1100px">'
            + "".join(f'<div class="lbl">{l}</div>{inline_banner(b)}' for l, b in [("Info", info), ("Reconnecting", banner_reconnect("0")), ("Success", banner_ok("0")), ("Error", err)])
            + '</div></div>')

    reduce = (f'<div class="sheet" data-theme="dark"><h2>Reduced motion (D16, D27)</h2><div style="display:grid;grid-template-columns:repeat(3,1fr);gap:20px 28px">'
              f'<div><div class="lbl">Default: spinner turns, cursor blinks, progress eases</div><div class="surf" style="display:flex;gap:18px;align-items:center">{spinner()}<span>Running 3.1 s</span><span class="cur"></span><div class="bar"><i style="width:73%"></i></div></div></div>'
              f'<div><div class="lbl">Reduced: static glyph with elapsed text, solid cursor, bar steps once a second</div><div class="surf" style="display:flex;gap:18px;align-items:center">{spinner(True)}<span>Running 3.1 s</span><span class="cur solid"></span><div class="bar"><i style="width:70%"></i></div></div></div>'
              f'<div><div class="lbl">Rules</div><div class="surf" style="font-size:12px;line-height:19px;color:var(--slate)">Follows the OS setting; override in Settings, Accessibility. No transitions. Motion only for state.</div></div></div></div>')

    # full frames
    second_agent = agent_block(height=None, hdr=agent_header("Reviewing zoom.rs", "Agent has input"), out=AGENT_OUT.replace("Edit ", "Read ").replace("  +12 -3", ""))
    w2_inner = f'<div style="display:flex;gap:8px;flex:1;min-width:0">{agent_block(height=None, focus=True)}{second_agent}</div>'
    w2 = frame(strip(2) + blocks(w2_inner) + ribbon(blocks="2 blocks"))
    w3 = frame(strip(3) + blocks(shell_block(width=None, hdr=shell_header(cwd="~/src/baileyai", branch="main", changes="clean"), runs=RUN_STATUS + RUN_TEST)) + ribbon(blocks="1 block"))
    new_hdr = shell_header(changes="no changes yet")
    new_block = shell_block(width=None, hdr=new_hdr, runs=EMPTY_RUNS, input_html=input_box("", "", placeholder="Type a command"))
    old_block = lambda w: shell_block(width=w, focus=False, runs=RUN_TEST + RUN_PUSH.replace(" sel", ""), input_html=input_box("", "", focus=False))
    split_r = frame(strip() + blocks(old_block("684px") + new_block) + ribbon(blocks="4 blocks"))
    top_col = (f'<div class="col" style="width:821px;flex:none">{shell_block(width=None, focus=False, runs=RUN_TEST, input_html=input_box("", "", focus=False))}{new_block}</div>')
    split_d = frame(strip() + blocks(top_col + f'<div class="col" style="flex:1">{agent_block()}{idle_block()}</div>') + ribbon(blocks="4 blocks"))
    close_pop = (f'<div class="pop pad" style="right:20px;top:512px;width:320px"><h4>Close this block?</h4><p>cargo watch is still running. Closing stops it.</p>'
                 f'<div class="acts">{btn("Cancel")}{btn("Close block")}</div></div>')
    close_f = std(overlay=close_pop)
    empty_runs = f'<div class="empty"><b>No runs yet</b><span>Type a command, or press {kc("Ctrl-]")} then {kc("|")} to split this window.</span></div>'
    empty_w = frame(strip(3) + blocks(shell_block(width=None, hdr=shell_header(cwd="~", branch="no repository", changes=""), runs=empty_runs,
                                                  input_html=input_box("", "", placeholder="Type a command"))) + ribbon(blocks="1 block"))

    def err_banner(title, detail, *acts):
        return f'<div class="banner err">{lead_alert()}<b>{title}</b><span class="d">{detail}</span>{"".join(acts)}</div>'
    off = ribbon("Not connected to baileyai", dot="bad")
    e1 = std(dim=True, overlay=err_banner("Can't reach baileyai", "Check the network and that rusty_mux is running there.", btn("Retry", "s", sm=True), btn("Edit connection", "g", sm=True)), rib=off)
    e2 = std(dim=True, overlay=err_banner("baileyai rejected this client's token", "The token expired or was revoked.", btn("Sign in again", "p", sm=True)), rib=off)
    gone = (f'<div class="blocks"><div class="block" style="flex:1"><div class="panel"><h3>{ic("alert", "oxide", 1.6)}Session rusty_mill ended</h3>'
            f'<p>The server closed it, or the last block exited. Your other sessions are still running.</p><div class="acts">{btn("Open switcher", "p")}</div></div></div></div>')
    e3 = frame(strip() + gone + ribbon("Connected to baileyai", blocks="0 blocks"))
    rail_oxide = '<div style="position:absolute;left:0;top:38px;bottom:0;width:3px;background:var(--oxide)"></div>'
    err_out = ('<span class="v">&gt; </span><span class="b">Edit </span>src/gui/zoom.rs  <span class="v">+12</span> <span class="o">-3</span>\n<span class="v">&gt; </span><span class="b">Run  </span>cargo check -p rusty_term\n'
               '<span class="o">error: </span><span class="b">agent process exited with code 1</span>')
    e4 = std(agent=agent_block(hdr=err_h, out=err_out, extra=rail_oxide))

    return page("States and variants (dark)", "Every control state, every domain state, other windows, split and close flows, empty window, errors. Tokens D5-D8, D17-D30.", [
        item("1. Control states", sheet1), item("2. Input states", input_states), item("3. Run states", runs),
        item("4. Block header states", hdrs), item("5. Window tab states", tabs_s), item("6. Banner states", bans),
        item("7. Reduced motion", reduce),
        item("W2 agents window", w2, "Window 2: two agent blocks side by side. Tab 2 selected; its thumbnail shows both cells lit."),
        item("W3 baileyai window", w3, "Window 3: one shell block on another folder, so the header differs (D9)."),
        item("Split right result", split_r, "A new empty block takes focus on the right of the shell block; its input is ready. The server owns the layout (D2)."),
        item("Split down result", split_d, "The shell block's column splits; the new block is below and focused."),
        item("Close block, confirm", close_f, "A confirm only appears when a run is active. An idle block closes without asking."),
        item("Empty window", empty_w, "One block, no runs, one hint line."),
        item("Error: can't reach host", e1), item("Error: token rejected", e2), item("Error: session ended", e3),
        item("Error: agent stopped", e4, "Agent errored. Status wording depends on open question Q5."),
    ])


# ══ light.html ══════════════════════════════════════════════════════════════
def light_page() -> str:
    def sw(n, hx, role):
        return (f'<div style="display:flex;flex-direction:column;gap:6px;width:120px"><div style="height:64px;border-radius:9px;background:var(--{n});border:1px solid var(--line)"></div>'
                f'<b style="font-size:13px">{n}</b><span class="mono" style="font-size:12px;color:var(--slate)">{hx}</span><span style="font-size:11px;color:var(--slate)">{role}</span></div>')
    toks = [("frame", "#D3DBDA", "window"), ("base", "#EEF2F1", "block"), ("raised", "#F8FAF9", "popover"), ("sunk", "#E1E8E7", "input"), ("line", "#B9C4C3", "border"),
            ("bone", "#182024", "ink"), ("slate", "#4A585D", "secondary"), ("dim", "#76848A", "ghost, icons"), ("verd", "#176B5A", "ok, focus"), ("oxide", "#A8460F", "failure")]
    sheet = (f'<div class="sheet" data-theme="light"><h2>Light tokens and components (D25)</h2><div style="display:flex;gap:12px;margin-bottom:28px">{"".join(sw(*t) for t in toks)}</div>'
             f'<div style="display:flex;gap:40px;align-items:center;margin-bottom:28px">{kc("|")}{kc("z", act=True)}{thumb(T_BUILD)}{tab(1, "build", T_BUILD, sel=True)}{tab(2, "agents", T_AGENTS)}'
             f'{btn("Reattach", "p")}{btn("Cancel")}{btn("Stop trying", "g")}</div>'
             f'<div style="display:grid;grid-template-columns:repeat(2,1fr);gap:20px 24px"><div class="surf" style="width:640px">{RUN_STATUS}</div><div class="surf" style="width:640px">{RUN_PUSH}</div>'
             f'<div style="width:640px">{input_box()}</div><div style="display:flex;flex-direction:column;gap:8px;width:640px">'
             f'<div class="block focus">{shell_header()}</div><div class="block">{agent_header()}</div></div></div></div>')
    note = ("Contrast on base: ink 14.6, slate 6.5, verdigris text 5.7, oxide text 5.2. Oxide text never sits on the frame colour (4.2). "
            "dim (2.7 to 3.7) is for the greyed-out suggestion and non-text icons only. Text on a verdigris fill is raised/sunk (5.1 to 6.1).")
    items = [item("L-F8 Light tokens and components", sheet, note)]
    for cap, f in [("L-F1 Rest state", f1), ("L-F2 Prefix mode", f2), ("L-F3a Take over, confirm", f3a), ("L-F3b Take over, after", f3b), ("L-F4 Session switcher", f4),
                   ("L-F5a Reconnecting", f5a), ("L-F5b Reconnected", f5b), ("L-F6 Detached block", f6), ("L-F7 Zoomed block", f7)]:
        items.append(item(cap, f("light")))
    return page("Light theme", "Same layout and components as dark; tokens re-pointed (D24, D25). Answers open question Q4.", items)


# ══ alt-layouts.html ════════════════════════════════════════════════════════
DOT = '<i style="width:7px;height:7px;border-radius:50%;background:var(--verd);display:inline-block"></i>'


def side_tree() -> str:
    def win(n, name, cells_, sel=False):
        return f'<div class="node{" sel" if sel else ""}">{thumb(cells_)}<span style="white-space:pre">{n}  {name}</span></div>'
    def child(glyph, title, sub): return f'<div class="node child">{glyph}<div class="n"><b>{title}</b><span>{sub}</span></div></div>'
    return (f'<div class="side"><div class="dots" style="padding:6px 14px 10px;margin:0"><i></i><i></i><i></i></div>'
            f'<div class="sh">{ic("server", "slate")}<b>rusty_mill</b><span>on baileyai</span>{ic("chevdown", "slate")}</div>'
            f'{win(1, "build", T_BUILD, True)}'
            f'{child(ic("cross", "oxide", 1.8), "~/src/rusty_mill", "Last run exit 1")}'
            f'{child(spinner(), "claude", "Running, agent has input")}'
            f'{child(ic("check", "verd", 1.8), "cargo watch -x check", "Idle")}'
            f'{win(2, "agents", T_AGENTS)}{win(3, "baileyai", T_ONE)}'
            f'<div class="node">{ic("plus", "dim")}<span>New window</span></div>'
            f'<div class="foot"><div>{DOT}Connected to baileyai</div><div>{ic("clients")}2 clients attached</div></div></div>')


def side_rail() -> str:
    return (f'<div class="side coll"><div class="dots" style="margin:4px 0 8px;flex-direction:column;gap:5px"><i></i><i></i><i></i></div>'
            f'<div class="rbtn">{ic("server")}</div><div class="rbtn sel">{thumb(T_BUILD)}</div><div class="rbtn">{thumb(T_AGENTS)}</div><div class="rbtn">{thumb(T_ONE)}</div>'
            f'<div class="rbtn">{ic("plus", "dim")}</div><div style="margin-top:auto" class="rbtn">{ic("clients")}</div><div class="rbtn">{ic("sidebar")}</div></div>')


def alt_a(collapsed=False) -> str:
    sw = 1392 - (44 if collapsed else 240) - 16 - 8
    inner = shell_block(width=f"{round(sw * .6)}px") + f'<div class="col" style="flex:1">{agent_block()}{idle_block()}</div>'
    r = ribbon(hints=f'<span>{kc("Ctrl-]")}then</span><span>{kc("|")}Split right</span><span>{kc("z")}Zoom</span><span>{kc("d")}Detach</span>')
    return frame(f'<div class="hbody">{side_rail() if collapsed else side_tree()}<div class="main">{blocks(inner)}{r}</div></div>')


def dock() -> str:
    return (f'<div class="dock"><div class="l"><div class="dots" style="margin:0 20px 0 6px"><i></i><i></i><i></i></div>'
            f'<span class="sess" style="font-size:12px">{ic("server")}<b style="color:var(--bone)">rusty_mill</b><span>on baileyai</span>{ic("chevup")}</span><div class="vdiv" style="height:20px"></div>'
            f'{tab(1, "build", T_BUILD, sel=True)}{tab(2, "agents", T_AGENTS)}{tab(3, "baileyai", T_ONE)}<span class="plus">{ic("plus")}</span></div>'
            f'<div class="r"><span>{DOT}Connected to baileyai</span>'
            f'<span>{ic("clients")}2 clients attached</span><span>{kc("Ctrl-]")}then</span><span>{kc("|")}Split right</span><span>{kc("z")}Zoom</span><span>{kc("d")}Detach</span></div></div>')


def alt_b(open_sw=False) -> str:
    inner = shell_block() + f'<div class="col" style="flex:1">{agent_block(height="500px")}{idle_block()}</div>'
    return frame(f'<div class="main" style="flex:1">{blocks(inner)}{dock()}</div>', "dark", pop_switcher(up=True) if open_sw else "")


def alt_page() -> str:
    tbl = ('<table class="cmp"><tr><th></th><th>v2 top strip (default)</th><th>Alt A: sidebar tree</th><th>Alt B: bottom dock</th></tr>'
           '<tr><td>Width cost</td><td>None</td><td>240 px (44 px collapsed)</td><td>None</td></tr>'
           '<tr><td>Vertical space</td><td>52 px strip plus 34 px ribbon</td><td>34 px ribbon only</td><td>36 px dock only (most)</td></tr>'
           '<tr><td>Many windows and blocks</td><td>Tabs overflow past about 8</td><td>Scales: tree scrolls, names stay visible</td><td>Crowded at 6 or more windows</td></tr>'
           '<tr><td>Block status at a glance</td><td>Header per block, thumbnail per window</td><td>Words and glyph per block in the tree</td><td>Header per block only</td></tr>'
           '<tr><td>Key hint discoverability</td><td>Ribbon, always visible</td><td>Ribbon, shortened</td><td>Shares the dock with tabs</td></tr>'
           '<tr><td>Switcher</td><td>Drops down</td><td>Tree head, drops down</td><td>Opens upward</td></tr>'
           '<tr><td>Recommendation</td><td>Default</td><td>Optional: Settings, Interface, Layout</td><td>Documented only</td></tr></table>')
    return page("Alternative layouts", "Same content and model (D2) in three arrangements (D29). Keep the top strip as default; offer the sidebar as an option.", [
        item("Alt A: sidebar tree", alt_a(), "Session, windows and blocks as a tree with status glyph plus words. Clients and connection at the foot."),
        item("Alt A: collapsed icon rail", alt_a(True), "44 px rail: session, window thumbnails, new window, clients, expand."),
        item("Alt B: bottom dock", alt_b(), "Tabs and hints merge into one 36 px bar; blocks get the full height."),
        item("Alt B: session switcher open", alt_b(True), "The switcher opens upward from the dock."),
        item("Comparison", f'<div class="sheet" data-theme="dark" style="width:1180px">{tbl}</div>')])


# ══ settings.html ═══════════════════════════════════════════════════════════
def seg(*opts, on=0, cls=""): return f'<span class="seg {cls}">' + "".join(f'<span class="{"on" if i == on else ""}">{o}</span>' for i, o in enumerate(opts)) + '</span>'
def tog(on=False, cls=""): return f'<span class="tog {"on" if on else ""} {cls}"></span>'
def sel_c(v, cls=""): return f'<span class="sel-c {cls}">{v}{ic("chevdown")}</span>'
def fld(v, cls="", mono=False): return f'<span class="fld {"mono" if mono else ""} {cls}">{v}</span>'
def step(v, cls=""): return f'<span class="step {cls}"><span>-</span><span class="val">{v}</span><span>+</span></span>'
def ro(v): return f'<span class="ro">{ic("lock", "slate", 1.3)}<span class="lockv">{v}</span></span>'


PROPOSED = '<span class="tag">Proposed</span>'


def srow(name, desc, ctl, prop=False):
    return f'<div class="srow"><div class="n"><b>{name}{PROPOSED if prop else ""}</b><span>{desc}</span></div><div class="ctl">{ctl}</div></div>'


SECTIONS = ["Interface", "Terminal", "Keys", "Agents", "Connections", "Accessibility"]


def settings_frame(section: str, content: str, sub: str) -> str:
    nav = "".join(f'<a class="{"sel is-focus" if s == section else ""}">{s}</a>' for s in SECTIONS)
    panel = (f'<div class="setwrap"><div class="setpanel"><div class="snav"><h5>Settings</h5>{nav}<div class="foot">{kc("Esc")}to close</div></div>'
             f'<div class="scont"><h2>{section}</h2><div class="sub">{sub}</div>{content}'
             f'<div class="sfoot">{btn("Reset to default", "g")}<span class="sl" style="font-size:12px">Changes apply as you make them.</span></div></div></div></div>')
    return std(overlay=panel)


def settings_page() -> str:
    iface = (srow("Theme", "Dark, light, or follow the system.", seg("Dark", "Light", "System", on=2))
             + srow("Layout", "Where windows and sessions live. The sidebar costs 240 px of width.", seg("Top strip", "Sidebar", on=0))
             + srow("Interface font", "Used by the bars and menus. The terminal has its own font.", sel_c("Instrument Sans")))
    term = (srow("Font", "Used for terminal text only.", sel_c("JetBrains Mono"))
            + srow("Size", "Terminal text size.", step("14 px"))
            + srow("Zoom", "Zoom scales terminal text only. The interface stays the same size.", f'<span class="ro">{kc("Ctrl+=")}{kc("Ctrl+-")}{kc("Ctrl+0")}<span>Text only</span></span>')
            + srow("Scrollback", "Lines kept per block.", step("10,000"), True)
            + srow("Fold long runs", "Fold output past this many lines. Failures stay visible.", step("40 lines"), True))
    keylist = "".join(f'<div>{kc(k)}{l}</div>' for k, l in [("d", "Detach"), ("h j k l", "Move focus"), ("1-9", "Jump to block"), ("z", "Zoom"), ("|", "Split right"), ("-", "Split down"), ("x", "Close block")])
    keys = (srow("Prefix key", "Press this first, then a layout key. On its own it does nothing.", f'<span class="kedit">{kc("Ctrl-]", act=True)}{btn("Change", "s", sm=True)}</span>')
            + f'<div class="keylist">{keylist}</div>'
            + f'<div class="srow" style="border:none"><div class="n"><b>Rebind individual keys{PROPOSED}</b><span>Not available yet.</span></div></div>')
    agents = (srow("Show who has input", "Each agent block says whether the agent or you has input.", tog(True))
              + srow("Confirm before Take over", "Ask first, or take input at once. Open question: which default.", seg("Ask first", "Act at once", on=0))
              + f'<div class="hostrow" style="margin-top:14px">{ic("eye", "slate", 1.3)}<div class="n"><b>Input ownership only</b><span>Taking over changes who types. It does not change resize or terminal control.</span></div>{PROPOSED}</div>')
    conns = (f'<div class="hostrow">{ic("server", "slate")}<div class="n"><b>baileyai</b><span>baileyai, remote</span></div><span class="tokst ok">{ic("check", "verd", 1.8)}Token confirmed</span>{btn("Edit", "s", sm=True)}</div>'
             f'<div class="hostrow">{ic("server", "slate")}<div class="n"><b>local</b><span>This machine</span></div><span class="tokst sl">No token needed</span>{btn("Edit", "s", sm=True)}</div>'
             + srow("Reconnect grace", "How long the server keeps your place after a drop. Set by rusty_mux.", ro("30 s")))
    a11y = (srow("Reduce motion", "Stops the spinner and cursor blink. System follows your OS setting.", seg("System", "On", "Off", on=0))
            + srow("Focus ring", "Always drawn on keyboard focus.", ro("Always on"))
            + srow("Status cues", "Every status has an icon and a word, never colour alone.", ro("Always on"))
            + srow("High contrast", "Stronger borders and text.", tog(False), True))
    frames = [("S1 Interface", "Interface", iface, "Theme, layout and the interface font."), ("S2 Terminal", "Terminal", term, "Font and behaviour of terminal text."),
              ("S3 Keys", "Keys", keys, "Layout keys, behind the prefix."), ("S4 Agents", "Agents", agents, "How you and agents share a block."),
              ("S5 Connections", "Connections", conns, "Hosts this client can reach."), ("S6 Accessibility", "Accessibility", a11y, "Motion, contrast and status cues.")]
    items = [item(c, settings_frame(s, ct, sb)) for c, s, ct, sb in frames]
    heads = ("Default", "Hover", "Focus", "Disabled")
    variants = lambda f: [f(""), f("is-hover"), f("is-focus"), f("is-disabled")]

    def mxs(rows):
        return ('<div class="mx" style="grid-template-columns:200px repeat(4,1fr)"><div></div>' + "".join(f'<div class="h">{h}</div>' for h in heads)
                + "".join(f'<div class="rn">{n}</div>{cells(*c)}' for n, c in rows) + '</div>')
    kedit = lambda s="", d="": f'<span class="kedit {d}">{kc("Ctrl-]", act=True)}{btn("Change", "s", s, True)}</span>'
    ctl = mxs([("Segmented control", variants(lambda c: seg("Dark", "Light", "System", on=2, cls=c))), ("Toggle, on", variants(lambda c: tog(True, c))), ("Toggle, off", variants(lambda c: tog(False, c))),
               ("Select", variants(lambda c: sel_c("Instrument Sans", c))), ("Text field", variants(lambda c: fld("baileyai", c))),
               ("Text field, mono", variants(lambda c: fld("Ctrl-]", c, True))), ("Stepper", variants(lambda c: step("14 px", c))),
               ("Keycap editor", [kedit(), kedit("is-hover"), f'<span class="kedit">{kc("Ctrl-]", act=True)}<span class="is-focus" style="display:inline-flex;border-radius:6px">{btn("Change", "s", sm=True)}</span></span>', kedit(d="is-disabled")]),
               ("Read-only value", [ro("30 s")] * 4)])
    items.append(item("S7 Controls", f'<div class="sheet" data-theme="dark"><h2>Setting controls and their states</h2>{ctl}</div>',
                      "Each input kind in default, hover, focus and disabled. Read-only values show a lock and never change."))
    return page("Settings and options", "A full-window overlay: 200 px nav, content up to 640 px, Esc closes, changes apply live, Reset to default per section (D30). Speculative items carry a Proposed tag.", items)


def index_page() -> str:
    links = [("states.html", "States and variants (dark)"), ("light.html", "Light theme"), ("alt-layouts.html", "Alternative layouts"), ("settings.html", "Settings and options"),
             ("terminal-mux.svg", "v2 concept (original)")]
    li = "".join(f'<li style="margin:8px 0"><a href="{h}" style="font-size:16px">{t}</a></li>' for h, t in links)
    return ('<!doctype html><meta charset="utf-8"><title>rusty_term GUI design</title><body style="font-family:Instrument Sans,system-ui;padding:40px">'
            f'<h1>rusty_term GUI design</h1><p>Standalone pages (Paper hit its weekly MCP limit). F1 to F8 are in the Paper file.</p><ul>{li}</ul></body>')


def main() -> None:
    for name, html in [("states.html", states_page()), ("light.html", light_page()), ("alt-layouts.html", alt_page()),
                       ("settings.html", settings_page()), ("index.html", index_page())]:
        (OUT / name).write_text(html, encoding="utf-8")
        print(name, len(html) // 1024, "KB")


if __name__ == "__main__":
    main()
