//! Self-contained HTML report rendering.
//!
//! Turns a [`LobbyReport`] (+ per-player SVG heatmaps) into one standalone HTML
//! document with inline CSS and inline SVG — no external assets, no JS. This is
//! the product page and the PDF-ready template (feed it to headless Chromium or
//! weasyprint for the §4.10 PDF; that renderer is the only remaining piece and
//! lives in the web layer, not here).

use std::fmt::Write;

use crate::lobby::LobbyReport;
use crate::report::{Confidence, Report};

/// Minimal HTML-text escaping for interpolated, possibly-hostile player names.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

fn team_class(team: Option<i32>) -> &'static str {
    match team {
        Some(1) => "orange",
        _ => "blue",
    }
}

fn mmss(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Render the full report as a standalone HTML document. `heatmaps` maps a
/// player's `target_pri` to its inline SVG.
pub fn html(lobby: &LobbyReport, heatmaps: &[(i32, String)]) -> String {
    let svg_for = |pri: i32| -> &str {
        heatmaps
            .iter()
            .find(|(p, _)| *p == pri)
            .map(|(_, s)| s.as_str())
            .unwrap_or("")
    };

    let mut h = String::with_capacity(16 * 1024);
    h.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    let _ = write!(
        h,
        "<title>Replay report — {}</title>",
        esc(&lobby.replay_id)
    );
    h.push_str(STYLE);
    h.push_str("</head><body><main>");

    // Header.
    let blue = lobby.team_scores.get(&0).copied().unwrap_or(0);
    let orange = lobby.team_scores.get(&1).copied().unwrap_or(0);
    let _ = write!(
        h,
        "<header><h1>Decision-discipline report</h1>\
         <div class=\"meta\"><span>{}</span><span class=\"score\">\
         <b class=\"blue\">{blue}</b> – <b class=\"orange\">{orange}</b></span>\
         <span>{}</span><span>{}</span><span class=\"cfg\">{}</span></div></header>",
        esc(lobby.map.as_deref().unwrap_or("unknown map")),
        mmss(lobby.duration_s),
        esc(&lobby.replay_id),
        esc(&lobby.score_config_version),
    );

    comparison_table(&mut h, lobby);

    // Player cards.
    h.push_str("<section class=\"cards\">");
    for p in &lobby.players {
        player_card(&mut h, p, svg_for(p.target_pri));
    }
    h.push_str("</section>");

    h.push_str("</main></body></html>");
    h
}

fn comparison_table(h: &mut String, lobby: &LobbyReport) {
    h.push_str("<section><h2>Lobby comparison</h2><table class=\"cmp\"><thead><tr><th>metric</th>");
    for p in &lobby.players {
        let _ = write!(
            h,
            "<th class=\"{}\">{}<small>{:.0}</small></th>",
            team_class(p.target_team),
            esc(&p.target_player),
            p.composite
        );
    }
    h.push_str("</tr></thead><tbody>");
    // Composite headline row.
    h.push_str("<tr class=\"composite\"><td>composite</td>");
    let best = lobby
        .players
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.composite.total_cmp(&b.composite))
        .map(|(i, _)| i);
    for (i, p) in lobby.players.iter().enumerate() {
        let lead = if Some(i) == best { " lead" } else { "" };
        let _ = write!(h, "<td class=\"num{lead}\">{:.0}</td>", p.composite);
    }
    h.push_str("</tr>");
    for row in &lobby.comparison {
        let _ = write!(h, "<tr><td>{}</td>", esc(&row.key));
        for (i, v) in row.normalized.iter().enumerate() {
            let lead = if Some(i) == row.leader { " lead" } else { "" };
            let _ = write!(h, "<td class=\"num{lead}\">{v:.0}</td>");
        }
        h.push_str("</tr>");
    }
    h.push_str("</tbody></table></section>");
}

fn player_card(h: &mut String, p: &Report, svg: &str) {
    let _ = write!(h, "<article class=\"card {}\">", team_class(p.target_team));
    let _ = write!(
        h,
        "<div class=\"cardhead\"><h3>{}</h3><span class=\"big\">{:.0}</span></div>",
        esc(&p.target_player),
        p.composite
    );
    if p.confidence == Confidence::LowConfidence {
        h.push_str(
            "<div class=\"warn\">low confidence — few analyzable frames / missing teammate</div>",
        );
    }
    let _ = write!(
        h,
        "<div class=\"tags\"><span class=\"tag\">{}</span><span class=\"tag\">{}</span></div>\
         <div class=\"subs\"><span>1st {:.0}</span><span>2nd {:.0}</span><span>gen {:.0}</span></div>\
         <div class=\"leak\">Focus: <b>{}</b> → {}</div>",
        esc(&p.licence),
        esc(&p.player_type),
        p.first_man,
        p.second_man,
        p.general,
        esc(&p.main_leak),
        esc(&p.focus_chapter),
    );

    // Metric bars.
    h.push_str("<div class=\"bars\">");
    for b in &p.metrics {
        let _ = write!(
            h,
            "<div class=\"bar\"><label>{}</label><div class=\"track\">\
             <div class=\"fill\" style=\"width:{:.0}%\"></div></div>\
             <span class=\"v\">{:.0}</span></div>",
            esc(&b.key),
            b.normalized.clamp(0.0, 100.0),
            b.normalized
        );
    }
    h.push_str("</div>");

    if !svg.is_empty() {
        let _ = write!(
            h,
            "<div class=\"heat\"><div class=\"heatcap\">position &amp; touches (attacking ↑)</div>{svg}</div>"
        );
    }
    h.push_str("</article>");
}

const STYLE: &str = "<style>\
:root{--bg:#0b0f14;--panel:#161c25;--line:#26303d;--ink:#e6edf3;--mut:#8b97a7;--blue:#4c8dff;--orange:#ff9a3c;--hot:#ff5a3c}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 system-ui,Segoe UI,Roboto,sans-serif}\
main{max-width:1100px;margin:0 auto;padding:24px}\
h1{font-size:20px;margin:0}h2{font-size:15px;color:var(--mut);text-transform:uppercase;letter-spacing:.06em;margin:28px 0 10px}\
header .meta{display:flex;gap:16px;align-items:baseline;flex-wrap:wrap;color:var(--mut);margin-top:6px}\
.score b{font-size:18px}.blue{color:var(--blue)}.orange{color:var(--orange)}.cfg{margin-left:auto;font-size:12px}\
table.cmp{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}\
.cmp th,.cmp td{border:1px solid var(--line);padding:5px 8px;text-align:right}\
.cmp th:first-child,.cmp td:first-child{text-align:left;color:var(--mut)}\
.cmp th small{display:block;color:var(--mut);font-weight:400}\
.cmp td.num{color:var(--ink)}.cmp td.lead{background:rgba(76,141,255,.16);font-weight:700}\
.cmp tr.composite td{border-bottom:2px solid var(--line);font-weight:600}\
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:16px;margin-top:14px}\
.card{background:var(--panel);border:1px solid var(--line);border-top:3px solid var(--mut);border-radius:8px;padding:14px}\
.card.blue{border-top-color:var(--blue)}.card.orange{border-top-color:var(--orange)}\
.cardhead{display:flex;justify-content:space-between;align-items:center}.cardhead h3{margin:0;font-size:16px}\
.big{font-size:30px;font-weight:800}\
.warn{margin:6px 0;color:#ffcf6b;font-size:12px}\
.tags{display:flex;gap:8px;margin:8px 0}.tag{background:#0f1620;border:1px solid var(--line);border-radius:999px;padding:2px 10px;font-size:12px}\
.subs{display:flex;gap:14px;color:var(--mut);font-size:12px;margin-bottom:8px}\
.leak{font-size:13px;margin-bottom:10px}\
.bars{display:flex;flex-direction:column;gap:3px;margin-bottom:10px}\
.bar{display:grid;grid-template-columns:150px 1fr 28px;gap:8px;align-items:center;font-size:11px}\
.bar label{color:var(--mut);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}\
.track{background:#0f1620;border-radius:4px;height:8px;overflow:hidden}\
.fill{height:100%;background:linear-gradient(90deg,#3a6fd0,#5fd08a)}\
.bar .v{text-align:right;font-variant-numeric:tabular-nums}\
.heat{margin-top:8px}.heatcap{color:var(--mut);font-size:11px;margin-bottom:4px}\
.heat svg{width:100%;height:auto;border:1px solid var(--line);border-radius:6px}\
</style>";
