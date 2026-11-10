//! The windowed front-end: a `winit` event loop driving one or more real OS
//! windows (C13), with `softbuffer` CPU presentation.
//!
//! [`App`] is a thin router owning a [`WindowState`] per open window: window
//! events dispatch by `WindowId`, PTY wakeups by pane id (ids come from a
//! shared counter so they're unique across windows). Every window shows a
//! source's panes ([`run_attached`]); local mode is retired (pack D146,
//! D159). The loop exits when the last window closes.
//!
//! Each window is borderless (`decorations(false)`) and draws its own chrome: a
//! one-cell-row bar across the top holding the session tabs, a `+` new-tab
//! button, and the minimize/maximize/close caption buttons, all laid out as
//! ordinary cells so both renderers composite it for free. The bars use their
//! own UI font at the configured size, so the Ctrl+=/Ctrl+- font zoom scales
//! only the terminal text. Dragging the bar
//! moves the window, double-click toggles maximize, and a thin band at the
//! window edges drag-resizes (the native frame is gone).
//!
//! Each tab owns a PTY session: its own grid, parser, writer, and a reader
//! thread pumping PTY output into the grid and waking the loop to repaint.
//! Key events are encoded natively (see [`super::input`]) and written to the
//! *active* tab's PTY; window resizes re-grid every tab + `TIOCSWINSZ`.
//!
//! Note: this drives a live window and so cannot be exercised in a headless
//! environment — the render, input-encoding, and font layers it composes are
//! unit-tested independently. Wired here: mouse drag-selection, clipboard
//! copy/paste (Ctrl+Shift+C / Ctrl+Shift+V, plus Ctrl+C / Ctrl+V on
//! Windows), OSC 52 get/set, SGR/1006 mouse reporting, Ctrl+click to open
//! OSC 8 hyperlinks, DECSCUSR cursor styles + blink, and a Windows
//! child-exit watcher.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::sync::Mutex;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, KeyLocation, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, ResizeDirection, UserAttentionType, Window, WindowId};

use super::blocks::{self, BodyState};
use super::chrome::{Chrome, Marks};
use super::confirm::{self, Answer, Confirm, ConfirmKey};
use super::font::{self, FontCache, GlyphSource};
use super::layout::{Dir, Layout, Rect};
use super::motion;
use super::pace::Pacer;
use super::popover::{self, DetachedRow, Kind, Menu, Pick, Popover, Row};
use super::prefix::{self, Command};
use super::render::{CpuRenderer, PaneFrame, Renderer, Scene, SearchColors, blend};
use super::ribbon;
use super::settings::{Field, Settings, SetupBaseline, Widget};
use super::source::{Act, Feed, PaneIo, Remote, Source, Tree};
use super::strip::{self, Spot, StripState};
use crate::backend::BackendHandle;
use crate::compact::Compactor;
use crate::config::{ClipboardPolicy, Config, LaunchMode};
use crate::core::{AnsiParser, Cell, Grid, Selection, Theme, WIDE_TRAILER, char_width};
use crate::gui::mouse::{MouseButtonKind, MouseEvent, MousePoint, SgrEncoder};
use crate::keymap::{Action, Chord};

/// Built-in defaults, overridable via the config file (`[window]` section).
const FONT_PX: f32 = 18.0;
/// The chrome's UI font size in logical px. Fixed: neither zoom nor the
/// configured terminal `font_size` resizes the bars, only the display DPI
/// does (STYLEGUIDE §Size). Equal to the default `FONT_PX`, so a default
/// config looks as before.
const UI_FONT_PX: f32 = 18.0;
/// Step for the Ctrl+=/Ctrl+- runtime font zoom (`Action::FontSizeUp/Down`).
const FONT_ZOOM_STEP: f32 = 1.0;
const INIT_COLS: u16 = 80;
const INIT_ROWS: u16 = 24;
/// Pixel band at the window edges acting as a resize handle (the native frame
/// is gone with decorations off).
const RESIZE_BORDER: f64 = 6.0;
/// Grid row where overlay (menu / settings) list rows begin (header sits above).
const OVERLAY_ITEMS_TOP: usize = 2;
/// Settings-page layout (cells): the category sidebar's width, the rows one
/// setting card occupies (title+widget, description, spacing), and the rows
/// reserved below the list (a blank spacer plus the key-hint footer).
const SETTINGS_SIDEBAR_W: usize = 16;
const SETTINGS_ROW_H: usize = 3;
const SETTINGS_FOOTER_H: usize = 2;
/// First content row of the settings page: one blank row of breathing space
/// under the chrome bar, so the sidebar band and page title don't sit flush
/// against the tab strip.
const SETTINGS_TOP: usize = 1;
/// Width of the command dock in cells (its divider column adds one more).
const DOCK_CELLS: usize = 30;
/// Narrowest pane area the dock may leave behind; below this it auto-hides.
const DOCK_MIN_PANE: usize = 40;
/// First grid row of the setting cards: the page title + its rule sit on
/// the two rows above (the strip tab names the page, so no header row).
const SETTINGS_CARDS_TOP: usize = SETTINGS_TOP + 2;
/// Width of the clickable `/ search` affordance at the title row's right.
const SETTINGS_SEARCH_W: usize = 14;
/// First column of a setting card's text (right of the sidebar + separator).
const SETTINGS_CARD_X: usize = SETTINGS_SIDEBAR_W + 3;
/// Cells left free at the cards' right edge (the value widget ends here).
const SETTINGS_RIGHT_PAD: usize = 3;
/// Cap on the card column's width. On a wide window an uncapped card puts
/// the value ~200 cells from its label — the eye loses the pairing — so the
/// content column stays readable-width like a settings page, not a table.
const SETTINGS_CARD_W_MAX: usize = 80;
/// Two clicks on the drag strip within this window toggle maximize.
const DOUBLE_CLICK_MS: u128 = 400;
/// Default inner margin around the pane area (`[window] padding` overrides).
const DEFAULT_PAD: u32 = 8;

/// Scrollback lines moved per mouse-wheel notch.
const WHEEL_LINES: isize = 3;
/// Dragging a selection past a pane's top/bottom edge by more than this many
/// pixels auto-scrolls the viewport, so text beyond what's currently visible
/// can still be reached — matches most editors/terminals with a scrollback.
const DRAG_SCROLL_MARGIN: f64 = 24.0;
/// Auto-scroll step cadence while the pointer is held past the edge.
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(50);

/// Wakeups sent from per-tab PTY reader threads into the winit loop, tagged
/// with the tab id they concern.
pub(crate) enum UserEvent {
    /// New output was parsed into the tab's grid; repaint if it's the active one.
    Redraw(u64),
    /// The config file changed on disk; reload and apply what can change live.
    ConfigChanged,
    /// An AccessKit event (initial-tree request, AT action, deactivation)
    /// routed back through the winit loop (C20).
    Accessibility(accesskit_winit::Event),
    /// The attached window's source laid its panes out anew; `None` when
    /// its window is gone ([`Remote::layout`]).
    Layout(Option<Tree>),
    /// The source moved the attached window's focus ([`Remote::focus`]).
    Focus(u64),
    /// The source zoomed a pane, or unzoomed with `None` ([`Remote::zoom`]).
    Zoom(Option<u64>),
    /// The source's window shows no block: all are detached
    /// ([`Remote::empty`]).
    Empty,
    /// The source's session changed ([`Remote::session`]).
    Session(super::source::Session),
    /// What the source says of the shown blocks, for their headers
    /// ([`Remote::blocks`]).
    Blocks(Vec<(u64, super::source::BlockInfo)>),
    /// How the source reaches its server ([`Remote::link`]).
    Link(super::source::Link),
    /// The shown session ended while the source was away
    /// ([`Remote::ended`]).
    Ended(String),
    /// An info notice for the banner ([`Remote::notice`]).
    Notice(Option<super::source::Notice>),
    /// Where taking pane `id`'s owner-only block over stands
    /// ([`super::source::Feed::Take`]).
    Take(u64, super::source::Take),
}

/// What an attached window draws besides its panes: the strip, the
/// body and its overlays, and an open popover.
fn attached_scene<'a>(
    strip: &'a StripState,
    body: &'a BodyState,
    popover: &'a Popover,
    confirm: Option<&'a Confirm>,
) -> Scene<'a> {
    let body = body.painted();
    Scene {
        strip: strip.painted().map(|s| &s.pixmap),
        body: body.map(|b| &b.pixmap),
        over: body.map_or(&[], |b| &b.over),
        pop: popover
            .painted()
            .or_else(|| confirm.and_then(Confirm::painted)),
    }
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        UserEvent::Accessibility(event)
    }
}

/// An in-flight cursor-trail hop (G36): from where, to where, started when.
#[derive(Clone, Copy)]
struct Trail {
    from: (usize, usize),
    to: (usize, usize),
    since: Instant,
}

/// Text coverage of a disabled settings row: 45% (pack D26), standing in for
/// opacity since a cell has no alpha.
const DISABLED_ALPHA: u8 = 115;

/// What a click on a given chrome-bar cell does.
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Minimize,
    Maximize,
    Close,
    /// Empty bar: drag moves the window, double-click toggles maximize.
    Drag,
    /// An attached window's session chip: its menu (gui-design D45 b).
    Session,
    /// Show the source's window `key` (an attached window's strip tab).
    Window(u64),
    /// Ask the source for a new window (the strip's "+").
    NewWindow,
    /// The detached-blocks chip: its popover (gui-design D61).
    Detached,
    /// A control on an attached window's blocks: a zoom button, "Exit
    /// zoom" or a rail chip (gui-design D49).
    Block(blocks::Spot),
}

impl Hit {
    /// The strip spot this hit is, for drawing its hover and press.
    fn spot(self) -> Option<Spot> {
        Some(match self {
            Hit::Session => Spot::Session,
            Hit::Window(key) => Spot::Tab(key),
            Hit::NewWindow => Spot::NewWindow,
            Hit::Detached => Spot::Detached,
            Hit::Minimize => Spot::Minimize,
            Hit::Maximize => Spot::Maximize,
            Hit::Close => Spot::Close,
            _ => return None,
        })
    }

    /// The body spot this hit is, for drawing its hover and press.
    fn block(self) -> Option<blocks::Spot> {
        match self {
            Hit::Block(spot) => Some(spot),
            _ => None,
        }
    }
}

impl From<Spot> for Hit {
    fn from(spot: Spot) -> Self {
        match spot {
            Spot::Session => Hit::Session,
            Spot::Tab(key) => Hit::Window(key),
            Spot::NewWindow => Hit::NewWindow,
            Spot::Detached => Hit::Detached,
            Spot::Minimize => Hit::Minimize,
            Spot::Maximize => Hit::Maximize,
            Spot::Close => Hit::Close,
        }
    }
}

/// A transient full-window page drawn over the active tab while open: the shell
/// launcher / settings dropdown menu, or the settings page. Keys and
/// below-chrome clicks route to it; clicking a tab or a chrome button dismisses
/// it.
enum Overlay {
    /// The `▾` dropdown: a list of menu rows to choose from.
    Menu { items: Vec<MenuItem>, sel: usize },
    /// The settings page.
    Settings(Settings),
}

/// One row of the dropdown menu and the action choosing it triggers.
struct MenuItem {
    label: String,
    kind: MenuKind,
}

#[derive(Clone)]
enum MenuKind {
    /// Open this URL with the OS handler (the visible-links menu).
    OpenUrl(String),
    /// Open the in-app settings page.
    Settings,
    /// Open the config file in the user's editor.
    EditConfig,
}

/// Keyboard copy-mode state: the moving cursor in viewport cells, and the
/// selection anchor (absolute coords) once `v`/Space pins one.
struct CopyMode {
    cur: (usize, usize),
    anchor: Option<(usize, usize)>,
}

/// One terminal session inside a tab (a split pane): its screen state and PTY
/// plumbing. The owning PTY handle lives here, so dropping a `Pane` tears the
/// session down (its reader thread then EOFs and its exit event becomes a no-op).
struct Pane {
    id: u64,
    grid: Arc<Mutex<Grid>>,
    /// Shared with the pane's reader thread; the loop takes it only on config reload.
    parser: Arc<Mutex<AnsiParser>>,
    writer: Box<dyn BackendHandle>,
    /// Raised by the reader thread when it sends a wakeup, lowered when the
    /// window handles it (see `reader_loop`).
    wake: WakeFlag,
    /// The frame last painted, shown again while a synchronized update
    /// (DEC 2026) is open so no other repaint shows it half-drawn.
    shown: std::cell::RefCell<Option<crate::core::FrameView>>,
    /// When to compact the history next: once output has gone quiet.
    compactor: Compactor,
}

/// A tab: one or more [`Pane`]s tiled by a [`Layout`], one of them focused.
struct Tab {
    panes: Vec<Pane>,
    layout: Layout,
    /// Id of the focused pane — it receives input and shows the cursor.
    focus: u64,
    /// Unseen-alert badge (bell or finished command while the tab was in the
    /// background / the window unfocused); cleared when the tab is active in
    /// a focused window.
    attention: bool,
    /// Pane zoom: the focused pane temporarily takes the whole tab area
    /// (toggled by `zoom_pane`); cleared by any split/close/focus change.
    zoomed: bool,
}

impl Pane {
    /// The pane's frame to paint (see [`frame_to_paint`]).
    fn frame_view(&self) -> crate::core::FrameView {
        frame_to_paint(&mut self.grid.lock(), self.shown.take())
    }
}

/// `grid`'s frame now, or `shown` (the frame last painted) while a
/// synchronized update is open and the view hasn't been resized or scrolled.
fn frame_to_paint(
    grid: &mut Grid,
    shown: Option<crate::core::FrameView>,
) -> crate::core::FrameView {
    let held = shown.filter(|f| {
        f.cols == grid.cols && f.rows == grid.rows && f.view_offset == grid.view_offset
    });
    match held {
        Some(frame) if grid.sync_output_active() => frame,
        _ => grid.frame_view(),
    }
}

/// One synchronized-output tick: from the panes `before` held and the
/// `(pane, timeout)` of each update `open` now, the panes now held, the
/// earliest timeout to wake at, and whether any pane was released.
fn sync_step(before: &[u64], open: &[(u64, Instant)]) -> (Vec<u64>, Option<Instant>, bool) {
    let held: Vec<u64> = open.iter().map(|&(id, _)| id).collect();
    let released = before.iter().any(|id| !held.contains(id));
    (held, open.iter().map(|&(_, t)| t).min(), released)
}

/// Where a block sits (gui-design D47): its cell rect, whose corner only
/// orders it among the others; the window pixel of its grid's top-left
/// corner; and its box in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    cells: Rect,
    x: usize,
    y: usize,
    block: Rect,
}

impl Tab {
    fn pane(&self, id: u64) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }
    fn focused(&self) -> Option<&Pane> {
        self.pane(self.focus)
    }
    fn focused_mut(&mut self) -> Option<&mut Pane> {
        let f = self.focus;
        self.panes.iter_mut().find(|p| p.id == f)
    }
}

/// Launch the windowed terminal. Blocks until the window closes or the last
/// tab's child exits. Returns an error if the window/PTY/font can't be set up.
/// Launch a window that shows `source`'s panes, laid out as `first` with
/// pane `focus` focused and then as the source says through the [`Remote`]
/// handed to `on_start` before the window opens. It opens no local shells.
/// Setup opens on `--setup`, or on a first run.
/// Blocks until the window closes or the source's window is gone.
pub fn run_attached(
    source: &dyn Source,
    first: Tree,
    focus: u64,
    config: &Config,
    on_start: impl FnOnce(Remote),
) -> Result<(), Box<dyn std::error::Error>> {
    launch(source, config, |window, args| {
        on_start(Remote(window.proxy.clone()));
        window.told = Some(focus);
        window.show_tree(Some(first));
        if crate::config::wants_setup(args) || crate::config::is_first_run(args) {
            window.open_setup();
        }
        Ok(())
    })
}

/// The event loop and first window behind [`run_attached`];
/// `seed` fills the first window, given the launch arguments.
fn launch<'a>(
    source: &'a dyn Source,
    config: &Config,
    seed: impl FnOnce(&mut WindowState<'a>, &[String]) -> Result<(), std::io::Error>,
) -> Result<(), Box<dyn std::error::Error>> {
    // The child renders through us now, so advertise a real terminal identity.
    unsafe {
        std::env::set_var("TERM", "xterm-256color");
        std::env::set_var("COLORTERM", "truecolor");
    }

    let mut event_loop_builder = EventLoop::<UserEvent>::with_user_event();
    let event_loop = build_event_loop(&mut event_loop_builder)?;
    let proxy = event_loop.create_proxy();

    // Config live reload: watch the file and wake the loop on changes.
    let all: Vec<String> = std::env::args().skip(1).collect();
    let args = crate::config::own_args(&all);
    let config_path = crate::config::Config::file_path(args);
    if let Some(path) = config_path.clone() {
        let watch_proxy = proxy.clone();
        crate::config::watch(path, move || {
            let _ = watch_proxy.send_event(UserEvent::ConfigChanged);
        });
    }

    let profile_override = crate::config::flag_value(args, "--profile").map(str::to_string);
    let mut app = App {
        source,
        config: config.clone(),
        config_path,
        profile_override,
        proxy,
        shells: crate::shells::detect_all(),
        windows: Vec::new(),
    };
    // The first window (its OS window comes with the loop's `resumed`).
    // Building it up front surfaces font/shell errors before the loop runs.
    let mut first = app.new_window_state()?;
    seed(&mut first, &all)?;
    app.windows.push(first);
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// A pane's one outstanding wakeup: the reader raises it when it sends a
/// `Redraw`, the window lowers it when it handles one. Output parsed while
/// it is raised needs no wakeup of its own, since the pending one repaints
/// the grid it went into.
#[derive(Clone, Default)]
struct WakeFlag(Arc<AtomicBool>);

impl WakeFlag {
    /// Mark output pending; `true` when no wakeup was, so one must be sent.
    fn raise(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }

    /// The window is handling the output; output parsed later wakes it again.
    fn lower(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// `builder.build()`, with a panic inside it turned into an error: on Linux
/// winit panics when a system library it loads at runtime (libxkbcommon,
/// libxkbcommon-x11) is missing, which used to abort with a backtrace.
/// Whether rusty_term's own options ask for the GPU renderer.
#[cfg(feature = "gui-gpu")]
fn gpu_asked() -> bool {
    let all: Vec<String> = std::env::args().skip(1).collect();
    crate::config::own_args(&all).iter().any(|a| a == "--gpu")
}

fn build_event_loop(
    builder: &mut winit::event_loop::EventLoopBuilder<UserEvent>,
) -> Result<EventLoop<UserEvent>, Box<dyn std::error::Error>> {
    // Silence the default hook's report for this one call; the message
    // goes into the returned error instead.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build()));
    std::panic::set_hook(hook);
    match built {
        Ok(event_loop) => Ok(event_loop?),
        Err(panic) => Err(format!("cannot open a window: {}", panic_text(panic.as_ref())).into()),
    }
}

/// A panic payload's message, when it has one.
fn panic_text(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("the windowing library panicked")
}

/// Pane reader loop (own thread, one per pane): apply the pane's feed to
/// its grid, send the parser's replies (DA/DSR/structured-channel) back
/// through `replies` when the window answers the program's queries, and
/// wake the window with the pane's id through `send` (`false` once the
/// event loop is gone). A wakeup is sent only when the window has handled
/// the last one (`wake`), so a flood costs the UI thread one event per
/// frame, not one per read. The loop ends when the feed does or a send
/// fails; the source's next layout drops the pane.
fn reader_loop(
    id: u64,
    feed: impl Iterator<Item = Feed>,
    mut replies: Option<Box<dyn BackendHandle>>,
    grid: Arc<Mutex<Grid>>,
    parser: Arc<Mutex<AnsiParser>>,
    wake: WakeFlag,
    mut send: impl FnMut(UserEvent) -> bool,
) {
    for step in feed {
        if let Feed::Take(take) = step {
            if !send(UserEvent::Take(id, take)) {
                break; // loop gone
            }
            continue;
        }
        let response = apply(step, &grid, &parser);
        if let Some(replies) = replies.as_mut().filter(|_| !response.is_empty()) {
            let _ = replies.write(&response);
        }
        // Signal even mid synchronized-output window: the `Redraw` handler
        // checks `sync_output_active` and skips painting. A wakeup still
        // unhandled already covers this output. Closing the pane leaves its
        // flag raised, so the loop then only parses until the feed ends, as
        // it did when every send succeeded: dropping the pane hangs up the
        // direct child, but a descendant holding the PTY open keeps it
        // reading.
        if wake.raise() && !send(UserEvent::Redraw(id)) {
            break; // loop gone
        }
    }
}

/// Apply one feed step to a pane's terminal, returning the parser's
/// replies to the program.
fn apply(step: Feed, grid: &Mutex<Grid>, parser: &Mutex<AnsiParser>) -> Vec<u8> {
    match step {
        Feed::Bytes(data) => {
            // Bulk: step aside for the UI thread between chunks.
            let mut g = grid.lock_bulk();
            let mut parser = parser.lock();
            parser.advance(&mut g, &data);
            // host_out (clipboard/title relay) has no host here; dropped.
            let _ = g.take_host_out();
            parser.take_responses()
        }
        Feed::Screen(new_grid, new_parser) => {
            let (mut g, mut p) = (grid.lock(), parser.lock());
            let (mut new_g, mut new_p) = (*new_grid, *new_parser);
            // The pane keeps its theme and display settings; the screen
            // comes in its source's. Retheming the parser it replaces is
            // how to read its theme back.
            let ours = p.retheme(Theme::default());
            let theirs = new_p.retheme(ours);
            new_g.retheme(&theirs, &ours);
            new_g.cell_px = g.cell_px;
            new_g.min_contrast = g.min_contrast;
            new_g.bidi = g.bidi;
            (*g, *p) = (new_g, new_p);
            Vec::new()
        }
        Feed::Resized { cols, rows } => {
            grid.lock().resize(cols.into(), rows.into());
            Vec::new()
        }
        // The reader loop tells the window instead.
        Feed::Take(_) => Vec::new(),
    }
}

/// One top-level OS window: its tabs/panes, renderer, chrome, and every bit
/// of per-window UI state (selection, overlays, search, copy mode, …). The
/// [`App`] router owns one of these per open window and dispatches winit
/// events to the right one by `WindowId`.
struct WindowState<'a> {
    /// The window's source of panes and layout. The window never edits its
    /// layout (pack D102).
    source: &'a dyn Source,
    /// The focus last told to `source` or told by it.
    told: Option<u64>,
    /// An attached window's prefix (Ctrl-]) is armed: the next key is a
    /// layout command (gui-design F2, D49).
    armed: bool,
    /// The window's top strip (gui-design D45).
    strip: StripState,
    /// The window's framed blocks under the strip (gui-design D47).
    body: BodyState,
    /// The window's popover: the session switcher (gui-design F4, D60) or
    /// the detached blocks (D61).
    popover: Popover,
    /// An attached window's open take-over confirm (gui-design F3, D64).
    confirm: Option<Confirm>,
    /// The effective config; refreshed on live reload so new tabs follow it.
    config: Config,
    /// The config file in effect, for the open shortcut + reload re-reads.
    config_path: Option<std::path::PathBuf>,
    /// The `--profile <name>` this instance was launched with, if any —
    /// reapplied after every `reload_config()` re-read (see its doc comment)
    /// so the launch-time profile override survives a config-file save.
    profile_override: Option<String>,
    tabs: Vec<Tab>,
    /// Index into `tabs` of the session being shown and fed input.
    active: usize,
    proxy: EventLoopProxy<UserEvent>,
    font: FontCache,
    cell_w: usize,
    cell_h: usize,
    /// The chrome bar's and status ribbon's font, at `ui_px`: neither the
    /// Ctrl+=/Ctrl+- zoom nor the configured font size changes it.
    ui_font: FontCache,
    /// `ui_font`'s cell size: one bar cell.
    bar_w: usize,
    bar_h: usize,
    /// Inner margin in pixels around the pane area (`[window] padding`). The
    /// chrome bar stays flush; the grid sits inset by this on the other three
    /// sides (and below the bar), painted in the theme background.
    pad: usize,
    window: Option<Arc<Window>>,
    renderer: Option<Box<dyn Renderer>>,
    mods: ModifiersState,
    /// Grid size in cells (the chrome bar adds one extra screen row on top).
    cols: u16,
    rows: u16,
    /// The theme in effect, painting the chrome bar and any new tab's grid.
    theme: Theme,
    /// System clipboard for copy/paste; `None` if unavailable (e.g. headless).
    clipboard: Option<arboard::Clipboard>,
    /// Current pointer position in physical pixels, used for hit-testing.
    mouse_pos: (f64, f64),
    /// Fractional remainder of the last `MouseWheel` delta-to-lines
    /// conversion. High-resolution wheels and slow trackpad flicks emit many
    /// events with less than one line's worth of delta each; rounding each
    /// event independently (the old behavior) dropped all of them on the
    /// floor and slow scrolling didn't move anything. Accumulating instead
    /// means every bit of delta eventually turns into a scrolled line.
    scroll_accum: f64,
    /// The left-button drag selection, live while the button is held.
    drag: SelectionDrag,
    /// Physical button currently held, if any — drives `?1002` drag-motion
    /// reporting (which button to report) and whether `?1002` reports motion
    /// at all (only while a button is down; `?1003` reports regardless).
    mouse_button_down: Option<MouseButtonKind>,
    /// Keys held down whose press a window shortcut consumed. Their repeats
    /// and releases never reach the child, so one reading key-ups
    /// (win32-input-mode, Kitty event types) sees no unpaired events.
    consumed_keys: ConsumedKeys,
    /// Per-cell click actions for the chrome bar, rebuilt with each layout.
    hits: Vec<Hit>,
    /// The chrome control the left button went down on, drawn pressed until
    /// the button is released (D26, D43).
    pressed: Option<Hit>,
    /// Chrome-bar element under the pointer, for hover feedback. `None` when
    /// the pointer is below the bar or over an inert (drag-strip) cell.
    hover: Option<Hit>,
    /// A Ctrl-hovered hyperlink under the pointer in the focused pane's grid,
    /// `(row, start_col, end_col)` — drives the pointer cursor and hover
    /// underline (G22). Recomputed on every `CursorMoved`/`ModifiersChanged`;
    /// `None` when Ctrl isn't held, the pointer is off the grid, or there's
    /// no link there.
    hover_link: Option<(usize, usize, usize)>,
    /// Time of the last single click on the drag strip (double-click detect).
    last_strip_click: Option<Instant>,
    /// Whether the window currently has OS focus. Gates the `?1004` focus
    /// reports (wave 1), bell attention requests, and command-finished
    /// notifications.
    focused: bool,
    /// Whether the find bar matches as a regex (`rusty_regx`, POSIX ERE)
    /// instead of plain text. Toggled with Ctrl+R inside search mode;
    /// remembered across searches.
    search_regex: bool,
    /// Whether the find bar's match is case-sensitive (skips Unicode case
    /// folding). Toggled with Alt+C inside search mode; remembered across
    /// searches, mirroring `search_regex`.
    search_case_sensitive: bool,
    /// Broadcast input (G28): while set, keystrokes and pastes go to every
    /// pane in the active tab (multi-host workflows), not just the focused
    /// one. Toggled per-window; the active tab shows a `⇉` marker.
    broadcast: bool,
    /// Keyboard copy mode (G18): a viewport-cell cursor plus an optional
    /// anchor (absolute coords). While `Some`, keys move the cursor /
    /// extend the selection instead of reaching the keymap or the child.
    copy_mode: Option<CopyMode>,
    /// Last left-click on pane content `(when, cell)` plus the current
    /// consecutive-click count — double-click selects a word, triple a
    /// logical line (`click_streak` cycles 1 → 2 → 3 → 1).
    last_grid_click: Option<(Instant, (usize, usize))>,
    click_streak: u8,
    /// Whether the blinking cursor is in its visible phase this frame.
    cursor_blink_on: bool,
    /// When the blink phase last toggled, paced by the event loop.
    last_blink: Instant,
    /// The active tab's panes the last [`tick_sync`](Self::tick_sync) found
    /// holding a synchronized update, so a tick that finds one of them
    /// released repaints.
    sync_held: Vec<u64>,
    /// Paces output-driven repaints to the display refresh (CPU renderer);
    /// set up by [`make_renderer`](Self::make_renderer).
    pacer: Pacer,
    /// Active scrollback-search query (windowed front-end); `Some` means search
    /// mode is on, intercepting keys and showing the find bar in the chrome.
    searching: Option<String>,
    /// Shells detected on this machine, for the launcher menu + settings page.
    shells: Vec<crate::shells::DetectedShell>,
    /// The open settings page / shell menu, if any (drawn over the active tab).
    overlay: Option<Overlay>,
    /// Current terminal font size in px, zoom included.
    font_px: f32,
    /// The configured (unzoomed) terminal font size in px at the current
    /// DPI: what Ctrl+0 resets the terminal font to.
    base_px: f32,
    /// The bars' font size in px: [`UI_FONT_PX`] at the current DPI.
    ui_px: f32,
    /// The window's current monitor DPI scale factor (1.0 = 100%), tracked so
    /// `WindowEvent::ScaleFactorChanged` can rescale `font_px` proportionally
    /// instead of leaving text rendered at the wrong physical size after a
    /// cross-monitor move. Winit reports this at window creation and on every
    /// change; there is no ambient way to query it in between.
    scale_factor: f64,
    /// Set when this window should close (last tab closed, × button, OS close
    /// request); the [`App`] router drops it and exits the loop when none are
    /// left.
    closed: bool,
    /// Cursor-trail state (G36): where the focused pane's cursor was last
    /// frame, and the in-flight trail if it just jumped. Tracked per window
    /// for its focused pane only.
    cursor_prev: Option<(u64, (usize, usize))>,
    trail: Option<Trail>,
    /// Whether the right-hand command dock is open (`toggle_dock`,
    /// Ctrl+Shift+K). It auto-hides — without flipping this — when the
    /// window is too narrow to fit it beside a usable pane area.
    dock_open: bool,
    /// The dock's click map, rebuilt with each redraw: for each dock grid
    /// row, the absolute scrollback line a click jumps the focused pane to
    /// (`None` for headers/blank rows).
    dock_items: Vec<Option<usize>>,
    /// AccessKit accessibility-tree state (C20), created alongside the OS
    /// window in `ensure_window`. `None` before the window exists.
    access: Option<super::access::AccessState>,
}

impl WindowState<'_> {
    /// The pane `id` drawn from `io`, sized `cols × rows`, with its reader
    /// thread.
    fn pane_from(
        &mut self,
        id: u64,
        io: PaneIo,
        cols: u16,
        rows: u16,
        theme: Option<Theme>,
    ) -> Result<Pane, std::io::Error> {
        let theme = theme.unwrap_or(self.theme);
        let mut g = Grid::new(cols as usize, rows as usize);
        if let Some(max) = self.config.scrollback {
            g.set_scrollback_max(max);
        }
        g.cell_px = Some((self.cell_w as u16, self.cell_h as u16));
        g.apply_theme(&theme);
        g.set_default_cursor(
            self.config.cursor_style.unwrap_or_default(),
            self.config.cursor_blink.unwrap_or(false),
        );
        g.min_contrast = self.config.minimum_contrast.unwrap_or(1.0);
        g.bidi = self.config.bidi.unwrap_or(false);
        let grid = Arc::new(Mutex::new(g));
        let parser = Arc::new(Mutex::new(AnsiParser::with_theme(theme)));

        // Reader thread: feed -> parser -> grid, writing replies back and
        // waking the loop. Its own reply handle, so it shares no lock with us.
        let replies = match io.answers_queries {
            true => Some(io.writer.try_clone()?),
            false => None,
        };
        let feed = io.feed;
        let reader_grid = Arc::clone(&grid);
        let reader_parser = Arc::clone(&parser);
        let reader_proxy = self.proxy.clone();
        let wake = WakeFlag::default();
        let reader_wake = wake.clone();
        std::thread::spawn(move || {
            reader_loop(
                id,
                feed,
                replies,
                reader_grid,
                reader_parser,
                reader_wake,
                |event| reader_proxy.send_event(event).is_ok(),
            )
        });
        Ok(Pane {
            id,
            grid,
            parser,
            writer: io.writer,
            wake,
            shown: std::cell::RefCell::new(None),
            compactor: Compactor::default(),
        })
    }

    /// Move focus to the next (`forward`) or previous pane of the active tab.
    fn focus_pane(&mut self, forward: bool) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.focus = tab.layout.cycle(tab.focus, forward);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Enter keyboard copy mode: a movable cursor (hjkl/arrows) over the
    /// viewport, `v`/Space to anchor a selection, `y`/Enter to copy and
    /// exit, Esc/q to leave. The cursor renders through the ordinary
    /// selection highlight (a one-cell selection until anchored).
    fn enter_copy_mode(&mut self) {
        let Some(p) = self.pane() else { return };
        let cur = {
            let mut g = p.grid.lock();
            let cur = (g.cursor.0.min(g.cols.saturating_sub(1)), g.cursor.1);
            let abs = g.abs_of_view_row(cur.1);
            g.selection = Some(Selection {
                anchor: (cur.0, abs),
                head: (cur.0, abs),
            });
            cur
        };
        self.copy_mode = Some(CopyMode { cur, anchor: None });
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Handle a key press while copy mode is active. Returns whether the key
    /// was consumed (always, while active — copy mode owns the keyboard).
    fn copy_mode_key(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let Some(mut cm) = self.copy_mode.take() else {
            return false;
        };
        let Some(p) = self.pane() else { return false };
        let mut exit = false;
        let mut yank = false;
        {
            let mut g = p.grid.lock();
            let (cols, rows) = (g.cols, g.rows);
            let mv = |cm: &mut CopyMode, dc: isize, dr: isize, g: &mut Grid| {
                cm.cur.0 = cm
                    .cur
                    .0
                    .saturating_add_signed(dc)
                    .min(cols.saturating_sub(1));
                if dr < 0 && cm.cur.1 == 0 {
                    g.scroll_view_up(dr.unsigned_abs()); // keep moving into history
                } else if dr > 0 && cm.cur.1 + 1 >= rows {
                    g.scroll_view_down(dr as usize);
                } else {
                    cm.cur.1 = cm
                        .cur
                        .1
                        .saturating_add_signed(dr)
                        .min(rows.saturating_sub(1));
                }
            };
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => exit = true,
                Key::Named(NamedKey::Enter) => {
                    yank = true;
                    exit = true;
                }
                Key::Named(NamedKey::ArrowLeft) => mv(&mut cm, -1, 0, &mut g),
                Key::Named(NamedKey::ArrowRight) => mv(&mut cm, 1, 0, &mut g),
                Key::Named(NamedKey::ArrowUp) => mv(&mut cm, 0, -1, &mut g),
                Key::Named(NamedKey::ArrowDown) => mv(&mut cm, 0, 1, &mut g),
                Key::Named(NamedKey::PageUp) => mv(&mut cm, 0, -(rows as isize), &mut g),
                Key::Named(NamedKey::PageDown) => mv(&mut cm, 0, rows as isize, &mut g),
                Key::Named(NamedKey::Home) => cm.cur.0 = 0,
                Key::Named(NamedKey::End) => cm.cur.0 = cols.saturating_sub(1),
                Key::Named(NamedKey::Space) => {
                    cm.anchor = Some((cm.cur.0, g.abs_of_view_row(cm.cur.1)));
                }
                Key::Character(s) => match s.as_str() {
                    "q" => exit = true,
                    "h" => mv(&mut cm, -1, 0, &mut g),
                    "l" => mv(&mut cm, 1, 0, &mut g),
                    "k" => mv(&mut cm, 0, -1, &mut g),
                    "j" => mv(&mut cm, 0, 1, &mut g),
                    "0" => cm.cur.0 = 0,
                    "$" => cm.cur.0 = cols.saturating_sub(1),
                    "g" => {
                        // Top of scrollback.
                        g.scroll_view_up(usize::MAX / 2);
                        cm.cur = (0, 0);
                    }
                    "G" => {
                        g.reset_view();
                        cm.cur = (0, rows.saturating_sub(1));
                    }
                    "v" => {
                        // Toggle the anchor (vi visual mode).
                        cm.anchor = match cm.anchor {
                            Some(_) => None,
                            None => Some((cm.cur.0, g.abs_of_view_row(cm.cur.1))),
                        };
                    }
                    "y" => {
                        yank = true;
                        exit = true;
                    }
                    _ => {}
                },
                _ => return true, // modifiers etc.: consumed, no effect
            }
            // Publish the selection: anchor..cursor, or the bare cursor cell.
            let head = (cm.cur.0, g.abs_of_view_row(cm.cur.1));
            let anchor = cm.anchor.unwrap_or(head);
            g.selection = if exit && !yank {
                None
            } else {
                Some(Selection { anchor, head })
            };
        }
        if yank {
            self.copy_selection();
            self.copy_selection_primary();
        }
        if exit {
            if let Some(p) = self.pane()
                && yank
            {
                p.grid.lock().selection = None;
            }
        } else {
            self.copy_mode = Some(cm);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    /// Move focus to the nearest pane in direction `(dx, dy)` (one axis at a
    /// time): the closest pane whose rect lies strictly beyond the focused
    /// pane's edge, by center distance. No wrap.
    fn focus_dir(&mut self, dx: isize, dy: isize) {
        let rects: Vec<(u64, Rect)> = self
            .places(self.active)
            .into_iter()
            .map(|(id, p)| (id, p.cells))
            .collect();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(&(_, f)) = rects.iter().find(|(id, _)| *id == tab.focus) else {
            return;
        };
        let center = |r: &Rect| {
            (
                r.col as isize * 2 + r.cols as isize,
                r.row as isize * 2 + r.rows as isize,
            )
        };
        let (fcx, fcy) = center(&f);
        let best = rects
            .iter()
            .filter(|(id, _)| *id != tab.focus)
            .filter(|(_, r)| match (dx, dy) {
                (1, _) => r.col >= f.col + f.cols,
                (-1, _) => r.col + r.cols <= f.col,
                (_, 1) => r.row >= f.row + f.rows,
                _ => r.row + r.rows <= f.row,
            })
            .min_by_key(|(_, r)| {
                let (cx, cy) = center(r);
                (cx - fcx).abs() + (cy - fcy).abs()
            })
            .map(|(id, _)| *id);
        if let Some(id) = best {
            tab.focus = id;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Resize tab `ti`'s panes (grids + PTYs) to their layout rectangles.
    /// Cells the command dock currently occupies (its width plus the divider
    /// column), `0` when closed or when opening it would squeeze the pane
    /// area below [`DOCK_MIN_PANE`] columns.
    fn dock_cols(&self) -> usize {
        if self.dock_open && self.cols as usize >= DOCK_CELLS + 1 + DOCK_MIN_PANE {
            DOCK_CELLS + 1
        } else {
            0
        }
    }

    /// The cell area the tab's panes tile: the full grid minus the command
    /// dock (which sits at the right edge, behind a one-cell divider).
    fn pane_area(&self) -> Rect {
        Rect::new(
            0,
            0,
            (self.cols as usize).saturating_sub(self.dock_cols()),
            self.rows as usize,
        )
    }

    /// Where each pane of `layout` sits (only `focus`'s when `zoomed`).
    /// Blocks tile pixels under the strip, each framed with its grid inside
    /// (gui-design D47).
    fn places_of(&self, layout: &Layout, zoomed: bool, focus: u64) -> Vec<(u64, Place)> {
        let (cw, ch) = (self.cell_w.max(1), self.cell_h.max(1));
        let oy = self.grid_oy();
        let scale = self.scale_factor as f32;
        let area = self.body_area();
        let boxes = match zoomed {
            true => {
                let others = layout.ids().len().saturating_sub(1);
                vec![(focus, blocks::zoomed(area, others, scale).0)]
            }
            false => layout.boxes(area, blocks::gap(scale)),
        };
        boxes
            .into_iter()
            .map(|(id, b)| {
                let g = blocks::grid_area(b, scale);
                // A block's cells count from the grid's corner only to
                // order blocks by where they sit (focus moves by direction).
                let cells = Rect::new(
                    g.col.saturating_sub(self.pad) / cw,
                    g.row.saturating_sub(oy) / ch,
                    g.cols / cw,
                    g.rows / ch,
                );
                let place = Place {
                    cells,
                    x: g.col,
                    y: g.row,
                    block: b,
                };
                (id, place)
            })
            .collect()
    }

    /// Where each of tab `ti`'s panes sits.
    fn places(&self, ti: usize) -> Vec<(u64, Place)> {
        match self.tabs.get(ti) {
            Some(t) => self.places_of(&t.layout, t.zoomed, t.focus),
            None => Vec::new(),
        }
    }

    /// Where pane `id` of the active tab sits.
    fn place(&self, id: u64) -> Option<Place> {
        self.places(self.active)
            .into_iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, p)| p)
    }

    fn layout_panes(&mut self, ti: usize) {
        let places = self.places(ti);
        let Some(tab) = self.tabs.get_mut(ti) else {
            return;
        };
        for (id, Place { cells: r, .. }) in places {
            if let Some(p) = tab.panes.iter_mut().find(|p| p.id == id) {
                let (c, rw) = (r.cols.max(1) as u16, r.rows.max(1) as u16);
                // A pane's size is its source's: the writer asks for this
                // one, and the feed tells what it got.
                p.grid.lock().selection = None;
                let _ = p.writer.set_winsize(c, rw);
            }
        }
    }

    /// Recompute the cell grid from the window's pixel size (minus the chrome
    /// row) and relay it to every pane of every tab.
    fn apply_size(&mut self, px_w: u32, px_h: u32) {
        // The padding band surrounds the pane area on three sides plus the
        // gap under the (flush, full-width) chrome bar.
        let inner_w = (px_w as usize).saturating_sub(2 * self.pad);
        let inner_h = (px_h as usize).saturating_sub(2 * self.pad);
        let cols = ((inner_w / self.cell_w).max(1)) as u16;
        // The top bar and the bottom ribbon take their rows.
        let rows = ((inner_h.saturating_sub(self.bars_h()) / self.cell_h).max(1)) as u16;
        // Blocks are laid out in pixels, so any change of size moves them.
        self.cols = cols;
        self.rows = rows;
        for ti in 0..self.tabs.len() {
            self.layout_panes(ti);
        }
    }

    /// Whether the chrome holds still (pack D27, D50): the config override,
    /// else the OS accessibility setting.
    fn reduce_motion(&self) -> bool {
        motion::resolve(self.config.reduce_motion, motion::os_reduces_motion)
    }

    /// The chrome colours for the current theme (pack D34-D36, D38).
    fn chrome_colors(&self) -> Chrome {
        Chrome::from_theme(&self.theme)
    }

    /// Paint the chrome bar + the active tab's panes through the renderer.
    fn redraw(&mut self) {
        self.pacer.painted(Instant::now());
        let chrome = self.chrome_row();
        if self.strip_shown() {
            self.repaint_strip();
        }
        let status: Vec<Cell> = Vec::new();
        let tokens = self.chrome_colors();
        let divider = tokens.line;
        if self.overlay.is_some() {
            let page = self.build_overlay_grid().frame_view();
            let (Some(renderer), Some(window)) = (self.renderer.as_mut(), self.window.as_ref())
            else {
                return;
            };
            let size = window.inner_size();
            let frame = PaneFrame {
                grid: &page,
                col0: 0,
                row0: 0,
                px: (0, 0),
                focused: false,
                cursor_on: false,
                trail: Vec::new(),
                hover_link: None,
                marks: Vec::new(),
                search: SearchColors::default(),
            };
            let strip = self.strip.painted();
            let scene = Scene {
                strip: strip.map(|s| &s.pixmap),
                ..Scene::default()
            };
            renderer.render(
                std::slice::from_ref(&frame),
                &chrome,
                scene,
                &status,
                &mut self.font,
                &mut self.ui_font,
                size.width,
                size.height,
                divider,
                tokens.base, // the page is chrome, padding included
                tokens.frame,
                self.pad,
            );
            return;
        }
        let area = self.pane_area();
        let places = self.places(self.active);
        self.repaint_body(&places);
        let scale = self.scale_factor as f32;
        let popover = &mut self.popover;
        popover.repaint(tokens, scale);
        if let Some(confirm) = &mut self.confirm {
            confirm.repaint(tokens, scale);
        }
        let oy = self.grid_oy();
        // The command dock renders as one more (synthetic, unfocused) pane
        // right of the pane area. Built before the renderer is borrowed and
        // before the panes lock their grids — it locks the focused grid
        // itself, and the lock isn't reentrant.
        let dock_grid = if self.dock_cols() > 0 {
            let built = self.pane().map(|p| {
                let g = p.grid.lock();
                build_dock_grid(&g, &self.chrome_colors(), DOCK_CELLS, self.rows as usize)
            });
            let (grid, items) =
                built.unwrap_or_else(|| (Grid::new(DOCK_CELLS, self.rows as usize), Vec::new()));
            self.dock_items = items;
            Some(grid.frame_view())
        } else {
            self.dock_items = Vec::new();
            None
        };
        let (Some(renderer), Some(window)) = (self.renderer.as_mut(), self.window.as_ref()) else {
            return;
        };
        let size = window.inner_size();
        let Some(tab) = self.tabs.get(self.active) else {
            // An attached window showing no block (gui-design D61): its
            // strip, body and popover alone.
            let scene = attached_scene(
                &self.strip,
                &self.body,
                &self.popover,
                self.confirm.as_ref(),
            );
            renderer.render(
                &[],
                &chrome,
                scene,
                &status,
                &mut self.font,
                &mut self.ui_font,
                size.width,
                size.height,
                divider,
                self.theme.bg,
                tokens.frame,
                self.pad,
            );
            return;
        };
        // Taskbar progress (G01 stretch, Windows only): resolved once per
        // frame like the title below, from the same winit `Window`.
        #[cfg(windows)]
        let hwnd = {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            window.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(h) => {
                    Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND)
                }
                _ => None,
            })
        };
        if let Some(p) = tab.focused() {
            let g = p.grid.lock();
            let fallback = self.config.title.as_deref().unwrap_or("rusty_term");
            let title = if g.title.is_empty() {
                fallback
            } else {
                &g.title
            };
            window.set_title(title);
            #[cfg(windows)]
            if let Some(hwnd) = hwnd {
                super::taskbar::sync(hwnd, g.progress);
            }
            // Screen-reader tree (C20): cheap to call every frame — the
            // extraction is grid-sized and the actual platform update
            // no-ops unless assistive tech is attached (see `AccessState`).
            if let Some(access) = self.access.as_mut() {
                access.sync(title, &g);
            }
        }
        // Snapshot each pane's grid under a brief lock, then render from the
        // snapshots, so the parser never waits out compositing or present
        // (the chrome bar occupies screen row 0, so panes start at row 1).
        let blink = self.cursor_blink_on;
        let focus = tab.focus;
        let mut snaps = Vec::new();
        let mut shown = Vec::new();
        for &(id, place) in &places {
            if let Some(p) = tab.pane(id) {
                // A pane larger than its place (an attached block another
                // client sized) shows its top-left.
                let r = place.cells;
                snaps.push((p.frame_view().cropped(r.cols, r.rows), place, id == focus));
                shown.push(p);
            }
        }
        // Cursor trail (G36): when the focused pane's cursor jumped since the
        // last frame, remember the hop and paint fading ghosts along it for
        // TRAIL_MS (the tick loop keeps frames coming while one is live).
        const TRAIL_MS: u64 = 150;
        let mut ghosts: Vec<(usize, usize, f32)> = Vec::new();
        if self.config.cursor_trail.unwrap_or(false)
            && !motion::resolve(self.config.reduce_motion, motion::os_reduces_motion)
        {
            let now = Instant::now();
            if let Some((g, _, _)) = snaps.iter().find(|(_, _, foc)| *foc) {
                let cur = (g.cursor_visible && g.view_offset == 0).then_some(g.cursor);
                match (self.cursor_prev, cur) {
                    (Some((pid, prev)), Some(cur)) if pid == focus && prev != cur => {
                        self.trail = Some(Trail {
                            from: prev,
                            to: cur,
                            since: now,
                        });
                    }
                    _ => {}
                }
                self.cursor_prev = cur.map(|c| (focus, c));
            }
            if let Some(tr) = self.trail {
                let t = tr.since.elapsed().as_millis() as f32 / TRAIL_MS as f32;
                ghosts = super::cpu::trail_ghosts(tr.from, tr.to, t);
                if ghosts.is_empty() {
                    self.trail = None;
                }
            }
        } else {
            self.trail = None;
        }
        // Command gutter marks (per block): each row's BlockMark against the
        // terminal background its padding shows (D41, D42).
        let marks_on = self.config.command_marks.unwrap_or(true);
        let marks = Marks::on(self.theme.bg, self.theme.cursor, &tokens);
        let mark_color = |m: crate::core::BlockMark| match m {
            crate::core::BlockMark::Success => marks.success,
            crate::core::BlockMark::Error => marks.failure,
            crate::core::BlockMark::Running => marks.running,
        };
        let search = SearchColors::from_theme(&self.theme);
        let mut frames: Vec<PaneFrame> = snaps
            .iter()
            .map(|(g, place, foc)| PaneFrame {
                grid: g,
                // A block sits in pixels.
                col0: 0,
                row0: 0,
                px: (place.x.saturating_sub(self.pad), place.y.saturating_sub(oy)),
                focused: *foc,
                cursor_on: blink,
                trail: if *foc {
                    std::mem::take(&mut ghosts)
                } else {
                    Vec::new()
                },
                hover_link: if *foc { self.hover_link } else { None },
                marks: if marks_on {
                    g.block_marks()
                        .iter()
                        .enumerate()
                        .filter_map(|(row, m)| {
                            // A block's marks sit in its padding, on the
                            // terminal background.
                            m.map(|m| (row, mark_color(m)))
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                search,
            })
            .collect();
        if let Some(dg) = &dock_grid {
            frames.push(PaneFrame {
                grid: dg,
                col0: area.cols + 1, // right of the pane area's divider column
                row0: 0,
                px: (0, 0),
                focused: false,
                cursor_on: false,
                trail: Vec::new(),
                hover_link: None,
                marks: Vec::new(),
                search: SearchColors::default(),
            });
        }
        let scene = attached_scene(
            &self.strip,
            &self.body,
            &self.popover,
            self.confirm.as_ref(),
        );
        renderer.render(
            &frames,
            &chrome,
            scene,
            &status,
            &mut self.font,
            &mut self.ui_font,
            size.width,
            size.height,
            divider,
            self.theme.bg,
            tokens.frame,
            self.pad,
        );
        drop(frames);
        for ((view, _, _), p) in snaps.into_iter().zip(shown) {
            p.shown.replace(Some(view));
        }
    }

    /// The active tab's focused pane — the input, cursor, and search target.
    fn pane(&self) -> Option<&Pane> {
        self.tabs.get(self.active).and_then(|t| t.focused())
    }
    fn pane_mut(&mut self) -> Option<&mut Pane> {
        self.tabs.get_mut(self.active).and_then(|t| t.focused_mut())
    }
    /// Any pane by id across all tabs (reader/exit threads signal by pane id).
    fn pane_by_id(&self, id: u64) -> Option<&Pane> {
        self.tabs.iter().find_map(|t| t.pane(id))
    }
    fn pane_by_id_mut(&mut self, id: u64) -> Option<&mut Pane> {
        self.tabs
            .iter_mut()
            .find_map(|t| t.panes.iter_mut().find(|p| p.id == id))
    }

    /// Lay out the chrome bar for the current tabs/size: tab labels and the
    /// `+` on the left, the caption buttons (─ □ ×) right-aligned, drag strip
    /// between. Rebuilds the per-cell hit map as it goes.
    /// Cells the chrome bar spans. The bar is flush with the window (unlike
    /// the pad-inset grid), so its width comes from the window's pixels, not
    /// `self.cols`: `paint` cells cover the full width (the last may clip at
    /// the edge), while controls lay out within `layout` whole cells so the
    /// caption buttons reach the right edge without being cut.
    fn bar_cells(&self) -> (usize, usize) {
        let px = self
            .window
            .as_ref()
            .map(|w| w.inner_size().width)
            .unwrap_or(0) as usize;
        if px == 0 {
            let c = self.cols as usize;
            return (c, c);
        }
        (px.div_ceil(self.bar_w), px / self.bar_w)
    }

    /// Pixel height the bottom ribbon takes (gui-design D49).
    fn bottom_h(&self) -> usize {
        ribbon::height(self.scale_factor as f32)
    }

    /// The top row while the search bar has it (else the strip does).
    fn chrome_row(&mut self) -> Vec<Cell> {
        let Some(query) = self.searching.clone() else {
            self.hits.clear();
            return Vec::new();
        };
        let (paint_cols, cols) = self.bar_cells();
        let tokens = self.chrome_colors();
        let mut row = vec![Cell::blank(); paint_cols];
        let bar_bg = tokens.raised;
        for c in &mut row {
            c.fg = tokens.text;
            c.bg = bar_bg;
        }
        let status = self.pane().and_then(|p| p.grid.lock().search_status());
        let count = match status {
            Some((cur, total)) => format!(" {cur}/{total} "),
            None if query.is_empty() => String::new(),
            None => " no matches ".to_string(),
        };
        let limit = cols.saturating_sub(count.chars().count());
        let mut hits = vec![Hit::Drag; paint_cols];
        let mut col = 0;
        let mode = if self.search_regex {
            " Find(re): "
        } else {
            " Find: "
        };
        put_text(
            &mut row,
            &mut hits,
            &mut col,
            limit,
            &format!("{mode}{query}"),
            tokens.text,
            bar_bg,
            Hit::Drag,
        );
        let mut ccol = limit;
        put_text(
            &mut row,
            &mut hits,
            &mut ccol,
            cols,
            &count,
            tokens.text,
            bar_bg,
            Hit::Drag,
        );
        self.hits = hits;
        row
    }

    /// Build the configured renderer for `window`: the GPU one when `--gpu` is
    /// passed and `gui-gpu` is built (falling back to CPU on failure), else CPU.
    /// Also sets output repaint pacing to suit it.
    fn make_renderer(&mut self, window: Arc<Window>) -> Option<Box<dyn Renderer>> {
        let renderer = self.build_renderer(window.clone())?;
        if renderer.waits_for_vsync() {
            self.pacer.disable();
        } else {
            let rate = window
                .current_monitor()
                .and_then(|m| m.refresh_rate_millihertz());
            self.pacer.set_refresh(rate);
        }
        Some(renderer)
    }

    fn build_renderer(&mut self, window: Arc<Window>) -> Option<Box<dyn Renderer>> {
        #[cfg(feature = "gui-gpu")]
        if gpu_asked() {
            match super::gpu::GpuRenderer::new(window.clone(), &mut self.font) {
                Ok(r) => return Some(Box::new(r)),
                Err(e) => eprintln!("GPU renderer unavailable ({e}); falling back to CPU"),
            }
        }
        match CpuRenderer::new(window) {
            Ok(r) => Some(Box::new(r)),
            Err(e) => {
                eprintln!("CPU renderer failed: {e}");
                None
            }
        }
    }

    /// Pixel height of the chrome bar plus its downward inset (the strip
    /// band above the tabs; see `render::bar_inset`): the bar's hit area.
    fn chrome_h(&self) -> usize {
        if self.strip_shown() {
            return strip::height(self.scale_factor as f32);
        }
        super::render::bar_inset(self.pad, self.bar_h) + self.bar_h
    }

    /// Whether the top strip stands in for the chrome bar: in an attached
    /// window, unless the search bar has the top row.
    fn strip_shown(&self) -> bool {
        self.searching.is_none()
    }

    /// The chrome control under window pixel `(x, y)`, which the caller
    /// knows is on the bar; `None` before a bar is laid out.
    fn bar_hit(&self, x: f64, y: f64) -> Option<Hit> {
        if self.strip_shown() {
            let strip = self.strip.painted()?;
            return Some(strip.spot(x as f32, y as f32).map_or(Hit::Drag, Hit::from));
        }
        (!self.hits.is_empty()).then(|| self.hits[self.bar_col(x)])
    }

    /// Paint the attached window's strip for the current state and size.
    fn repaint_strip(&mut self) {
        let width = self
            .window
            .as_ref()
            .map_or(0, |w| w.inner_size().width as usize);
        let look = strip::Look {
            chrome: self.chrome_colors(),
            scale: self.scale_factor as f32,
            hover: self.hover.and_then(Hit::spot),
            pressed: self.pressed.and_then(Hit::spot),
            detached_open: self.popover_kind() == Some(Kind::Detached),
            // An ended session takes no new window either.
            offline: self.offline() || self.body.ended().is_some(),
        };
        self.strip.repaint(look, width);
    }

    /// Paint the attached window's blocks at `places` for the current
    /// state, focus and size: each block's frame and header, its F2 badge
    /// while the prefix is armed, the zoomed window's rail and the ribbon.
    fn repaint_body(&mut self, places: &[(u64, Place)]) {
        let Some(size) = self.window.as_ref().map(|w| w.inner_size()) else {
            return;
        };
        let detached = self.strip.session().detached.len();
        let Some(tab) = self.tabs.get(self.active) else {
            self.repaint_empty(detached);
            return;
        };
        let (focus, zoomed, ids) = (tab.focus, tab.zoomed, tab.layout.ids());
        let scale = self.scale_factor as f32;
        let top = self.chrome_h();
        let area = self.body_area();
        let look = blocks::Look {
            chrome: self.chrome_colors(),
            bg: self.theme.bg,
            scale,
            hover: self.hover.and_then(Hit::block),
            pressed: self.pressed.and_then(Hit::block),
        };
        let armed = self.armed;
        let body = &mut self.body;
        // F2 numbers follow the layout's order, 1 to 9 (handoff D104).
        let number = |id: u64| {
            ids.iter()
                .position(|&k| k == id)
                .map(|i| i + 1)
                .filter(|&n| n <= 9)
        };
        let up = |r: Rect| Rect::new(r.col, r.row.saturating_sub(top), r.cols, r.rows);
        let shown = places
            .iter()
            .map(|&(id, p)| blocks::Shown {
                key: id,
                rect: up(p.block),
                focused: id == focus,
                zoomed,
                header: body.header(id, read_git_branch),
                badge: number(id).filter(|_| armed),
            })
            .collect();
        let hidden: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&k| zoomed && k != focus)
            .collect();
        let rail = blocks::zoomed(area, hidden.len(), scale)
            .1
            .map(|rail| blocks::chips(rail, hidden.len(), scale))
            .unwrap_or_default();
        let rail = hidden
            .iter()
            .zip(rail)
            .map(|(&key, rect)| blocks::Chip {
                key,
                rect: up(rect),
                lead: body.lead(key),
                number: number(key).unwrap_or_default(),
            })
            .collect();
        let scene = blocks::Scene {
            blocks: shown,
            rail,
            ribbon: body.ribbon(ids.len(), detached, Some(focus), armed),
            empty: None,
            banner: body.banner(),
            dim: body.offline(),
            ended: None,
        };
        let height = (size.height as usize).saturating_sub(top);
        body.repaint(size.width as usize, height, scene, look);
    }

    /// Paint a body showing no block: with `detached` blocks out of the
    /// layout it says so (gui-design D61); with none yet, nothing.
    fn repaint_empty(&mut self, detached: usize) {
        let Some(size) = self.window.as_ref().map(|w| w.inner_size()) else {
            return;
        };
        let top = self.chrome_h();
        let look = blocks::Look {
            chrome: self.chrome_colors(),
            bg: self.theme.bg,
            scale: self.scale_factor as f32,
            hover: None,
            pressed: None,
        };
        let armed = self.armed;
        let body = &mut self.body;
        let ended = body.ended().map(str::to_owned);
        if detached == 0 && body.banner().is_none() && ended.is_none() {
            return;
        }
        let scene = blocks::Scene {
            ribbon: body.ribbon(0, detached, None, armed),
            empty: (detached > 0 && ended.is_none()).then_some(detached),
            ended,
            banner: body.banner(),
            dim: body.offline(),
            ..blocks::Scene::default()
        };
        let height = (size.height as usize).saturating_sub(top);
        body.repaint(size.width as usize, height, scene, look);
    }

    /// The pixel area an attached window's blocks tile: under the strip,
    /// above the ribbon, inset (gui-design D47).
    fn body_area(&self) -> Rect {
        let size = self.window.as_ref().map(|w| w.inner_size());
        let (width, height) = size.map_or((0, 0), |s| (s.width as usize, s.height as usize));
        let scale = self.scale_factor as f32;
        blocks::area(width, height, self.chrome_h(), self.bottom_h(), scale)
    }

    /// The body control under window pixel `(x, y)`, if any.
    fn body_hit(&self, x: f64, y: f64) -> Option<blocks::Spot> {
        let body = self.body.painted()?;
        body.spot(x as f32, (y as f32) - self.chrome_h() as f32)
    }

    /// Pixel height both bars take from the pane area: the top bar (with
    /// its inset) plus the bottom ribbon.
    fn bars_h(&self) -> usize {
        self.chrome_h() + self.bottom_h()
    }

    /// The grid content's vertical pixel offset: the padding band below the
    /// chrome bar (see `render::grid_top`).
    fn grid_oy(&self) -> usize {
        self.pad + self.chrome_h()
    }

    /// Whether window pixel row `y` is on the chrome bar.
    fn in_chrome(&self, y: f64) -> bool {
        (y.max(0.0) as usize) < self.chrome_h()
    }

    /// The chrome bar cell under window pixel column `x`, clamped to the hit
    /// map. Callers check `hits` is non-empty first.
    fn bar_col(&self, x: f64) -> usize {
        (x.max(0.0) as usize / self.bar_w).min(self.hits.len() - 1)
    }

    /// Map a physical pixel position to a clamped `(col, row)` *grid* cell.
    fn cell_at(&self, px: f64, py: f64) -> (usize, usize) {
        let pad = self.pad as f64;
        let col = ((px - pad).max(0.0) as usize / self.cell_w)
            .min((self.cols as usize).saturating_sub(1));
        let row = ((py - self.grid_oy() as f64).max(0.0) as usize / self.cell_h)
            .min((self.rows as usize).saturating_sub(1));
        (col, row)
    }

    /// The resize direction for a pointer near the window edge, `None` away
    /// from the edges or while maximized (a maximized window doesn't resize).
    fn resize_zone(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        let window = self.window.as_ref()?;
        if window.is_maximized() {
            return None;
        }
        let size = window.inner_size();
        let (w, h) = (size.width as f64, size.height as f64);
        // `x`/`y` are physical pixels (as is `inner_size()`); scale the grab
        // zone by the monitor's DPI factor so it stays a consistent *logical*
        // size instead of shrinking to a barely-hittable sliver on HiDPI
        // displays (6 physical px is ~3 logical px at 200%).
        let border = RESIZE_BORDER * self.scale_factor;
        let (l, r) = (x < border, x > w - border);
        let (t, b) = (y < border, y > h - border);
        Some(match (l, r, t, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    /// Left button pressed: edge band starts a drag-resize, the chrome bar
    /// dispatches its hit action, anywhere else anchors a drag-selection.
    fn on_left_press(&mut self) {
        let (x, y) = self.mouse_pos;
        if let Some(dir) = self.resize_zone(x, y) {
            if let Some(window) = &self.window {
                let _ = window.drag_resize_window(dir);
            }
            return;
        }
        // A click ends the prefix mode, then acts as usual (D49).
        self.set_armed(false);
        if self.in_chrome(y) {
            return self.on_bar_click(x);
        }
        if self.overlay.is_some() {
            return self.overlay_click(x, y);
        }
        if let Some(spot) = self.body_hit(x, y) {
            return self.on_body_click(spot);
        }
        // Dimmed blocks take no clicks (D62).
        if self.offline() {
            return;
        }
        // A click on the command dock jumps the focused pane's scrollback to
        // the clicked command block; dock clicks never reach the terminal.
        if self.dock_cols() > 0 {
            let (col, row) = self.cell_at(x, y);
            if col >= self.pane_area().cols {
                if let Some(abs) = self.dock_items.get(row).copied().flatten()
                    && let Some(p) = self.pane()
                {
                    p.grid.lock().scroll_to_abs(abs);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                return;
            }
        }
        if let Some(id) = self.pane_under(x, y)
            && let Some(tab) = self.tabs.get_mut(self.active)
        {
            tab.focus = id; // click focuses the pane under the pointer
        }
        let cell = self.cell_in_focused(x, y);
        // Bidi: selection anchors, word/line select, click-to-move, and the
        // summary check below are all logical-cell consumers.
        let cell = match self.pane() {
            Some(p) => (p.grid.lock().logical_col(cell.0, cell.1), cell.1),
            None => cell,
        };
        // A click on a fold-summary line expands the folded command block
        // (C17') instead of starting a selection.
        if let Some(p) = self.pane()
            && p.grid.lock().unfold_summary_at(cell.1)
        {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        }
        // Click-to-move-cursor (G21): a plain first click at the shell
        // prompt sends the arrow presses that walk the readline cursor to
        // the clicked cell. Mouse-tracking apps own their clicks, so this
        // only fires when reporting is off — or Shift bypasses it, same as
        // `report_mouse`'s escape hatch below; drag-selection still arms
        // below either way (the arrows only ever move within the prompt's
        // own line).
        if self.config.click_to_move.unwrap_or(true)
            && let Some(p) = self.pane()
        {
            let moves = {
                let g = p.grid.lock();
                if g.mouse_modes.active() && !self.mods.shift_key() {
                    None
                } else {
                    g.prompt_cursor_moves(cell.0, cell.1)
                        .map(|m| (m, g.app_cursor_keys))
                }
            };
            if let Some(((dx, dy), app_cursor)) = moves {
                let bytes = arrow_presses(dx, dy, app_cursor);
                self.pacer.input(Instant::now());
                if let Some(p) = self.pane_mut() {
                    let _ = p.writer.write(&bytes);
                }
            }
        }
        // Consecutive clicks on the same cell escalate the selection: one
        // arms a drag, two select the word under the pointer, three the
        // whole (soft-wrap-joined) line; a fourth starts over.
        let now = Instant::now();
        self.click_streak = match self.last_grid_click {
            Some((t, c)) if c == cell && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS => {
                self.click_streak % 3 + 1
            }
            _ => 1,
        };
        self.last_grid_click = Some((now, cell));
        let anchor = self
            .pane()
            .map(|p| (cell.0, p.grid.lock().abs_of_view_row(cell.1)));
        // Every streak arms a drag now, not just a plain click: dragging
        // after a double/triple click extends the selection by whole
        // word/line (`CursorMoved` below reads `click_streak` to pick the
        // granularity), instead of the drag being inert past the initial
        // word/line as it was before.
        self.drag.start(anchor);
        if let Some(p) = self.pane() {
            let mut g = p.grid.lock();
            match self.click_streak {
                2 => g.select_word_at(cell.0, cell.1),
                3 => g.select_line_at(cell.1),
                _ => g.selection = None, // cleared until the drag moves
            }
        }
        if self.click_streak > 1 {
            self.copy_selection_primary(); // copy-on-select
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The pane under pixel `(px, py)` in the active tab's grid area, if any.
    fn pane_under(&self, px: f64, py: f64) -> Option<u64> {
        let (x, y) = (px.max(0.0) as usize, py.max(0.0) as usize);
        // Anywhere on a block is that block (handoff D102).
        self.places(self.active)
            .into_iter()
            .find(|(_, p)| p.block.contains(x, y))
            .map(|(id, _)| id)
    }

    /// Map pixel `(px, py)` to a cell within the *focused* pane, clamped to it.
    fn cell_in_focused(&self, px: f64, py: f64) -> (usize, usize) {
        match self.tabs.get(self.active) {
            Some(tab) => self.cell_in_pane(tab.focus, px, py),
            None => self.cell_at(px, py),
        }
    }

    /// Map pixel `(px, py)` to a cell within pane `id`, clamped to it. Used
    /// both for the focused pane (`cell_in_focused`) and for routing wheel
    /// events to whichever pane is under the pointer, regardless of focus.
    fn cell_in_pane(&self, id: u64, px: f64, py: f64) -> (usize, usize) {
        let Some(p) = self.place(id) else {
            return self.cell_at(px, py);
        };
        let col = (px - p.x as f64).max(0.0) as usize / self.cell_w.max(1);
        let row = (py - p.y as f64).max(0.0) as usize / self.cell_h.max(1);
        (
            col.min(p.cells.cols.saturating_sub(1)),
            row.min(p.cells.rows.saturating_sub(1)),
        )
    }

    /// Where the focused pane sits, `None` before any tab exists.
    fn focused_place(&self) -> Option<Place> {
        let tab = self.tabs.get(self.active)?;
        self.place(tab.focus)
    }

    /// The focused pane's id, `None` before any tab exists.
    fn focused_pane_id(&self) -> Option<u64> {
        self.tabs.get(self.active).map(|t| t.focus)
    }

    /// Update the in-progress drag-selection's head to the cell at pixel
    /// `(hx, hy)` (clamped to the focused pane), applying word/line
    /// extension per `click_streak`. Returns whether a selection was
    /// updated. Shared by `CursorMoved`'s live drag and the edge-auto-scroll
    /// tick below, which re-applies it each scroll step at the pointer's
    /// last known (off-pane) position.
    fn update_drag_selection_head(&mut self, hx: f64, hy: f64) -> bool {
        let (Some(anchor), Some(p)) = (self.drag.live_anchor(), self.pane()) else {
            return false;
        };
        let (hc, hr) = self.cell_in_focused(hx, hy);
        let mut g = p.grid.lock();
        let hc = g.logical_col(hc, hr);
        let head = (hc, g.abs_of_view_row(hr));
        match self.click_streak {
            2 => g.extend_word_selection(anchor, head),
            3 => g.extend_line_selection(anchor.1, head.1),
            _ => g.selection = Some(Selection { anchor, head }),
        }
        true
    }

    /// While dragging a selection with the pointer past the focused pane's
    /// top/bottom edge, scroll the viewport one line into/out of history so
    /// text beyond what's currently visible can still be reached, extending
    /// the selection to the new edge row. Returns whether it scrolled, so
    /// `tick` knows to keep waking up at `DRAG_SCROLL_INTERVAL` — held past
    /// the edge with the pointer stationary should keep scrolling, not just
    /// scroll once per `CursorMoved`.
    fn drag_edge_autoscroll(&mut self) -> bool {
        if !self.drag.is_live() {
            return false;
        }
        let Some(p) = self.focused_place() else {
            return false;
        };
        let top_px = p.y as f64;
        let bottom_px = (p.y + p.cells.rows * self.cell_h) as f64;
        let Some(scroll_up) =
            drag_scroll_direction(self.mouse_pos.1, top_px, bottom_px, DRAG_SCROLL_MARGIN)
        else {
            return false;
        };
        let Some(p) = self.pane() else { return false };
        let moved = {
            let mut g = p.grid.lock();
            if scroll_up {
                g.scroll_view_up(1)
            } else {
                g.scroll_view_down(1)
            }
        };
        if !moved {
            return false;
        }
        self.update_drag_selection_head(self.mouse_pos.0, self.mouse_pos.1);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    /// A click on a block control (gui-design D49).
    fn on_body_click(&mut self, spot: blocks::Spot) {
        self.pressed = Some(Hit::Block(spot));
        match spot {
            blocks::Spot::Zoom(key) => self.act_on_source(|_, _| Act::Zoom(key)),
            blocks::Spot::Unzoom => self.act_on_source(|_, _| Act::Unzoom),
            // Focusing a hidden block unzooms, as on the server.
            blocks::Spot::Chip(key) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.focus = key;
                }
            }
            blocks::Spot::Banner(action) => self.on_banner(action),
            blocks::Spot::OpenSwitcher => self.toggle_popover(Kind::Switcher),
            blocks::Spot::Take(key, button) => self.on_take(key, button),
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Dispatch a click on the chrome bar through the hit map.
    fn on_bar_click(&mut self, x: f64) {
        let Some(hit) = self.bar_hit(x, self.mouse_pos.1) else {
            return; // no frame laid out yet
        };
        let Some(window) = self.window.clone() else {
            return;
        };
        if hit != Hit::Drag {
            self.pressed = Some(hit);
            window.request_redraw();
        }
        match hit {
            Hit::Window(key) => {
                self.close_overlay();
                self.source.select(key);
            }
            Hit::NewWindow => {
                self.close_overlay();
                self.source.new_window();
            }
            Hit::Session => {
                self.close_overlay();
                self.toggle_popover(Kind::Switcher);
            }
            Hit::Detached => {
                self.close_overlay();
                self.toggle_popover(Kind::Detached);
            }
            Hit::Minimize => window.set_minimized(true),
            Hit::Maximize => window.set_maximized(!window.is_maximized()),
            Hit::Close => self.closed = true,
            // Never on the bar: block controls sit under it.
            Hit::Block(_) => {}
            Hit::Drag => {
                let now = Instant::now();
                if self
                    .last_strip_click
                    .take()
                    .is_some_and(|last| now.duration_since(last).as_millis() <= DOUBLE_CLICK_MS)
                {
                    window.set_maximized(!window.is_maximized());
                } else {
                    self.last_strip_click = Some(now);
                    let _ = window.drag_window();
                }
            }
        }
    }

    /// [`Action::CopyOrSend`] (Ctrl+C on Windows): copy the selection and
    /// clear it so the next press interrupts. Returns whether the key was
    /// consumed; with nothing selected it was not, and the caller lets it
    /// reach the child as ^C. A failed copy keeps the selection for a retry
    /// and still consumes the key, so it never turns into a surprise ^C.
    fn copy_or_send(&mut self) -> bool {
        let copied = self.copy_selection();
        if copied.clears_selection() {
            if let Some(p) = self.pane() {
                p.grid.lock().selection = None;
            }
            // End a drag still in progress, or the next pointer move or
            // edge auto-scroll tick would rebuild the selection.
            self.drag.cancel();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        copied.consumes_key()
    }

    /// Copy the active tab's selection to the system clipboard (Ctrl+Shift+C).
    /// A failed write is reported on stderr and leaves the selection alone.
    fn copy_selection(&mut self) -> Copied {
        let Some(p) = self.pane() else {
            return Copied::NothingSelected;
        };
        let (text, html) = {
            let g = p.grid.lock();
            let html = if self.config.copy_html.unwrap_or(true) {
                g.selected_html()
            } else {
                None
            };
            (g.selected_text(), html)
        };
        // The HTML flavor carries the selection's colors/attributes for
        // rich-paste targets (G29); plain editors read the text flavor.
        let write = self.clipboard.as_mut().map(|cb| {
            move |text: String| {
                match html {
                    Some(html) => cb.set_html(html, Some(text)),
                    None => cb.set_text(text),
                }
                .map_err(|e| e.to_string())
            }
        });
        let copied = copy_text(text, write);
        if let Copied::Failed(e) = &copied {
            eprintln!("rusty_term: copy: {e}");
        }
        copied
    }

    /// Service a tab's pending OSC 52 clipboard request recorded by the parser.
    /// A set copies the child's text to the system clipboard; a query replies to
    /// the child from the system clipboard. Called on a tab's output, so
    /// background tabs are serviced too.
    ///
    /// Gated by the `clipboard` config policy (default write-only): with no
    /// gate, any program that can write to the pty could read `52;c;?` and
    /// have the terminal hand back whatever the user last copied — passwords,
    /// tokens, anything on the clipboard — with zero user interaction. Sets
    /// are lower-risk (the common case is a program copying its own output,
    /// e.g. over SSH) so they're allowed by default; queries require opting
    /// into `clipboard = "read-write"`.
    fn service_clipboard(&mut self, id: u64) {
        let policy = self.config.clipboard.unwrap_or_default();
        let (set, set_primary, query, query_primary) = {
            let Some(p) = self.pane_by_id(id) else { return };
            let mut g = p.grid.lock();
            if g.clipboard_set.is_none()
                && g.clipboard_set_primary.is_none()
                && !g.clipboard_query
                && !g.clipboard_query_primary
            {
                return;
            }
            (
                g.clipboard_set.take(),
                g.clipboard_set_primary.take(),
                std::mem::take(&mut g.clipboard_query),
                std::mem::take(&mut g.clipboard_query_primary),
            )
        };
        // Drain the pending request regardless of policy (above) so a denied
        // request doesn't re-fire on every subsequent poll; only actually
        // touch the clipboard when the policy allows it.
        let allow_write = policy != ClipboardPolicy::Off;
        let allow_read = policy == ClipboardPolicy::ReadWrite;
        if allow_write
            && let Some(text) = set
            && let Some(cb) = self.clipboard.as_mut()
        {
            let _ = cb.set_text(text);
        }
        if allow_write
            && let Some(text) = set_primary
            && let Some(cb) = self.clipboard.as_mut()
        {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                use arboard::SetExtLinux as _;
                let _ = cb
                    .set()
                    .clipboard(arboard::LinuxClipboardKind::Primary)
                    .text(text);
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            let _ = cb.set_text(text); // no primary selection: best effort
        }
        if allow_read
            && query
            && let Some(text) = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok())
        {
            let reply = osc52_reply('c', &text);
            if let Some(p) = self.pane_by_id_mut(id) {
                let _ = p.writer.write(&reply);
            }
        }
        if allow_read
            && query_primary
            && let Some(text) = self.primary_text()
        {
            let reply = osc52_reply('p', &text);
            if let Some(p) = self.pane_by_id_mut(id) {
                let _ = p.writer.write(&reply);
            }
        }
    }

    /// Raise any desktop notifications (OSC 9/777) the tab's child queued. Run on
    /// a tab's output, so background tabs notify too.
    fn service_notifications(&mut self, id: u64) {
        let notes = {
            let Some(p) = self.pane_by_id(id) else { return };
            let mut g = p.grid.lock();
            if g.notifications.is_empty() {
                return;
            }
            std::mem::take(&mut g.notifications)
        };
        for (title, body) in notes {
            let title = if title.is_empty() {
                "rusty_term"
            } else {
                title.as_str()
            };
            notify(title, &body);
        }
    }

    /// Drain a pane's bell ring and finished-command records and raise the
    /// configured alerts: a bell requests window attention when the window is
    /// unfocused and badges a background tab; a command that ran at least
    /// `command_notify_secs` and finished while the window was unfocused (or
    /// its tab in the background) raises a desktop notification. Alerts for
    /// the active tab of a focused window are dropped — the user watched it
    /// happen.
    fn service_alerts(&mut self, id: u64) {
        let (bell, finished) = {
            let Some(p) = self.pane_by_id(id) else { return };
            let mut g = p.grid.lock();
            (
                std::mem::take(&mut g.bell),
                std::mem::take(&mut g.finished_commands),
            )
        };
        if !bell && finished.is_empty() {
            return;
        }
        let tab_idx = self
            .tabs
            .iter()
            .position(|t| t.panes.iter().any(|p| p.id == id));
        let background = tab_idx != Some(self.active);
        let unseen = background || !self.focused;
        let mut badge = false;
        if bell && self.config.bell.unwrap_or(true) && unseen {
            if !self.focused
                && let Some(window) = &self.window
            {
                window.request_user_attention(Some(UserAttentionType::Informational));
            }
            badge = true;
        }
        let threshold = self
            .config
            .command_notify_secs
            .unwrap_or(super::settings::DEFAULT_NOTIFY_SECS);
        for (exit, runtime) in finished {
            if threshold != 0 && runtime.as_secs() >= threshold && unseen {
                let status = match exit {
                    Some(0) => "succeeded".to_string(),
                    Some(code) => format!("failed (exit {code})"),
                    None => "finished".to_string(),
                };
                notify(
                    "Command finished",
                    &format!("A command {status} after {}s", runtime.as_secs()),
                );
                badge = true;
            }
        }
        if badge
            && let Some(i) = tab_idx
            && let Some(tab) = self.tabs.get_mut(i)
        {
            tab.attention = true;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    /// Run a key's action; returns whether it consumed the key. Only
    /// [`Action::CopyOrSend`] with nothing selected leaves it for the child.
    fn exec_action(&mut self, action: Action) -> bool {
        if action == Action::CopyOrSend {
            return self.copy_or_send();
        }
        self.run_action(action);
        true
    }

    /// Dispatch a terminal-owned [`Action`] resolved from the keymap.
    fn run_action(&mut self, action: Action) {
        match action {
            Action::Copy => {
                self.copy_selection();
            }
            Action::CopyOrSend => {
                self.copy_or_send();
            }
            Action::Paste => self.paste(),
            // Local tabs and windows retired with local mode (D159); the
            // actions stay bindable (public `keymap`) and do nothing here.
            Action::NewTab | Action::NewWindow => {}
            // Collapse/expand the last finished command's output (C17'):
            // its rows fold to a one-line "N lines hidden" summary.
            Action::FoldOutput => {
                if let Some(p) = self.pane()
                    && p.grid.lock().toggle_last_fold()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
            }
            // Toggle the command dock: the pane area shrinks/grows, so every
            // tab relayouts (grids + PTY winsize) to the new width.
            Action::ToggleDock => {
                self.dock_open = !self.dock_open;
                for ti in 0..self.tabs.len() {
                    self.layout_panes(ti);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Action::CloseTab => self.act_on_source(|key, _| Act::Close(key)),
            Action::NextTab => self.cycle_tab(true),
            Action::PrevTab => self.cycle_tab(false),
            Action::OpenConfig => self.open_config(),
            Action::OpenSettings => self.open_settings(),
            Action::OpenSetup => self.open_setup(),
            Action::Search => self.start_search(),
            Action::OpenLinks => self.open_links_menu(),
            Action::SplitRight | Action::SplitDown => {
                let dir = match action {
                    Action::SplitRight => Dir::Vertical,
                    _ => Dir::Horizontal,
                };
                self.act_on_source(|key, _| Act::Split { key, dir });
            }
            Action::FocusNext => self.focus_pane(true),
            Action::FocusLeft => self.focus_block_dir(-1, 0),
            Action::FocusRight => self.focus_block_dir(1, 0),
            Action::FocusUp => self.focus_block_dir(0, -1),
            Action::FocusDown => self.focus_block_dir(0, 1),
            // The source lays the window out (pack D102).
            Action::ResizeLeft | Action::ResizeRight | Action::ResizeUp | Action::ResizeDown => {}
            Action::ZoomPane => self.toggle_source_zoom(),
            Action::CopyMode => self.enter_copy_mode(),
            Action::Broadcast => {
                self.broadcast = !self.broadcast;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Action::ScrollPageUp => self.scroll_key(false, true),
            Action::ScrollPageDown => self.scroll_key(false, false),
            Action::ScrollPromptUp => self.scroll_key(true, true),
            Action::ScrollPromptDown => self.scroll_key(true, false),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::FontSizeUp => self.zoom_font(Zoom::In),
            Action::FontSizeDown => self.zoom_font(Zoom::Out),
            Action::FontSizeReset => self.zoom_font(Zoom::Reset),
        }
    }

    /// Flip the window in/out of fullscreen (F11 / Alt+Enter conventions —
    /// previously only reachable via `launch_mode` at startup).
    fn toggle_fullscreen(&mut self) {
        let Some(window) = &self.window else { return };
        let now_fullscreen = window.fullscreen().is_some();
        window.set_fullscreen((!now_fullscreen).then_some(Fullscreen::Borderless(None)));
    }

    /// Zoom the terminal text (Ctrl+=/Ctrl+-/Ctrl+0, the Chrome/VS Code/
    /// iTerm2/Windows Terminal convention); see [`zoom_px`]. Only the panes'
    /// font changes: the chrome and status bars keep the UI font, and the
    /// configured size is untouched, so a zoom never persists.
    fn zoom_font(&mut self, zoom: Zoom) {
        let px = zoom_px(zoom, self.font_px, self.base_px, self.scale_factor);
        let ligatures = self.config.ligatures.unwrap_or(true);
        self.rebuild_font_at(px, ligatures);
    }

    /// Enter incremental scrollback-search mode with an empty query.
    fn start_search(&mut self) {
        self.searching = Some(String::new());
        self.grid_clear_search();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Handle a key while in search mode: edit the query, step matches (Enter /
    /// Shift+Enter), or exit (Esc). Returns whether it was consumed — `false`
    /// when not searching, so normal handling continues.
    fn search_key(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if self.searching.is_none() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.searching = None;
                self.grid_clear_search();
            }
            Key::Named(NamedKey::Enter) => {
                let forward = !self.mods.shift_key();
                if let Some(p) = self.pane() {
                    p.grid.lock().search_jump(forward);
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(q) = self.searching.as_mut() {
                    q.pop();
                }
                self.run_search();
            }
            Key::Character(s) if self.mods.control_key() && s.as_str() == "r" => {
                self.search_regex = !self.search_regex;
                self.run_search();
            }
            Key::Character(s)
                if self.mods.alt_key() && !self.mods.control_key() && s.as_str() == "c" =>
            {
                self.search_case_sensitive = !self.search_case_sensitive;
                self.run_search();
            }
            Key::Character(s) if self.mods.control_key() && s.as_str() == "v" => {
                let text = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok());
                self.paste_into_search(text);
            }
            Key::Character(s) if !self.mods.control_key() && !self.mods.alt_key() => {
                if let Some(q) = self.searching.as_mut() {
                    q.push_str(s);
                }
                self.run_search();
            }
            _ => {}
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    /// Re-run the active tab's search for the current query.
    fn run_search(&mut self) {
        let q = self.searching.clone().unwrap_or_default();
        let (regex, case_sensitive) = (self.search_regex, self.search_case_sensitive);
        if let Some(p) = self.pane() {
            p.grid.lock().search_with_case(&q, regex, case_sensitive);
        }
    }

    /// Append clipboard text into the find bar's query — Ctrl+V (regular
    /// clipboard) or a middle-click while it's open (PRIMARY selection,
    /// mirroring `paste_primary`'s convention). Only the first line
    /// contributes: a query is one line, and a pasted multi-line clipboard
    /// would otherwise produce a query no match could ever satisfy.
    fn paste_into_search(&mut self, text: Option<String>) {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return;
        };
        if let Some(q) = self.searching.as_mut() {
            q.push_str(text.lines().next().unwrap_or(""));
        }
        self.run_search();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Clear the active tab's search highlights.
    fn grid_clear_search(&mut self) {
        if let Some(p) = self.pane() {
            p.grid.lock().clear_search();
        }
    }

    /// Switch to the next (`forward`) or previous tab, wrapping; a no-op with one
    /// tab. Repaints since the active grid changed.
    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 1 {
            self.active = if forward {
                (self.active + 1) % n
            } else {
                (self.active + n - 1) % n
            };
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    /// Jump straight to tab `i` (Ctrl+Alt+digit); out-of-range is ignored.
    fn select_tab(&mut self, i: usize) {
        if i < self.tabs.len() && i != self.active {
            self.active = i;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    /// Tell the IME where the text cursor is, so its candidate/composition popup
    /// appears at the terminal cursor rather than the window origin.
    fn update_ime_area(&self) {
        let (Some(window), Some(p)) = (&self.window, self.pane()) else {
            return;
        };
        let (col, row) = p.grid.lock().cursor;
        // The cursor is pane-local; add the focused pane's own rect offset
        // within the split layout, or the popup lands at the cursor's
        // position within an unsplit *first* pane regardless of which split
        // pane is actually focused (e.g. a right/bottom split composing IME
        // would show the candidate window over the wrong pane entirely).
        let (x0, y0) = self
            .focused_place()
            .map_or((self.pad, self.grid_oy()), |p| (p.x, p.y));
        let (x, y) = ime_cursor_area_origin(x0, y0, self.cell_w, self.cell_h, (0, 0), (col, row));
        window.set_ime_cursor_area(
            winit::dpi::PhysicalPosition::new(x, y),
            winit::dpi::PhysicalSize::new(self.cell_w as u32, self.cell_h as u32),
        );
    }

    /// Write child input to the focused pane — or, while broadcast is on,
    /// to every pane in the active tab.
    fn write_child(&mut self, bytes: &[u8]) {
        self.pacer.input(Instant::now());
        if !self.broadcast {
            if let Some(p) = self.pane_mut() {
                let _ = p.writer.write(bytes);
            }
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            for p in &mut tab.panes {
                let _ = p.writer.write(bytes);
            }
        }
    }

    /// Copy the focused pane's selection to the PRIMARY selection
    /// (X11/Wayland copy-on-select; a no-op elsewhere — macOS/Windows have
    /// no primary selection concept).
    fn copy_selection_primary(&mut self) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let Some(p) = self.pane() else { return };
            let text = p.grid.lock().selected_text();
            if let (Some(text), Some(cb)) =
                (text.filter(|t| !t.is_empty()), self.clipboard.as_mut())
            {
                use arboard::SetExtLinux as _;
                let _ = cb
                    .set()
                    .clipboard(arboard::LinuxClipboardKind::Primary)
                    .text(text);
            }
        }
    }

    /// The PRIMARY selection's text (X11/Wayland), or the regular clipboard
    /// where no primary selection exists.
    fn primary_text(&mut self) -> Option<String> {
        let cb = self.clipboard.as_mut()?;
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use arboard::GetExtLinux as _;
            cb.get()
                .clipboard(arboard::LinuxClipboardKind::Primary)
                .text()
                .ok()
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            cb.get_text().ok()
        }
    }

    /// Middle-click pastes the PRIMARY selection (X11 muscle memory), unless
    /// the child is tracking the mouse — then the click is the child's.
    fn paste_primary(&mut self) {
        let Some(text) = self.primary_text().filter(|t| !t.is_empty()) else {
            return;
        };
        self.pacer.input(Instant::now());
        if let Some(p) = self.pane_mut() {
            let bracketed = p.grid.lock().bracketed_paste;
            let _ = p.writer.write(&encode_paste(&text, bracketed));
        }
        self.snap_to_bottom();
    }

    /// Paste the system clipboard into the active tab's child (Ctrl+Shift+V, and
    /// Ctrl+V on Windows).
    fn paste(&mut self) {
        let Some(cb) = self.clipboard.as_mut() else {
            return;
        };
        let Ok(text) = cb.get_text() else { return };
        if text.is_empty() {
            return;
        }
        let Some(p) = self.pane() else { return };
        let bracketed = p.grid.lock().bracketed_paste;
        let bytes = encode_paste(&text, bracketed);
        self.write_child(&bytes);
        // Middle-click paste (`paste_primary`) already does this; pasting
        // while scrolled into history left the viewport there, so the user
        // couldn't see what they'd just pasted appear at the live prompt.
        self.snap_to_bottom();
    }

    /// Encode a native mouse event as SGR/1006 and send it to the active tab's
    /// child — but only when the child enabled mouse reporting (`?1000`/`?1002`/
    /// `?1003`). `build` turns the cell under the pointer into the event. Returns
    /// whether bytes were sent, so the wheel path can fall back to local
    /// scrollback browsing when the child isn't tracking the mouse.
    ///
    /// Holding Shift bypasses app mouse tracking entirely, the same escape
    /// hatch xterm/iTerm2/gnome-terminal have used for decades: a mouse-
    /// tracking full-screen app (vim, tmux, htop) grabs every click, so
    /// without this there would be no way to select text or scroll the
    /// scrollback at all while one is running.
    ///
    /// `id` is the pane the event is addressed to — the focused pane for
    /// press/release/drag-motion (a click already refocuses to the pane
    /// under the pointer in `on_left_press`, so "focused" and "under
    /// pointer" agree by the time this runs), or whichever pane is under
    /// the pointer for wheel events, which don't go through `on_left_press`
    /// and so would otherwise always report to the wrong pane in a split.
    fn report_mouse(
        &mut self,
        id: Option<u64>,
        build: impl FnOnce(usize, usize) -> MouseEvent,
    ) -> bool {
        if self.mods.shift_key() {
            return false;
        }
        let Some(id) = id else { return false };
        let Some(p) = self.pane_by_id(id) else {
            return false;
        };
        let modes = p.grid.lock().mouse_modes;
        if !modes.active() {
            return false;
        }
        let (col, row) = self.cell_in_pane(id, self.mouse_pos.0, self.mouse_pos.1);
        // Bidi: apps address cells in logical order; a click on a reordered
        // row reports the logical cell shown at the pointer's visual slot.
        let col = p.grid.lock().logical_col(col, row);
        let mut e = build(col, row);
        // SGR-pixel mode (`?1016`): the same SGR encoding, but the position
        // is the pointer's pixel offset within the target pane's text area
        // (this pane is its own terminal, so pane-relative is the analogue of
        // xterm's text-area-relative pixels), below the chrome bar.
        if modes.extended & 8 != 0
            && let Some(p) = self.place(id)
        {
            e.point = pane_pixel_point(
                self.mouse_pos,
                (p.x, p.y),
                (p.cells.cols * self.cell_w, p.cells.rows * self.cell_h),
            );
        }
        let mut out = Vec::new();
        SgrEncoder::new(modes).write(e, &mut out);
        if out.is_empty() {
            return false;
        }
        self.pacer.input(Instant::now());
        let Some(p) = self.pane_by_id_mut(id) else {
            return false;
        };
        let _ = p.writer.write(&out);
        true
    }

    /// If an OSC 8 hyperlink covers the cell under the pointer, open it with the
    /// OS handler. Returns whether a link was opened, so a Ctrl+click can
    /// suppress the normal selection / mouse-report path.
    fn open_link_under_pointer(&self) -> bool {
        let Some(p) = self.pane() else { return false };
        let (col, row) = self.cell_in_focused(self.mouse_pos.0, self.mouse_pos.1);
        let url = {
            let g = p.grid.lock();
            let col = g.logical_col(col, row);
            // An explicit OSC 8 link wins; otherwise scan the cell's logical
            // line for a plain-text URL (G16) — most programs never emit OSC 8.
            g.link_at(col, row)
                .map(str::to_owned)
                .or_else(|| g.url_at(col, row))
        };
        url.is_some_and(|u| open_url(&u))
    }

    /// Recompute `hover_link` for the pointer's current position: only set
    /// while Ctrl is held and the pointer is over grid content (not the
    /// chrome bar), and only when a hyperlink is actually there — mirrors
    /// `open_link_under_pointer`'s OSC-8-then-plain-text lookup, but via
    /// `hover_link_at` for the column span instead of a single cell.
    /// Returns whether the value changed, so callers know whether to
    /// repaint.
    fn update_hover_link(&mut self) -> bool {
        let (x, y) = self.mouse_pos;
        let new = if self.mods.control_key() && !self.in_chrome(y) {
            self.pane().and_then(|p| {
                let (col, row) = self.cell_in_focused(x, y);
                let g = p.grid.lock();
                let col = g.logical_col(col, row);
                g.hover_link_at(col, row)
                    .map(|(start, end, _)| (row, start, end))
            })
        } else {
            None
        };
        if new != self.hover_link {
            self.hover_link = new;
            true
        } else {
            false
        }
    }

    /// Open a dropdown listing every link visible in the focused pane —
    /// explicit OSC 8 hyperlinks and detected plain-text URLs — so links are
    /// reachable without the mouse (Ctrl+Shift+O; the keyboard side of G16).
    fn open_links_menu(&mut self) {
        let links = match self.pane() {
            Some(p) => p.grid.lock().visible_links(),
            None => return,
        };
        if links.is_empty() {
            return;
        }
        let items = links
            .into_iter()
            .map(|u| {
                // Menu rows are narrow; elide the middle of long URLs.
                let label = if u.chars().count() > 46 {
                    let head: String = u.chars().take(30).collect();
                    let tail: String = u
                        .chars()
                        .rev()
                        .take(13)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("{head}…{tail}")
                } else {
                    u.clone()
                };
                MenuItem {
                    label,
                    kind: MenuKind::OpenUrl(u),
                }
            })
            .collect();
        self.overlay = Some(Overlay::Menu { items, sel: 0 });
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Whether alternate scroll mode (`?1007`) should intercept wheel input
    /// for pane `id` right now: the mode is on *and* its alternate screen is
    /// active (mode 1007 only ever applies there — the primary screen keeps
    /// browsing rusty_term's own scrollback, same as xterm).
    fn alt_scroll_active(&self, id: u64) -> bool {
        self.pane_by_id(id).is_some_and(|p| {
            let g = p.grid.lock();
            g.alt_scroll && g.in_alt_screen()
        })
    }

    /// Translate a wheel scroll into repeated Up/Down (DECCKM-aware) key
    /// presses for alternate scroll mode (`?1007`) on pane `id`, so the wheel
    /// drives a pager (`less`, `man`, …) that never registered native mouse
    /// support.
    fn send_alt_scroll_keys(&mut self, id: u64, lines: isize) {
        let app_cursor = self
            .pane_by_id(id)
            .is_some_and(|p| p.grid.lock().app_cursor_keys);
        let seq: &[u8] = match (lines >= 0, app_cursor) {
            (true, true) => b"\x1bOA",
            (true, false) => b"\x1b[A",
            (false, true) => b"\x1bOB",
            (false, false) => b"\x1b[B",
        };
        self.pacer.input(Instant::now());
        if let Some(p) = self.pane_by_id_mut(id) {
            for _ in 0..lines.unsigned_abs() {
                let _ = p.writer.write(seq);
            }
        }
    }

    /// Browse pane `id`'s scrollback: move the viewport by `lines` (positive
    /// = up into history, negative = back toward the live bottom), clamped to
    /// the available history. Repaints if the view actually moved. Takes a
    /// pane id (rather than always the focused pane) so wheel-scroll can
    /// target whichever pane is under the pointer, tmux/iTerm2-style, without
    /// stealing focus.
    fn scroll_active(&mut self, id: u64, lines: isize) {
        let Some(p) = self.pane_by_id(id) else { return };
        let moved = {
            let mut g = p.grid.lock();
            if lines >= 0 {
                g.scroll_view_up(lines as usize)
            } else {
                g.scroll_view_down((-lines) as usize)
            }
        };
        if moved && let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Apply a scrollback-browse key to the active tab: Shift+PageUp/Down page
    /// through history, the same with Ctrl jumps prompt-to-prompt (OSC 133
    /// marks). Repaints if the view moved. Mirrors the TUI's scroll keys.
    fn scroll_key(&mut self, ctrl: bool, up: bool) {
        let page = (self.rows.saturating_sub(1)).max(1) as usize;
        let Some(p) = self.pane() else { return };
        let moved = {
            let mut g = p.grid.lock();
            match (ctrl, up) {
                (false, true) => g.scroll_view_up(page),
                (false, false) => g.scroll_view_down(page),
                (true, true) => g.scroll_to_prev_prompt(),
                (true, false) => g.scroll_to_next_prompt(),
            }
        };
        if moved && let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Snap the active tab's viewport back to the live bottom (e.g. after the
    /// user types), repainting if it had been scrolled into history.
    fn snap_to_bottom(&mut self) {
        let Some(p) = self.pane() else { return };
        if p.grid.lock().reset_view()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }

    /// Open the config file in the user's editor (Ctrl+Shift+,), creating it
    /// from the commented template first if needed. The live-reload watcher
    /// then applies any save the user makes.
    fn open_config(&self) {
        let Some(path) = &self.config_path else {
            return;
        };
        if let Err(e) = crate::config::open_in_editor(path) {
            eprintln!("rusty_term: open config: {e}");
        }
    }

    /// Re-read the config file and apply what can change live: theme (every
    /// tab's parser palette + grid recolor, the chrome bar, the window border)
    /// and scrollback cap. Shell changes apply to tabs opened afterwards;
    /// font and window size are launch-time choices. Parse warnings go to
    /// stderr, same as at startup.
    ///
    /// `Config::load` only sees this reload's own `--config <path>` args, so
    /// a `--profile` given at launch is reapplied via `profile_override`
    /// below — otherwise this window's profile-selected theme would silently
    /// revert to the file's top-level default on every save (mirrors
    /// `runtime::reactor::watch_config`'s fix for the TUI front-end).
    fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let args = vec!["--config".to_string(), path.to_string_lossy().into_owned()];
        let (mut new, mut warnings) = crate::config::Config::load(&args);
        if let Some(name) = &self.profile_override
            && let Some(w) = new.apply_profile(name)
        {
            warnings.push(w);
        }
        for w in &warnings {
            eprintln!("rusty_term: {w}");
        }
        for tab in &self.tabs {
            for p in &tab.panes {
                let mut g = p.grid.lock();
                let old = p.parser.lock().retheme(new.theme);
                if old != new.theme {
                    g.retheme(&old, &new.theme);
                }
                g.set_scrollback_max(new.scrollback.unwrap_or(crate::core::SCROLLBACK_MAX));
            }
        }
        self.theme = new.theme;
        let status_changed =
            new.status_bar.unwrap_or(true) != self.config.status_bar.unwrap_or(true);
        self.config = new;
        let new_pad = self.config.padding.unwrap_or(DEFAULT_PAD) as usize;
        let pad_changed = new_pad != self.pad;
        self.pad = new_pad;
        if (pad_changed || status_changed)
            && let Some(size) = self.window.as_ref().map(|w| w.inner_size())
        {
            // The band / status ribbon grew or shrank; refit the grid to the
            // same window.
            self.apply_size(size.width, size.height);
        }
        if let Some(window) = &self.window {
            apply_chrome(window, &self.theme);
            window.request_redraw();
        }
    }
}

impl WindowState<'_> {
    /// The `[keys]` action bound to `key` with the current modifiers.
    fn chord_action(&self, key: PhysicalKey) -> Option<Action> {
        match key {
            PhysicalKey::Code(code) => chord_key(code).and_then(|key| {
                self.config.keys.action(Chord::new(
                    self.mods.control_key(),
                    self.mods.shift_key(),
                    self.mods.alt_key(),
                    key,
                ))
            }),
            PhysicalKey::Unidentified(_) => None,
        }
    }

    /// Open the in-app settings page over the active tab, seeded from the live
    /// configuration. `Ctrl+,` and the dropdown's *Settings* entry route here.
    fn open_settings(&mut self) {
        // Ctrl+, toggles: with the page already open, close (and save) it
        // instead of silently rebuilding it and losing the user's position.
        if matches!(self.overlay, Some(Overlay::Settings(_))) {
            self.close_overlay();
            return;
        }
        let opacity_supported = self.renderer.as_ref().is_some_and(|r| r.supports_opacity());
        let s = Settings::new(
            &self.config,
            &self.theme,
            self.config_font_size(),
            &self.shells,
            opacity_supported,
        );
        self.overlay = Some(Overlay::Settings(s));
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Open the first-run setup (O11): the settings page in setup mode, a few
    /// rows one step at a time. Opens by itself on a first run (see
    /// [`crate::config::is_first_run`]), and from `--setup` or the
    /// `open_setup` action at any time; replaces whatever overlay is open.
    fn open_setup(&mut self) {
        self.close_overlay();
        let opacity_supported = self.renderer.as_ref().is_some_and(|r| r.supports_opacity());
        let s = Settings::new(
            &self.config,
            &self.theme,
            self.config_font_size(),
            &self.shells,
            opacity_supported,
        )
        .into_setup(&self.config, self.theme);
        self.overlay = Some(Overlay::Settings(s));
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Leave setup without keeping its choices: put every row it touched
    /// back to its exact opening value. Skip writes nothing (D65): a missing
    /// config stays missing, so a first run's setup returns on the next
    /// plain launch, and an existing one stays byte-identical.
    fn skip_setup(&mut self) {
        let Some(Overlay::Settings(s)) = self.overlay.take() else {
            return;
        };
        if let Some(base) = s.setup_baseline() {
            for &field in s.touched() {
                self.restore_setting(field, base);
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Finish setup: save only the rows it really changed (onto the
    /// commented template when there is no file yet) and close. A row
    /// changed and changed back is restored exactly, never written.
    fn finish_setup(&mut self) {
        if let Some(Overlay::Settings(mut s)) = self.overlay.take() {
            if let Some(field) = s.commit_edit() {
                self.overlay = Some(Overlay::Settings(s.clone()));
                self.apply_setting(field);
                self.overlay = None;
            }
            let back = s.settle_setup();
            if let Some(base) = s.setup_baseline() {
                for field in back {
                    self.restore_setting(field, base);
                }
            }
            self.persist_settings(&s);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Put setup row `field` back to the exact value setup opened over,
    /// recolouring live for the theme.
    fn restore_setting(&mut self, field: Field, base: &SetupBaseline) {
        if field == Field::Theme {
            self.apply_theme_live(base.theme);
        }
        base.restore(&mut self.config, field);
    }

    /// Create the config file from the commented template if it is missing.
    fn ensure_config_file(&self) {
        if let Some(path) = self.config_path.as_ref().filter(|p| !p.exists())
            && let Err(e) = crate::config::save_settings(path, &[])
        {
            eprintln!("rusty_term: create config: {e}");
        }
    }

    /// Close any open overlay, persisting settings changes on the way out.
    /// Closing setup any way but finishing it (Esc, its tab's ×, another
    /// page) is a skip.
    fn close_overlay(&mut self) {
        if matches!(&self.overlay, Some(Overlay::Settings(s)) if s.is_setup()) {
            self.skip_setup();
            return;
        }
        if let Some(Overlay::Settings(mut s)) = self.overlay.take() {
            // A still-pending number edit commits like any blur; its live
            // apply comes from the reload watcher re-reading the save.
            s.commit_edit();
            if s.dirty() {
                self.persist_settings(&s);
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Write the rows the page changed to the config file; with none, only
    /// make sure the file exists. The live-reload watcher re-reads the save;
    /// live application already happened per change.
    fn persist_settings(&self, s: &Settings) {
        let edits = s.edits();
        if edits.is_empty() {
            self.ensure_config_file();
            return;
        }
        let Some(path) = &self.config_path else {
            return;
        };
        if let Err(e) = crate::config::save_settings(path, &edits) {
            eprintln!("rusty_term: save settings: {e}");
        }
    }

    /// Route a key to the open overlay (it owns all input while up): arrows
    /// navigate / change, Enter activates, digits pick a menu row, Esc closes.
    /// The settings page has richer input (search typing, number editing) and
    /// routes through [`Self::settings_key`].
    fn overlay_key(&mut self, event: &KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        if matches!(self.overlay, Some(Overlay::Settings(_))) {
            self.settings_key(event);
        } else {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => self.close_overlay(),
                Key::Named(NamedKey::ArrowUp) => self.overlay_move(false),
                Key::Named(NamedKey::ArrowDown) => self.overlay_move(true),
                Key::Named(NamedKey::Enter) => self.overlay_activate(),
                Key::Character(s) => {
                    if let Some(d) = s.chars().next().and_then(|c| c.to_digit(10)) {
                        self.menu_pick_index(d as usize);
                    }
                }
                _ => {}
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Keys on the settings page. An in-progress number edit owns the
    /// keyboard first (digits, Backspace, Enter commit, Esc cancel); then
    /// Esc peels one layer at a time (edit → filter → page); typing filters
    /// (or, digits on a number row, starts an edit); the rest navigates.
    fn settings_key(&mut self, event: &KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        let Some(Overlay::Settings(s)) = &mut self.overlay else {
            return;
        };
        let editing = s.editing.is_some();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if editing {
                    s.cancel_edit();
                } else if s.filter.is_some() {
                    s.clear_filter();
                } else {
                    self.close_overlay();
                }
            }
            Key::Named(NamedKey::Enter) => {
                if editing {
                    let field = s.commit_edit();
                    if let Some(field) = field {
                        self.apply_setting(field);
                    }
                } else if s.on_last_step() {
                    self.finish_setup();
                } else if s.is_setup() {
                    // Enter is setup's primary action: the next step.
                    s.cycle_category(true);
                } else if !s.begin_edit(None) {
                    // Not an editable number: Enter cycles, mirroring →.
                    self.overlay_change(true);
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if editing {
                    s.edit_backspace();
                } else {
                    s.filter_backspace();
                }
            }
            Key::Named(NamedKey::ArrowUp) => self.overlay_move(false),
            Key::Named(NamedKey::ArrowDown) => self.overlay_move(true),
            Key::Named(NamedKey::ArrowLeft) if !editing => self.overlay_change(false),
            Key::Named(NamedKey::ArrowRight) if !editing => self.overlay_change(true),
            // Sidebar: Tab / Shift+Tab steps the category (dropping any
            // filter — the user is navigating away from the results).
            Key::Named(NamedKey::Tab) => {
                let forward = !self.mods.shift_key();
                if let Some(Overlay::Settings(s)) = &mut self.overlay {
                    s.clear_filter();
                    s.cycle_category(forward);
                }
            }
            // Numbers: Home/End jump to the bound.
            Key::Named(NamedKey::Home) if !editing => self.overlay_jump(false),
            Key::Named(NamedKey::End) if !editing => self.overlay_jump(true),
            // Space extends an active search query ("click to move"); it
            // arrives as a named key, not a Character.
            Key::Named(NamedKey::Space) if !editing && s.filter.is_some() => {
                s.filter_input(' ');
            }
            Key::Character(text) => {
                let Some(c) = text.chars().next() else { return };
                if s.is_setup() && !editing {
                    // Setup has no search; `t` on its notifications step
                    // sends a test, so the user sees what an alert looks like.
                    if c.eq_ignore_ascii_case(&'t') && on_notify_step(s) {
                        notify("rusty_term", "This is how a finished command is announced.");
                    }
                    return;
                }
                if self.mods.control_key() {
                    // Ctrl+F opens the search, matching the find bar.
                    if text.as_str() == "f"
                        && let Some(Overlay::Settings(s)) = &mut self.overlay
                    {
                        s.start_filter();
                    }
                    return;
                }
                if editing {
                    s.edit_input(c);
                } else if c.is_ascii_digit() && s.filter.is_none() {
                    // A digit on a number row starts an edit with that digit;
                    // on anything else it falls through to the filter.
                    if !s.begin_edit(Some(c)) {
                        s.filter_input(c);
                    }
                } else if c == '/' && s.filter.is_none() {
                    // `/` opens the search (vim/less convention) rather than
                    // becoming the query's first character.
                    s.start_filter();
                } else if !c.is_control() {
                    s.filter_input(c);
                }
            }
            _ => {}
        }
    }

    /// Move the highlight within the overlay. Moving off a number row with
    /// an edit in progress commits it (blur), like any form.
    fn overlay_move(&mut self, forward: bool) {
        let mut committed = None;
        match &mut self.overlay {
            Some(Overlay::Menu { items, sel }) => {
                let n = items.len();
                if n > 0 {
                    *sel = if forward {
                        (*sel + 1) % n
                    } else {
                        (*sel + n - 1) % n
                    };
                }
            }
            Some(Overlay::Settings(s)) => {
                committed = s.commit_edit();
                s.move_sel(forward);
                let visible = settings_visible(self.rows as usize);
                s.ensure_visible(visible);
            }
            None => {}
        }
        if let Some(field) = committed {
            self.apply_setting(field);
        }
    }

    /// Change the highlighted setting (no effect on the menu) and apply it
    /// live. Shift multiplies a stepped number's step by 10.
    fn overlay_change(&mut self, forward: bool) {
        let big = self.mods.shift_key();
        let field = match &mut self.overlay {
            Some(Overlay::Settings(s)) => s.change_by(forward, big),
            _ => return,
        };
        if let Some(field) = field {
            self.apply_setting(field);
        }
    }

    /// Jump the highlighted stepped number to its bound (Home/End) and apply
    /// it live; no effect on choices, toggles, or the menu.
    fn overlay_jump(&mut self, to_max: bool) {
        let field = match &mut self.overlay {
            Some(Overlay::Settings(s)) => s.jump(to_max),
            _ => return,
        };
        if let Some(field) = field {
            self.apply_setting(field);
        }
    }

    /// Activate the highlighted overlay row: pick a menu item, or cycle the
    /// highlighted setting (Enter mirrors →).
    fn overlay_activate(&mut self) {
        match &self.overlay {
            Some(Overlay::Menu { items, sel }) => {
                if let Some(kind) = items.get(*sel).map(|i| i.kind.clone()) {
                    self.menu_pick(kind);
                }
            }
            Some(Overlay::Settings(_)) => self.overlay_change(true),
            None => {}
        }
    }

    /// Pick menu row `n` (1-based, from a digit key); ignored off the menu.
    fn menu_pick_index(&mut self, n: usize) {
        let kind = match &self.overlay {
            Some(Overlay::Menu { items, .. }) if n >= 1 => items.get(n - 1).map(|i| i.kind.clone()),
            _ => None,
        };
        if let Some(kind) = kind {
            self.menu_pick(kind);
        }
    }

    /// Carry out a chosen menu item.
    fn menu_pick(&mut self, kind: MenuKind) {
        match kind {
            MenuKind::OpenUrl(url) => {
                open_url(&url);
            }
            MenuKind::Settings => self.open_settings(),
            MenuKind::EditConfig => {
                self.overlay = None;
                self.open_config();
            }
        }
    }

    /// The overlay-grid cell under window pixel `(x, y)`, `None` above the
    /// chrome bar. Mirrors the overlay's own layout: the bar sits flush on
    /// top, the grid below it inset by `pad`.
    fn overlay_cell(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        if self.in_chrome(y) {
            return None;
        }
        let col = ((x - self.pad as f64).max(0.0) as usize) / self.cell_w;
        let row = ((y - self.grid_oy() as f64).max(0.0) as usize) / self.cell_h;
        Some((col, row))
    }

    /// Handle a click in the overlay body (below the chrome bar): select the
    /// row under the pointer (activating it for a menu); on the settings page
    /// also switch sidebar categories and cycle a clicked value widget.
    fn overlay_click(&mut self, x: f64, y: f64) {
        let Some((col, grid_row)) = self.overlay_cell(x, y) else {
            return;
        };
        let mut apply: Vec<Field> = Vec::new();
        let mut activate = false;
        match &mut self.overlay {
            Some(Overlay::Menu { items, sel }) => {
                if let Some(i) = grid_row.checked_sub(OVERLAY_ITEMS_TOP)
                    && i < items.len()
                {
                    *sel = i;
                    activate = true;
                }
            }
            Some(Overlay::Settings(s)) => {
                // A click anywhere blurs an in-progress number edit,
                // committing it first (like any form).
                apply.extend(s.commit_edit());
                let grid_rows = self.rows as usize;
                let grid_cols = self.cols as usize;
                let sections = s.sections().len();
                match settings_hit(
                    col,
                    grid_row,
                    grid_cols,
                    grid_rows,
                    s.scroll,
                    s.len(),
                    sections,
                ) {
                    SettingsHit::Category(i) => s.set_category(i),
                    SettingsHit::Search => s.start_filter(),
                    SettingsHit::Row(i) => {
                        s.select(i);
                        // Inside the value widget: the left arrow cell cycles
                        // back, anywhere else forward (toggles just flip).
                        let row = &s.rows()[i];
                        let wtext = widget_text(&row.widget, true);
                        let value_end = settings_value_end(grid_cols);
                        let wcol = value_end.saturating_sub(wtext.chars().count());
                        if col >= wcol && col < value_end {
                            apply.extend(s.change(col != wcol));
                        }
                    }
                    SettingsHit::None => {}
                }
            }
            None => {}
        }
        for field in apply {
            self.apply_setting(field);
        }
        if activate {
            self.overlay_activate();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Apply a just-changed setting to the running terminal. Values are
    /// snapshotted first so the overlay borrow ends before we mutate `self`.
    /// Launch-time fields (launch mode, cols, rows) only update the config —
    /// their rows say "applies at next launch" — but still count as applied
    /// so persistence picks them up on close.
    fn apply_setting(&mut self, field: Field) {
        let Some(Overlay::Settings(s)) = self.overlay.as_ref() else {
            return;
        };
        let theme = crate::config::preset(s.theme_name());
        let font_size = s.font_size();
        let cursor = s.cursor();
        let blink = s.blink();
        let ligatures = s.ligatures();
        let cursor_trail = s.cursor_trail();
        let reduce_motion = s.reduce_motion();
        let min_contrast = s.min_contrast();
        let scrollback = s.scrollback();
        let shell = s.shell_path();
        let copy_html = s.copy_html();
        let clipboard = s.clipboard();
        let click_to_move = s.click_to_move();
        let bell = s.bell();
        let command_notify = s.command_notify_secs();
        let padding = s.padding();
        let status_bar = s.status_bar();
        let command_marks = s.command_marks();
        let opacity = s.opacity();
        let launch_mode = s.launch_mode_value();
        let cols = s.cols_value();
        let rows = s.rows_value();
        match field {
            Field::Theme => {
                if let Some(t) = theme {
                    self.apply_theme_live(t);
                }
            }
            Field::FontSize => {
                let px = physical_px(font_size, self.scale_factor);
                if self.rebuild_fonts(px, self.config.ligatures.unwrap_or(true)) {
                    self.base_px = px;
                    self.config.font_size = Some(font_size);
                }
            }
            Field::Ligatures => {
                if self.rebuild_fonts(self.font_px, ligatures) {
                    self.config.ligatures = Some(ligatures);
                }
            }
            Field::Cursor => {
                self.config.cursor_style = Some(cursor);
                let blink = self.config.cursor_blink.unwrap_or(false);
                self.set_cursor_all(cursor, blink);
            }
            Field::Blink => {
                self.config.cursor_blink = Some(blink);
                let shape = self.config.cursor_style.unwrap_or_default();
                self.set_cursor_all(shape, blink);
            }
            // Read live at draw time; updating the config is the whole apply.
            Field::CursorTrail => self.config.cursor_trail = Some(cursor_trail),
            // Read live by the draw and tick paths, like the trail.
            Field::ReduceMotion => self.config.reduce_motion = reduce_motion,
            Field::MinContrast => {
                self.config.minimum_contrast = min_contrast;
                for tab in &self.tabs {
                    for p in &tab.panes {
                        p.grid.lock().min_contrast = min_contrast.unwrap_or(1.0);
                    }
                }
            }
            Field::Scrollback => {
                self.config.scrollback = Some(scrollback);
                for tab in &self.tabs {
                    for p in &tab.panes {
                        p.grid.lock().set_scrollback_max(scrollback);
                    }
                }
            }
            Field::Shell => self.config.shell = shell,
            // Read live at copy/paste/alert time; the config *is* the state.
            Field::CopyHtml => self.config.copy_html = Some(copy_html),
            Field::Clipboard => self.config.clipboard = Some(clipboard),
            Field::ClickToMove => self.config.click_to_move = Some(click_to_move),
            Field::Bell => self.config.bell = Some(bell),
            Field::CommandNotify => self.config.command_notify_secs = Some(command_notify),
            Field::Padding => {
                self.config.padding = Some(padding);
                self.pad = padding as usize;
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    // The band grew or shrank; refit the grid to the same window.
                    self.apply_size(size.width, size.height);
                }
            }
            // Read live at draw time; updating the config is the whole apply.
            Field::CommandMarks => self.config.command_marks = Some(command_marks),
            Field::StatusBar => {
                self.config.status_bar = Some(status_bar);
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    // The ribbon claimed or released a row; refit the grid.
                    self.apply_size(size.width, size.height);
                }
            }
            Field::Opacity => {
                self.config.opacity = Some(opacity);
                self.apply_opacity();
            }
            // Launch-time only: persisted on close, applied by the next launch.
            Field::LaunchMode => self.config.launch_mode = launch_mode,
            Field::Cols => self.config.cols = Some(cols),
            Field::Rows => self.config.rows = Some(rows),
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Set the default (and live) cursor on every pane of every tab.
    fn set_cursor_all(&self, shape: crate::core::CursorShape, blink: bool) {
        for tab in &self.tabs {
            for p in &tab.panes {
                p.grid.lock().set_default_cursor(shape, blink);
            }
        }
    }

    /// Recolor every tab to `new` (mirrors the config live-reload path) and
    /// repaint the chrome / window border.
    fn apply_theme_live(&mut self, new: Theme) {
        for tab in &mut self.tabs {
            for p in &mut tab.panes {
                let (report, changed) = {
                    let mut g = p.grid.lock();
                    let was_dark = g.appearance_is_dark();
                    let old = p.parser.lock().retheme(new);
                    if old != new {
                        g.retheme(&old, &new);
                    }
                    let report = g.report_color_scheme && g.appearance_is_dark() != was_dark;
                    (g.color_scheme_report(), report)
                };
                // Mode 2031: tell a subscribed child its world flipped
                // light/dark (Neovim flips `background` off this).
                if changed {
                    let _ = p.writer.write(&report);
                }
            }
        }
        self.theme = new;
        self.config.theme = new;
        if let Some(window) = &self.window {
            apply_chrome(window, &self.theme);
        }
    }

    /// Resolve `theme = "auto"` against the OS appearance: `hint` is a live
    /// `ThemeChanged` value, else the window's current report (dark when
    /// unknown). Presets come from `theme_dark` / `theme_light`, defaulting
    /// to the built-in default (dark) and Solarized Light.
    fn auto_theme(&self, hint: Option<winit::window::Theme>) -> Theme {
        let dark = !matches!(
            hint.or_else(|| self.window.as_ref().and_then(|w| w.theme())),
            Some(winit::window::Theme::Light)
        );
        if dark {
            self.config.theme_dark.unwrap_or_default()
        } else {
            self.config
                .theme_light
                .or_else(|| crate::config::preset("solarized-light"))
                .unwrap_or_default()
        }
    }

    /// Push the configured window opacity (`[window] opacity` / `--opacity`)
    /// to the current renderer. Only the GPU renderer honors it (see
    /// `Renderer::set_opacity`'s default no-op); called whenever the renderer
    /// is (re)built, since a fresh one starts back at fully opaque.
    fn apply_opacity(&mut self) {
        let opacity = self.config.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
        if let Some(r) = &mut self.renderer {
            r.set_opacity(opacity);
        }
    }

    /// The configured terminal font size in logical px, as the settings page
    /// shows and saves it (not DPI-scaled, unlike `base_px`).
    fn config_font_size(&self) -> f32 {
        self.config.font_size.unwrap_or(FONT_PX)
    }

    /// Rebuild both fonts, the bars' at their fixed `ui_px` and the panes' at
    /// `font_px`, re-fit the grid and rebuild the renderer (the GPU atlas is
    /// font-bound). The settings font-size and ligatures fields call this;
    /// the caller persists the changed setting when it returns `true`.
    fn rebuild_fonts(&mut self, font_px: f32, ligatures: bool) -> bool {
        self.rescale_ui_font(self.ui_px, ligatures) && self.rebuild_font_at(font_px, ligatures)
    }

    /// Core of [`Self::rebuild_fonts`] for the panes alone: rebuild the
    /// glyph cache at `px` / `ligatures` (at the current `scale_factor`),
    /// re-fit the grid to the new cell size, and rebuild the renderer (the
    /// GPU atlas is font-bound). Returns whether the font actually rebuilt (a
    /// missing/unparseable font leaves state untouched).
    fn rebuild_font_at(&mut self, px: f32, ligatures: bool) -> bool {
        if !self.rescale_font_cache(px, ligatures, self.scale_factor) {
            return false;
        }
        self.after_font_rescale();
        true
    }

    /// Rebuild just the glyph cache / cell size / tracked scale factor — no
    /// renderer rebuild or grid re-fit, so this is safe to call before an OS
    /// window exists (see the HiDPI sizing in `ensure_window`). Returns
    /// whether it succeeded (a missing/unparseable font leaves state
    /// untouched).
    fn rescale_font_cache(&mut self, px: f32, ligatures: bool, scale_factor: f64) -> bool {
        let Some(font) = load_font(&self.config, px, ligatures) else {
            return false;
        };
        let (cw, ch) = font.cell_size();
        self.font = font;
        self.cell_w = cw.max(1);
        self.cell_h = ch.max(1);
        self.font_px = px;
        self.scale_factor = scale_factor;
        true
    }

    /// Rebuild the bars' UI font at `px` — no renderer rebuild, so this is
    /// safe before an OS window exists. Returns whether it succeeded (a
    /// missing/unparseable font leaves state untouched).
    fn rescale_ui_font(&mut self, px: f32, ligatures: bool) -> bool {
        let Some(font) = load_font(&self.config, px, ligatures) else {
            return false;
        };
        let (bw, bh) = font.cell_size();
        self.ui_font = font;
        self.bar_w = bw.max(1);
        self.bar_h = bh.max(1);
        self.ui_px = px;
        true
    }

    /// The renderer-facing tail of a font rescale: keep every pane's XTWINOPS
    /// pixel-size answer (14t/16t) current, and — once an OS window exists —
    /// rebuild the renderer and re-fit the grid to the new cell size.
    fn after_font_rescale(&mut self) {
        let cell_px = Some((self.cell_w as u16, self.cell_h as u16));
        for tab in &self.tabs {
            for pane in &tab.panes {
                pane.grid.lock().cell_px = cell_px;
            }
        }
        if let Some(window) = self.window.clone() {
            if let Some(r) = self.make_renderer(window.clone()) {
                self.renderer = Some(r);
            }
            self.apply_opacity();
            let size = window.inner_size();
            self.apply_size(size.width, size.height);
            // A keymap zoom (Ctrl+=/Ctrl+-) returns without any other
            // repaint, so the new cell size would otherwise only show on the
            // next unrelated redraw (child output, a mouse move).
            window.request_redraw();
        }
    }

    /// Handle `WindowEvent::ScaleFactorChanged` (the window moved to a monitor
    /// with a different DPI scale, or the OS scale setting changed). Winit
    /// reports `inner_size()` and pointer coordinates in physical pixels
    /// throughout this file, but font rasterization was never scaled by the
    /// monitor's factor — dragging the window from a 100% to a 200% display
    /// used to render text at half the intended visual size. Rescale the
    /// *current* physical font size proportionally to the factor change
    /// rather than recomputing from the configured (logical, monitor-
    /// independent) `font_size`, so the user's chosen size is preserved
    /// exactly when they move back to the original monitor. This
    /// deliberately does not touch `self.config.font_size` — a DPI move is
    /// not a user preference change.
    fn on_scale_factor_changed(&mut self, new_scale: f64) {
        // Equivalent to `!(new_scale > 0.0)` but spelled without a negated
        // partial-order comparison: reject NaN explicitly rather than
        // leaning on "not greater than" to catch it.
        if new_scale.is_nan() || new_scale <= 0.0 || (new_scale - self.scale_factor).abs() < 1e-9 {
            return;
        }
        let ratio = new_scale / self.scale_factor;
        let ligatures = self.config.ligatures.unwrap_or(true);
        let px = ((self.font_px as f64) * ratio) as f32;
        self.base_px = ((self.base_px as f64) * ratio) as f32;
        self.rescale_ui_font(physical_px(UI_FONT_PX, new_scale), ligatures);
        if self.rescale_font_cache(px, ligatures, new_scale) {
            self.after_font_rescale();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Render the open overlay into a fresh full-area grid (chrome stays on top).
    fn build_overlay_grid(&self) -> Grid {
        let (cols, rows) = (self.cols as usize, self.rows as usize);
        let mut g = Grid::new(cols, rows);
        // Overlay pages are chrome: a `base` page with pack text (D38).
        let tokens = self.chrome_colors();
        let (fg, bg) = (tokens.text, tokens.base);
        let dim = tokens.muted;
        for y in 0..rows {
            fill_row(&mut g, y, 0, cols, fg, bg);
        }
        let bar = cols.min(56);
        let footer = rows.saturating_sub(2);
        match self.overlay.as_ref() {
            Some(Overlay::Menu { items, sel }) => {
                write_row(&mut g, 0, 2, "Open a new shell or page", dim, bg);
                for (i, item) in items.iter().enumerate() {
                    let r = OVERLAY_ITEMS_TOP + i;
                    if r >= footer {
                        break;
                    }
                    let on = i == *sel;
                    let (rfg, rbg) = if on { (fg, tokens.raised) } else { (fg, bg) };
                    if on {
                        fill_row(&mut g, r, 0, bar, rfg, rbg);
                        write_row(&mut g, r, 0, "\u{258C}", tokens.accent, rbg);
                    }
                    write_row(
                        &mut g,
                        r,
                        2,
                        &format!("{}. {}", i + 1, item.label),
                        rfg,
                        rbg,
                    );
                }
                write_row(
                    &mut g,
                    footer,
                    2,
                    "Up/Down select   Enter/1-9 open   Esc close",
                    dim,
                    bg,
                );
            }
            Some(Overlay::Settings(s)) => {
                // No header row: the strip's own "Settings" tab names the
                // page, so the content starts at the top and the save state
                // lives in the footer.
                let footer_top = rows.saturating_sub(SETTINGS_FOOTER_H);
                // Hover and selection both raise the row (D38); only the
                // selection carries the accent bar, so the two never read as
                // the same state when both are on screen.
                let sel_bg = tokens.raised;
                let hover_bg = tokens.raised;
                // Selected-card description: full text on the raised band.
                let mid = fg;
                let accent = tokens.accent;

                // Category sidebar: active gets an accent bar + tint (flush
                // to the `│` rule separating it from the cards), the rest
                // render dim; a category holding modified settings carries an
                // accent dot. While a search is on, results span categories,
                // so no sidebar entry is "the" active one.
                let filtering = s.filter.is_some();
                let setup = s.is_setup();
                for (i, (label, _)) in s.sections().iter().enumerate() {
                    let r = SETTINGS_TOP + i * 2;
                    if r >= footer_top {
                        break;
                    }
                    let active = i == s.cat && !filtering;
                    let (cfg, cbg) = if active {
                        fill_row(&mut g, r, 0, SETTINGS_SIDEBAR_W + 1, fg, sel_bg);
                        write_row(&mut g, r, 0, "\u{258C}", accent, sel_bg);
                        (fg, sel_bg)
                    } else {
                        (dim, bg)
                    };
                    // Setup numbers its steps, so the order reads as a path.
                    let label = if setup {
                        format!("{}  {label}", i + 1)
                    } else {
                        label.to_string()
                    };
                    write_row(&mut g, r, 2, &label, cfg, cbg);
                    if s.section_modified(i) {
                        let dcol = 2 + label.chars().count() + 1;
                        write_row(&mut g, r, dcol, "\u{2022}", accent, cbg);
                    }
                }
                for r in SETTINGS_TOP..footer_top {
                    write_row(&mut g, r, SETTINGS_SIDEBAR_W + 1, "\u{2502}", dim, bg);
                }
                // The sidebar's idle bottom answers "what file does Esc save
                // into, and which build is this" without opening the docs.
                if footer_top >= 2 {
                    if let Some(name) = self
                        .config_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                    {
                        let shown: String = name.chars().take(SETTINGS_SIDEBAR_W - 2).collect();
                        write_row(&mut g, footer_top - 2, 2, &shown, dim, bg);
                    }
                    write_row(
                        &mut g,
                        footer_top - 1,
                        2,
                        concat!("v", env!("CARGO_PKG_VERSION")),
                        dim,
                        bg,
                    );
                }

                // Title row: the active category's name — or the search
                // result count — left, and the search affordance right (the
                // live query while one is active, a dim `/ search` invite
                // otherwise). The rule underneath anchors the column width.
                let value_end = settings_value_end(cols);
                let title = if filtering {
                    match s.len() {
                        0 => "No settings match".to_string(),
                        1 => "1 result".to_string(),
                        n => format!("{n} results"),
                    }
                } else if setup {
                    format!(
                        "Set up rusty_term: step {} of {}",
                        s.cat + 1,
                        s.sections().len()
                    )
                } else {
                    s.sections()[s.cat].0.to_string()
                };
                write_row(&mut g, SETTINGS_TOP, SETTINGS_CARD_X, &title, fg, bg);
                match &s.filter {
                    _ if setup => {} // no search in setup
                    Some(q) => {
                        let qcol = value_end.saturating_sub(q.chars().count() + 2);
                        write_row(&mut g, SETTINGS_TOP, qcol, &format!("/{q}"), fg, bg);
                        write_row(
                            &mut g,
                            SETTINGS_TOP,
                            value_end.saturating_sub(1),
                            "_",
                            dim,
                            bg,
                        );
                    }
                    None => {
                        let t = "/ search";
                        write_row(
                            &mut g,
                            SETTINGS_TOP,
                            value_end.saturating_sub(t.len()),
                            t,
                            dim,
                            bg,
                        );
                    }
                }
                let rule_w = value_end.saturating_sub(SETTINGS_CARD_X);
                write_row(
                    &mut g,
                    SETTINGS_TOP + 1,
                    SETTINGS_CARD_X,
                    &"\u{2500}".repeat(rule_w),
                    dim,
                    bg,
                );

                // Setting cards: title + right-aligned value widget, then a
                // dim one-line description. Highlight bands stop at the card
                // column's edge, not the window's. The scroll clamp guards a
                // scroll left past the end by a resize.
                let visible = settings_visible(rows);
                let list = s.rows();
                let scroll = s.scroll.min(list.len().saturating_sub(1));
                for (vi, row) in list.iter().enumerate().skip(scroll).take(visible) {
                    let top = SETTINGS_CARDS_TOP + (vi - scroll) * SETTINGS_ROW_H;
                    let selected = vi == s.sel;
                    // A disabled row takes no hover (D26).
                    let hovered = s.hover == Some(vi) && !row.disabled;
                    let rbg = if selected {
                        sel_bg
                    } else if hovered {
                        hover_bg
                    } else {
                        bg
                    };
                    if rbg != bg {
                        fill_row(&mut g, top, SETTINGS_SIDEBAR_W + 2, value_end + 1, fg, rbg);
                        fill_row(
                            &mut g,
                            top + 1,
                            SETTINGS_SIDEBAR_W + 2,
                            value_end + 1,
                            fg,
                            rbg,
                        );
                    }
                    if selected {
                        write_row(&mut g, top, SETTINGS_SIDEBAR_W + 2, "\u{258C}", accent, rbg);
                    }
                    // Disabled = 45% opacity (D26, D43), blended since a cell
                    // has no alpha; exempt from the contrast floor.
                    let disabled_fg = blend(rbg, fg, DISABLED_ALPHA);
                    let label_fg = if row.disabled { disabled_fg } else { fg };
                    write_row(&mut g, top, SETTINGS_CARD_X, row.label, label_fg, rbg);
                    // An accent dot marks a value changed from its default —
                    // the same gate that decides what lands in the config
                    // file, so customizations are visible at a glance.
                    let mut after = SETTINGS_CARD_X + row.label.chars().count();
                    if row.modified {
                        write_row(&mut g, top, after + 1, "\u{2022}", accent, rbg);
                        after += 2;
                    }
                    // Search results span categories; tag each row with its
                    // home so "Scrollback" isn't ambiguous out of context.
                    if let Some(cat) = row.category {
                        write_row(&mut g, top, after + 2, &format!("\u{B7} {cat}"), dim, rbg);
                    }
                    let desc_fg = if selected { mid } else { dim };
                    write_row(
                        &mut g,
                        top + 1,
                        SETTINGS_CARD_X,
                        row.description,
                        desc_fg,
                        rbg,
                    );
                    // The theme row previews its palette: the 16 ANSI colors
                    // as swatches on the description row, so cycling themes
                    // is a visual choice rather than a name lottery. Skipped
                    // when a narrow column would run them into the
                    // description text — a broken overlap is worse than no
                    // preview.
                    let swatch0 = value_end.saturating_sub(16);
                    if row.field == Field::Theme
                        && swatch0 >= SETTINGS_CARD_X + row.description.chars().count() + 2
                        && let Some(t) = crate::config::preset(s.theme_name())
                    {
                        for (k, color) in t.palette16.iter().enumerate() {
                            let mut cell = Cell::blank();
                            cell.ch = '\u{2588}';
                            cell.fg = *color;
                            cell.bg = rbg;
                            g.set_cell(swatch0 + k, top + 1, cell);
                        }
                    }
                    // An in-progress edit shows the typed buffer + a cursor
                    // cell in place of the value.
                    let editing_this = selected && s.editing.is_some();
                    // Hover gets the `< >` affordance too — but never on a
                    // disabled row, which renders inert.
                    let active = (selected || hovered) && !row.disabled && !editing_this;
                    let wtext = match (&s.editing, editing_this) {
                        (Some(buf), true) => format!("{buf}_"),
                        _ => widget_text(&row.widget, active),
                    };
                    let wlen = wtext.chars().count();
                    let wcol = value_end.saturating_sub(wlen);
                    let wfg = if row.disabled {
                        disabled_fg
                    } else {
                        match row.widget {
                            Widget::Toggle(false) if !editing_this => dim,
                            _ => fg,
                        }
                    };
                    write_row(&mut g, top, wcol, &wtext, wfg, rbg);
                    if active && matches!(row.widget, Widget::Choice(_) | Widget::Number(_)) {
                        // The cycle arrows read as controls, not value text.
                        write_row(&mut g, top, wcol, "<", dim, rbg);
                        write_row(&mut g, top, value_end.saturating_sub(1), ">", dim, rbg);
                    }
                    // "next launch" tag: scannable, out of the description.
                    if row.next_launch && !editing_this {
                        let tag = "next launch";
                        let tcol = wcol.saturating_sub(tag.len() + 2);
                        write_row(&mut g, top, tcol, tag, dim, rbg);
                    }
                }
                // Proportional scrollbar when the list continues past the
                // window: position and extent at a glance, not just "more".
                if list.len() > visible && visible > 0 {
                    let track_top = SETTINGS_CARDS_TOP;
                    let track_h = (visible * SETTINGS_ROW_H).saturating_sub(1).max(1);
                    let thumb_h = (track_h * visible / list.len()).max(1);
                    let denom = (list.len() - visible).max(1);
                    let thumb_top = track_top + (track_h - thumb_h) * scroll / denom;
                    for r in track_top..track_top + track_h {
                        let (ch, color) = if (thumb_top..thumb_top + thumb_h).contains(&r) {
                            ('\u{2590}', dim) // ▐ thumb
                        } else {
                            ('\u{2502}', tokens.line) // │ track
                        };
                        let mut cell = Cell::blank();
                        cell.ch = ch;
                        cell.fg = color;
                        cell.bg = bg;
                        g.set_cell(value_end + 2, r, cell);
                    }
                }
                // Footer: keycap-styled contextual hints (keys bright, verbs
                // dim, so the eye can find the key it needs) with the save
                // state at the right — live changes persist on close, so
                // "modified" is a promise, not a warning.
                let hints: &[(&str, &str)] = if s.editing.is_some() {
                    &[("Enter", "apply"), ("Esc", "keep old value")]
                } else if setup && s.on_last_step() {
                    // Most-needed first: a narrow window drops the tail.
                    &[
                        ("Enter", "finish"),
                        ("t", "test alert"),
                        ("Esc", "skip setup"),
                        ("Left/Right", "change"),
                    ]
                } else if setup {
                    &[
                        ("Enter", "next step"),
                        ("Esc", "skip setup"),
                        ("Left/Right", "change"),
                        ("Shift+Tab", "back"),
                    ]
                } else if filtering {
                    &[
                        ("type", "to refine"),
                        ("Up/Down", "result"),
                        ("Esc", "back"),
                    ]
                } else {
                    &[
                        ("Tab", "category"),
                        ("Up/Down", "row"),
                        ("Left/Right", "change"),
                        ("/", "search"),
                        ("Enter", "edit"),
                        ("Esc", "close & save"),
                    ]
                };
                let status = if setup {
                    "saves when you finish"
                } else if s.dirty() {
                    "modified - saves on close"
                } else {
                    "saved"
                };
                let hrow = rows.saturating_sub(1);
                let limit = cols.saturating_sub(status.chars().count() + 4);
                let mut hcol = 2;
                for (key, verb) in hints {
                    if hcol + key.chars().count() + verb.chars().count() + 4 > limit {
                        break; // drop trailing hints rather than clip mid-word
                    }
                    write_row(&mut g, hrow, hcol, key, fg, bg);
                    hcol += key.chars().count() + 1;
                    write_row(&mut g, hrow, hcol, verb, dim, bg);
                    hcol += verb.chars().count() + 3;
                }
                let scol = cols.saturating_sub(status.chars().count() + 2);
                write_row(
                    &mut g,
                    hrow,
                    scol,
                    status,
                    if s.dirty() && !setup { fg } else { dim },
                    bg,
                );
            }
            None => {}
        }
        g
    }
}

/// The `▾` dropdown's items: *Settings* and *Open config file*.
fn menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: "Settings".to_string(),
            kind: MenuKind::Settings,
        },
        MenuItem {
            label: "Open config file".to_string(),
            kind: MenuKind::EditConfig,
        },
    ]
}

/// Whether setup is on its notifications step (where `t` sends a test).
fn on_notify_step(s: &Settings) -> bool {
    s.is_setup() && s.sections()[s.cat].1.contains(&Field::CommandNotify)
}

/// How many setting cards fit in a `grid_rows`-tall overlay grid.
fn settings_visible(grid_rows: usize) -> usize {
    grid_rows.saturating_sub(SETTINGS_CARDS_TOP + SETTINGS_FOOTER_H) / SETTINGS_ROW_H
}

/// The column a card's value widget ends at (exclusive): the window's right
/// pad, or the card-width cap on a wide window — whichever is nearer.
fn settings_value_end(cols: usize) -> usize {
    (cols.saturating_sub(SETTINGS_RIGHT_PAD)).min(SETTINGS_SIDEBAR_W + 2 + SETTINGS_CARD_W_MAX)
}

/// What a settings-page cell `(col, row)` lands on, for clicks and hover.
/// `scroll`/`len` describe the active category's list; `Row` carries the
/// *list* index (scroll already added). `cols` sizes the search affordance's
/// zone at the title row's right edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SettingsHit {
    Category(usize),
    Row(usize),
    /// The `/ search` affordance on the title row.
    Search,
    None,
}

fn settings_hit(
    col: usize,
    row: usize,
    cols: usize,
    grid_rows: usize,
    scroll: usize,
    len: usize,
    sections: usize,
) -> SettingsHit {
    let footer = grid_rows.saturating_sub(SETTINGS_FOOTER_H);
    if row < SETTINGS_TOP || row >= footer {
        return SettingsHit::None;
    }
    if col < SETTINGS_SIDEBAR_W {
        // Sidebar entries sit on every other row, and the whole strip is
        // clickable: the gap row belongs to the entry above it.
        let i = (row - SETTINGS_TOP) / 2;
        if i < sections {
            return SettingsHit::Category(i);
        }
        return SettingsHit::None;
    }
    // Title row: its right end is the search affordance.
    let value_end = settings_value_end(cols);
    if row == SETTINGS_TOP {
        if col >= value_end.saturating_sub(SETTINGS_SEARCH_W) && col < value_end {
            return SettingsHit::Search;
        }
        return SettingsHit::None;
    }
    // Card area (below the page title + rule): a card's title and
    // description rows both count; the spacing row between cards is dead.
    let Some(r) = row.checked_sub(SETTINGS_CARDS_TOP) else {
        return SettingsHit::None;
    };
    let card = r / SETTINGS_ROW_H;
    if r % SETTINGS_ROW_H < 2 && card < settings_visible(grid_rows) {
        let i = scroll + card;
        if i < len {
            return SettingsHit::Row(i);
        }
    }
    SettingsHit::None
}

/// A value widget's cell text. Toggles render a block-glyph switch (boxdraw
/// synthesizes U+2588/2591, so no font dependency); a selected *or hovered*
/// choice/number grows `< value >` cycle arrows — hover gets the affordance
/// too, so values don't read as static text until clicked.
fn widget_text(w: &Widget, active: bool) -> String {
    match w {
        Widget::Toggle(true) => "[\u{2588}\u{2588}] on".to_string(),
        Widget::Toggle(false) => "[\u{2591}\u{2591}] off".to_string(),
        Widget::Choice(v) | Widget::Number(v) => {
            if active {
                format!("< {v} >")
            } else {
                v.clone()
            }
        }
    }
}

/// Write ASCII `text` into grid `row` from `col` (width-1 cells) in the given
/// colors, clipping at the grid edge. Overlay-page labels are all ASCII.
fn write_row(grid: &mut Grid, row: usize, col: usize, text: &str, fg: u32, bg: u32) {
    for (i, ch) in text.chars().enumerate() {
        let c = col + i;
        if c >= grid.cols || row >= grid.rows {
            break;
        }
        let mut cell = Cell::blank();
        cell.ch = ch;
        cell.fg = fg;
        cell.bg = bg;
        grid.set_cell(c, row, cell);
    }
}

/// Fill grid `row` cells `[col0, col1)` with a blank cell in the given colors
/// (the selection bar behind a highlighted overlay row).
fn fill_row(grid: &mut Grid, row: usize, col0: usize, col1: usize, fg: u32, bg: u32) {
    for c in col0..col1.min(grid.cols) {
        let mut cell = Cell::blank();
        cell.fg = fg;
        cell.bg = bg;
        grid.set_cell(c, row, cell);
    }
}

/// Write `text` into the chrome `row` from `*col` up to (not including)
/// `limit`, advancing wide glyphs by two cells with a flagged trailer and
/// stopping at the boundary. Every written cell adopts `fg`/`bg` and `hit`.
#[allow(clippy::too_many_arguments)]
fn put_text(
    row: &mut [Cell],
    hits: &mut [Hit],
    col: &mut usize,
    limit: usize,
    text: &str,
    fg: u32,
    bg: u32,
    hit: Hit,
) {
    let limit = limit.min(row.len());
    for ch in text.chars() {
        if *col >= limit {
            break;
        }
        let w = char_width(ch);
        if w == 0 {
            continue; // zero-width scalars don't occupy a cell
        }
        if w == 2 && *col + 1 >= limit {
            break; // a wide glyph's trailer wouldn't fit
        }
        row[*col].ch = ch;
        row[*col].fg = fg;
        row[*col].bg = bg;
        hits[*col] = hit;
        *col += 1;
        if w == 2 {
            row[*col].ch = ' ';
            row[*col].fg = fg;
            row[*col].bg = bg;
            row[*col].flags = WIDE_TRAILER;
            hits[*col] = hit;
            *col += 1;
        }
    }
}

/// Fold a wheel/trackpad delta (already converted to a "lines" unit) plus a
/// carried fractional remainder into a whole line count and the remainder to
/// carry into the next event. Pulled out as a pure function (rather than
/// inlined in the `MouseWheel` handler) so the accumulation math — the fix
/// for slow scrolling silently doing nothing — is unit-testable without a
/// live window.
fn accumulate_scroll_lines(accum: f64, raw_lines: f64) -> (isize, f64) {
    let total = accum + raw_lines;
    let lines = total.trunc() as isize;
    (lines, total - lines as f64)
}

/// Per-channel mix of `t/255` of `b` into `a` (`0xRRGGBB`) — used to derive
/// the chrome bar's bg and dimmed text from the theme without new config keys.
/// Build the command dock's grid (`cols × rows`) from the focused pane's
/// state: a header, then the running command and the finished command
/// blocks newest-first — exit glyph (✓ green / ✗ red / · when the shell
/// reported no code), the command line (the prompt line just above the
/// block's output), and the runtime right-aligned. Returns the grid plus
/// the per-row click map (the absolute line a click jumps to).
fn build_dock_grid(
    g: &Grid,
    tokens: &Chrome,
    cols: usize,
    rows: usize,
) -> (Grid, Vec<Option<usize>>) {
    let mut d = Grid::new(cols, rows);
    // The dock is chrome on the `sunk` surface, beside the grid (D38).
    let (fg, bg) = (tokens.text, tokens.sunk);
    let dim = tokens.muted;
    for r in 0..rows {
        fill_row(&mut d, r, 0, cols, fg, bg);
    }
    let mut items: Vec<Option<usize>> = vec![None; rows];
    write_row(&mut d, 0, 1, "Commands", dim, bg);

    // Entries newest-first: the running command (if any), then finished
    // blocks in reverse stream order.
    let mut entries: Vec<(char, u32, String, usize)> = Vec::new();
    if let Some(start) = g.running_command() {
        entries.push(('\u{25b6}', tokens.accent, "\u{2026}".to_string(), start));
    }
    for b in g.fold_blocks().iter().rev() {
        let (glyph, color) = match b.exit {
            Some(0) => ('\u{2713}', tokens.success),
            Some(_) => ('\u{2717}', tokens.failure),
            None => ('\u{00b7}', dim),
        };
        let dur = b.runtime.map(fmt_runtime).unwrap_or_default();
        entries.push((glyph, color, dur, b.start));
    }

    if entries.is_empty() {
        write_row(&mut d, 2, 1, "No commands yet.", dim, bg);
        write_row(&mut d, 4, 1, "Needs OSC 133 shell", dim, bg);
        write_row(&mut d, 5, 1, "integration.", dim, bg);
        return (d, items);
    }
    for (row, (glyph, color, dur, start)) in (2..).zip(entries) {
        if row >= rows {
            break;
        }
        // The command's text: the prompt line just above its output (where
        // it was typed); the first output line when there is none.
        let mut label = g.abs_line_text(start.saturating_sub(1));
        if label.is_empty() {
            label = g.abs_line_text(start);
        }
        write_row(&mut d, row, 1, &glyph.to_string(), color, bg);
        let label_max = cols.saturating_sub(4 + dur.chars().count() + 1);
        let label: String = label.chars().take(label_max).collect();
        write_row(&mut d, row, 3, &label, fg, bg);
        let dcol = cols.saturating_sub(dur.chars().count() + 1);
        write_row(&mut d, row, dcol, &dur, dim, bg);
        items[row] = Some(start);
    }
    (d, items)
}

/// A command runtime for the dock: `<1s`, `42s`, `3m07s`, `2h05m`.
fn fmt_runtime(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        0 => "<1s".to_string(),
        1..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// The current git branch for `dir`: walk up to the nearest `.git`, follow a
/// worktree/submodule `gitdir:` file if that's what it is, and parse `HEAD` —
/// `ref: refs/heads/<name>` yields the name, a detached head yields the
/// short hash. Pure file reads (no `git` subprocess), `None` outside a repo
/// or on any parse surprise.
fn read_git_branch(dir: &std::path::Path) -> Option<String> {
    let mut cur = Some(dir);
    let git = loop {
        let d = cur?;
        let candidate = d.join(".git");
        if candidate.exists() {
            break candidate;
        }
        cur = d.parent();
    };
    let head_path = if git.is_file() {
        // A worktree/submodule: `.git` is a one-line `gitdir: <path>` file.
        let text = std::fs::read_to_string(&git).ok()?;
        let target = text.strip_prefix("gitdir:")?.trim();
        let target = std::path::Path::new(target);
        let base = if target.is_absolute() {
            target.to_path_buf()
        } else {
            git.parent()?.join(target)
        };
        base.join("HEAD")
    } else {
        git.join("HEAD")
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()),
        None if head.len() >= 8 => Some(head[..8].to_string()),
        None => None,
    }
}

/// Paint the window border with the theme background so the frame reads as
/// part of the terminal (the title bar itself is ours now), and ask DWM to
/// round the window corners so the borderless frame matches native Windows 11
/// windows. Both are Windows 11 only (DWM ignores the attributes on 10);
/// a no-op on other platforms.
fn apply_chrome(window: &Window, theme: &Theme) {
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::{Color, WindowExtWindows};
        let c = |rgb: u32| Color::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        window.set_border_color(Some(c(theme.bg)));
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(h) = handle.as_raw()
        {
            use windows_sys::Win32::Graphics::Dwm::{
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
            };
            let pref: i32 = DWMWCP_ROUND;
            // SAFETY: a live HWND from winit; DWM copies the 4-byte value.
            unsafe {
                DwmSetWindowAttribute(
                    h.hwnd.get() as windows_sys::Win32::Foundation::HWND,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    &pref as *const i32 as *const core::ffi::c_void,
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, theme);
    }
}

impl WindowState<'_> {
    /// This window's timers ([`tick_cursor`](Self::tick_cursor) and
    /// synchronized-update timeouts): the next wake deadline, or `None` when
    /// it can sleep until a real event (the router picks the earliest across
    /// windows).
    fn tick(&mut self, now: Instant) -> Option<Instant> {
        let cursor = self.tick_cursor(now);
        let sync = self.tick_sync();
        let compact = self.tick_compact(now);
        let paced = self.tick_paced(now);
        cursor
            .into_iter()
            .chain(sync)
            .chain(compact)
            .chain(paced)
            .min()
    }

    /// Paint output held back by the [`Pacer`] once its frame is due; return
    /// the deadline while it is still held.
    fn tick_paced(&mut self, now: Instant) -> Option<Instant> {
        let (due, next) = self.pacer.poll(now);
        if due && let Some(window) = &self.window {
            window.request_redraw();
        }
        next
    }

    /// Run every pane's due compaction step, background tabs included, and
    /// return the earliest next one.
    fn tick_compact(&mut self, now: Instant) -> Option<Instant> {
        let panes = self.tabs.iter_mut().flat_map(|t| &mut t.panes);
        panes
            .filter_map(|p| {
                // Locked only when a step is due: this runs after every event.
                if p.compactor.is_due(now) {
                    p.compactor.run(&mut p.grid.lock(), now);
                }
                p.compactor.due()
            })
            .min()
    }

    /// The next timeout of a synchronized update open on the active tab.
    /// When any pane's update has ended since the last tick, repaint: it may
    /// have timed out with no output to close it, while other panes still
    /// hold theirs (an extra repaint after a normal close is harmless).
    fn tick_sync(&mut self) -> Option<Instant> {
        let open: Vec<(u64, Instant)> = self.tabs.get(self.active).map_or_else(Vec::new, |tab| {
            tab.panes
                .iter()
                .filter_map(|p| Some((p.id, p.grid.lock().sync_output_until()?)))
                .collect()
        });
        let (held, next, released) = sync_step(&self.sync_held, &open);
        if released && let Some(window) = &self.window {
            window.request_redraw();
        }
        self.sync_held = held;
        next
    }

    /// Drive cursor blink + Kitty animations for this window: when the active
    /// tab's cursor blinks and is visible, toggle its phase on a fixed
    /// interval; while an animation plays, tick at the frame floor. Returns
    /// the next wake deadline, or `None` when nothing needs one.
    fn tick_cursor(&mut self, now: Instant) -> Option<Instant> {
        const BLINK: Duration = Duration::from_millis(530);
        /// Kitty animation tick — the floor gap is 40ms, so ticking at it
        /// hits every frame boundary within a frame's tolerance.
        const ANIM: Duration = Duration::from_millis(40);
        // Kitty graphics animations: advance every visible pane's playing
        // images; a frame change repaints.
        let mut animating = false;
        if let Some(tab) = self.tabs.get(self.active) {
            for p in &tab.panes {
                let mut g = p.grid.lock();
                if g.advance_animations(now)
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
                animating |= g.kitty_images.iter().any(|i| i.playing);
            }
        }
        // Reduced motion (D50): the cursor draws solid, whoever asked it to
        // blink (config or a program's DECSCUSR).
        let blinking = !self.reduce_motion()
            && self.pane().is_some_and(|p| {
                let g = p.grid.lock();
                g.cursor_blink && g.cursor_visible && g.view_offset == 0
            });
        if !blinking {
            self.cursor_blink_on = true;
        }
        // A live cursor trail needs frames until it fades (~150ms).
        if self.trail.is_some() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return Some(now + ANIM);
        }
        // Dragging a selection past the pane's edge keeps scrolling on its
        // own timer as long as the pointer stays there, even with no new
        // `CursorMoved` events (the pointer held stationary past the edge).
        if self.drag_edge_autoscroll() {
            return Some(now + DRAG_SCROLL_INTERVAL);
        }
        if !blinking && !animating {
            self.cursor_blink_on = true;
            return None;
        }
        if blinking && now.duration_since(self.last_blink) >= BLINK {
            self.cursor_blink_on = !self.cursor_blink_on;
            self.last_blink = now;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        Some(if animating {
            now + ANIM
        } else {
            self.last_blink + BLINK
        })
    }

    /// Create this state's OS window + renderer if it doesn't have one yet.
    /// Returns whether the window is usable; `false` marks the state closed
    /// (the router drops it).
    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if self.window.is_some() {
            return true;
        }
        // `theme = "auto"`: seed from the OS appearance if winit can tell us
        // (falling back to dark); `ThemeChanged` keeps following it live.
        if self.config.theme_auto {
            self.theme = self.auto_theme(None);
            self.config.theme = self.theme;
        }
        // Size the font for the monitor this window is about to land on
        // *before* computing its pixel size below. `font_px`/`cell_w`/`cell_h`
        // were computed assuming scale factor 1.0 (`new_window_state` runs
        // before any OS window — and thus any monitor — exists); launching
        // directly on a HiDPI display without this would start with text at
        // half the intended visual size, and no `ScaleFactorChanged` event
        // ever fires to correct it if the window never changes monitors.
        let target_scale = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        if (target_scale - self.scale_factor).abs() > 1e-9 {
            let ratio = target_scale / self.scale_factor;
            let px = ((self.font_px as f64) * ratio) as f32;
            let ligatures = self.config.ligatures.unwrap_or(true);
            self.base_px = ((self.base_px as f64) * ratio) as f32;
            self.rescale_ui_font(physical_px(UI_FONT_PX, target_scale), ligatures);
            self.rescale_font_cache(px, ligatures, target_scale);
        }
        let width = (self.cols as usize * self.cell_w + 2 * self.pad) as u32;
        // The top bar and the bottom ribbon, plus the padding band.
        let height = (self.rows as usize * self.cell_h + self.bars_h() + 2 * self.pad) as u32;
        // `--title` (or a config `title` key) seeds the initial title; the
        // child's own OSC 0/2 still wins once it emits one (see the per-frame
        // `set_title` above).
        // Only request a transparent (alpha-capable) surface when opacity is
        // actually configured below 1.0 — an unnecessarily transparent
        // window can cost a compositor extra work for no visible benefit.
        let transparent = self.config.opacity.is_some_and(|o| o < 1.0);
        let attrs = Window::default_attributes()
            .with_title(self.config.title.as_deref().unwrap_or("rusty_term"))
            .with_decorations(false)
            .with_transparent(transparent)
            // Created hidden and shown below, after the AccessKit adapter is
            // attached (C20) — the adapter must be created before the window
            // is first shown, or it panics.
            .with_visible(false)
            .with_inner_size(winit::dpi::PhysicalSize::new(width, height));
        // Keep the DWM drop shadow so the borderless window still reads as
        // raised above the desktop.
        #[cfg(target_os = "windows")]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_undecorated_shadow(true)
        };
        // `--maximized` / `--fullscreen` (or a `[window] launch_mode` config
        // key); unset leaves the normal windowed default.
        let attrs = match self.config.launch_mode {
            Some(LaunchMode::Maximized) => attrs.with_maximized(true),
            Some(LaunchMode::Fullscreen) => {
                attrs.with_fullscreen(Some(Fullscreen::Borderless(None)))
            }
            None => attrs,
        };
        let Ok(window) = event_loop.create_window(attrs) else {
            self.closed = true;
            return false;
        };
        let window = Arc::new(window);
        apply_chrome(&window, &self.theme);
        self.window = Some(window.clone());
        // Let the OS deliver IME composition events (CJK, dead keys).
        window.set_ime_allowed(true);
        match self.make_renderer(window.clone()) {
            Some(r) => self.renderer = Some(r),
            None => {
                self.window = None;
                self.closed = true;
                return false;
            }
        }
        self.apply_opacity();
        // An attached window's blocks are laid out in the window's pixels,
        // which it has only now.
        for ti in 0..self.tabs.len() {
            self.layout_panes(ti);
        }
        // Must be created before the window is shown (see the `with_visible`
        // note above); `with_event_loop_proxy` routes AccessKit's own events
        // back through the ordinary winit event loop as `UserEvent::Accessibility`.
        let adapter = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        self.access = Some(super::access::AccessState::new(adapter));
        window.set_visible(true);
        window.request_redraw();
        true
    }

    /// This state's OS window id, once the window exists.
    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id())
    }

    /// Handle an AccessKit event routed back through the winit loop (C20).
    /// `ActionRequested` (e.g. a screen reader's scroll/focus request) is
    /// acknowledged implicitly by AccessKit already having the node — this
    /// terminal doesn't yet act on requested actions beyond exposing content,
    /// a documented limitation rather than a silent no-op.
    fn on_accessibility_event(&mut self, event: accesskit_winit::WindowEvent) {
        if let accesskit_winit::WindowEvent::InitialTreeRequested = event
            && let Some(access) = self.access.as_mut()
        {
            access.push_initial();
        }
    }

    /// Show the source's `tree` in the attached window's one tab: open the
    /// panes new to it, drop the ones it left out (which ends them at the
    /// source), keep the focus if it is still shown. `None` closes the
    /// window.
    fn show_tree(&mut self, tree: Option<Tree>) {
        let source = self.source;
        let Some(tree) = tree else {
            self.closed = true;
            return;
        };
        self.body.set_ended(None);
        let layout = Layout::from_tree(&tree);
        let first = layout.ids().first().copied().unwrap_or_default();
        let rects: Vec<(u64, Rect)> = self
            .places_of(&layout, false, first)
            .into_iter()
            .map(|(id, p)| (id, p.cells))
            .collect();
        let mut panes = match self.tabs.pop() {
            Some(tab) => tab.panes,
            None => Vec::new(),
        };
        panes.retain(|p| rects.iter().any(|(id, _)| *id == p.id));
        for &(key, r) in &rects {
            if panes.iter().any(|p| p.id == key) {
                continue;
            }
            let (cols, rows) = (r.cols.max(1) as u16, r.rows.max(1) as u16);
            let opened = source
                .open(key, cols, rows)
                .and_then(|io| self.pane_from(key, io, cols, rows, None));
            match opened {
                Ok(pane) => panes.push(pane),
                Err(e) => eprintln!("rusty_term: pane {key}: {e}"),
            }
        }
        let focus = self
            .told
            .filter(|f| rects.iter().any(|(id, _)| id == f))
            .unwrap_or(rects[0].0);
        self.tabs.push(Tab {
            panes,
            layout,
            focus,
            attention: false,
            zoomed: false,
        });
        self.active = 0;
        self.layout_panes(0);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Show that the shown session, `name`, ended while the source was
    /// away (gui-design D62, handoff D114): no block, and a panel with
    /// "Open switcher" until a layout comes again.
    fn show_ended(&mut self, name: String) {
        self.body.set_ended(Some(name));
        self.tabs.clear();
        self.active = 0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Show no block: every block of the source's window is detached
    /// (gui-design D61). The body says so until a layout comes again.
    fn show_empty(&mut self) {
        self.body.set_ended(None);
        self.tabs.clear();
        self.active = 0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Show how the source reaches its server (gui-design D62): the
    /// banner, the dimmed blocks and the ribbon's dot. Attached anew, the
    /// window drops its tabs; the layout that follows opens them again.
    fn show_link(&mut self, link: super::source::Link) {
        let body = &mut self.body;
        body.set_link(link);
        let offline = body.offline();
        if link == (super::source::Link::Back { kept: false }) {
            self.tabs.clear();
            self.active = 0;
        }
        if offline && self.popover_kind() == Some(Kind::Detached) {
            self.popover.close();
        }
        let session = self.strip.session().clone();
        self.refresh_popover(&session);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Show `notice` in the banner's place while the link is up (D63).
    fn show_notice(&mut self, notice: Option<super::source::Notice>) {
        self.body.set_notice(notice);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The link is down: the server-bound controls take no input (D62).
    fn offline(&self) -> bool {
        self.body.offline()
    }

    /// A banner button: ask the source to retry or stop, or hide F5b.
    fn on_banner(&mut self, action: super::banner::Action) {
        use super::banner::Action;
        match action {
            Action::Retry => self.source.retry(),
            Action::Stop => self.source.stop(),
            Action::Dismiss => self.show_link(super::source::Link::Up),
            Action::DismissInfo => self.show_notice(None),
        }
    }

    /// Show the source's `session` on the top strip.
    fn show_session(&mut self, session: super::source::Session) {
        self.body.set_session(session.local, &session.host);
        self.refresh_popover(&session);
        self.strip.show(session);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Show what the source says of its blocks in their headers.
    fn show_blocks(&mut self, blocks: Vec<(u64, super::source::BlockInfo)>) {
        let body = &mut self.body;
        body.show(blocks);
        let strip = &mut self.strip;
        strip.set_running(body.running());
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        // Detached rows say their blocks' states.
        if self.popover_kind() == Some(Kind::Detached) {
            let session = self.strip.session().clone();
            self.refresh_popover(&session);
        }
    }

    /// The open popover's rows for `session` now (D60, D61: rows update
    /// live).
    fn refresh_popover(&mut self, session: &super::source::Session) {
        let rows = match self.popover_kind() {
            Some(Kind::Switcher) => self.switcher_rows(session),
            Some(Kind::Detached) => self.detached_rows(session),
            None => return,
        };
        self.popover.update(rows);
    }

    /// The source moved the focus to `key`.
    fn focus_from_source(&mut self, key: u64) {
        self.told = Some(key);
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.layout.ids().contains(&key) {
            tab.focus = key;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    /// The source zoomed pane `key`, or unzoomed with `None`
    /// (gui-design F7). A zoom moves the server's focus to the block too.
    fn zoom_from_source(&mut self, key: Option<u64>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let key = key.filter(|k| tab.layout.ids().contains(k));
        if let Some(k) = key {
            tab.focus = k;
            self.told = Some(k);
        }
        if tab.zoomed == key.is_some() {
            return;
        }
        tab.zoomed = key.is_some();
        self.layout_panes(self.active);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Tell the source where the user moved the focus, once per move. As
    /// on the server, moving it off a zoomed block unzooms the window.
    fn tell_focus(&mut self) {
        let source = self.source;
        let focus = self.tabs.get(self.active).map(|t| t.focus);
        let Some(key) = focus.filter(|f| self.told != Some(*f)) else {
            return;
        };
        self.told = Some(key);
        source.focus(key);
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.zoomed
        {
            tab.zoomed = false;
            self.layout_panes(self.active);
        }
    }

    /// Ask the source for `act` on the focused pane's window (handoff
    /// D104); the layout changes when the source says so.
    fn act_on_source(&self, act: impl FnOnce(u64, bool) -> Act) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        self.source.act(act(tab.focus, tab.zoomed));
    }

    /// Zoom the focused block, or unzoom (gui-design F7).
    fn toggle_source_zoom(&self) {
        self.act_on_source(|focus, zoomed| match zoomed {
            true => Act::Unzoom,
            false => Act::Zoom(focus),
        });
    }

    /// Carry out the key after the armed prefix (gui-design F2, handoff
    /// D104). Returns whether the mode stays armed.
    fn run_prefix(&mut self, command: Command) -> bool {
        match command {
            Command::Wait => return true,
            Command::Cancel | Command::Unknown => {}
            Command::Type => self.write_child(&[prefix::PREFIX]),
            Command::Split(dir) => self.act_on_source(|key, _| Act::Split { key, dir }),
            Command::Zoom => self.toggle_source_zoom(),
            Command::Close => self.act_on_source(|key, _| Act::Close(key)),
            Command::Focus(dx, dy) => self.focus_block_dir(dx, dy),
            Command::Jump(n) => {
                let key = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.layout.ids().get(n - 1).copied());
                if let (Some(key), Some(tab)) = (key, self.tabs.get_mut(self.active)) {
                    tab.focus = key;
                }
            }
            // Dropping the panes detaches each block; the session runs on.
            Command::Detach => self.closed = true,
        }
        false
    }

    /// Move an attached window's focus to the neighbour `(dx, dy)` of the
    /// focused block in the unzoomed layout, as the TUI finds it.
    fn focus_block_dir(&mut self, dx: isize, dy: isize) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let zoomed = std::mem::take(&mut tab.zoomed);
        let before = tab.focus;
        self.focus_dir(dx, dy);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            // Still zoomed when there was nowhere to go.
            tab.zoomed = zoomed && tab.focus == before;
        }
    }

    /// Arm or disarm the prefix, redrawing the ribbon and badges.
    fn set_armed(&mut self, armed: bool) {
        if self.armed == armed {
            return;
        }
        self.armed = armed;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Which popover is open, if any.
    fn popover_kind(&self) -> Option<Kind> {
        self.popover.kind()
    }

    /// Open the `kind` popover, or close it if it is the one open: the
    /// session switcher under the session chip (gui-design D60), or the
    /// detached blocks under their chip (D61).
    fn toggle_popover(&mut self, kind: Kind) {
        self.set_armed(false);
        let open = self.popover_kind();
        self.popover.close();
        if open != Some(kind) {
            self.open_popover(kind);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Open the `kind` popover on the session shown now.
    fn open_popover(&mut self, kind: Kind) {
        let session = self.strip.session();
        let s = self.scale_factor as f32;
        let top = (popover::TOP * s).round() as usize;
        let (header, rows, x) = match kind {
            Kind::Switcher => (
                session.host.clone(),
                self.switcher_rows(session),
                (popover::SWITCHER_X * s).round() as usize,
            ),
            Kind::Detached => {
                // Right edge on the chip's right edge (D61).
                let Some(chip) = self.strip.painted().and_then(|p| {
                    p.spots
                        .iter()
                        .find(|(_, spot)| *spot == Spot::Detached)
                        .map(|(r, _)| *r)
                }) else {
                    return;
                };
                let x = (chip.x + chip.w - popover::DETACHED_W * s).max(0.0);
                (
                    "Detached blocks".to_owned(),
                    self.detached_rows(session),
                    x.round() as usize,
                )
            }
        };
        if rows.is_empty() {
            return;
        }
        let menu = Menu { kind, header, rows };
        self.popover.open(menu, (x, top));
    }

    /// The switcher's rows: the server's sessions, then the menu's items.
    fn switcher_rows(&self, session: &super::source::Session) -> Vec<Row> {
        // With the link down no session can be shown (D62).
        let offline = self.offline();
        let sessions = session.sessions.iter().cloned().map(|mut s| {
            s.can_show &= !offline;
            Row::Session(s)
        });
        let items = self
            .switcher_items()
            .into_iter()
            .map(|i| Row::Item(i.label));
        sessions.chain(items).collect()
    }

    /// The detached popover's rows: each detached block's name, lead
    /// glyph, state and age (D61).
    fn detached_rows(&self, session: &super::source::Session) -> Vec<Row> {
        let now = Instant::now();
        session
            .detached
            .iter()
            .map(|d| {
                let state = self.body.state(d.key);
                let age = d.since.map(|t| now.saturating_duration_since(t));
                Row::Detached(DetachedRow {
                    key: d.key,
                    name: d.name.clone(),
                    detail: popover::detached_detail(state, age),
                    lead: self.body.lead(d.key),
                })
            })
            .collect()
    }

    /// The menu's items an attached window lists under its sessions: the
    /// `▾` menu's (D45 b).
    fn switcher_items(&self) -> Vec<MenuItem> {
        menu_items()
    }

    /// A key while a popover is open: Up/Down move, Enter picks, Esc
    /// closes; the rest are dropped.
    fn popover_key(&mut self, key: &Key) {
        use winit::keyboard::NamedKey;
        if self.confirm.is_some() {
            self.confirm_key(key);
            return;
        }
        let popover = &mut self.popover;
        match key {
            Key::Named(NamedKey::ArrowUp) => popover.step(false),
            Key::Named(NamedKey::ArrowDown) => popover.step(true),
            Key::Named(NamedKey::Escape) => popover.close(),
            Key::Named(NamedKey::Enter) => {
                let picked = popover.selected();
                self.popover_pick(picked);
            }
            _ => return,
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Carry out `picked` and close the popover: show another session,
    /// run a menu item or reattach a block; the current session only
    /// closes.
    fn popover_pick(&mut self, picked: Option<Pick>) {
        let popover = &mut self.popover;
        let Some(picked) = picked else {
            return; // a disabled row stays open
        };
        let current = matches!(picked, Pick::Session(key) if popover.is_current(key));
        popover.close();
        match picked {
            Pick::Session(_) if current => {}
            Pick::Session(key) => {
                self.source.switch(key);
            }
            Pick::Item(i) => {
                if let Some(item) = self.switcher_items().into_iter().nth(i) {
                    self.menu_pick(item.kind);
                }
            }
            // Needs no shown block: it may fill an empty window (D61).
            Pick::Reattach(key) => {
                self.source.act(Act::Reattach(key));
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// A click on an owner-only block's take-over button (gui-design
    /// D64): "Take over" focuses the block and opens the confirm under
    /// the button; the others go to the source at once.
    fn on_take(&mut self, key: u64, button: blocks::TakeButton) {
        match button {
            blocks::TakeButton::TakeOver => self.open_confirm(key),
            blocks::TakeButton::CancelAsk => self.act_on_source(|_, _| Act::CancelAsk(key)),
            blocks::TakeButton::GiveBack => self.act_on_source(|_, _| Act::GiveBack(key)),
        }
    }

    /// Focus block `key` and open its take-over confirm, right-aligned
    /// with its button and 8 px under its header (D64 2).
    fn open_confirm(&mut self, key: u64) {
        let spot = blocks::Spot::Take(key, blocks::TakeButton::TakeOver);
        let Some((program, owner)) = self.body.owned(key) else {
            return;
        };
        let Some(button) = self
            .body
            .painted()
            .and_then(|b| b.spots.iter().find(|(_, s)| *s == spot).map(|(r, _)| *r))
        else {
            return;
        };
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.focus = key;
        }
        self.tell_focus();
        self.popover.close();
        let s = self.scale_factor as f32;
        let header_bottom = button.y + button.h / 2.0 + blocks::HEADER_HALF * s;
        let top = self.chrome_h() as f32 + header_bottom + confirm::DROP * s;
        self.confirm = Confirm::new(
            key,
            confirm::copy(&program, &owner),
            button.x + button.w,
            top,
            s,
        );
    }

    /// A key while the confirm is open: Tab and Shift-Tab move between
    /// its buttons, Enter presses one, Esc cancels; the rest are dropped.
    fn confirm_key(&mut self, key: &Key) {
        use winit::keyboard::NamedKey;
        let key = match key {
            Key::Named(NamedKey::Tab) if self.mods.shift_key() => ConfirmKey::BackTab,
            Key::Named(NamedKey::Tab) => ConfirmKey::Tab,
            Key::Named(NamedKey::Enter) => ConfirmKey::Enter,
            Key::Named(NamedKey::Escape) => ConfirmKey::Escape,
            _ => return,
        };
        let answer = match &mut self.confirm {
            Some(confirm) => confirm.key(key),
            None => return,
        };
        self.confirm_answer(answer);
    }

    /// Carry out the confirm's `answer`: take its block over, or close.
    fn confirm_answer(&mut self, answer: Answer) {
        let key = match answer {
            Answer::Stay => None,
            Answer::Cancel => {
                self.confirm = None;
                None
            }
            Answer::TakeOver => self.confirm.take().map(|c| c.key),
        };
        if let Some(key) = key {
            self.source.act(Act::TakeOver(key));
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Note where taking pane `key`'s block over stands (D64).
    fn show_take(&mut self, key: u64, take: super::source::Take) {
        self.body.set_take(key, take);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Whether a popover is open.
    fn popover_open(&self) -> bool {
        self.popover_kind().is_some() || self.confirm.is_some()
    }

    /// The pointer moved to `(x, y)`: light the row under it.
    fn popover_hover(&mut self, x: f64, y: f64) {
        let changed =
            self.popover.hover(x, y) || self.confirm.as_mut().is_some_and(|c| c.hover(x, y));
        if let (true, Some(window)) = (changed, &self.window) {
            window.request_redraw();
        }
    }

    /// A click while a popover is open: a row picks, a click elsewhere on
    /// the panel does nothing, and one off it closes it. A click on the
    /// open popover's own chip closes it too, through the chip.
    fn popover_click(&mut self, x: f64, y: f64) {
        if let Some(confirm) = &self.confirm {
            let answer = confirm.click(x, y);
            self.confirm_answer(answer);
            return;
        }
        match self.popover.hit(x, y) {
            (None, true) => {}
            (Some(picked), _) => self.popover_pick(Some(picked)),
            (None, false) => {
                self.popover.close();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    /// An attached window's prefix keys (gui-design F2): Ctrl-] arms, and
    /// the next key runs. Returns whether the key was taken.
    fn prefix_key(&mut self, key: &Key, physical: PhysicalKey) -> bool {
        if !self.armed {
            if !prefix::is_prefix(key, physical, self.mods) {
                return false;
            }
            self.set_armed(true);
            return true;
        }
        let stay = self.run_prefix(prefix::command(key, physical, self.mods));
        self.set_armed(stay);
        true
    }

    /// Whether pane `id` (a reader-thread wakeup tag) belongs to this window.
    fn has_pane(&self, id: u64) -> bool {
        self.tabs.iter().any(|t| t.panes.iter().any(|p| p.id == id))
    }

    /// Handle one winit event addressed to this window.
    fn on_window_event(&mut self, event: WindowEvent) {
        if let (Some(access), Some(window)) = (self.access.as_mut(), self.window.as_ref()) {
            access.process_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => self.closed = true,
            WindowEvent::ThemeChanged(t) => {
                if self.config.theme_auto {
                    let new = self.auto_theme(Some(t));
                    self.apply_theme_live(new);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                // Focus reporting (`?1004`): tell the focused pane's child it
                // gained/lost focus, if it asked. Redraw either way — the
                // cursor renders only while focused, and regaining focus
                // clears the active tab's attention badge (in `chrome_row`).
                self.focused = focused;
                if !focused {
                    self.set_armed(false);
                }
                // A release that lands after focus moves away never reaches
                // us, so drop the pressed look here rather than leave it stuck.
                self.pressed = pressed_after_focus(self.pressed, focused);
                if let Some(p) = self.pane_mut()
                    && p.grid.lock().focus_reporting
                {
                    let _ = p.writer.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::DroppedFile(path) => {
                // Paste the dropped file's shell-quoted path (plus a trailing
                // space, so several drops land as separate arguments), same
                // bracketed-paste handling as a clipboard paste.
                let quoted = format!("{} ", shell_quote(&path.to_string_lossy()));
                if let Some(p) = self.pane_mut() {
                    let bracketed = p.grid.lock().bracketed_paste;
                    let _ = p.writer.write(&encode_paste(&quoted, bracketed));
                }
                self.snap_to_bottom();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(size) => {
                self.apply_size(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.on_scale_factor_changed(scale_factor);
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods.state();
                // Ctrl pressed/released with the pointer stationary should
                // show/hide the link affordance immediately, not wait for
                // the next `CursorMoved`.
                if self.update_hover_link()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    let (app_cursor, kitty_flags, win32_input) = self
                        .pane()
                        .map(|p| {
                            let g = p.grid.lock();
                            (g.app_cursor_keys, g.kitty_keyboard_flags(), g.win32_input)
                        })
                        .unwrap_or_default();
                    let ui_owns_keys = self.overlay.is_some()
                        || self.popover_open()
                        || self.searching.is_some()
                        || self.copy_mode.is_some();
                    let release = KeyRelease {
                        physical: event.physical_key,
                        logical: &event.logical_key,
                        mods: self.mods,
                        app_cursor,
                        kitty_flags,
                        win32_input,
                    };
                    if let Some(bytes) =
                        release_input(&mut self.consumed_keys, ui_owns_keys, &release)
                    {
                        self.write_child(&bytes);
                    }
                    return;
                }
                // An open popover owns every key (gui-design D60, D61).
                if self.popover_open() {
                    self.popover_key(&event.logical_key);
                    self.consumed_keys.consume(event.physical_key);
                    return;
                }
                // A settings page / shell menu, if open, owns all key input,
                // except the chord that toggles the page closed again (it
                // closes setup as a skip).
                if self.overlay.is_some() {
                    let action = self.chord_action(event.physical_key);
                    if overlay_passes_through(action) {
                        let mut consumed = std::mem::take(&mut self.consumed_keys);
                        route_key_down(
                            &mut consumed,
                            event.physical_key,
                            event.repeat,
                            action,
                            |a| self.exec_action(a),
                        );
                        self.consumed_keys = consumed;
                    } else {
                        self.overlay_key(&event);
                    }
                    return;
                }
                // With the link down, keys reach no block and run no
                // prefix; only the settings chord still works (D62).
                if self.offline() {
                    if overlay_passes_through(self.chord_action(event.physical_key)) {
                        self.exec_action(Action::OpenSettings);
                    }
                    self.consumed_keys.consume(event.physical_key);
                    return;
                }
                // In search mode, keys edit the query / step matches; nothing
                // reaches the keymap or the child until Esc exits.
                if self.search_key(&event) {
                    return;
                }
                // Copy mode owns the keyboard while active.
                if self.copy_mode_key(&event) {
                    return;
                }
                // An attached window's prefix and the key after it never
                // reach the block or the keymap.
                if self.prefix_key(&event.logical_key, event.physical_key) {
                    self.consumed_keys.consume(event.physical_key);
                    return;
                }
                // Terminal-owned shortcuts (configurable via the `[keys]` config
                // section) are looked up before native encoding, so a bound chord
                // never reaches the child.
                let action = self.chord_action(event.physical_key);
                let mut consumed = std::mem::take(&mut self.consumed_keys);
                let taken = route_key_down(
                    &mut consumed,
                    event.physical_key,
                    event.repeat,
                    action,
                    |a| self.exec_action(a),
                );
                self.consumed_keys = consumed;
                if taken {
                    return;
                }
                // Ctrl+Alt+1..9 jump straight to that tab (the Windows
                // Terminal convention; 9 = last tab, browser-style). After
                // the keymap lookup so a user chord still wins, and on the
                // Ctrl+Alt layer so plain Ctrl+digit stays encodable to
                // kitty-protocol apps.
                if self.mods.control_key()
                    && self.mods.alt_key()
                    && !self.mods.shift_key()
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    let digit = match code {
                        KeyCode::Digit1 => Some(0),
                        KeyCode::Digit2 => Some(1),
                        KeyCode::Digit3 => Some(2),
                        KeyCode::Digit4 => Some(3),
                        KeyCode::Digit5 => Some(4),
                        KeyCode::Digit6 => Some(5),
                        KeyCode::Digit7 => Some(6),
                        KeyCode::Digit8 => Some(7),
                        KeyCode::Digit9 => Some(usize::MAX), // last tab
                        _ => None,
                    };
                    if let Some(d) = digit {
                        let i = if d == usize::MAX {
                            self.tabs.len().saturating_sub(1)
                        } else {
                            d
                        };
                        self.select_tab(i);
                        return;
                    }
                }
                // While the IME is composing, it owns key input; don't also
                // encode it (the committed text arrives via `WindowEvent::Ime`).
                if self
                    .pane()
                    .is_some_and(|p| !p.grid.lock().ime_preedit.is_empty())
                {
                    return;
                }
                let (app_cursor, app_keypad, kitty_flags, win32_input) = self
                    .pane()
                    .map(|p| {
                        let g = p.grid.lock();
                        (
                            g.app_cursor_keys,
                            g.app_keypad,
                            g.kitty_keyboard_flags(),
                            g.win32_input,
                        )
                    })
                    .unwrap_or_default();
                // win32-input-mode (?9001) supersedes every VT encoding: the
                // child asked for raw key records, presses and releases alike.
                if win32_input {
                    if let Some(bytes) = super::input::encode_win32(
                        event.physical_key,
                        &event.logical_key,
                        self.mods,
                        true,
                    ) {
                        self.write_child(&bytes);
                        self.snap_to_bottom();
                    }
                    return;
                }
                // Application keypad mode (DECKPAM/DECNKM): a key physically
                // on the numpad encodes as its SS3 sequence; everything else
                // (mode off, modifiers held) falls through to normal encoding.
                let numpad = (event.location == KeyLocation::Numpad)
                    .then(|| super::input::encode_numpad(&event.logical_key, self.mods, app_keypad))
                    .flatten();
                let phase = if event.repeat {
                    super::input::KeyPhase::Repeat
                } else {
                    super::input::KeyPhase::Press
                };
                if let Some(bytes) = numpad.or_else(|| {
                    super::input::encode_full(
                        &event.logical_key,
                        self.mods,
                        app_cursor,
                        kitty_flags,
                        phase,
                        kitty_alternate(&event.logical_key, self.mods),
                        event.text.as_deref(),
                    )
                }) {
                    self.write_child(&bytes);
                    // Typing returns the view to the live bottom, as most
                    // terminals do, so the echoed input is visible.
                    self.snap_to_bottom();
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    if let Some(p) = self.pane() {
                        p.grid.lock().ime_preedit = text;
                    }
                    self.update_ime_area();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Ime::Commit(text) => {
                    if let Some(p) = self.pane() {
                        p.grid.lock().ime_preedit.clear();
                    }
                    // The find bar owns composed text while it's open — mirror
                    // `search_key`'s `Key::Character` arm above instead of
                    // leaking the committed IME text into the background
                    // shell underneath it. An open overlay (settings/shell
                    // menu) or copy mode has no free-text field of its own,
                    // so committed text is simply dropped rather than
                    // reaching the child either.
                    match ime_commit_target(
                        self.searching.is_some(),
                        self.overlay.is_some(),
                        self.copy_mode.is_some(),
                    ) {
                        ImeCommitTarget::SearchBar => {
                            if let Some(q) = self.searching.as_mut() {
                                q.push_str(&text);
                            }
                            self.run_search();
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                        ImeCommitTarget::Pane => {
                            self.pacer.input(Instant::now());
                            if let Some(p) = self.pane_mut() {
                                let _ = p.writer.write(text.as_bytes());
                            }
                            self.snap_to_bottom();
                        }
                        ImeCommitTarget::Dropped => {}
                    }
                }
                Ime::Enabled => {}
                Ime::Disabled => {
                    if let Some(p) = self.pane() {
                        p.grid.lock().ime_preedit.clear();
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            },
            WindowEvent::CursorLeft { .. } => {
                // Drop any chrome hover highlight when the pointer leaves.
                if self.hover.take().is_some()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
                if self.hover_link.take().is_some()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x, position.y);
                // Chrome-bar hover feedback: track the element under the
                // pointer and repaint when it changes (drag-strip cells and
                // anything below the bar count as no hover).
                let hover = if self.in_chrome(position.y) {
                    self.bar_hit(position.x, position.y)
                        .filter(|&h| h != Hit::Drag)
                } else {
                    self.body_hit(position.x, position.y).map(Hit::Block)
                };
                if hover != self.hover {
                    self.hover = hover;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                self.popover_hover(position.x, position.y);
                // Settings-page hover: highlight the card under the pointer.
                if let Some((col, grid_row)) = self.overlay_cell(position.x, position.y) {
                    let grid_rows = self.rows as usize;
                    let grid_cols = self.cols as usize;
                    if let Some(Overlay::Settings(s)) = &mut self.overlay {
                        let hovered = match settings_hit(
                            col,
                            grid_row,
                            grid_cols,
                            grid_rows,
                            s.scroll,
                            s.len(),
                            s.sections().len(),
                        ) {
                            SettingsHit::Row(i) => Some(i),
                            _ => None,
                        };
                        if hovered != s.hover {
                            s.hover = hovered;
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                    }
                }
                if self.update_hover_link()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
                // The edge band shows a resize cursor; over pane content (below
                // the chrome bar) a Ctrl-hovered hyperlink shows a pointer, the
                // child's OSC 22 request wins if it made one, and plain grid
                // text otherwise gets the usual I-beam (any other text area);
                // everywhere else (the chrome bar itself) default.
                let icon = match self.resize_zone(position.x, position.y) {
                    Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => {
                        CursorIcon::NwseResize
                    }
                    Some(ResizeDirection::NorthEast | ResizeDirection::SouthWest) => {
                        CursorIcon::NeswResize
                    }
                    Some(ResizeDirection::West | ResizeDirection::East) => CursorIcon::EwResize,
                    Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
                    // An open overlay (menu / settings) is a page of controls,
                    // not selectable text — the I-beam below would mislead.
                    None if self.overlay.is_some() => CursorIcon::Default,
                    None if !self.in_chrome(position.y) => {
                        if self.hover_link.is_some() {
                            CursorIcon::Pointer
                        } else {
                            self.pane()
                                .and_then(|p| {
                                    p.grid
                                        .lock()
                                        .cursor_icon
                                        .as_deref()
                                        .and_then(parse_cursor_icon)
                                })
                                .unwrap_or(CursorIcon::Text)
                        }
                    }
                    None => CursorIcon::Default,
                };
                if let Some(window) = &self.window {
                    window.set_cursor(icon);
                }
                if self.drag.is_live()
                    && self.update_drag_selection_head(position.x, position.y)
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
                // Motion reporting (`?1002` while a button is held, `?1003`
                // regardless) is independent of the local drag-selection
                // above — both can be active at once. `report_mouse` bypasses
                // itself when Shift is held, so a Shift-drag is pure local
                // selection with nothing forwarded to the app.
                let (sh, al, ct) = (
                    self.mods.shift_key(),
                    self.mods.alt_key(),
                    self.mods.control_key(),
                );
                let dragging = self.mouse_button_down.is_some();
                let button = self.mouse_button_down.unwrap_or_default();
                self.report_mouse(self.focused_pane_id(), |c, r| {
                    MouseEvent::new_point(c, r)
                        .with_move(dragging)
                        .with_button_kind(button)
                        .with_modifiers(sh, al, ct)
                });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(kind) = (match button {
                    MouseButton::Left => Some(MouseButtonKind::Left),
                    MouseButton::Middle => Some(MouseButtonKind::Middle),
                    MouseButton::Right => Some(MouseButtonKind::Right),
                    _ => None, // no SGR encoding for side/other buttons
                }) else {
                    return;
                };
                match state {
                    // An open popover takes the click: a row picks, any
                    // other place closes it (gui-design D60).
                    ElementState::Pressed if self.popover_open() => {
                        self.popover_click(self.mouse_pos.0, self.mouse_pos.1);
                    }
                    ElementState::Pressed => {
                        // Ctrl+click follows an OSC 8 hyperlink under the
                        // pointer, suppressing selection and mouse reporting
                        // for that click. Left-button only: the resize/chrome/
                        // selection semantics in `on_left_press` are as well.
                        if kind == MouseButtonKind::Left {
                            if self.mods.control_key() && self.open_link_under_pointer() {
                                return;
                            }
                            self.on_left_press();
                        }
                        if kind == MouseButtonKind::Middle {
                            if self.searching.is_some() {
                                let text = self.primary_text();
                                self.paste_into_search(text);
                                return;
                            }
                            if self.mods.shift_key()
                                || !self
                                    .pane()
                                    .is_some_and(|p| p.grid.lock().mouse_modes.active())
                            {
                                self.paste_primary();
                                return;
                            }
                        }
                        self.mouse_button_down = Some(kind);
                        let (sh, al, ct) = (
                            self.mods.shift_key(),
                            self.mods.alt_key(),
                            self.mods.control_key(),
                        );
                        self.report_mouse(self.focused_pane_id(), |c, r| {
                            MouseEvent::new_point(c, r)
                                .with_button(true)
                                .with_button_kind(kind)
                                .with_modifiers(sh, al, ct)
                        });
                    }
                    ElementState::Released => {
                        if kind == MouseButtonKind::Left {
                            if self.drag.release() {
                                self.copy_selection_primary(); // copy-on-select
                            }
                            if self.pressed.take().is_some()
                                && let Some(window) = &self.window
                            {
                                window.request_redraw();
                            }
                        }
                        if self.mouse_button_down == Some(kind) {
                            self.mouse_button_down = None;
                        }
                        let (sh, al, ct) = (
                            self.mods.shift_key(),
                            self.mods.alt_key(),
                            self.mods.control_key(),
                        );
                        self.report_mouse(self.focused_pane_id(), |c, r| {
                            MouseEvent::new_point(c, r)
                                .with_button(false)
                                .with_button_kind(kind)
                                .with_modifiers(sh, al, ct)
                        });
                    }
                }
            }
            // Wheel up browses into scrollback history, wheel down back toward
            // the live bottom. A notch is `WHEEL_LINES`; trackpads report pixels.
            WindowEvent::MouseWheel { delta, .. } => {
                // Accumulate fractional delta across events instead of
                // rounding each one independently — a high-resolution wheel
                // or a slow trackpad flick emits many sub-line events, and
                // rounding per-event dropped all of them (scrolling did
                // nothing until a large enough single delta arrived).
                let raw_lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64 * WHEEL_LINES as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / self.cell_h as f64,
                };
                let (lines, remainder) = accumulate_scroll_lines(self.scroll_accum, raw_lines);
                self.scroll_accum = remainder;
                if lines == 0 {
                    return;
                }
                // The settings overlay owns the wheel while it's up: scroll
                // its list (wheel-up = toward the top, like the scrollback).
                if let Some(Overlay::Settings(s)) = &mut self.overlay {
                    let visible = settings_visible(self.rows as usize);
                    s.scroll_by(-lines, visible);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                if self.overlay.is_some() {
                    return; // the menu has no scroll; don't scroll the pane under it
                }
                // Unlike click/drag (which already refocus to the pane under
                // the pointer via `on_left_press`), the wheel never changes
                // focus — it targets whichever pane the pointer happens to
                // be over, tmux/iTerm2-style, without stealing focus from
                // whatever the user was typing into.
                let Some(id) = self.pane_under(self.mouse_pos.0, self.mouse_pos.1) else {
                    return;
                };
                let (sh, al, ct) = (
                    self.mods.shift_key(),
                    self.mods.alt_key(),
                    self.mods.control_key(),
                );
                if self.report_mouse(Some(id), |c, r| {
                    MouseEvent::new_point(c, r)
                        .with_scroll(lines)
                        .with_modifiers(sh, al, ct)
                }) {
                    return;
                }
                if self.alt_scroll_active(id) {
                    self.send_alt_scroll_keys(id, lines);
                    return;
                }
                self.scroll_active(id, lines);
            }
            _ => {}
        }
    }

    /// New PTY output arrived on pane `id`: service its host-side requests
    /// (clipboard, notifications, alerts) and repaint if it's visible.
    fn on_pane_output(&mut self, id: u64) {
        // Cleared first, so output parsed from here on sends a new wakeup.
        if let Some(p) = self.pane_by_id_mut(id) {
            p.wake.lower();
            p.compactor.on_change(Instant::now());
        }
        self.service_clipboard(id);
        self.service_notifications(id);
        self.service_alerts(id);
        // A synchronized-output window on this pane suppresses the
        // repaint until it closes (or times out), so a multi-write
        // frame update never paints half-drawn.
        let syncing = self
            .pane_by_id_mut(id)
            .is_some_and(|p| p.grid.lock().sync_output_active());
        // Output on a background tab doesn't repaint; its bar label
        // refreshes with the next frame the active tab causes. A flood
        // repaints at most once per display refresh (`Pacer`).
        if !syncing
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.panes.iter().any(|p| p.id == id))
            && self.pacer.output(Instant::now())
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}

/// The winit application: a set of top-level windows (C13), each a full
/// [`WindowState`], plus what's needed to open new ones at runtime. Events
/// route to the owning window by `WindowId` (window events) or pane id
/// (reader-thread wakeups); the loop exits when the last window closes.
struct App<'a> {
    source: &'a dyn Source,
    /// The launch config, template for every new window.
    config: Config,
    config_path: Option<std::path::PathBuf>,
    /// The `--profile <name>` this instance was launched with, if any; copied
    /// into every [`WindowState`] (see its field doc).
    profile_override: Option<String>,
    proxy: EventLoopProxy<UserEvent>,
    shells: Vec<crate::shells::DetectedShell>,
    windows: Vec<WindowState<'a>>,
}

impl<'a> App<'a> {
    /// Build a [`WindowState`] from the launch config (its OS window comes
    /// later via `ensure_window`). Each window owns its font, clipboard, and
    /// config copy so per-window settings changes stay per-window.
    fn new_window_state(&self) -> Result<WindowState<'a>, String> {
        let font_px = self.config.font_size.unwrap_or(FONT_PX);
        let font_set = font::load_set(
            self.config.font.as_deref(),
            self.config.font_bold.as_deref(),
            self.config.font_italic.as_deref(),
            self.config.font_bold_italic.as_deref(),
            self.config.font_fallback.as_deref(),
        )
        .ok_or("no monospace font found")?;
        let ligatures = self.config.ligatures.unwrap_or(true);
        let ui_font = FontCache::new(font_set.clone(), UI_FONT_PX, ligatures)
            .ok_or("font failed to parse")?;
        let font = FontCache::new(font_set, font_px, ligatures).ok_or("font failed to parse")?;
        let (cell_w, cell_h) = font.cell_size();
        let (bar_w, bar_h) = ui_font.cell_size();
        Ok(WindowState {
            source: self.source,
            told: None,
            armed: false,
            strip: StripState::new().ok_or("the interface font failed to parse")?,
            body: BodyState::new().ok_or("the interface font failed to parse")?,
            popover: Popover::new().ok_or("the interface font failed to parse")?,
            confirm: None,
            config: self.config.clone(),
            config_path: self.config_path.clone(),
            profile_override: self.profile_override.clone(),
            tabs: Vec::new(),
            active: 0,
            proxy: self.proxy.clone(),
            font,
            cell_w: cell_w.max(1),
            cell_h: cell_h.max(1),
            ui_font,
            bar_w: bar_w.max(1),
            bar_h: bar_h.max(1),
            pad: self.config.padding.unwrap_or(DEFAULT_PAD) as usize,
            window: None,
            renderer: None,
            mods: ModifiersState::empty(),
            cols: self.config.cols.unwrap_or(INIT_COLS),
            rows: self.config.rows.unwrap_or(INIT_ROWS),
            theme: self.config.theme,
            clipboard: arboard::Clipboard::new().ok(),
            mouse_pos: (0.0, 0.0),
            scroll_accum: 0.0,
            drag: SelectionDrag::default(),
            hover: None,
            hover_link: None,
            focused: true,
            search_regex: false,
            search_case_sensitive: false,
            broadcast: false,
            copy_mode: None,
            last_grid_click: None,
            click_streak: 0,
            mouse_button_down: None,
            consumed_keys: ConsumedKeys::default(),
            hits: Vec::new(),
            pressed: None,
            last_strip_click: None,
            cursor_blink_on: true,
            last_blink: Instant::now(),
            sync_held: Vec::new(),
            pacer: Pacer::unpaced(),
            searching: None,
            shells: self.shells.clone(),
            overlay: None,
            font_px,
            base_px: font_px,
            ui_px: UI_FONT_PX,
            // Updated to the real value once the OS window exists (see
            // `ensure_window`); 1.0 is the correct assumption until then.
            scale_factor: 1.0,
            closed: false,
            cursor_prev: None,
            trail: None,
            dock_open: false,
            dock_items: Vec::new(),
            access: None,
        })
    }

    /// Drop windows marked closed; exits the loop when no window remains.
    fn reap_and_spawn(&mut self, event_loop: &ActiveEventLoop) {
        for w in &mut self.windows {
            w.tell_focus();
        }
        self.windows.retain(|w| !w.closed);
        if self.windows.is_empty() {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<UserEvent> for App<'_> {
    /// Drive cursor blink and Kitty animations across every window, waking at
    /// the earliest deadline any of them needs (no idle wakeups otherwise).
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let next = self.windows.iter_mut().filter_map(|w| w.tick(now)).min();
        event_loop.set_control_flow(match next {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        for w in &mut self.windows {
            w.ensure_window(event_loop);
        }
        self.reap_and_spawn(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.window_id() == Some(id)) {
            w.on_window_event(event);
        }
        self.reap_and_spawn(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw(id) => {
                if let Some(w) = self.windows.iter_mut().find(|w| w.has_pane(id)) {
                    w.on_pane_output(id);
                }
            }
            UserEvent::Layout(tree) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_tree(tree);
                }
            }
            UserEvent::Focus(key) => {
                if let Some(w) = self.windows.first_mut() {
                    w.focus_from_source(key);
                }
            }
            UserEvent::Zoom(key) => {
                if let Some(w) = self.windows.first_mut() {
                    w.zoom_from_source(key);
                }
            }
            UserEvent::Empty => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_empty();
                }
            }
            UserEvent::Session(session) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_session(session);
                }
            }
            UserEvent::Blocks(blocks) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_blocks(blocks);
                }
            }
            UserEvent::Link(link) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_link(link);
                }
            }
            UserEvent::Take(id, take) => {
                if let Some(w) = self.windows.iter_mut().find(|w| w.has_pane(id)) {
                    w.show_take(id, take);
                }
            }
            UserEvent::Ended(name) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_ended(name);
                }
            }
            UserEvent::Notice(notice) => {
                if let Some(w) = self.windows.first_mut() {
                    w.show_notice(notice);
                }
            }
            UserEvent::ConfigChanged => {
                for w in &mut self.windows {
                    w.reload_config();
                }
            }
            UserEvent::Accessibility(event) => {
                if let Some(w) = self
                    .windows
                    .iter_mut()
                    .find(|w| w.window_id() == Some(event.window_id))
                {
                    w.on_accessibility_event(event.window_event);
                }
            }
        }
        self.reap_and_spawn(event_loop);
    }
}

/// A left-button drag selection. A press arms it at `anchor` (absolute
/// `(col, abs_row)`, so it survives scrolling mid-drag); while live, pointer
/// moves and edge auto-scroll extend the selection from it. Releasing the
/// button or copying with Ctrl+C ends it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectionDrag {
    anchor: Option<(usize, usize)>,
    live: bool,
}

impl SelectionDrag {
    fn start(&mut self, anchor: Option<(usize, usize)>) {
        *self = Self { anchor, live: true };
    }

    fn is_live(&self) -> bool {
        self.live
    }

    /// The anchor to extend from; `None` once the drag has ended.
    fn live_anchor(&self) -> Option<(usize, usize)> {
        self.anchor.filter(|_| self.live)
    }

    /// The button went up. Returns whether a drag was live (copy-on-select).
    fn release(&mut self) -> bool {
        std::mem::replace(&mut self.live, false)
    }

    /// End the drag without a release, so nothing can rebuild the selection.
    fn cancel(&mut self) {
        *self = Self::default();
    }
}

/// A key release and the focused pane's keyboard reporting modes.
struct KeyRelease<'a> {
    physical: PhysicalKey,
    logical: &'a winit::keyboard::Key,
    mods: ModifiersState,
    app_cursor: bool,
    kitty_flags: u8,
    win32_input: bool,
}

/// The bytes a key release sends the child, if any. Releases are terminal
/// input only under win32-input-mode (?9001) or Kitty keyboard flag 2 (report
/// event types). None when a window shortcut consumed the press (its release
/// would arrive unpaired) or a UI layer (overlay, search, copy mode) owns the
/// keyboard. The swallowed key is cleared first, whatever owns the keys.
fn release_input(
    consumed: &mut ConsumedKeys,
    ui_owns_keys: bool,
    r: &KeyRelease,
) -> Option<Vec<u8>> {
    if consumed.release(r.physical) || ui_owns_keys {
        return None;
    }
    if r.win32_input {
        return super::input::encode_win32(r.physical, r.logical, r.mods, false);
    }
    if r.kitty_flags & 2 == 0 {
        return None;
    }
    super::input::encode_full(
        r.logical,
        r.mods,
        r.app_cursor,
        r.kitty_flags,
        super::input::KeyPhase::Release,
        kitty_alternate(r.logical, r.mods),
        None,
    )
}

/// Physical keys held down whose press a window shortcut consumed. Each
/// stays until its own release, so overlapping shortcuts are independent.
#[derive(Debug, Default)]
struct ConsumedKeys(Vec<PhysicalKey>);

impl ConsumedKeys {
    /// A keydown of `key`. Returns whether it repeats a consumed press. A
    /// fresh press of a key still listed means its release was lost (focus
    /// change), so the stale entry is dropped and the press is new.
    fn press(&mut self, key: PhysicalKey, repeat: bool) -> bool {
        if !self.0.contains(&key) {
            return false;
        }
        if repeat {
            return true;
        }
        self.release(key);
        false
    }

    fn consume(&mut self, key: PhysicalKey) {
        if !self.0.contains(&key) {
            self.0.push(key);
        }
    }

    /// A key-up of `key`. Returns whether its press was consumed.
    fn release(&mut self, key: PhysicalKey) -> bool {
        let Some(i) = self.0.iter().position(|k| *k == key) else {
            return false;
        };
        self.0.swap_remove(i);
        true
    }
}

/// Whether a key bound to `action` skips the open overlay's own key handling
/// and runs as an action: only the settings toggle (`Ctrl+,` by default),
/// which closes the page it opened, and setup as a skip.
fn overlay_passes_through(action: Option<Action>) -> bool {
    action == Some(Action::OpenSettings)
}

/// Route a keydown the keymap resolved to `action` (None: unbound). `exec`
/// runs an action and returns whether it consumed the key. Returns whether
/// the key is consumed; if not, it goes on to the child. Who owns a held key
/// is settled by its first press, so the child always sees whole
/// down/repeat/up runs:
/// - A repeat of a consumed press never reaches the child: a held Ctrl+C
///   that copied must not turn into ^C once the selection is gone. Other
///   actions still repeat (zoom, scrolling); a held copy does not re-copy.
/// - A repeat of a press the child got stays the child's, even if the chord
///   or the selection changed meanwhile, or its release would be swallowed.
fn route_key_down(
    consumed: &mut ConsumedKeys,
    key: PhysicalKey,
    repeat: bool,
    action: Option<Action>,
    exec: impl FnOnce(Action) -> bool,
) -> bool {
    let held = consumed.press(key, repeat);
    if repeat && !held {
        return false;
    }
    let taken = match action {
        Some(Action::CopyOrSend) if held => true,
        Some(action) => exec(action),
        None => held,
    };
    if taken {
        consumed.consume(key);
    }
    taken
}

/// What copying the selection achieved.
#[derive(Debug, PartialEq, Eq)]
enum Copied {
    NothingSelected,
    Done,
    /// There was a selection but the clipboard is missing or refused it.
    Failed(String),
}

impl Copied {
    /// Whether a copy-or-send key press is used up (else it goes to the child).
    fn consumes_key(&self) -> bool {
        !matches!(self, Copied::NothingSelected)
    }

    /// Whether the selection should go: only once it is safely copied.
    fn clears_selection(&self) -> bool {
        matches!(self, Copied::Done)
    }
}

/// Copy `text` (the selection, if any) with `write` (the clipboard, if any).
fn copy_text<W>(text: Option<String>, write: Option<W>) -> Copied
where
    W: FnOnce(String) -> Result<(), String>,
{
    let Some(text) = text else {
        return Copied::NothingSelected;
    };
    let Some(write) = write else {
        return Copied::Failed("no system clipboard".to_string());
    };
    match write(text) {
        Ok(()) => Copied::Done,
        Err(e) => Copied::Failed(e),
    }
}

/// Encode clipboard `text` for the child: normalize line endings to CR, and
/// when `bracketed` wrap it in `ESC[200~`/`ESC[201~`, stripping any embedded
/// end marker first so the payload can't close the bracket early (a
/// paste-injection guard). Without bracketed-paste support to tell the child
/// "this is literal data, don't interpret it", C0/DEL control bytes are
/// stripped instead — otherwise a paste containing a hidden `ESC` (trivial to
/// embed in text copied from a web page, and invisible in the source) could
/// smuggle an arbitrary escape sequence into the child's input, or a hidden
/// bell/backspace could do something surprising at the prompt. `\t` and the
/// `\r` newlines were just normalized to are kept — running each pasted line
/// as its own command is the expected, visible behavior for an app with no
/// bracketed-paste support, not something this needs to guard against.
fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // Strip until no marker remains: a single pass is bypassable, since
        // removing one occurrence can splice a new one together (e.g.
        // `ESC[2` + `ESC[201~` + `01~` re-forms the end marker).
        let mut text = text;
        while text.contains("\x1b[201~") {
            text = text.replace("\x1b[201~", "");
        }
        let mut out = Vec::with_capacity(text.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.bytes()
            .filter(|&b| b == b'\t' || b == b'\r' || !b.is_ascii_control())
            .collect()
    }
}

/// The Kitty flag-4 "alternate key": the shifted form of a text key while
/// Shift is held (winit's logical key already has Shift applied), used as
/// the `code:shifted` sub-parameter. `None` for named keys or unshifted
/// presses.
fn kitty_alternate(key: &winit::keyboard::Key, mods: ModifiersState) -> Option<char> {
    if !mods.shift_key() {
        return None;
    }
    match key {
        winit::keyboard::Key::Character(s) => s.chars().next(),
        _ => None,
    }
}

/// The DECCKM-aware arrow-key byte sequence that moves a readline cursor by
/// `(dx, dy)` cells: vertical first (multiline editing), then horizontal,
/// capped so a pathological distance can't flood the child.
fn arrow_presses(dx: isize, dy: isize, app_cursor: bool) -> Vec<u8> {
    let prefix: &[u8] = if app_cursor { b"\x1bO" } else { b"\x1b[" };
    let mut out = Vec::new();
    let mut push = |n: isize, pos: u8, neg: u8| {
        let final_byte = if n >= 0 { pos } else { neg };
        for _ in 0..n.unsigned_abs().min(400) {
            out.extend_from_slice(prefix);
            out.push(final_byte);
        }
    };
    push(dy, b'B', b'A'); // Down / Up
    push(dx, b'C', b'D'); // Right / Left
    out
}

/// The pointer's position in pixels relative to a pane's pixel origin,
/// clamped into the pane (`0..size-1` per axis) so a pointer over the chrome
/// or a divider never reports an out-of-pane position. 0-based; the SGR
/// encoder adds the protocol's 1-basing.
/// Where committed IME text should go, given which UI layer currently owns
/// keyboard input. Pulled out of the `Ime::Commit` handler as a pure
/// decision so the routing (find bar > pane, overlay/copy-mode swallow it)
/// is unit-testable without a live `WindowState`.
#[derive(Debug, PartialEq, Eq)]
enum ImeCommitTarget {
    /// The find bar is open: append to its query string.
    SearchBar,
    /// Nothing else owns the keyboard: forward to the focused pane's child.
    Pane,
    /// An overlay (settings/shell menu) or copy mode owns the keyboard and
    /// has no free-text field of its own — the committed text is dropped
    /// rather than leaking into the background shell underneath it.
    Dropped,
}

fn ime_commit_target(searching: bool, overlay: bool, copy_mode: bool) -> ImeCommitTarget {
    if searching {
        ImeCommitTarget::SearchBar
    } else if overlay || copy_mode {
        ImeCommitTarget::Dropped
    } else {
        ImeCommitTarget::Pane
    }
}

/// A Ctrl+=/Ctrl+-/Ctrl+0 font zoom.
#[derive(Clone, Copy, Debug)]
enum Zoom {
    In,
    Out,
    Reset,
}

/// The terminal font size, in physical px, after `zoom` from `current`.
/// `base` is the configured size at the current DPI: a reset returns to it
/// exactly. A step stays within the settings range scaled to the monitor
/// (`FONT_MIN..=FONT_MAX` logical px), widened to include `base`, so a
/// DPI-scaled size outside the logical range is still reachable.
fn zoom_px(zoom: Zoom, current: f32, base: f32, scale: f64) -> f32 {
    let step = match zoom {
        Zoom::Reset => return base,
        Zoom::In => FONT_ZOOM_STEP,
        Zoom::Out => -FONT_ZOOM_STEP,
    };
    let scale = scale as f32;
    let lo = (super::settings::FONT_MIN * scale).min(base);
    let hi = (super::settings::FONT_MAX * scale).max(base);
    (current + step).clamp(lo, hi)
}

/// `logical` px at DPI `scale` (1.0 = 100%), in the physical px fonts use.
fn physical_px(logical: f32, scale: f64) -> f32 {
    ((logical as f64) * scale) as f32
}

/// Load the configured font set and build a glyph cache at `px`, `None` when
/// no usable font is found.
fn load_font(config: &Config, px: f32, ligatures: bool) -> Option<FontCache> {
    let set = font::load_set(
        config.font.as_deref(),
        config.font_bold.as_deref(),
        config.font_italic.as_deref(),
        config.font_bold_italic.as_deref(),
        config.font_fallback.as_deref(),
    )?;
    FontCache::new(set, px, ligatures)
}

/// The IME candidate-popup origin, in physical pixels, for a cursor at
/// pane-local `(col, row)` within the pane whose rect starts at
/// `pane_origin` (in cells) — pulled out of `update_ime_area` so the split
/// offset math is unit-testable without a live `winit::Window`.
fn ime_cursor_area_origin(
    pad: usize,
    oy: usize,
    cell_w: usize,
    cell_h: usize,
    pane_origin: (usize, usize),
    cursor: (usize, usize),
) -> (f64, f64) {
    let (base_col, base_row) = pane_origin;
    let (col, row) = cursor;
    let x = (pad + (base_col + col) * cell_w) as f64;
    // The cell's top, below the grid's own vertical offset (padding band +
    // the chrome bar).
    let y = (oy + (base_row + row) * cell_h) as f64;
    (x, y)
}

/// Where a dragged tab should sit for pointer column `col`: the position of
/// the nearest span (moving out from the dragged tab's own) whose *midpoint*
/// the pointer has crossed, returned as the tab index currently holding that
/// position. Midpoint-based so unequal adaptive widths can't oscillate — a
/// swap only fires once the pointer is past the point where swapping back
/// would immediately retrigger. `from` when nothing was crossed.
/// Losing focus cancels the pressed look: the release that would end it may
/// land in another window and never reach this one.
fn pressed_after_focus(pressed: Option<Hit>, focused: bool) -> Option<Hit> {
    if focused { pressed } else { None }
}

/// Whether a drag-selection past a pane's `[top_px, bottom_px]` edge should
/// auto-scroll: `Some(true)` above the top margin, `Some(false)` below the
/// bottom margin, `None` within the pane (plus its margin) so the caller
/// does nothing. Pulled out of `drag_edge_autoscroll` for unit testing
/// without a live `WindowState`.
fn drag_scroll_direction(y: f64, top_px: f64, bottom_px: f64, margin: f64) -> Option<bool> {
    if y < top_px - margin {
        Some(true)
    } else if y > bottom_px + margin {
        Some(false)
    } else {
        None
    }
}

fn pane_pixel_point(pos: (f64, f64), origin: (usize, usize), size: (usize, usize)) -> MousePoint {
    let clamp = |v: f64, o: usize, s: usize| {
        ((v.max(0.0) as usize).saturating_sub(o)).min(s.saturating_sub(1))
    };
    MousePoint {
        col: clamp(pos.0, origin.0, size.0),
        row: clamp(pos.1, origin.1, size.1),
    }
}

/// Quote `path` for pasting into a shell command line (a drag-and-dropped
/// file). Unix: single quotes, with embedded `'` rewritten to `'\''` — safe
/// for every byte a path can contain. Windows: double quotes when the path
/// needs them (paths can't contain `"` there, so no escaping is required).
fn shell_quote(path: &str) -> String {
    #[cfg(windows)]
    {
        if path.is_empty() || path.contains(' ') {
            return format!("\"{path}\"");
        }
        path.to_string()
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// Open `url` with the OS default handler when its scheme is one we allow
/// (see [`is_openable_url`]). Ctrl+click on an OSC 8 hyperlink routes here.
/// Returns whether a handler was launched.
fn open_url(url: &str) -> bool {
    if !is_openable_url(url) {
        return false;
    }
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();
    spawned.is_ok()
}

/// Whether `url`'s scheme is one we're willing to hand to the OS opener.
/// Restricting it keeps arbitrary or custom-scheme URIs from terminal output
/// from reaching the shell's URL handler. The scheme match is case-insensitive.
fn is_openable_url(url: &str) -> bool {
    const ALLOWED: [&str; 5] = ["http://", "https://", "ftp://", "file://", "mailto:"];
    let b = url.as_bytes();
    ALLOWED
        .iter()
        .any(|p| b.len() > p.len() && b[..p.len()].eq_ignore_ascii_case(p.as_bytes()))
}

/// Map an OSC 22-requested pointer shape — a CSS `cursor` keyword, the
/// convention Kitty and others use — to a winit cursor icon. `None` for a
/// name we don't recognize (the child's own concern; we just fall back to
/// the default arrow rather than guessing).
fn parse_cursor_icon(name: &str) -> Option<CursorIcon> {
    Some(match name {
        "default" => CursorIcon::Default,
        "pointer" => CursorIcon::Pointer,
        "text" => CursorIcon::Text,
        "wait" => CursorIcon::Wait,
        "progress" => CursorIcon::Progress,
        "crosshair" => CursorIcon::Crosshair,
        "move" => CursorIcon::Move,
        "not-allowed" => CursorIcon::NotAllowed,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "help" => CursorIcon::Help,
        "cell" => CursorIcon::Cell,
        "copy" => CursorIcon::Copy,
        "alias" => CursorIcon::Alias,
        "context-menu" => CursorIcon::ContextMenu,
        "vertical-text" => CursorIcon::VerticalText,
        "no-drop" => CursorIcon::NoDrop,
        "all-scroll" => CursorIcon::AllScroll,
        "zoom-in" => CursorIcon::ZoomIn,
        "zoom-out" => CursorIcon::ZoomOut,
        "col-resize" => CursorIcon::ColResize,
        "row-resize" => CursorIcon::RowResize,
        _ => return None,
    })
}

/// Raise an OS desktop notification (OSC 9/777), per-platform with no extra
/// crates (mirroring [`open_url`]). The untrusted title/body are passed as
/// environment variables so they can't inject into the spawned PowerShell /
/// AppleScript / `notify-send` command.
fn notify(title: &str, body: &str) {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    let mut cmd = {
        const PS: &str = "Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.BalloonTipTitle = $env:RT_TITLE; $n.BalloonTipText = $env:RT_BODY; $n.Visible = $true; $n.ShowBalloonTip(6000); Start-Sleep -Seconds 7; $n.Dispose()";
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            PS,
        ]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("osascript");
        c.args(["-e", "display notification (system attribute \"RT_BODY\") with title (system attribute \"RT_TITLE\")"]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("notify-send");
        c.arg(title).arg(body);
        c
    };
    let _ = cmd.env("RT_TITLE", title).env("RT_BODY", body).spawn();
}

/// Map a winit physical key to a toolkit-free [`crate::keymap::Key`] for binding
/// lookup, or `None` for keys that can't be bound (they fall through to native
/// encoding). Letters and digits are position-based, matching most terminals.
fn chord_key(code: KeyCode) -> Option<crate::keymap::Key> {
    use crate::keymap::Key;
    Some(match code {
        KeyCode::Tab => Key::Tab,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::Space => Key::Space,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Enter => Key::Enter,
        KeyCode::Insert => Key::Insert,
        KeyCode::Delete => Key::Delete,
        KeyCode::Escape => Key::Escape,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::Comma => Key::Char(','),
        KeyCode::Period => Key::Char('.'),
        // The font-zoom chords (Ctrl+=, Ctrl+Shift+=, Ctrl+-) are bound to
        // these; without them those presses fell through to the child.
        KeyCode::Equal => Key::Char('='),
        KeyCode::Minus => Key::Char('-'),
        KeyCode::KeyA => Key::Char('a'),
        KeyCode::KeyB => Key::Char('b'),
        KeyCode::KeyC => Key::Char('c'),
        KeyCode::KeyD => Key::Char('d'),
        KeyCode::KeyE => Key::Char('e'),
        KeyCode::KeyF => Key::Char('f'),
        KeyCode::KeyG => Key::Char('g'),
        KeyCode::KeyH => Key::Char('h'),
        KeyCode::KeyI => Key::Char('i'),
        KeyCode::KeyJ => Key::Char('j'),
        KeyCode::KeyK => Key::Char('k'),
        KeyCode::KeyL => Key::Char('l'),
        KeyCode::KeyM => Key::Char('m'),
        KeyCode::KeyN => Key::Char('n'),
        KeyCode::KeyO => Key::Char('o'),
        KeyCode::KeyP => Key::Char('p'),
        KeyCode::KeyQ => Key::Char('q'),
        KeyCode::KeyR => Key::Char('r'),
        KeyCode::KeyS => Key::Char('s'),
        KeyCode::KeyT => Key::Char('t'),
        KeyCode::KeyU => Key::Char('u'),
        KeyCode::KeyV => Key::Char('v'),
        KeyCode::KeyW => Key::Char('w'),
        KeyCode::KeyX => Key::Char('x'),
        KeyCode::KeyY => Key::Char('y'),
        KeyCode::KeyZ => Key::Char('z'),
        KeyCode::Digit0 => Key::Char('0'),
        KeyCode::Digit1 => Key::Char('1'),
        KeyCode::Digit2 => Key::Char('2'),
        KeyCode::Digit3 => Key::Char('3'),
        KeyCode::Digit4 => Key::Char('4'),
        KeyCode::Digit5 => Key::Char('5'),
        KeyCode::Digit6 => Key::Char('6'),
        KeyCode::Digit7 => Key::Char('7'),
        KeyCode::Digit8 => Key::Char('8'),
        KeyCode::Digit9 => Key::Char('9'),
        _ => return None,
    })
}

/// Build the OSC 52 clipboard query reply for the child: `OSC 52 ; c ; <b64>`
/// (BEL-terminated), answering a `OSC 52 ; … ; ?` query from the system
/// clipboard.
fn osc52_reply(sel: char, text: &str) -> Vec<u8> {
    let mut out = Vec::from(&b"\x1b]52;"[..]);
    out.push(sel as u8);
    out.push(b';');
    out.extend_from_slice(crate::core::base64_encode(text.as_bytes()).as_bytes());
    out.push(0x07);
    out
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::shell_quote;
    use super::{
        ConsumedKeys, Copied, Hit, ImeCommitTarget, KeyRelease, MenuKind, SETTINGS_CARDS_TOP,
        SETTINGS_FOOTER_H, SETTINGS_ROW_H, SETTINGS_SEARCH_W, SETTINGS_SIDEBAR_W, SelectionDrag,
        SettingsHit, UI_FONT_PX, Widget, Zoom, accumulate_scroll_lines, arrow_presses, copy_text,
        drag_scroll_direction, encode_paste, fmt_runtime, ime_commit_target,
        ime_cursor_area_origin, is_openable_url, menu_items, osc52_reply, pane_pixel_point,
        physical_px, pressed_after_focus, put_text, read_git_branch, release_input, route_key_down,
        settings_hit, settings_value_end, settings_visible, widget_text, zoom_px,
    };
    use crate::core::Cell;

    #[test]
    fn the_font_zoom_keys_reach_their_default_bindings() {
        use super::chord_key;
        use crate::keymap::{Action, Chord, Keymap};
        use winit::keyboard::KeyCode;
        let km = Keymap::default();
        let action = |shift: bool, code: KeyCode| {
            let key = chord_key(code).expect("the key maps to a chord key");
            km.action(Chord::new(true, shift, false, key))
        };
        assert_eq!(action(false, KeyCode::Equal), Some(Action::FontSizeUp));
        assert_eq!(action(true, KeyCode::Equal), Some(Action::FontSizeUp));
        assert_eq!(action(false, KeyCode::Minus), Some(Action::FontSizeDown));
        assert_eq!(action(false, KeyCode::Digit0), Some(Action::FontSizeReset));
    }

    #[test]
    fn a_synchronized_update_repaints_the_last_frame_until_it_closes() {
        use super::frame_to_paint;
        use crate::core::{AnsiParser, Grid};
        let mut g = Grid::new(20, 4);
        let mut p = AnsiParser::new();
        p.advance(&mut g, b"ab");
        let shown = frame_to_paint(&mut g, None);
        assert_eq!(shown.cursor, (2, 0));
        p.advance(&mut g, b"\x1b[?2026hcd");
        let held = frame_to_paint(&mut g, Some(shown));
        assert_eq!(held.cursor, (2, 0), "open: the last frame again");
        let fresh = frame_to_paint(&mut g, None);
        assert_eq!(fresh.cursor, (4, 0), "nothing shown yet: the grid");
        p.advance(&mut g, b"\x1b[?2026l");
        assert_eq!(frame_to_paint(&mut g, Some(held)).cursor, (4, 0), "closed");
    }

    #[test]
    fn a_resize_during_a_synchronized_update_paints_the_grid() {
        use super::frame_to_paint;
        use crate::core::{AnsiParser, Grid};
        let mut g = Grid::new(20, 4);
        let mut p = AnsiParser::new();
        let shown = frame_to_paint(&mut g, None);
        p.advance(&mut g, b"\x1b[?2026h");
        g.resize(30, 4);
        assert_eq!(frame_to_paint(&mut g, Some(shown)).cols, 30);
    }

    #[test]
    fn a_pane_whose_update_ends_repaints_while_another_still_holds() {
        use super::sync_step;
        use std::time::{Duration, Instant};
        // A opened at 0 ms, B at 400 ms: at 800 ms A has timed out and B
        // holds until 1200 ms, with no input, blink or animation to help.
        let t0 = Instant::now();
        let (a, b) = (1, 2);
        let (held, next, released) = sync_step(&[], &[(a, t0 + Duration::from_millis(800))]);
        assert!(!released);
        assert_eq!(next, Some(t0 + Duration::from_millis(800)));
        let both = [
            (a, t0 + Duration::from_millis(800)),
            (b, t0 + Duration::from_millis(1200)),
        ];
        let (held, _, released) = sync_step(&held, &both);
        assert_eq!((held.as_slice(), released), ([a, b].as_slice(), false));
        let (held, next, released) = sync_step(&held, &[(b, t0 + Duration::from_millis(1200))]);
        assert!(released, "A's end repaints now, not at B's");
        assert_eq!(
            next,
            Some(t0 + Duration::from_millis(1200)),
            "and B still wakes later"
        );
        let (held, next, released) = sync_step(&held, &[]);
        assert!(released && held.is_empty() && next.is_none());
        assert!(!sync_step(&held, &[]).2, "nothing held: no repaint");
    }

    #[test]
    fn panes_report_their_open_updates_to_the_sync_tick() {
        use super::sync_step;
        use crate::core::{AnsiParser, Grid};
        let mut grids = [Grid::new(10, 2), Grid::new(10, 2)];
        let mut p = AnsiParser::new();
        for g in &mut grids {
            p.advance(g, b"\x1b[?2026h");
        }
        let open = |grids: &mut [Grid; 2]| -> Vec<_> {
            (0u64..)
                .zip(grids.iter_mut())
                .filter_map(|(id, g)| Some((id, g.sync_output_until()?)))
                .collect()
        };
        let (held, _, _) = sync_step(&[], &open(&mut grids));
        assert_eq!(held, [0, 1]);
        p.advance(&mut grids[0], b"\x1b[?2026l");
        let (held, next, released) = sync_step(&held, &open(&mut grids));
        assert_eq!((held.as_slice(), released), ([1].as_slice(), true));
        assert!(next.is_some(), "pane 1 still holds");
    }

    #[test]
    fn panic_text_reads_str_and_string_payloads() {
        use super::panic_text;
        let caught = |f: fn()| std::panic::catch_unwind(f).expect_err("panics");
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let fixed = caught(|| panic!("Library libxkbcommon-x11.so could not be loaded."));
        let formatted = caught(|| panic!("{} could not be loaded.", "libxkbcommon.so"));
        let other = caught(|| std::panic::panic_any(7_u8));
        std::panic::set_hook(hook);
        assert_eq!(
            panic_text(fixed.as_ref()),
            "Library libxkbcommon-x11.so could not be loaded."
        );
        assert_eq!(
            panic_text(formatted.as_ref()),
            "libxkbcommon.so could not be loaded."
        );
        assert_eq!(panic_text(other.as_ref()), "the windowing library panicked");
    }

    #[test]
    fn settings_visible_scales_with_the_window_and_never_underflows() {
        // 24 grid rows: 1 top pad + 2 title/rule + 2 footer leaves 19, at
        // 3 rows per card = 6.
        assert_eq!(settings_visible(24), 6);
        // A taller window fits another card.
        assert_eq!(settings_visible(26), 7);
        // A tiny window still returns 0 rather than wrapping.
        assert_eq!(settings_visible(0), 0);
        assert_eq!(settings_visible(SETTINGS_CARDS_TOP + SETTINGS_FOOTER_H), 0);
    }

    #[test]
    fn the_settings_toggle_reaches_an_open_overlay() {
        // Ctrl+, (the default chord) must close the page and skip setup, so
        // it runs as an action even while the overlay owns the keyboard.
        use crate::keymap::{Action, Chord, Key};
        let ctrl_comma = Chord::new(true, false, false, Key::Char(','));
        let action = crate::config::Config::default().keys.action(ctrl_comma);
        assert!(super::overlay_passes_through(action));
        for other in [None, Some(Action::OpenSetup), Some(Action::CopyOrSend)] {
            assert!(!super::overlay_passes_through(other), "{other:?}");
        }
    }

    #[test]
    fn settings_hit_reaches_every_setup_step_in_the_sidebar() {
        let (cols, rows) = (100, 30);
        let fourth = super::SETTINGS_TOP + 3 * 2;
        assert_eq!(
            settings_hit(2, fourth, cols, rows, 0, 1, 3),
            SettingsHit::None
        );
        assert_eq!(
            settings_hit(2, fourth, cols, rows, 0, 1, 4),
            SettingsHit::Category(3)
        );
    }

    #[test]
    fn settings_value_end_caps_the_card_column_on_a_wide_window() {
        // Narrow window: the right pad governs.
        assert_eq!(settings_value_end(80), 77);
        // Wide window: the card-width cap governs, so the value stays near
        // its label instead of drifting to the window edge.
        assert_eq!(settings_value_end(240), SETTINGS_SIDEBAR_W + 2 + 80);
    }

    #[test]
    fn settings_hit_maps_sidebar_cards_search_and_dead_space() {
        let (cols, rows) = (100, 24); // 6 visible cards (see above)
        // The top-pad row is dead everywhere.
        assert_eq!(settings_hit(2, 0, cols, rows, 0, 7, 3), SettingsHit::None);
        // Sidebar: categories every other row, with the gap row belonging to
        // the entry above it — the whole strip is clickable.
        let t = super::SETTINGS_TOP;
        assert_eq!(
            settings_hit(2, t, cols, rows, 0, 7, 3),
            SettingsHit::Category(0)
        );
        assert_eq!(
            settings_hit(2, t + 1, cols, rows, 0, 7, 3),
            SettingsHit::Category(0)
        );
        assert_eq!(
            settings_hit(0, t + 2, cols, rows, 0, 7, 3),
            SettingsHit::Category(1)
        );
        assert_eq!(
            settings_hit(2, t + 4, cols, rows, 0, 7, 3),
            SettingsHit::Category(2)
        );
        // Below the last category the strip is dead.
        assert_eq!(
            settings_hit(2, t + 6, cols, rows, 0, 7, 3),
            SettingsHit::None
        );
        // The title row's right end is the search affordance; its left is dead.
        let value_end = settings_value_end(cols);
        assert_eq!(
            settings_hit(value_end - 1, t, cols, rows, 0, 7, 3),
            SettingsHit::Search
        );
        assert_eq!(
            settings_hit(value_end - SETTINGS_SEARCH_W, t, cols, rows, 0, 7, 3),
            SettingsHit::Search,
        );
        assert_eq!(
            settings_hit(SETTINGS_SIDEBAR_W + 4, t, cols, rows, 0, 7, 3),
            SettingsHit::None
        );
        // Card area starts below the title + rule; the rule row and the
        // spacer between cards are dead.
        let x = SETTINGS_SIDEBAR_W + 4;
        assert_eq!(
            settings_hit(x, t + 1, cols, rows, 0, 7, 3),
            SettingsHit::None
        );
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP, cols, rows, 0, 7, 3),
            SettingsHit::Row(0)
        );
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP + 1, cols, rows, 0, 7, 3),
            SettingsHit::Row(0)
        );
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP + 2, cols, rows, 0, 7, 3),
            SettingsHit::None
        );
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP + SETTINGS_ROW_H, cols, rows, 0, 7, 3),
            SettingsHit::Row(1),
        );
        // Scroll offsets the list index the same card resolves to.
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP, cols, rows, 3, 7, 3),
            SettingsHit::Row(3)
        );
        // Past the end of a short list: dead.
        assert_eq!(
            settings_hit(x, SETTINGS_CARDS_TOP + SETTINGS_ROW_H, cols, rows, 0, 1, 3),
            SettingsHit::None,
        );
        // Footer rows never hit.
        assert_eq!(
            settings_hit(x, rows - 1, cols, rows, 0, 7, 3),
            SettingsHit::None
        );
    }

    #[test]
    fn widget_text_grows_cycle_arrows_only_when_selected() {
        let choice = Widget::Choice("dracula".into());
        assert_eq!(widget_text(&choice, false), "dracula");
        assert_eq!(widget_text(&choice, true), "< dracula >");
        // Toggles render the same switch either way — selection flips, it
        // doesn't cycle.
        let on = Widget::Toggle(true);
        assert_eq!(widget_text(&on, false), widget_text(&on, true));
        assert!(widget_text(&on, true).ends_with("on"));
        assert!(widget_text(&Widget::Toggle(false), true).ends_with("off"));
    }

    #[test]
    fn scroll_accumulator_carries_sub_line_deltas_instead_of_dropping_them() {
        // Five events at 0.3 lines each used to round to 0 individually and
        // scroll nothing; accumulated, they must eventually emit whole lines
        // and the running remainder must never lose or invent a line.
        let mut accum = 0.0;
        let mut total_lines = 0isize;
        for _ in 0..5 {
            let (lines, next) = accumulate_scroll_lines(accum, 0.3);
            accum = next;
            total_lines += lines;
        }
        assert_eq!(
            total_lines, 1,
            "5 * 0.3 == 1.5, so exactly one whole line has fired so far"
        );
        assert!(
            (accum - 0.5).abs() < 1e-9,
            "the remaining 0.5 must still be carried: {accum}"
        );
    }

    #[test]
    fn scroll_accumulator_handles_a_single_large_delta_and_negative_direction() {
        assert_eq!(accumulate_scroll_lines(0.0, 3.0), (3, 0.0));
        let (lines, remainder) = accumulate_scroll_lines(0.0, -0.7);
        assert_eq!(lines, 0);
        assert!((remainder + 0.7).abs() < 1e-9);
        let (lines, remainder) = accumulate_scroll_lines(remainder, -0.7);
        assert_eq!(lines, -1);
        assert!((remainder + 0.4).abs() < 1e-9);
    }

    type Write = fn(String) -> Result<(), String>;

    use crate::keymap::Action as KeyAction;
    use winit::keyboard::{Key as WKey, KeyCode, ModifiersState, PhysicalKey};

    const C: PhysicalKey = PhysicalKey::Code(KeyCode::KeyC);
    const V: PhysicalKey = PhysicalKey::Code(KeyCode::KeyV);

    /// Bytes a release of `key` sends under win32-input-mode or Kitty
    /// event types (`win32` picks which).
    fn up(consumed: &mut ConsumedKeys, key: PhysicalKey, win32: bool) -> Option<Vec<u8>> {
        let ch = if key == C { "c" } else { "v" };
        let logical = WKey::Character(ch.into());
        let release = KeyRelease {
            physical: key,
            logical: &logical,
            mods: ModifiersState::CONTROL,
            app_cursor: false,
            kitty_flags: if win32 { 0 } else { 2 | 1 },
            win32_input: win32,
        };
        release_input(consumed, false, &release)
    }

    /// A keydown of plain Ctrl+C bound to CopyOrSend, with `selected`
    /// standing in for the pane selection. Returns whether it was consumed
    /// and whether the copy ran.
    fn ctrl_c_down(consumed: &mut ConsumedKeys, repeat: bool, selected: &mut bool) -> (bool, bool) {
        let mut ran = false;
        let taken = route_key_down(consumed, C, repeat, Some(KeyAction::CopyOrSend), |_| {
            ran = true;
            std::mem::replace(selected, false)
        });
        (taken, ran)
    }

    #[test]
    fn a_held_copy_never_interrupts_and_stays_paired() {
        for win32 in [true, false] {
            let mut consumed = ConsumedKeys::default();
            let mut selected = true;
            // Selected Ctrl+C: copies, consumed.
            assert_eq!(
                ctrl_c_down(&mut consumed, false, &mut selected),
                (true, true)
            );
            // Held: repeats are swallowed and do not re-run the copy, so
            // they never become ^C now the selection is gone.
            assert_eq!(
                ctrl_c_down(&mut consumed, true, &mut selected),
                (true, false)
            );
            assert_eq!(
                ctrl_c_down(&mut consumed, true, &mut selected),
                (true, false)
            );
            // Its release is swallowed too.
            assert_eq!(up(&mut consumed, C, win32), None, "win32={win32}");
            // A later, unselected Ctrl+C reaches the child down and up.
            assert_eq!(
                ctrl_c_down(&mut consumed, false, &mut selected),
                (false, true)
            );
            // Its repeat is the child's too, without re-running the copy.
            assert_eq!(
                ctrl_c_down(&mut consumed, true, &mut selected),
                (false, false)
            );
            assert!(up(&mut consumed, C, win32).is_some(), "win32={win32}");
        }
    }

    #[test]
    fn a_key_the_child_got_stays_the_childs_while_held() {
        for win32 in [true, false] {
            let mut consumed = ConsumedKeys::default();
            let mut selected = false;
            // Unselected Ctrl+C goes to the child.
            assert_eq!(
                ctrl_c_down(&mut consumed, false, &mut selected),
                (false, true)
            );
            // Text gets selected while C is held: repeats stay the child's
            // and do not copy, so its release is not swallowed either.
            selected = true;
            assert_eq!(
                ctrl_c_down(&mut consumed, true, &mut selected),
                (false, false)
            );
            assert!(selected);
            assert!(up(&mut consumed, C, win32).is_some(), "win32={win32}");
            // The next fresh press copies as usual.
            assert_eq!(
                ctrl_c_down(&mut consumed, false, &mut selected),
                (true, true)
            );
        }
        // Same for a chord that appears mid-hold (C held, then Ctrl).
        let mut consumed = ConsumedKeys::default();
        assert!(!route_key_down(&mut consumed, V, false, None, |_| true));
        assert!(!route_key_down(
            &mut consumed,
            V,
            true,
            Some(KeyAction::Paste),
            |_| { panic!("a repeat of the child's key must not run a shortcut") }
        ));
        assert!(up(&mut consumed, V, true).is_some());
    }

    #[test]
    fn overlapping_consumed_shortcuts_release_in_either_order() {
        for (first, second) in [(C, V), (V, C)] {
            let mut consumed = ConsumedKeys::default();
            let mut selected = true;
            assert!(ctrl_c_down(&mut consumed, false, &mut selected).0);
            assert!(route_key_down(
                &mut consumed,
                V,
                false,
                Some(KeyAction::Paste),
                |_| true
            ));
            assert_eq!(up(&mut consumed, first, true), None);
            assert_eq!(up(&mut consumed, second, true), None);
            // Both cleared: the next releases are ordinary input again.
            assert!(up(&mut consumed, C, true).is_some());
            assert!(up(&mut consumed, V, true).is_some());
        }
    }

    #[test]
    fn other_shortcuts_still_repeat() {
        let mut consumed = ConsumedKeys::default();
        let mut runs = 0;
        for repeat in [false, true, true] {
            assert!(route_key_down(
                &mut consumed,
                V,
                repeat,
                Some(KeyAction::Paste),
                |_| {
                    runs += 1;
                    true
                }
            ));
        }
        assert_eq!(runs, 3);
    }

    #[test]
    fn a_held_consumed_key_stays_swallowed_after_its_modifier_lifts() {
        let mut consumed = ConsumedKeys::default();
        let mut selected = true;
        assert!(ctrl_c_down(&mut consumed, false, &mut selected).0);
        // Ctrl released first: C now repeats as an unbound key.
        assert!(route_key_down(&mut consumed, C, true, None, |_| false));
        assert_eq!(up(&mut consumed, C, true), None);
    }

    #[test]
    fn a_lost_release_does_not_eat_the_next_press() {
        let mut consumed = ConsumedKeys::default();
        let mut selected = true;
        assert!(ctrl_c_down(&mut consumed, false, &mut selected).0);
        // The key-up never came (focus moved). A fresh, unbound press of
        // the same key is ordinary input, and so is its release.
        assert!(!route_key_down(&mut consumed, C, false, None, |_| false));
        assert!(up(&mut consumed, C, true).is_some());
    }

    #[test]
    fn a_swallowed_release_clears_even_while_the_ui_owns_keys() {
        let mut consumed = ConsumedKeys::default();
        consumed.consume(C);
        let logical = WKey::Character("c".into());
        let release = KeyRelease {
            physical: C,
            logical: &logical,
            mods: ModifiersState::CONTROL,
            app_cursor: false,
            kitty_flags: 0,
            win32_input: true,
        };
        assert_eq!(release_input(&mut consumed, true, &release), None);
        assert!(up(&mut consumed, C, true).is_some());
    }

    #[test]
    fn a_cancelled_drag_cannot_extend_the_selection() {
        let mut drag = SelectionDrag::default();
        drag.start(Some((3, 7)));
        // Pointer moves and edge auto-scroll extend only from a live anchor.
        assert!(drag.is_live());
        assert_eq!(drag.live_anchor(), Some((3, 7)));
        drag.cancel(); // Ctrl+C copied mid-drag
        assert!(!drag.is_live());
        assert_eq!(drag.live_anchor(), None);
        // The later button-up is not a copy-on-select.
        assert!(!drag.release());
    }

    #[test]
    fn releasing_a_drag_copies_on_select_once() {
        let mut drag = SelectionDrag::default();
        drag.start(Some((0, 0)));
        assert!(drag.release());
        assert!(!drag.release());
        assert_eq!(drag.live_anchor(), None);
    }

    #[test]
    fn copy_or_send_copies_then_clears_the_selection() {
        let copied = copy_text(Some("ls".into()), Some((|_| Ok(())) as Write));
        assert_eq!(copied, Copied::Done);
        assert!(copied.consumes_key() && copied.clears_selection());
    }

    #[test]
    fn copy_or_send_with_no_selection_sends_the_key() {
        let copied = copy_text(None, Some((|_| Ok(())) as Write));
        assert_eq!(copied, Copied::NothingSelected);
        assert!(!copied.consumes_key() && !copied.clears_selection());
    }

    #[test]
    fn copy_or_send_without_a_clipboard_keeps_selection_and_key() {
        let copied = copy_text(Some("ls".into()), None::<Write>);
        assert!(matches!(copied, Copied::Failed(_)));
        assert!(copied.consumes_key() && !copied.clears_selection());
    }

    #[test]
    fn copy_or_send_on_write_failure_keeps_selection_and_key() {
        let copied = copy_text(Some("ls".into()), Some((|_| Err("busy".into())) as Write));
        assert_eq!(copied, Copied::Failed("busy".into()));
        assert!(copied.consumes_key() && !copied.clears_selection());
    }

    #[test]
    fn paste_normalizes_newlines_to_cr() {
        assert_eq!(encode_paste("a\r\nb\nc", false), b"a\rb\rc");
    }

    #[test]
    fn unbracketed_paste_is_raw_after_newline_fix() {
        assert_eq!(encode_paste("hello", false), b"hello");
    }

    #[test]
    fn bracketed_paste_wraps_and_strips_end_marker() {
        // An embedded end marker must not close the bracket early.
        assert_eq!(encode_paste("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn bracketed_paste_strips_spliced_end_marker() {
        // Removing one marker can splice a new one together; the sanitizer
        // must iterate until none remain or the bracket closes early.
        let out = encode_paste("\x1b[2\x1b[201~01~rm -rf ~", true);
        let body = &out[6..out.len() - 6];
        assert!(
            !body.windows(6).any(|w| w == b"\x1b[201~"),
            "end marker survived sanitization: {:?}",
            String::from_utf8_lossy(body)
        );
    }

    #[test]
    fn bracketed_paste_wraps_plain_text() {
        assert_eq!(encode_paste("ls", true), b"\x1b[200~ls\x1b[201~");
    }

    #[test]
    fn unbracketed_paste_strips_hidden_control_bytes() {
        // With no bracketed-paste support, the child interprets pasted text
        // as if it were typed — a hidden ESC (trivially embedded in text
        // copied from a web page, and invisible in the source) could smuggle
        // an arbitrary escape sequence, and a hidden BEL/backspace could do
        // something surprising at the prompt. Both must be stripped.
        assert_eq!(encode_paste("a\x1bb", false), b"ab");
        assert_eq!(encode_paste("rm\x07 -rf ~", false), b"rm -rf ~");
        assert_eq!(encode_paste("a\x08b", false), b"ab"); // backspace
        assert_eq!(encode_paste("a\x7fb", false), b"ab"); // DEL
    }

    #[test]
    fn unbracketed_paste_keeps_tabs_and_newlines() {
        // Newlines (normalized to CR) running each pasted line as its own
        // command is the expected, visible behavior without bracketed-paste
        // support — not something the control-byte filter should touch.
        assert_eq!(encode_paste("a\tb\nc\r\nd", false), b"a\tb\rc\rd");
    }

    #[test]
    fn unbracketed_paste_preserves_multibyte_utf8() {
        // UTF-8 continuation/lead bytes (>= 0x80) are never mistaken for the
        // ASCII control bytes the filter strips.
        assert_eq!(encode_paste("héllo→wörld", false), "héllo→wörld".as_bytes());
    }

    #[test]
    fn only_known_url_schemes_are_openable() {
        assert!(is_openable_url("https://example.com"));
        assert!(is_openable_url("HTTP://Example.COM")); // scheme is case-insensitive
        assert!(is_openable_url("mailto:a@b.com"));
        assert!(is_openable_url("file:///etc/hosts"));
        assert!(!is_openable_url("javascript:alert(1)"));
        assert!(!is_openable_url("data:text/html,x"));
        assert!(!is_openable_url("https://")); // nothing after the scheme
        assert!(!is_openable_url("notaurl"));
    }

    #[test]
    fn osc52_reply_wraps_base64() {
        // "hi" -> base64 "aGk=", framed as an OSC 52 clipboard reply (BEL).
        assert_eq!(osc52_reply('c', "hi"), b"\x1b]52;c;aGk=\x07");
        assert_eq!(osc52_reply('p', "hi"), b"\x1b]52;p;aGk=\x07");
    }

    #[test]
    fn put_text_writes_cells_and_hits_up_to_limit() {
        let mut row = vec![Cell::blank(); 8];
        let mut hits = vec![Hit::Drag; 8];
        let mut col = 1;
        put_text(
            &mut row,
            &mut hits,
            &mut col,
            4,
            "abcdef",
            0x111111,
            0x222222,
            Hit::Close,
        );
        assert_eq!(col, 4, "stops at the limit");
        assert_eq!(row[1].ch, 'a');
        assert_eq!(row[3].ch, 'c');
        assert_eq!(row[4].ch, ' ', "cell past the limit untouched");
        assert_eq!(row[1].fg, 0x111111);
        assert!(hits[1] == Hit::Close && hits[3] == Hit::Close);
        assert!(hits[4] == Hit::Drag);
    }

    #[test]
    fn put_text_gives_wide_glyphs_a_trailer() {
        use crate::core::WIDE_TRAILER;
        let mut row = vec![Cell::blank(); 8];
        let mut hits = vec![Hit::Drag; 8];
        let mut col = 0;
        put_text(
            &mut row,
            &mut hits,
            &mut col,
            8,
            "你x",
            0xAAAAAA,
            0x0,
            Hit::Close,
        );
        assert_eq!(col, 3, "wide glyph advances two cells");
        assert_eq!(row[0].ch, '你');
        assert_ne!(row[1].flags & WIDE_TRAILER, 0, "trailer flagged");
        assert_eq!(row[2].ch, 'x');
    }

    #[test]
    fn put_text_breaks_when_wide_trailer_does_not_fit() {
        let mut row = vec![Cell::blank(); 4];
        let mut hits = vec![Hit::Drag; 4];
        let mut col = 0;
        put_text(
            &mut row,
            &mut hits,
            &mut col,
            2,
            "a你",
            0x0,
            0x0,
            Hit::Close,
        );
        assert_eq!(col, 1, "wide head would straddle the limit; stops");
        assert_eq!(row[1].ch, ' ');
    }

    #[test]
    fn menu_lists_settings_then_config() {
        let items = menu_items();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].kind, MenuKind::Settings));
        assert!(matches!(items[1].kind, MenuKind::EditConfig));
    }

    #[test]
    fn arrow_presses_encode_decckm_aware_sequences() {
        assert_eq!(arrow_presses(2, 0, false), b"\x1b[C\x1b[C");
        assert_eq!(arrow_presses(-1, 1, false), b"\x1b[B\x1b[D");
        assert_eq!(arrow_presses(0, -2, true), b"\x1bOA\x1bOA");
        // Capped so a pathological delta can't flood the child.
        assert_eq!(arrow_presses(10_000, 0, false).len(), 400 * 3);
    }

    #[test]
    fn pane_pixel_point_offsets_and_clamps() {
        // Pointer at window (25, 40), pane origin (10, 16), pane 100x80 px.
        let p = pane_pixel_point((25.0, 40.0), (10, 16), (100, 80));
        assert_eq!((p.col, p.row), (15, 24));
        // Above/left of the pane clamps to 0; beyond it clamps to size-1.
        let p = pane_pixel_point((3.0, 3.0), (10, 16), (100, 80));
        assert_eq!((p.col, p.row), (0, 0));
        let p = pane_pixel_point((500.0, 500.0), (10, 16), (100, 80));
        assert_eq!((p.col, p.row), (99, 79));
    }

    #[test]
    fn zoom_reset_restores_the_dpi_scaled_size_exactly() {
        // Configured 48 px on a 200% monitor: 96 physical px, above the
        // logical FONT_MAX. Reset lands on it; zoom steps reach past it.
        assert_eq!(zoom_px(Zoom::Reset, 72.0, 96.0, 2.0), 96.0);
        assert_eq!(zoom_px(Zoom::In, 96.0, 96.0, 2.0), 97.0);
        assert_eq!(zoom_px(Zoom::Out, 96.0, 96.0, 2.0), 95.0);
        // Configured 4 px at 100% (below FONT_MIN): reset is exact too.
        assert_eq!(zoom_px(Zoom::Reset, 10.0, 4.0, 1.0), 4.0);
        assert_eq!(zoom_px(Zoom::Out, 4.0, 4.0, 1.0), 4.0);
    }

    #[test]
    fn physical_px_scales_logical_px_by_dpi() {
        assert_eq!(physical_px(UI_FONT_PX, 1.0), 18.0);
        assert_eq!(physical_px(UI_FONT_PX, 1.5), 27.0);
        assert_eq!(physical_px(14.0, 2.0), 28.0);
    }

    #[test]
    fn zoom_steps_clamp_to_the_dpi_scaled_range() {
        assert_eq!(zoom_px(Zoom::In, 72.0, 18.0, 1.0), 72.0);
        assert_eq!(zoom_px(Zoom::Out, 6.0, 18.0, 1.0), 6.0);
        assert_eq!(zoom_px(Zoom::In, 144.0, 36.0, 2.0), 144.0);
        assert_eq!(zoom_px(Zoom::Out, 12.0, 36.0, 2.0), 12.0);
        assert_eq!(zoom_px(Zoom::In, 18.0, 18.0, 1.0), 19.0);
    }

    #[test]
    fn ime_cursor_area_origin_is_window_relative_for_an_unsplit_pane() {
        // Unsplit pane origin (0, 0): the grid offset (padding band and
        // chrome bar) plus the cursor's cell.
        let (x, y) = ime_cursor_area_origin(4, 4, 10, 20, (0, 0), (3, 2));
        assert_eq!((x, y), ((4 + 3 * 10) as f64, (4 + 2 * 20) as f64));
    }

    #[test]
    fn ime_cursor_area_origin_adds_the_split_panes_own_offset() {
        // A pane starting at cell (40, 0) in a right-hand split: the popup
        // must land at the *pane's* cursor position, not the cursor's
        // position within an imaginary pane at the window origin — this is
        // the bug: before the fix, a right/bottom split's IME popup always
        // appeared over the leftmost/topmost pane instead.
        let unsplit = ime_cursor_area_origin(4, 4, 10, 20, (0, 0), (3, 2));
        let split = ime_cursor_area_origin(4, 4, 10, 20, (40, 0), (3, 2));
        assert_eq!(split.0 - unsplit.0, 40.0 * 10.0);
        assert_eq!(split.1, unsplit.1); // same row offset, no vertical split here
    }

    #[test]
    fn ime_commit_routes_to_the_find_bar_when_searching() {
        // Regardless of overlay/copy-mode state, an open find bar wins — it's
        // the innermost, most specific input owner.
        assert_eq!(
            ime_commit_target(true, false, false),
            ImeCommitTarget::SearchBar
        );
        assert_eq!(
            ime_commit_target(true, true, true),
            ImeCommitTarget::SearchBar
        );
    }

    #[test]
    fn ime_commit_is_dropped_under_an_overlay_or_copy_mode() {
        assert_eq!(
            ime_commit_target(false, true, false),
            ImeCommitTarget::Dropped
        );
        assert_eq!(
            ime_commit_target(false, false, true),
            ImeCommitTarget::Dropped
        );
    }

    #[test]
    fn ime_commit_reaches_the_pane_only_when_nothing_else_owns_the_keyboard() {
        assert_eq!(
            ime_commit_target(false, false, false),
            ImeCommitTarget::Pane
        );
    }

    #[test]
    fn focus_loss_without_release_clears_pressed() {
        // press -> focus lost -> focus back, with no release in between
        let pressed = Some(Hit::Close);
        let lost = pressed_after_focus(pressed, false);
        assert!(lost.is_none());
        assert!(pressed_after_focus(lost, true).is_none());
        // gaining focus alone never cancels a live press
        assert!(pressed_after_focus(pressed, true) == pressed);
    }

    #[test]
    fn drag_scroll_direction_fires_only_past_the_margin() {
        // Pane spans pixel rows [20, 100); a 5px margin around it.
        let (top, bottom, margin) = (20.0, 100.0, 5.0);
        assert_eq!(drag_scroll_direction(60.0, top, bottom, margin), None); // well inside
        assert_eq!(drag_scroll_direction(16.0, top, bottom, margin), None); // inside the margin
        assert_eq!(drag_scroll_direction(14.0, top, bottom, margin), Some(true)); // past it: scroll up
        assert_eq!(drag_scroll_direction(104.0, top, bottom, margin), None); // inside the margin
        assert_eq!(
            drag_scroll_direction(106.0, top, bottom, margin),
            Some(false)
        ); // past it: scroll down
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_quote_wraps_and_escapes_single_quotes() {
        assert_eq!(shell_quote("/tmp/plain"), "'/tmp/plain'");
        assert_eq!(shell_quote("/tmp/with space"), "'/tmp/with space'");
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }

    #[test]
    fn fmt_runtime_formats_ranges() {
        use std::time::Duration;
        assert_eq!(fmt_runtime(Duration::from_millis(300)), "<1s");
        assert_eq!(fmt_runtime(Duration::from_secs(42)), "42s");
        assert_eq!(fmt_runtime(Duration::from_secs(187)), "3m07s");
        assert_eq!(fmt_runtime(Duration::from_secs(7500)), "2h05m");
    }

    #[test]
    fn dock_grid_lists_blocks_newest_first_with_click_map() {
        use crate::core::AnsiParser;
        let mut g = crate::core::Grid::new(40, 8);
        let mut p = AnsiParser::new();
        p.advance(
            &mut g,
            b"$ make ok\r\n\x1b]133;C\x07fine\r\n\x1b]133;D;0\x07",
        );
        p.advance(
            &mut g,
            b"$ make bad\r\n\x1b]133;C\x07boom\r\n\x1b]133;D;3\x07",
        );
        let tokens = super::Chrome::from_theme(&crate::core::Theme::default());
        let (d, items) = super::build_dock_grid(&g, &tokens, 30, 8);
        // Header on row 0; entries start on row 2, newest first.
        assert_eq!(
            d.viewport_cell(1, 2).ch,
            '\u{2717}',
            "newest (failed) block first"
        );
        assert_eq!(
            d.viewport_cell(1, 2).fg,
            tokens.failure,
            "failure glyph is the failure token"
        );
        let label: String = (3..14).map(|c| d.viewport_cell(c, 2).ch).collect();
        assert_eq!(
            label.trim_end(),
            "$ make bad",
            "label is the prompt line above the block"
        );
        assert_eq!(d.viewport_cell(1, 3).ch, '\u{2713}', "older success below");
        assert_eq!(
            d.viewport_cell(1, 3).fg,
            tokens.success,
            "success glyph is the success token"
        );
        // Click map: row 2 jumps to the failed block's start, row 3 to the
        // successful one's; header/blank rows are inert.
        assert_eq!(items[2], Some(3));
        assert_eq!(items[3], Some(1));
        assert_eq!(items[0], None);
        assert_eq!(items[1], None);
    }

    #[test]
    fn dock_grid_empty_state_has_no_click_targets() {
        let g = crate::core::Grid::new(40, 8);
        let tokens = super::Chrome::from_theme(&crate::core::Theme::default());
        let (d, items) = super::build_dock_grid(&g, &tokens, 30, 8);
        assert!(items.iter().all(Option::is_none));
        let msg: String = (1..17).map(|c| d.viewport_cell(c, 2).ch).collect();
        assert_eq!(msg.trim_end(), "No commands yet.");
    }

    #[test]
    fn read_git_branch_walks_up_and_parses_head() {
        let root = std::env::temp_dir().join(format!("rt_git_test_{}", std::process::id()));
        let sub = root.join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        // A symbolic ref resolves to the branch name, from a subdirectory.
        std::fs::write(
            root.join(".git/HEAD"),
            "ref: refs/heads/feature/status-bar\n",
        )
        .unwrap();
        assert_eq!(read_git_branch(&sub).as_deref(), Some("feature/status-bar"));
        // A detached head shows the short hash.
        std::fs::write(
            root.join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(read_git_branch(&sub).as_deref(), Some("01234567"));
        // A worktree-style `.git` *file* follows its gitdir pointer.
        let wt = std::env::temp_dir().join(format!("rt_git_wt_{}", std::process::id()));
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", root.join(".git").display()),
        )
        .unwrap();
        assert_eq!(read_git_branch(&wt).as_deref(), Some("01234567"));
        // Outside any repository: None.
        let bare = std::env::temp_dir().join(format!("rt_git_none_{}", std::process::id()));
        std::fs::create_dir_all(&bare).unwrap();
        assert_eq!(read_git_branch(&bare), None);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&wt);
        let _ = std::fs::remove_dir_all(&bare);
    }

    mod reader {
        use super::super::{AnsiParser, Feed, Grid, Mutex, UserEvent, WakeFlag, reader_loop};
        use crate::backend::BackendHandle;
        use std::io::Error;
        use std::sync::Arc;

        /// Each part as its own feed step.
        fn bytes(parts: &[&[u8]]) -> Vec<Feed> {
            parts.iter().map(|p| Feed::Bytes(p.to_vec())).collect()
        }

        /// What `reader_loop` sent for `steps`, with `lower` standing in
        /// for the window handling each `Redraw` and `deliver` for whether
        /// a send reaches the loop. Ends with the grid's first line.
        fn run(
            steps: Vec<Feed>,
            wake: &WakeFlag,
            lower: bool,
            deliver: bool,
        ) -> (Vec<String>, String) {
            let grid = Arc::new(Mutex::new(Grid::new(20, 2)));
            let parser = Arc::new(Mutex::new(AnsiParser::new()));
            let sent = feed(steps, None, &grid, &parser, wake, lower, deliver);
            let line = grid.lock().line_text(0).unwrap_or_default();
            (sent, line)
        }

        /// Runs `reader_loop` over `steps` into `grid` and `parser`.
        fn feed(
            steps: Vec<Feed>,
            replies: Option<Box<dyn BackendHandle>>,
            grid: &Arc<Mutex<Grid>>,
            parser: &Arc<Mutex<AnsiParser>>,
            wake: &WakeFlag,
            lower: bool,
            deliver: bool,
        ) -> Vec<String> {
            let mut sent = Vec::new();
            let handled = wake.clone();
            reader_loop(
                7,
                steps.into_iter(),
                replies,
                Arc::clone(grid),
                Arc::clone(parser),
                wake.clone(),
                |event| {
                    let name = match event {
                        UserEvent::Redraw(7) => "redraw",
                        _ => "other",
                    };
                    sent.push(name.to_string());
                    if lower && name == "redraw" {
                        handled.lower();
                    }
                    deliver
                },
            );
            sent
        }

        #[test]
        fn output_while_a_wakeup_is_pending_sends_no_more() {
            let wake = WakeFlag::default();
            let (sent, line) = run(bytes(&[b"a", b"b", b"c"]), &wake, false, true);
            assert_eq!(sent, ["redraw"]);
            assert_eq!(line, "abc", "every batch is still parsed");
        }

        #[test]
        fn output_after_the_window_handles_a_wakeup_sends_another() {
            let wake = WakeFlag::default();
            let (sent, _) = run(bytes(&[b"a", b"b"]), &wake, true, true);
            assert_eq!(sent, ["redraw", "redraw"]);
        }

        #[test]
        fn output_before_the_flag_is_lowered_rides_the_pending_wakeup() {
            let wake = WakeFlag::default();
            assert!(wake.raise(), "a wakeup is in flight");
            let (sent, _) = run(bytes(&[b"a"]), &wake, false, true);
            assert!(sent.is_empty());
            wake.lower();
            assert!(wake.raise(), "once lowered, the next output wakes again");
        }

        #[test]
        fn a_failed_delivery_stops_reading() {
            let wake = WakeFlag::default();
            let (sent, line) = run(bytes(&[b"a", b"b"]), &wake, false, false);
            assert_eq!(sent, ["redraw"]);
            assert_eq!(line, "a", "nothing is parsed after the failed send");
        }

        #[test]
        fn a_closed_pane_with_a_pending_wakeup_reads_until_its_feed_ends() {
            // A closed pane never lowers its flag: the loop sends nothing
            // more and stops only when the feed ends (PTY EOF or error).
            let wake = WakeFlag::default();
            assert!(wake.raise());
            let (sent, line) = run(bytes(&[b"a", b"b"]), &wake, false, true);
            assert!(sent.is_empty());
            assert_eq!(line, "ab");
        }

        #[test]
        fn a_screen_replaces_the_terminal_and_output_continues_on_it() {
            let mut shown = Grid::new(30, 3);
            let mut its_parser = AnsiParser::new();
            its_parser.advance(&mut shown, b"from the server ");
            let steps = vec![
                Feed::Bytes(b"local".to_vec()),
                Feed::Screen(Box::new(shown), Box::new(its_parser)),
                Feed::Bytes(b"live".to_vec()),
            ];
            let (sent, line) = run(steps, &WakeFlag::default(), true, true);
            assert_eq!(sent, ["redraw", "redraw", "redraw"]);
            assert_eq!(line.trim_end(), "from the server live");
        }

        #[test]
        fn a_screen_keeps_the_panes_theme_and_display_settings() {
            let ours = crate::core::Theme {
                fg: 0x00aa_bbcc,
                bg: 0x0011_2233,
                ..Default::default()
            };
            let mut pane_grid = Grid::new(20, 2);
            pane_grid.apply_theme(&ours);
            pane_grid.min_contrast = 4.5;
            pane_grid.cell_px = Some((9, 18));
            let grid = Arc::new(Mutex::new(pane_grid));
            let parser = Arc::new(Mutex::new(AnsiParser::with_theme(ours)));
            let steps = vec![
                Feed::Screen(Box::new(Grid::new(20, 2)), Box::new(AnsiParser::new())),
                Feed::Bytes(b"x".to_vec()),
            ];
            feed(
                steps,
                None,
                &grid,
                &parser,
                &WakeFlag::default(),
                true,
                true,
            );
            let g = grid.lock();
            assert_eq!(
                g.viewport_cell(0, 0).fg,
                ours.fg,
                "drawn in the pane's theme"
            );
            assert_eq!(g.viewport_cell(1, 0).bg, ours.bg);
            assert_eq!((g.min_contrast, g.cell_px), (4.5, Some((9, 18))));
        }

        #[test]
        fn a_resize_from_the_source_resizes_the_grid() {
            let grid = Arc::new(Mutex::new(Grid::new(20, 2)));
            let parser = Arc::new(Mutex::new(AnsiParser::new()));
            let steps = vec![Feed::Resized { cols: 33, rows: 5 }];
            feed(
                steps,
                None,
                &grid,
                &parser,
                &WakeFlag::default(),
                true,
                true,
            );
            let g = grid.lock();
            assert_eq!((g.cols, g.rows), (33, 5));
        }

        /// A writer that keeps what it is sent.
        struct Recorded(Arc<std::sync::Mutex<Vec<u8>>>);

        impl BackendHandle for Recorded {
            fn read_into(&mut self, _buf: &mut [u8]) -> Result<usize, Error> {
                Ok(0)
            }
            fn write(&mut self, data: &[u8]) -> Result<(), Error> {
                self.0.lock().expect("unpoisoned").extend_from_slice(data);
                Ok(())
            }
            fn try_clone(&self) -> Result<Box<dyn BackendHandle>, Error> {
                Ok(Box::new(Recorded(Arc::clone(&self.0))))
            }
            fn set_winsize(&mut self, _cols: u16, _rows: u16) -> Result<(), Error> {
                Ok(())
            }
            #[cfg(unix)]
            fn pty_fd(&self) -> std::os::unix::io::RawFd {
                -1
            }
        }

        #[test]
        fn queries_are_answered_only_with_a_reply_writer() {
            // DA1: the parser answers it, so a pane that answers writes back.
            let query = || bytes(&[b"\x1b[c"]);
            let grid = Arc::new(Mutex::new(Grid::new(20, 2)));
            let parser = Arc::new(Mutex::new(AnsiParser::new()));
            let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
            let writer = Box::new(Recorded(Arc::clone(&kept)));
            feed(
                query(),
                Some(writer),
                &grid,
                &parser,
                &WakeFlag::default(),
                true,
                true,
            );
            assert!(!kept.lock().expect("unpoisoned").is_empty(), "answered");
            // Without one (a mux block: its server answers), nothing is sent
            // and the parser's reply is still drained.
            feed(
                query(),
                None,
                &grid,
                &parser,
                &WakeFlag::default(),
                true,
                true,
            );
            assert!(parser.lock().take_responses().is_empty());
        }
    }
}
