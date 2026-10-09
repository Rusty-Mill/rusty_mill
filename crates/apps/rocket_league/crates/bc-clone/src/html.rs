//! Render a [`BcReplayDoc`] (+ the canonical grid for the heatmaps) as a
//! **self-contained, ballchasing-style HTML dashboard** — no JS framework, no
//! external assets (CSS-only tabs, inline bars + SVG heatmaps). Mirrors
//! ballchasing's replay-page tabs (Overview / Core / Ball / Boost / Movement /
//! Positioning / Heatmaps / Demos) over the stats we emit; see
//! `docs/ballchasing-dashboard-spec.md` for the field-by-field coverage.

use crate::{BcReplayDoc, Player, Side};
use replay_analyzer::analyze::boost_pads::pad_pickups;
use replay_analyzer::field::{BACK_WALL_Y, BIG_BOOST_PADS, SIDE_WALL_X, SMALL_BOOST_PADS};
use replay_analyzer::model::{CanonicalMatch, Event, StatKind};
use std::collections::HashMap;
use std::fmt::Write;

const BLUE: &str = "#3a8ee6";
const ORANGE: &str = "#e8643c";

/// Field-position samples for one entity (world `(x, y)`, uu).
type Pts = Vec<(f32, f32)>;

/// Per-player car positions over the resampled grid, keyed by player name, plus
/// the ball positions — the substrate for the heatmaps.
fn positions(m: &CanonicalMatch) -> (HashMap<String, Pts>, Pts) {
    let pri_name: HashMap<i32, &str> = m
        .tracks
        .iter()
        .map(|t| (t.pri, t.player.as_str()))
        .collect();
    let mut by_player: HashMap<String, Pts> = HashMap::new();
    let mut ball: Pts = Vec::new();
    for f in &m.resampled.frames {
        if let Some(b) = &f.ball {
            ball.push((b.p.x, b.p.y));
        }
        for c in &f.cars {
            if let Some(n) = pri_name.get(&c.pri) {
                by_player
                    .entry(n.to_string())
                    .or_default()
                    .push((c.p.x, c.p.y));
            }
        }
    }
    (by_player, ball)
}

/// A coarse density heatmap of field positions as an inline SVG. The field is
/// drawn **landscape** (goal-to-goal along the long axis = world `y`; sidelines =
/// world `x`), matching ballchasing's diagrams; cells are tinted green by density.
fn heatmap_svg(pts: &Pts) -> String {
    const NX: usize = 26; // cells along the goal axis (world y)
    const NY: usize = 20; // cells along the sideline (world x)
    const W: f32 = 312.0;
    const H: f32 = 240.0;
    let mut grid = vec![0u32; NX * NY];
    let mut max = 1u32;
    for &(x, y) in pts {
        let cx = (((y + BACK_WALL_Y) / (2.0 * BACK_WALL_Y)) * NX as f32).clamp(0.0, NX as f32 - 1.0)
            as usize;
        let cy = (((x + SIDE_WALL_X) / (2.0 * SIDE_WALL_X)) * NY as f32).clamp(0.0, NY as f32 - 1.0)
            as usize;
        let i = cy * NX + cx;
        grid[i] += 1;
        max = max.max(grid[i]);
    }
    let (cw, ch) = (W / NX as f32, H / NY as f32);
    let mut cells = String::new();
    for (i, &c) in grid.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let (cx, cy) = (i % NX, i / NX);
        let op = (c as f32 / max as f32).powf(0.6); // gamma so low cells still show
        let _ = write!(
            cells,
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{cw:.1}\" height=\"{ch:.1}\" fill=\"#46c463\" opacity=\"{op:.2}\"/>",
            cx as f32 * cw,
            cy as f32 * ch,
        );
    }
    format!(
        "<svg viewBox=\"0 0 {W} {H}\" class=\"hm\"><rect width=\"{W}\" height=\"{H}\" fill=\"#eef4fb\"/>\
         {cells}<line x1=\"{mid:.1}\" y1=\"0\" x2=\"{mid:.1}\" y2=\"{H}\" stroke=\"#c4d2e0\"/>\
         <rect width=\"{W}\" height=\"{H}\" fill=\"none\" stroke=\"#aebfce\"/></svg>",
        mid = W / 2.0,
    )
}

/// World `(x, y)` → SVG `(cx, cy)` on the landscape field (goal axis = world `y`).
fn field_to_svg(px: f32, py: f32, w: f32, h: f32) -> (f32, f32) {
    (
        (py + BACK_WALL_Y) / (2.0 * BACK_WALL_Y) * w,
        (px + SIDE_WALL_X) / (2.0 * SIDE_WALL_X) * h,
    )
}

/// Per-player pad-pickup counts keyed by player name then pad `(x, y)` (rounded).
/// Uses the gauge-step pad attribution (`analyze::boost_pads`).
fn pickup_counts(m: &CanonicalMatch) -> HashMap<String, HashMap<(i32, i32), u32>> {
    let signs = &m.resampled.team_attack_sign;
    let mut out = HashMap::new();
    for t in &m.tracks {
        let sign = t.team.and_then(|tm| signs.get(&tm).copied()).unwrap_or(1);
        let mut c: HashMap<(i32, i32), u32> = HashMap::new();
        for p in pad_pickups(t, sign) {
            *c.entry((p.pad.0 as i32, p.pad.1 as i32)).or_default() += 1;
        }
        out.insert(t.player.clone(), c);
    }
    out
}

/// A boost pickup-map SVG: every pad drawn at its location, tinted/labelled by how
/// many times this player collected it (big pads larger).
fn pickup_map_svg(counts: &HashMap<(i32, i32), u32>) -> String {
    const W: f32 = 312.0;
    const H: f32 = 240.0;
    let max = counts.values().copied().max().unwrap_or(1).max(1);
    let mut marks = String::new();
    for (pads, big) in [(&BIG_BOOST_PADS[..], true), (&SMALL_BOOST_PADS[..], false)] {
        for &(px, py) in pads {
            let c = counts.get(&(px as i32, py as i32)).copied().unwrap_or(0);
            let (cx, cy) = field_to_svg(px, py, W, H);
            let r = if big { 6.0 } else { 4.0 };
            if c == 0 {
                let _ = write!(
                    marks,
                    "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"#d2dde8\"/>"
                );
            } else {
                let op = 0.35 + 0.65 * (c as f32 / max as f32);
                let _ = write!(
                    marks,
                    "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\" fill=\"#2f9e4f\" opacity=\"{op:.2}\"/>\
                     <text x=\"{cx:.1}\" y=\"{:.1}\" class=\"padc\">{c}</text>",
                    r + 2.0,
                    cy + 3.0,
                );
            }
        }
    }
    format!(
        "<svg viewBox=\"0 0 {W} {H}\" class=\"hm\"><rect width=\"{W}\" height=\"{H}\" fill=\"#eef4fb\"/>\
         <line x1=\"{mid:.1}\" y1=\"0\" x2=\"{mid:.1}\" y2=\"{H}\" stroke=\"#c4d2e0\"/>{marks}\
         <rect width=\"{W}\" height=\"{H}\" fill=\"none\" stroke=\"#aebfce\"/></svg>",
        mid = W / 2.0,
    )
}

/// The "Pickup maps" section: a per-player grid of pickup-map SVGs.
fn pickup_maps_section(
    doc: &BcReplayDoc,
    counts: &HashMap<String, HashMap<(i32, i32), u32>>,
) -> String {
    let mut cards = String::new();
    for (p, team) in all_players(doc) {
        let empty = HashMap::new();
        let svg = pickup_map_svg(counts.get(&p.name).unwrap_or(&empty));
        let _ = write!(
            cards,
            "<div class=\"hm-card\"><div class=\"hm-name {team}\">{}</div>{svg}</div>",
            esc(&p.name),
        );
    }
    format!(
        "<h3>Pickup maps</h3><p class=\"note\">Boost pads each player collected (count per pad; \
         big pads drawn larger). Inferred from boost-gauge gains.</p><div class=\"hm-grid\">{cards}</div>"
    )
}

/// Escape the few characters that matter inside HTML text / attributes.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn secs(v: f32) -> String {
    format!("{v:.1}s")
}
fn pct(v: f32) -> String {
    format!("{v:.1}%")
}
fn n0(v: f32) -> String {
    format!("{}", v.round() as i64)
}

/// A team-vs-team split bar (blue left, orange right), proportional to the values.
fn compare_bar(label: &str, blue: f32, orange: f32, fmt: fn(f32) -> String) -> String {
    let total = (blue + orange).abs().max(1e-6);
    let bw = (blue / total * 100.0).clamp(0.0, 100.0);
    let ow = 100.0 - bw;
    format!(
        "<div class=\"cmp\"><div class=\"cmp-l\">{bl}</div>\
         <div class=\"cmp-bar\"><span class=\"b\" style=\"width:{bw:.1}%\"></span>\
         <span class=\"o\" style=\"width:{ow:.1}%\"></span></div>\
         <div class=\"cmp-r\">{ol}</div>\
         <div class=\"cmp-lab\">{label}</div></div>",
        bl = fmt(blue),
        ol = fmt(orange),
        label = esc(label),
    )
}

/// A per-player horizontal bar group (each bar scaled to the group max), colored
/// by team — ballchasing's "Players overview" style.
fn player_bars(
    title: &str,
    doc: &BcReplayDoc,
    val: fn(&Player) -> f32,
    fmt: fn(f32) -> String,
) -> String {
    let players: Vec<(&Player, &str)> = doc
        .blue
        .players
        .iter()
        .map(|p| (p, BLUE))
        .chain(doc.orange.players.iter().map(|p| (p, ORANGE)))
        .collect();
    let max = players
        .iter()
        .map(|(p, _)| val(p))
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let mut rows = String::new();
    for (p, color) in &players {
        let v = val(p);
        let w = (v / max * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            rows,
            "<div class=\"pb\"><div class=\"pb-n\">{n}</div>\
             <div class=\"pb-bar\"><span style=\"width:{w:.1}%;background:{color}\"></span></div>\
             <div class=\"pb-v\">{v}</div></div>",
            n = esc(&p.name),
            v = fmt(v),
        );
    }
    format!("<div class=\"chart\"><h4>{}</h4>{rows}</div>", esc(title))
}

/// Render `rows` (header + body cells) as a table with a caption.
fn table(caption: &str, head: &[&str], body: Vec<Vec<String>>) -> String {
    let mut h = String::new();
    for c in head {
        let _ = write!(h, "<th>{}</th>", esc(c));
    }
    let mut b = String::new();
    for row in body {
        b.push_str("<tr>");
        for (i, cell) in row.iter().enumerate() {
            let tag = if i == 0 { "th" } else { "td" };
            let _ = write!(b, "<{tag}>{cell}</{tag}>");
        }
        b.push_str("</tr>");
    }
    format!("<div class=\"tbl\"><h4>{}</h4><table><thead><tr>{h}</tr></thead><tbody>{b}</tbody></table></div>", esc(caption))
}

/// All players (blue then orange) with a team-color class for the name cell.
fn all_players(doc: &BcReplayDoc) -> Vec<(&Player, &'static str)> {
    doc.blue
        .players
        .iter()
        .map(|p| (p, "blue"))
        .chain(doc.orange.players.iter().map(|p| (p, "orange")))
        .collect()
}

fn named(p: &Player, team: &str) -> String {
    format!("<span class=\"{team}\">{}</span>", esc(&p.name))
}

/// The full dashboard HTML for a decoded replay (`m` supplies the positional grid
/// for the heatmaps; the rest comes from the ballchasing-shaped `doc`).
pub fn render_html(doc: &BcReplayDoc, m: &CanonicalMatch) -> String {
    let title = esc(&doc.id);
    let map = esc(doc.map_name.as_deref().unwrap_or("?"));
    let score = format!(
        "{} – {}",
        doc.blue.stats.core.goals, doc.orange.stats.core.goals
    );

    let (by_player, ball_pts) = positions(m);
    let pickups = pickup_counts(m);
    let overview = overview_tab(doc, m);
    let core = core_tab(doc);
    let ball = ball_tab(doc, &heatmap_svg(&ball_pts));
    let boost = boost_tab(doc, &pickup_maps_section(doc, &pickups));
    let movement = movement_tab(doc);
    let positioning = positioning_tab(doc);
    let heatmaps = heatmaps_tab(doc, &by_player);
    let demos = demos_tab(doc);

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} — replay stats</title>
<style>{css}</style></head><body>
<header><h1>{title}</h1>
<div class="meta"><span>{map}</span><span>{ts}v{ts}</span><span class="score">{score}</span><span>{dur}s</span></div></header>
<main>
<input type="radio" name="tab" id="t-ov" checked>
<input type="radio" name="tab" id="t-co">
<input type="radio" name="tab" id="t-ba">
<input type="radio" name="tab" id="t-bo">
<input type="radio" name="tab" id="t-mo">
<input type="radio" name="tab" id="t-po">
<input type="radio" name="tab" id="t-he">
<input type="radio" name="tab" id="t-de">
<nav class="tabbar">
<label for="t-ov">Overview</label><label for="t-co">Core</label><label for="t-ba">Ball</label><label for="t-bo">Boost</label>
<label for="t-mo">Movement</label><label for="t-po">Positioning</label><label for="t-he">Heatmaps</label><label for="t-de">Demos</label>
</nav>
<section class="panel" id="p-ov">{overview}</section>
<section class="panel" id="p-co">{core}</section>
<section class="panel" id="p-ba">{ball}</section>
<section class="panel" id="p-bo">{boost}</section>
<section class="panel" id="p-mo">{movement}</section>
<section class="panel" id="p-po">{positioning}</section>
<section class="panel" id="p-he">{heatmaps}</section>
<section class="panel" id="p-de">{demos}</section>
</main>
<footer>Generated by <code>bc-clone</code> — ballchasing-shaped stats from a Rocket League replay.
Values validated to within a few % of ballchasing (see <code>docs/ballchasing-comparison.md</code>).</footer>
</body></html>"#,
        ts = doc.team_size.unwrap_or(0),
        dur = doc.duration,
        css = CSS,
    )
}

fn scoreboard_block(side: &Side, team: &str) -> String {
    let mut rows = String::new();
    for p in &side.players {
        let c = &p.stats.core;
        let star = if c.mvp { " ★" } else { "" };
        let car = p.car_name.as_deref().unwrap_or("—");
        let _ =
            write!(
            rows,
            "<tr><th>{name}{star}</th><td class=\"car\">{car}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            c.score, c.goals, c.assists, c.saves, c.shots,
            name = esc(&p.name), star = star, car = esc(car),
        );
    }
    format!(
        "<table class=\"score {team}\"><thead><tr><th class=\"teamhdr\">{g}</th>\
         <th>CAR</th><th>SCORE</th><th>GOALS</th><th>ASSISTS</th><th>SAVES</th><th>SHOTS</th></tr></thead>\
         <tbody>{rows}</tbody></table>",
        g = side.stats.core.goals,
    )
}

fn ball_tab(doc: &BcReplayDoc, ball_heatmap: &str) -> String {
    format!(
        "<h3>Possession</h3><div class=\"cmp-grid\">{poss}</div>\
         <p class=\"note\">Time each team controlled the ball (from a team's touch until the opponent's).</p>\
         <h3>Pressure — time the ball is in a side</h3><div class=\"cmp-grid\">{pres}</div>\
         <p class=\"note\">Share of time the ball sat in each team's own half (lower = more pressure on the opponent). Approximate — sampled over ball-present frames.</p>\
         <h3>Ball heatmap</h3><div class=\"hm-wrap\">{ball_heatmap}</div>",
        poss = compare_bar("Possession", doc.possession.blue, doc.possession.orange, pct),
        pres = compare_bar("Pressure", doc.pressure.blue, doc.pressure.orange, pct),
    )
}

fn heatmaps_tab(doc: &BcReplayDoc, by_player: &HashMap<String, Pts>) -> String {
    let mut cards = String::new();
    for (p, team) in all_players(doc) {
        let svg = by_player.get(&p.name).map(heatmap_svg).unwrap_or_default();
        let _ = write!(
            cards,
            "<div class=\"hm-card\"><div class=\"hm-name {team}\">{}</div>{svg}</div>",
            esc(&p.name),
        );
    }
    format!(
        "<h3>Positioning heatmaps</h3><p class=\"note\">Where each car spent its time \
         (goal-to-goal is horizontal; the vertical line is midfield).</p>\
         <div class=\"hm-grid\">{cards}</div>"
    )
}

/// `t` seconds → a compact `m:ss` clock label.
fn mmss(t: f32) -> String {
    let t = t.max(0.0) as i32;
    format!("{}:{:02}", t / 60, t % 60)
}

/// One Game-Timeline glyph (per event type) centered at `(cx, cy)`.
fn timeline_glyph(kind: &str, cx: f32, cy: f32) -> String {
    match kind {
        "goal" => format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"4.4\" fill=\"#f2b01e\" stroke=\"#fff\" stroke-width=\"1\"/>"
        ),
        "shot" => format!("<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"2.8\" fill=\"#3a8ee6\"/>"),
        "save" => format!(
            "<polygon points=\"{cx:.1},{a:.1} {b:.1},{cy:.1} {cx:.1},{c:.1} {d:.1},{cy:.1}\" fill=\"#2f9e4f\"/>",
            a = cy - 3.3,
            b = cx + 3.3,
            c = cy + 3.3,
            d = cx - 3.3,
        ),
        "assist" => format!(
            "<polygon points=\"{cx:.1},{a:.1} {b:.1},{c:.1} {d:.1},{c:.1}\" fill=\"#9b59b6\"/>",
            a = cy - 3.3,
            b = cx + 3.2,
            c = cy + 3.1,
            d = cx - 3.2,
        ),
        "kill" => format!(
            "<path d=\"M{l:.1} {u:.1} L{r:.1} {dn:.1} M{l:.1} {dn:.1} L{r:.1} {u:.1}\" stroke=\"#e8643c\" stroke-width=\"1.6\" stroke-linecap=\"round\"/>",
            l = cx - 2.8,
            r = cx + 2.8,
            u = cy - 2.8,
            dn = cy + 2.8,
        ),
        "death" => format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"2.8\" fill=\"none\" stroke=\"#8294a4\" stroke-width=\"1.4\"/>"
        ),
        _ => String::new(),
    }
}

/// The **Game Timeline**: one horizontal lane per player (blue then orange) with
/// event glyphs placed along match time, a running score band on top, and a time
/// axis below. Self-contained inline SVG.
fn game_timeline_svg(doc: &BcReplayDoc, m: &CanonicalMatch) -> String {
    const W: f32 = 900.0;
    const GUT: f32 = 104.0; // left gutter for player names
    const RPAD: f32 = 16.0;
    const TOP: f32 = 28.0; // score-progression band
    const ROW: f32 = 20.0;
    const BOT: f32 = 22.0; // time axis

    let players = all_players(doc);
    let n = players.len().max(1);
    let plot_bottom = TOP + n as f32 * ROW;
    let h = plot_bottom + BOT;
    let dur = m.duration_s.max(1.0);
    let x_of = |t: f32| GUT + (t / dur).clamp(0.0, 1.0) * (W - GUT - RPAD);
    let y_of = |i: usize| TOP + i as f32 * ROW + ROW / 2.0;

    let lane: HashMap<&str, usize> = players
        .iter()
        .enumerate()
        .map(|(i, (p, _))| (p.name.as_str(), i))
        .collect();

    // Team-tinted lane backgrounds + right-aligned player names.
    let mut lanes = String::new();
    for (i, (p, team)) in players.iter().enumerate() {
        let y = TOP + i as f32 * ROW;
        let bg = if *team == "blue" {
            "#eef5fd"
        } else {
            "#fdf1ec"
        };
        let _ = write!(
            lanes,
            "<rect x=\"{GUT}\" y=\"{y:.1}\" width=\"{lw:.1}\" height=\"{ROW}\" fill=\"{bg}\"/>\
             <text x=\"{nx:.1}\" y=\"{ny:.1}\" class=\"tl-name {team}\">{name}</text>",
            lw = W - GUT - RPAD + RPAD,
            nx = GUT - 6.0,
            ny = y_of(i) + 3.5,
            name = esc(&p.name),
        );
    }

    // Event glyphs + running-score band (goals drive the score progression).
    let mut marks = String::new();
    let mut goal_lines = String::new();
    let mut goals: Vec<(f32, Option<i32>)> = Vec::new();
    for e in &m.events {
        match e {
            Event::Goal { t, scorer, team } => {
                goals.push((*t, *team));
                if let Some(i) = scorer.as_deref().and_then(|s| lane.get(s)) {
                    marks += &timeline_glyph("goal", x_of(*t), y_of(*i));
                }
            }
            Event::Stat {
                t, player, kind, ..
            } => {
                if let Some(i) = player.as_deref().and_then(|p| lane.get(p)) {
                    let k = match kind {
                        StatKind::Shot => "shot",
                        StatKind::Save => "save",
                        StatKind::Assist => "assist",
                    };
                    marks += &timeline_glyph(k, x_of(*t), y_of(*i));
                }
            }
            Event::Demo {
                t,
                attacker,
                victim,
                ..
            } => {
                if let Some(i) = attacker.as_deref().and_then(|a| lane.get(a)) {
                    marks += &timeline_glyph("kill", x_of(*t), y_of(*i));
                }
                if let Some(i) = victim.as_deref().and_then(|v| lane.get(v)) {
                    marks += &timeline_glyph("death", x_of(*t), y_of(*i));
                }
            }
            _ => {}
        }
    }
    goals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut bg, mut og) = (0, 0);
    for (t, team) in &goals {
        match team {
            Some(0) => bg += 1,
            Some(1) => og += 1,
            _ => {}
        }
        let x = x_of(*t);
        let _ = write!(
            goal_lines,
            "<line x1=\"{x:.1}\" y1=\"{TOP:.1}\" x2=\"{x:.1}\" y2=\"{plot_bottom:.1}\" stroke=\"#d8c98a\" stroke-dasharray=\"2 3\"/>\
             <text x=\"{x:.1}\" y=\"19\" class=\"tl-score\">{bg}\u{2013}{og}</text>",
        );
    }

    // Time axis: five evenly spaced ticks (m:ss).
    let mut axis = String::new();
    for k in 0..=4 {
        let frac = k as f32 / 4.0;
        let x = GUT + frac * (W - GUT - RPAD);
        let _ = write!(
            axis,
            "<line x1=\"{x:.1}\" y1=\"{plot_bottom:.1}\" x2=\"{x:.1}\" y2=\"{ty:.1}\" stroke=\"#cdd8e3\"/>\
             <text x=\"{x:.1}\" y=\"{ly:.1}\" class=\"tl-axis\">{lab}</text>",
            ty = plot_bottom + 4.0,
            ly = plot_bottom + 15.0,
            lab = mmss(frac * dur),
        );
    }

    format!(
        "<svg viewBox=\"0 0 {W} {h:.0}\" class=\"tl\" preserveAspectRatio=\"xMidYMid meet\">\
         <rect width=\"{W}\" height=\"{h:.0}\" fill=\"#fff\"/>{lanes}{goal_lines}\
         <line x1=\"{GUT}\" y1=\"{plot_bottom:.1}\" x2=\"{xr:.1}\" y2=\"{plot_bottom:.1}\" stroke=\"#aebfce\"/>\
         {axis}{marks}</svg>",
        xr = W - RPAD,
    )
}

/// The Game-Timeline section: heading, legend, and the SVG.
fn game_timeline_section(doc: &BcReplayDoc, m: &CanonicalMatch) -> String {
    let legend = [
        ("goal", "Goal"),
        ("shot", "Shot"),
        ("save", "Save"),
        ("assist", "Assist"),
        ("kill", "Kill"),
        ("death", "Death"),
    ]
    .iter()
    .map(|(k, lbl)| {
        format!(
            "<span class=\"tl-key\"><svg viewBox=\"0 0 12 12\" class=\"tl-swatch\">{}</svg>{lbl}</span>",
            timeline_glyph(k, 6.0, 6.0),
        )
    })
    .collect::<String>();
    format!(
        "<h3>Game timeline</h3><div class=\"tl-legend\">{legend}</div>\
         <div class=\"tl-wrap\">{}</div>",
        game_timeline_svg(doc, m),
    )
}

fn overview_tab(doc: &BcReplayDoc, m: &CanonicalMatch) -> String {
    let (b, o) = (&doc.blue.stats, &doc.orange.stats);
    let bars = [
        compare_bar("Goals", b.core.goals as f32, o.core.goals as f32, n0),
        compare_bar("Shots", b.core.shots as f32, o.core.shots as f32, n0),
        compare_bar("Assists", b.core.assists as f32, o.core.assists as f32, n0),
        compare_bar("Saves", b.core.saves as f32, o.core.saves as f32, n0),
        compare_bar(
            "Shooting %",
            b.core.shooting_percentage,
            o.core.shooting_percentage,
            pct,
        ),
        compare_bar(
            "Demos inflicted",
            b.demo.inflicted as f32,
            o.demo.inflicted as f32,
            n0,
        ),
        compare_bar("BPM", b.boost.bpm, o.boost.bpm, n0),
        compare_bar(
            "Boost collected",
            b.boost.amount_collected,
            o.boost.amount_collected,
            n0,
        ),
        compare_bar(
            "Boost stolen",
            b.boost.amount_stolen,
            o.boost.amount_stolen,
            n0,
        ),
        compare_bar(
            "Big pads",
            b.boost.count_collected_big as f32,
            o.boost.count_collected_big as f32,
            n0,
        ),
        compare_bar(
            "Small pads",
            b.boost.count_collected_small as f32,
            o.boost.count_collected_small as f32,
            n0,
        ),
    ]
    .concat();
    format!(
        "<h3>Scoreboard</h3><div class=\"scoreboards\">{blue}{orange}</div>\
         {timeline}\
         <h3>Team stats overview</h3><div class=\"cmp-grid\">{bars}</div>\
         {camera}",
        blue = scoreboard_block(&doc.blue, "blue"),
        orange = scoreboard_block(&doc.orange, "orange"),
        timeline = game_timeline_section(doc, m),
        camera = camera_section(doc),
    )
}

/// A short number with no trailing `.0` (camera values are integers or one decimal).
fn num(v: f32) -> String {
    if (v.fract()).abs() < 1e-3 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

/// The per-player "Camera & settings" table (ballchasing's camera profile +
/// steering sensitivity). Players whose profile never replicated show "—".
fn camera_section(doc: &BcReplayDoc) -> String {
    let head = &[
        "",
        "FOV",
        "Distance",
        "Height",
        "Angle",
        "Stiffness",
        "Swivel speed",
        "Transition",
        "Steering sens.",
    ];
    let rows = player_rows(doc, |p| match &p.camera {
        Some(c) => vec![
            num(c.fov),
            num(c.distance),
            num(c.height),
            num(c.pitch),
            num(c.stiffness),
            num(c.swivel_speed),
            num(c.transition_speed),
            p.steering_sensitivity
                .map(num)
                .unwrap_or_else(|| "—".into()),
        ],
        None => vec!["—".into(); 8],
    });
    table("Camera & settings", head, rows)
}

fn team_rows(doc: &BcReplayDoc, cells: fn(&Side) -> Vec<String>) -> Vec<Vec<String>> {
    vec![
        std::iter::once("<span class=\"blue\">Blue</span>".to_string())
            .chain(cells(&doc.blue))
            .collect(),
        std::iter::once("<span class=\"orange\">Orange</span>".to_string())
            .chain(cells(&doc.orange))
            .collect(),
    ]
}

fn player_rows(doc: &BcReplayDoc, cells: fn(&Player) -> Vec<String>) -> Vec<Vec<String>> {
    all_players(doc)
        .iter()
        .map(|(p, team)| std::iter::once(named(p, team)).chain(cells(p)).collect())
        .collect()
}

fn core_tab(doc: &BcReplayDoc) -> String {
    let head = &[
        "",
        "Score",
        "Shots",
        "Goals",
        "Shooting %",
        "Assists",
        "Saves",
        "Demos inf.",
        "Demos tkn.",
    ];
    let team_head = &head[..8];
    let teams = table(
        "Teams",
        team_head,
        team_rows(doc, |s| {
            let c = &s.stats.core;
            vec![
                n0(c.score as f32),
                n0(c.shots as f32),
                n0(c.goals as f32),
                pct(c.shooting_percentage),
                n0(c.assists as f32),
                n0(c.saves as f32),
                n0(s.stats.demo.inflicted as f32),
            ]
        }),
    );
    let players = table(
        "Players",
        head,
        player_rows(doc, |p| {
            let c = &p.stats.core;
            vec![
                n0(c.score as f32),
                n0(c.shots as f32),
                n0(c.goals as f32),
                pct(c.shooting_percentage),
                n0(c.assists as f32),
                n0(c.saves as f32),
                n0(p.stats.demo.inflicted as f32),
                n0(p.stats.demo.taken as f32),
            ]
        }),
    );
    let bars = [
        player_bars("Score", doc, |p| p.stats.core.score as f32, n0),
        player_bars("Shots", doc, |p| p.stats.core.shots as f32, n0),
    ]
    .concat();
    format!("{teams}{players}<div class=\"charts\">{bars}</div>")
}

fn boost_tab(doc: &BcReplayDoc, pickup_maps: &str) -> String {
    let team_head = &[
        "",
        "BPM",
        "Avg",
        "Time 0",
        "Time 100",
        "Collected",
        "Stolen",
        "Big",
        "Small",
        "Stln big",
        "Stln small",
    ];
    let teams = table(
        "Teams",
        team_head,
        team_rows(doc, |s| {
            let x = &s.stats.boost;
            vec![
                n0(x.bpm),
                n0(x.avg_amount),
                secs(x.time_zero_boost),
                secs(x.time_full_boost),
                n0(x.amount_collected),
                n0(x.amount_stolen),
                n0(x.count_collected_big as f32),
                n0(x.count_collected_small as f32),
                n0(x.count_stolen_big as f32),
                n0(x.count_stolen_small as f32),
            ]
        }),
    );
    let p_head = &[
        "", "BPM", "Avg", "Time 0", "Time 100", "0–25", "25–50", "50–75", "75–100", "Coll.",
        "Stln", "Big", "Small", "Overfill", "SS used*",
    ];
    let players = table(
        "Players",
        p_head,
        player_rows(doc, |p| {
            let x = &p.stats.boost;
            vec![
                n0(x.bpm),
                n0(x.avg_amount),
                secs(x.time_zero_boost),
                secs(x.time_full_boost),
                secs(x.time_boost_0_25),
                secs(x.time_boost_25_50),
                secs(x.time_boost_50_75),
                secs(x.time_boost_75_100),
                n0(x.amount_collected),
                n0(x.amount_stolen),
                n0(x.count_collected_big as f32),
                n0(x.count_collected_small as f32),
                n0(x.amount_overfill),
                n0(x.amount_used_while_supersonic),
            ]
        }),
    );
    let bars = [
        player_bars("BPM", doc, |p| p.stats.boost.bpm, n0),
        player_bars(
            "Amount collected",
            doc,
            |p| p.stats.boost.amount_collected,
            n0,
        ),
    ]
    .concat();
    let note = "<p class=\"note\">* <b>SS used</b> = boost burned while supersonic on the \
        ground — approximate (near-exact on clean replays, but reconstruction-noisy, \
        ±~25% and occasionally 2× on sparse ones).</p>";
    format!("{teams}{players}{note}<div class=\"charts\">{bars}</div>{pickup_maps}")
}

fn movement_tab(doc: &BcReplayDoc) -> String {
    let team_head = &[
        "", "Tot dist", "Slow", "Boost", "Super", "Ground", "Low air", "High air", "PS time",
        "PS count",
    ];
    let teams = table(
        "Teams",
        team_head,
        team_rows(doc, |s| {
            let m = &s.stats.movement;
            vec![
                n0(m.total_distance),
                secs(m.time_slow_speed),
                secs(m.time_boost_speed),
                secs(m.time_supersonic_speed),
                secs(m.time_ground),
                secs(m.time_low_air),
                secs(m.time_high_air),
                secs(m.time_powerslide),
                n0(m.count_powerslide as f32),
            ]
        }),
    );
    let p_head = &[
        "",
        "Avg spd %",
        "Tot dist",
        "Slow",
        "Boost",
        "Super",
        "Ground",
        "Low air",
        "High air",
        "PS time",
        "PS avg",
        "PS count",
    ];
    let players = table(
        "Players",
        p_head,
        player_rows(doc, |p| {
            let m = &p.stats.movement;
            vec![
                pct(m.avg_speed_percentage),
                n0(m.total_distance),
                secs(m.time_slow_speed),
                secs(m.time_boost_speed),
                secs(m.time_supersonic_speed),
                secs(m.time_ground),
                secs(m.time_low_air),
                secs(m.time_high_air),
                secs(m.time_powerslide),
                secs(m.avg_powerslide_duration),
                n0(m.count_powerslide as f32),
            ]
        }),
    );
    let bars = [
        player_bars(
            "Distance travelled",
            doc,
            |p| p.stats.movement.total_distance,
            n0,
        ),
        player_bars(
            "Avg speed %",
            doc,
            |p| p.stats.movement.avg_speed_percentage,
            pct,
        ),
    ]
    .concat();
    format!("{teams}{players}<div class=\"charts\">{bars}</div>")
}

fn positioning_tab(doc: &BcReplayDoc) -> String {
    let p_head = &[
        "",
        "Def ⅓",
        "Neut ⅓",
        "Off ⅓",
        "Def ½",
        "Off ½",
        "< ball",
        "> ball",
        "Most back",
        "Most fwd",
        "Closest",
        "Farthest",
        "Dist ball",
        "Dist (poss)",
        "Dist (no poss)",
        "Dist mates",
        "GA last def",
    ];
    let players = table(
        "Players",
        p_head,
        player_rows(doc, |p| {
            let q = &p.stats.positioning;
            vec![
                pct(q.percent_defensive_third),
                pct(q.percent_neutral_third),
                pct(q.percent_offensive_third),
                pct(q.percent_defensive_half),
                pct(q.percent_offensive_half),
                pct(q.percent_behind_ball),
                pct(q.percent_infront_ball),
                pct(q.percent_most_back),
                pct(q.percent_most_forward),
                pct(q.percent_closest_to_ball),
                pct(q.percent_farthest_from_ball),
                n0(q.avg_distance_to_ball),
                n0(q.avg_distance_to_ball_possession),
                n0(q.avg_distance_to_ball_no_possession),
                n0(q.avg_distance_to_mates),
                n0(q.goals_against_while_last_defender as f32),
            ]
        }),
    );
    let bars = [
        player_bars(
            "Distance to team mates",
            doc,
            |p| p.stats.positioning.avg_distance_to_mates,
            n0,
        ),
        player_bars(
            "Avg distance to ball",
            doc,
            |p| p.stats.positioning.avg_distance_to_ball,
            n0,
        ),
        player_bars(
            "Time in front of ball",
            doc,
            |p| p.stats.positioning.percent_infront_ball,
            pct,
        ),
        player_bars(
            "Time closest to ball",
            doc,
            |p| p.stats.positioning.percent_closest_to_ball,
            pct,
        ),
    ]
    .concat();
    format!("{players}<div class=\"charts\">{bars}</div>")
}

fn demos_tab(doc: &BcReplayDoc) -> String {
    let players = table(
        "Players",
        &["", "Inflicted", "Taken"],
        player_rows(doc, |p| {
            vec![
                n0(p.stats.demo.inflicted as f32),
                n0(p.stats.demo.taken as f32),
            ]
        }),
    );
    let bars = [
        player_bars(
            "Demos inflicted",
            doc,
            |p| p.stats.demo.inflicted as f32,
            n0,
        ),
        player_bars("Demos taken", doc, |p| p.stats.demo.taken as f32, n0),
    ]
    .concat();
    let team = compare_bar(
        "Demos inflicted",
        doc.blue.stats.demo.inflicted as f32,
        doc.orange.stats.demo.inflicted as f32,
        n0,
    );
    format!("<div class=\"cmp-grid\">{team}</div>{players}<div class=\"charts\">{bars}</div>")
}

const CSS: &str = r#"
:root{--blue:#3a8ee6;--orange:#e8643c}
*{box-sizing:border-box}body{margin:0;font:14px/1.4 system-ui,Segoe UI,Roboto,sans-serif;color:#1d2733;background:#f4f6f8}
header{padding:18px 24px;background:#fff;border-bottom:1px solid #e3e8ee}
h1{margin:0;font-size:22px}
.meta{margin-top:6px;display:flex;gap:10px;color:#5a6b7b;font-size:13px}
.meta .score{font-weight:700;color:#1d2733}
.meta span{background:#eef2f6;border-radius:4px;padding:2px 8px}
main{max-width:1100px;margin:18px auto;padding:0 16px}
input[name=tab]{position:absolute;opacity:0;pointer-events:none}
.tabbar{display:flex;gap:4px;border-bottom:2px solid #e3e8ee;margin-bottom:16px;flex-wrap:wrap}
.tabbar label{padding:8px 14px;cursor:pointer;color:#5a6b7b;border-bottom:2px solid transparent;margin-bottom:-2px}
.tabbar label:hover{color:#1d2733}
.panel{display:none;background:#fff;border:1px solid #e3e8ee;border-radius:8px;padding:18px}
#t-ov:checked~#p-ov,#t-co:checked~#p-co,#t-ba:checked~#p-ba,#t-bo:checked~#p-bo,#t-mo:checked~#p-mo,#t-po:checked~#p-po,#t-he:checked~#p-he,#t-de:checked~#p-de{display:block}
#t-ov:checked~.tabbar label[for=t-ov],#t-co:checked~.tabbar label[for=t-co],#t-ba:checked~.tabbar label[for=t-ba],#t-bo:checked~.tabbar label[for=t-bo],#t-mo:checked~.tabbar label[for=t-mo],#t-po:checked~.tabbar label[for=t-po],#t-he:checked~.tabbar label[for=t-he],#t-de:checked~.tabbar label[for=t-de]{color:#1d2733;border-bottom-color:var(--blue);font-weight:600}
.note{color:#8294a4;font-size:12px;margin:6px 0 14px}
.hm{width:100%;height:auto;border-radius:4px;display:block}
.padc{fill:#103a1f;font-size:7px;font-weight:700;text-anchor:middle;font-family:sans-serif}
.tl-wrap{overflow-x:auto;margin:4px 0 8px}
.tl{width:100%;min-width:560px;height:auto;display:block;border:1px solid #e3e8ee;border-radius:6px}
.tl-name{font-size:10px;text-anchor:end;font-family:sans-serif}
.tl-score{fill:#8a7320;font-size:10px;font-weight:700;text-anchor:middle;font-family:sans-serif}
.tl-axis{fill:#8294a4;font-size:9px;text-anchor:middle;font-family:sans-serif}
.tl-legend{display:flex;flex-wrap:wrap;gap:14px;margin:8px 0 2px;color:#5a6b7b;font-size:12px}
.tl-key{display:inline-flex;align-items:center;gap:5px}
.tl-swatch{width:13px;height:13px}
.hm-wrap{max-width:360px}
.hm-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:16px}
.hm-card{background:#fafcfe;border:1px solid #e3e8ee;border-radius:6px;padding:8px}
.hm-name{font-size:13px;font-weight:600;margin-bottom:6px}
h3{margin:18px 0 8px;font-size:16px}h4{margin:14px 0 6px;font-size:13px;color:#5a6b7b;font-weight:600}
.scoreboards{display:flex;flex-direction:column;gap:10px}
table{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}
.tbl{overflow-x:auto;margin:10px 0}
th,td{padding:5px 8px;text-align:right;border-bottom:1px solid #eef2f6;white-space:nowrap}
th:first-child,td:first-child{text-align:left}
thead th{color:#5a6b7b;font-weight:600;font-size:12px;border-bottom:2px solid #e3e8ee}
table.score thead th{background:#eef2f6}table.score.blue .teamhdr{background:var(--blue);color:#fff}table.score.orange .teamhdr{background:var(--orange);color:#fff}
.blue{color:var(--blue);font-weight:600}.orange{color:var(--orange);font-weight:600}
td.car{text-align:left;color:#5a6b7b;font-size:12px}
.cmp-grid{display:grid;grid-template-columns:1fr 1fr;gap:6px 24px}
.cmp{display:grid;grid-template-columns:48px 1fr 48px;grid-template-areas:"l bar r" "lab lab lab";align-items:center;gap:4px;margin:4px 0}
.cmp-l{grid-area:l;text-align:right;color:var(--blue);font-weight:600}.cmp-r{grid-area:r;color:var(--orange);font-weight:600}
.cmp-bar{grid-area:bar;display:flex;height:14px;border-radius:3px;overflow:hidden;background:#eef2f6}
.cmp-bar .b{background:var(--blue)}.cmp-bar .o{background:var(--orange)}
.cmp-lab{grid-area:lab;text-align:center;color:#5a6b7b;font-size:12px}
.charts{display:grid;grid-template-columns:1fr 1fr;gap:18px;margin-top:12px}
.chart h4{margin-bottom:8px}
.pb{display:grid;grid-template-columns:120px 1fr 56px;align-items:center;gap:8px;margin:3px 0}
.pb-n{text-align:right;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.pb-bar{height:12px;background:#eef2f6;border-radius:3px;overflow:hidden}.pb-bar span{display:block;height:100%}
.pb-v{font-size:12px;color:#5a6b7b}
footer{max-width:1100px;margin:18px auto;padding:0 16px 40px;color:#8294a4;font-size:12px}
@media(max-width:760px){.cmp-grid,.charts{grid-template-columns:1fr}}
"#;
