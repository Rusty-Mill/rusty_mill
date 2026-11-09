//! An attached window's connection banner (gui-design F5, D62): under the
//! strip, over the top of the blocks, saying the window is reconnecting,
//! is back, or can't reach its server, with the buttons that act on it.
//! The same box carries an info notice while the link is up (D26, D63),
//! such as a server older than this client.
//!
//! Pure: [`paint`] draws a [`Banner`] into a [`Painted`] with its buttons'
//! boxes. Sizes are the pack's logical px times the DPI scale.

use super::blocks::{Glyph, paint_glyph};
use super::chrome::Chrome;
use super::paint::{Pixmap, Rect, icons};
use super::source::Notice;
use super::text::{UiText, Weight};

/// Banner height, its rules included.
const HEIGHT: f32 = 44.0;
const RULE: f32 = 1.0;
/// The error banner's top border.
const ERROR_RULE: f32 = 2.0;
/// Padding (left, right) and the space between items.
const PAD: (f32, f32) = (22.0, 20.0);
const GAP: f32 = 14.0;
const ICON: f32 = 16.0;
const ICON_STROKE: f32 = 1.5;
const CHECK_STROKE: f32 = 1.8;
const TITLE_PX: f32 = 13.0;
const DETAIL_PX: f32 = 12.0;
/// The countdown bar and its corner radius.
const BAR: (f32, f32) = (200.0, 4.0);
const BAR_R: f32 = 2.0;
/// The grace the countdown runs down (handoff D44).
const GRACE: u32 = 30;
/// Small buttons: height, side padding, radius, text size.
const BUTTON_H: f32 = 24.0;
const BUTTON_PAD: f32 = 10.0;
const BUTTON_R: f32 = 6.0;
const BUTTON_PX: f32 = 12.0;
/// What a disabled button keeps of its look (D62: 45%).
const DISABLED: f32 = 0.45;

/// Which banner shows (D62).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Banner {
    /// F5a: the link is lost; `left` seconds of grace remain, and an
    /// attempt runs while `trying`.
    Reconnecting { left: u32, trying: bool },
    /// F5b: back. `kept` when this client took its clients over, leases
    /// as they were; else it attached anew.
    Back { kept: bool },
    /// The grace ran out or the user stopped trying; a Retry runs while
    /// `trying`.
    Down { trying: bool },
    /// An info notice (D63), shown until dismissed.
    Info(Notice),
}

/// What a banner button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// "Retry now" or "Retry": try at once.
    Retry,
    /// "Stop trying": show the error banner now.
    Stop,
    /// "Dismiss": hide F5b.
    Dismiss,
    /// "Dismiss" on an info notice: hide it for this connection.
    DismissInfo,
}

/// A painted banner: its pixels and the buttons that can be clicked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Painted {
    pub pixmap: Pixmap,
    pub buttons: Vec<(Rect, Action)>,
}

/// A button: its label, whether it is secondary (else ghost), what it
/// does and whether it can be clicked.
struct Button {
    label: &'static str,
    secondary: bool,
    action: Action,
    enabled: bool,
}

/// The banner's height in px at `scale`.
pub(crate) fn height(scale: f32) -> usize {
    (HEIGHT * scale).round() as usize
}

/// "Reconnecting to {target}, {n} s left" and the rest of `banner`'s words.
fn words(banner: &Banner, target: &str) -> (String, String) {
    let (title, detail) = match banner {
        Banner::Reconnecting { left, .. } => (
            format!("Reconnecting to {target}, {left} s left"),
            "Your blocks keep running on the server.",
        ),
        Banner::Back { kept: true } => (
            format!("Reconnected to {target}"),
            "This client took over your session (token confirmed).",
        ),
        Banner::Back { kept: false } => (
            format!("Reconnected to {target}"),
            "Connected again. Input leases from before were released.",
        ),
        Banner::Down { .. } => (
            format!("Can't reach {target}"),
            "Check the network and that rusty_mux is running there.",
        ),
        Banner::Info(notice) => return (notice.title.clone(), notice.detail.clone()),
    };
    (title, detail.to_owned())
}

fn buttons(banner: &Banner) -> Vec<Button> {
    let button = |label, secondary, action, enabled| Button {
        label,
        secondary,
        action,
        enabled,
    };
    match banner {
        // Retry now tries at once; the countdown goes on (D62).
        &Banner::Reconnecting { trying, .. } => vec![
            button("Retry now", true, Action::Retry, !trying),
            button("Stop trying", false, Action::Stop, true),
        ],
        Banner::Back { .. } => vec![button("Dismiss", true, Action::Dismiss, true)],
        &Banner::Down { trying } => vec![button("Retry", true, Action::Retry, !trying)],
        Banner::Info(_) => vec![button("Dismiss", true, Action::DismissInfo, true)],
    }
}

/// Paint `banner` for `target` across `width` px.
pub(crate) fn paint(
    banner: &Banner,
    target: &str,
    width: usize,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) -> Painted {
    let mut p = Pixmap::new(width, height(s), c.raised);
    let w = width as f32;
    let h = p.height as f32;
    let rule = |y: f32, thick: f32, color: u32, p: &mut Pixmap| {
        let r = Rect {
            x: 0.0,
            y,
            w,
            h: (thick * s).max(1.0).round(),
        };
        p.fill(r, 0.0, color);
    };
    let error = matches!(banner, Banner::Down { .. });
    match error {
        true => rule(0.0, ERROR_RULE, c.failure, &mut p),
        false => rule(0.0, RULE, c.line, &mut p),
    }
    rule(h - (RULE * s).max(1.0).round(), RULE, c.line, &mut p);
    let mid = h / 2.0;
    let icon_y = mid - ICON * s / 2.0;
    let mut x = PAD.0 * s;

    // The lead icon: static spinner, success check or alert.
    match banner {
        Banner::Reconnecting { .. } => {
            paint_glyph(&mut p, Glyph::Spinner, x, icon_y, s, c.muted, c.line)
        }
        Banner::Back { .. } => p.icon(
            icons::CHECK,
            x,
            icon_y,
            ICON * s,
            CHECK_STROKE * s,
            c.success,
        ),
        Banner::Down { .. } => p.icon(
            icons::ALERT,
            x,
            icon_y,
            ICON * s,
            ICON_STROKE * s,
            c.failure,
        ),
        // Slate, not oxide: nothing failed (D63).
        Banner::Info(_) => p.icon(icons::ALERT, x, icon_y, ICON * s, ICON_STROKE * s, c.muted),
    }
    x += (ICON + GAP) * s;

    // The buttons from the right, so the detail knows its room.
    let plan = buttons(banner);
    let mut right = w - PAD.1 * s;
    let mut placed = Vec::new();
    for b in plan.iter().rev() {
        let text_w = font.width(b.label, Weight::Medium, BUTTON_PX * s);
        let bw = text_w + 2.0 * BUTTON_PAD * s;
        let r = Rect {
            x: right - bw,
            y: mid - BUTTON_H * s / 2.0,
            w: bw,
            h: BUTTON_H * s,
        };
        right = r.x - GAP * s;
        placed.push((r, b));
    }
    placed.reverse();

    let (title, detail) = words(banner, target);
    // A notice's title comes from the server, so it may be long.
    let title = font.fit(&title, Weight::Medium, TITLE_PX * s, (right - x).max(0.0));
    let title_base = mid + font.centre_drop(TITLE_PX * s);
    x += p.text(
        font,
        &title,
        Weight::Medium,
        TITLE_PX * s,
        x,
        title_base,
        c.text,
    ) + GAP * s;
    if let &Banner::Reconnecting { left, .. } = banner {
        let track = Rect {
            x,
            y: mid - BAR.1 * s / 2.0,
            w: BAR.0 * s,
            h: BAR.1 * s,
        };
        p.fill(track, BAR_R * s, c.sunk);
        let share = left.min(GRACE) as f32 / GRACE as f32;
        if share > 0.0 {
            p.fill(
                Rect {
                    w: track.w * share,
                    ..track
                },
                BAR_R * s,
                c.text,
            );
        }
        x += track.w + GAP * s;
    }
    let room = (right - x).max(0.0);
    let detail = font.fit(&detail, Weight::Regular, DETAIL_PX * s, room);
    let detail_base = mid + font.centre_drop(DETAIL_PX * s);
    p.text(
        font,
        &detail,
        Weight::Regular,
        DETAIL_PX * s,
        x,
        detail_base,
        c.muted,
    );

    let mut out = Vec::new();
    let label_base = mid + font.centre_drop(BUTTON_PX * s);
    for (r, b) in placed {
        if b.secondary {
            p.fill(r, BUTTON_R * s, c.raised);
            p.border(r, BUTTON_R * s, s.max(1.0).round(), c.line);
        }
        let ink = if b.secondary { c.text } else { c.muted };
        let text_w = font.width(b.label, Weight::Medium, BUTTON_PX * s);
        let tx = r.x + (r.w - text_w) / 2.0;
        p.text(
            font,
            b.label,
            Weight::Medium,
            BUTTON_PX * s,
            tx,
            label_base,
            ink,
        );
        if b.enabled {
            out.push((r, b.action));
        } else {
            p.fade(r, c.raised, DISABLED);
        }
    }
    Painted {
        pixmap: p,
        buttons: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> Chrome {
        Chrome::from_theme(&crate::core::Theme::default())
    }

    fn font() -> UiText {
        UiText::embedded().expect("the embedded faces parse")
    }

    fn actions(p: &Painted) -> Vec<Action> {
        p.buttons.iter().map(|(_, a)| *a).collect()
    }

    #[test]
    fn the_banner_is_44_px_and_full_width() {
        let p = paint(
            &Banner::Down { trying: false },
            "vm",
            900,
            (&chrome(), 2.0),
            &mut font(),
        );
        assert_eq!((p.pixmap.width, p.pixmap.height), (900, 88));
    }

    #[test]
    fn each_banner_has_its_buttons() {
        let c = chrome();
        let mut f = font();
        let mut at = |b: Banner| actions(&paint(&b, "vm", 1100, (&c, 1.0), &mut f));
        let reconnecting = Banner::Reconnecting {
            left: 22,
            trying: false,
        };
        assert_eq!(at(reconnecting), [Action::Retry, Action::Stop]);
        assert_eq!(at(Banner::Back { kept: true }), [Action::Dismiss]);
        assert_eq!(at(Banner::Down { trying: false }), [Action::Retry]);
    }

    #[test]
    fn an_info_notice_has_dismiss_alone() {
        let info = Banner::Info(Notice {
            title: "vm runs an older rusty_mux".into(),
            detail: "It lacks shared focus.".into(),
        });
        let p = paint(&info, "vm", 1100, (&chrome(), 1.0), &mut font());
        assert_eq!(actions(&p), [Action::DismissInfo]);
    }

    #[test]
    fn a_retry_running_disables_retry() {
        let c = chrome();
        let mut f = font();
        let trying = Banner::Reconnecting {
            left: 20,
            trying: true,
        };
        assert_eq!(
            actions(&paint(&trying, "vm", 1100, (&c, 1.0), &mut f)),
            [Action::Stop]
        );
        let down = Banner::Down { trying: true };
        assert!(
            paint(&down, "vm", 1100, (&c, 1.0), &mut f)
                .buttons
                .is_empty()
        );
    }

    #[test]
    fn the_error_banner_has_an_oxide_top_border() {
        let c = chrome();
        let mut f = font();
        let down = paint(
            &Banner::Down { trying: false },
            "vm",
            600,
            (&c, 1.0),
            &mut f,
        );
        let back = paint(&Banner::Back { kept: true }, "vm", 600, (&c, 1.0), &mut f);
        let px = |p: &Painted, y: usize| p.pixmap.pixels()[y * p.pixmap.width + 300];
        assert_eq!((px(&down, 0), px(&down, 1)), (c.failure, c.failure));
        assert_eq!((px(&back, 0), px(&back, 1)), (c.line, c.raised));
    }

    #[test]
    fn the_bar_runs_down_with_the_grace() {
        let c = chrome();
        let mut f = font();
        let mut bone = |left| {
            let b = Banner::Reconnecting {
                left,
                trying: false,
            };
            let p = paint(&b, "vm", 1200, (&c, 1.0), &mut f);
            let row = p.pixmap.height / 2;
            p.pixmap.pixels()[row * p.pixmap.width..][..p.pixmap.width]
                .iter()
                .filter(|&&px| px == c.text)
                .count()
        };
        let (full, half, none) = (bone(30), bone(15), bone(0));
        assert!(full > half && half > none);
    }

    #[test]
    fn words_say_what_happened() {
        assert_eq!(
            words(&Banner::Back { kept: false }, "vm").1,
            "Connected again. Input leases from before were released."
        );
        assert_eq!(
            words(
                &Banner::Reconnecting {
                    left: 7,
                    trying: false
                },
                "vm"
            )
            .0,
            "Reconnecting to vm, 7 s left"
        );
    }
}
