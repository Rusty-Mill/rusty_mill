//! The command palette and directory browse of an attached window
//! (gui-design D77, D78): a search box over actions, sessions and windows,
//! and a folder listing read from the block's host.
//!
//! Pure: [`Palette`] keeps the query, the selection and the browse state,
//! answers keys and clicks with an [`Out`] for the window to carry out, and
//! gives the [`Line`]s [`super::palette_view`] draws. [`Listing`] is the
//! text of the server's `terminal.list-dir` (handoff D303).

use super::source::Listed;

/// Rows a PageUp or PageDown moves, and the list's height in rows (D77).
pub(crate) const PAGE: usize = 8;
/// Rows each group shows while the query is empty (D77).
const FIRST: usize = 5;
/// Folders and files the browse lists (the server lists 1000, D78).
const SHOWN: usize = 1000;

/// What a palette action does; the window knows how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cmd {
    SplitRight,
    SplitDown,
    Zoom,
    CloseBlock,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NewWindow,
    Switcher,
    /// Show or leave the session overview (gui-design D97).
    Overview,
    Settings,
    ReloadConfig,
    TakeOver,
    GiveBack,
}

/// What a row stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Id {
    Cmd(Cmd),
    /// A session, by the source's key.
    Session(u64),
    /// A window of the session, by the source's key.
    Window(u64),
    /// "Go to directory" (D78).
    GoTo,
    /// "Insert path" (D78).
    Insert,
}

/// A row's group, in the order the palette lists them (D77).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Actions,
    Sessions,
    Windows,
    Folders,
}

impl Group {
    const ALL: [Group; 4] = [
        Group::Actions,
        Group::Sessions,
        Group::Windows,
        Group::Folders,
    ];

    fn title(self) -> &'static str {
        match self {
            Group::Actions => "Actions",
            Group::Sessions => "Sessions",
            Group::Windows => "Windows",
            Group::Folders => "Folders",
        }
    }
}

/// One thing the palette can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub id: Id,
    pub label: String,
    /// Right-aligned, slate: "2 windows", "Take over first".
    pub detail: String,
    pub enabled: bool,
    /// The session or window shown now.
    pub current: bool,
}

impl Entry {
    fn group(&self) -> Group {
        match self.id {
            Id::Cmd(_) => Group::Actions,
            Id::Session(_) => Group::Sessions,
            Id::Window(_) => Group::Windows,
            Id::GoTo | Id::Insert => Group::Folders,
        }
    }
}

/// A folder entry's kind, as the server lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A folder, or a link to one.
    Dir,
    File,
    /// Any other link.
    Link,
    Other,
}

/// A folder listing: the text of `terminal.list-dir`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Listing {
    /// The folder read, absolute.
    pub path: String,
    pub items: Vec<(Kind, String)>,
    /// Entries the server left out, and whether that is a lower bound.
    pub more: usize,
    pub at_least: bool,
    /// Entries it could not read or name.
    pub unreadable: usize,
}

impl Listing {
    /// Reads the server's reply: the path, then `KIND<TAB>NAME` lines and
    /// the `+`, `>` and `!` counts. A line it doesn't know is skipped, so
    /// a newer server may say more.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let path = lines
            .next()
            .filter(|p| !p.is_empty())
            .ok_or("the server named no folder")?;
        let path = unescape(path).ok_or("the server named its folder badly")?;
        let mut out = Listing {
            path: plain_path(path),
            ..Listing::default()
        };
        for line in lines {
            let Some((kind, rest)) = line.split_once('\t') else {
                continue;
            };
            let kind = match kind {
                "d" => Kind::Dir,
                "f" => Kind::File,
                "l" => Kind::Link,
                "o" => Kind::Other,
                "+" | ">" => {
                    out.more = out.more.saturating_add(rest.parse().unwrap_or(0));
                    out.at_least |= kind == ">";
                    continue;
                }
                "!" => {
                    out.unreadable = out.unreadable.saturating_add(rest.parse().unwrap_or(0));
                    continue;
                }
                _ => continue,
            };
            match unescape(rest) {
                Some(name) => out.items.push((kind, name)),
                None => out.unreadable = out.unreadable.saturating_add(1),
            }
        }
        Ok(out)
    }

    /// Entries not shown: left out by the server or unreadable.
    fn hidden(&self) -> usize {
        self.more.saturating_add(self.unreadable)
    }
}

/// Undoes the server's escaping: `\\`, `\t`, `\n`, `\r` and `\u{HEX}`;
/// `None` for an escape it doesn't know, which the server never writes.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        c => hex.push(c),
                    }
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Where `query` matches a label, for ranking and for lighting letters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Match {
    /// Higher is better.
    pub score: i32,
    /// The matched characters, by index.
    pub at: Vec<usize>,
}

/// Case-insensitive subsequence match of `query` on `label` (D77). Words
/// split on space, `-`, `_` and `/`. A prefix of the label scores over a
/// prefix of a word, over a run, over scattered letters.
pub(crate) fn matches(query: &str, label: &str) -> Option<Match> {
    let fold = |s: &str| -> Vec<char> {
        s.chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect()
    };
    let (q, l) = (fold(query.trim_start()), fold(label));
    if q.is_empty() {
        return Some(Match {
            score: 0,
            at: Vec::new(),
        });
    }
    let run = |start: usize| (start..start + q.len()).collect::<Vec<_>>();
    if l.starts_with(&q) {
        return Some(Match {
            score: 3000,
            at: run(0),
        });
    }
    let starts_word = |i: usize| matches!(l[i - 1], ' ' | '-' | '_' | '/');
    if let Some(i) = (1..l.len()).find(|&i| starts_word(i) && l[i..].starts_with(&q)) {
        return Some(Match {
            score: 2000,
            at: run(i),
        });
    }
    let mut at = Vec::with_capacity(q.len());
    let mut from = 0;
    for &c in &q {
        let found = l[from..].iter().position(|&x| x == c)? + from;
        at.push(found);
        from = found + 1;
    }
    let longest = at
        .windows(2)
        .fold((1, 1), |(best, now), w| {
            let now = if w[1] == w[0] + 1 { now + 1 } else { 1 };
            (best.max(now), now)
        })
        .0;
    let gaps = at[at.len() - 1] - at[0] + 1 - at.len();
    Some(Match {
        score: 1000 + 10 * longest - gaps as i32,
        at,
    })
}

/// A ranked entry: its score, its index and the characters matched.
type Hit = (i32, usize, Vec<usize>);

/// A line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A group's header (D77), or "Folders" over a browse (D78).
    Head(String),
    Item(Item),
    /// A line that says something and picks nothing: the empty states,
    /// "Reading", "{n} more not shown".
    Note(Note),
}

/// A row that picks something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub pick: Pick,
    pub label: String,
    pub detail: String,
    /// Label characters the query matched.
    pub at: Vec<usize>,
    pub enabled: bool,
    pub current: bool,
    /// A folder icon and trailing chevron (D78).
    pub folder: bool,
}

/// What a row asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Run(Id),
    /// Browse: "This folder".
    Here,
    /// Browse: "..".
    Up,
    /// Browse: a folder or file of the listing.
    Entry(Kind, String),
}

/// A line that picks nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    pub title: String,
    pub hint: Option<String>,
    /// An oxide alert icon (D78 unreadable).
    pub alert: bool,
    /// The static spinner (D27) while a folder is read.
    pub spinner: bool,
}

impl Line {
    /// Height in logical px (D77: header 28, rows 40, a note 40 or 58).
    pub(crate) fn height(&self) -> f32 {
        match self {
            Line::Head(_) => 28.0,
            Line::Item(_) => 40.0,
            Line::Note(n) if n.hint.is_some() => 58.0,
            Line::Note(_) => 40.0,
        }
    }

    fn enabled_item(&self) -> bool {
        matches!(self, Line::Item(i) if i.enabled)
    }
}

/// What the window should do after a key or click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    /// Redraw; nothing else.
    Stay,
    /// Close the palette.
    Close,
    /// Run `Id` and close.
    Run(Id),
    /// Read a folder: the window asks its source, and answers with
    /// [`Palette::listed`] under `ask`.
    List { ask: u64, path: Option<String> },
    /// Type this text into the block, without Enter, and close.
    Type(String),
}

/// A key the palette reads, as the window maps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(String),
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    Tab,
    Backspace,
    Esc,
}

/// Where a folder read stands.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Load {
    Reading,
    Ready(Listing),
    /// The path asked for, and why the host refused.
    Failed(String, String),
}

/// A directory browse (D78).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Browse {
    /// "Insert path": files too, and no `cd`.
    insert: bool,
    ask: u64,
    /// The folder asked for, `None` for the server's own start.
    at: Option<String>,
    load: Load,
}

/// An open palette.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Palette {
    entries: Vec<Entry>,
    query: String,
    browse: Option<Browse>,
    /// The selected line.
    sel: usize,
    /// The first line shown.
    top: usize,
    asks: u64,
    /// Text mid-composition (IME), shown after the query, not yet typed.
    preedit: String,
}

impl Palette {
    /// Opens on `entries`, the first row selected, numbering folder
    /// requests after `asks`, so an answer meant for an earlier palette
    /// never matches a later one.
    pub(crate) fn open(entries: Vec<Entry>, asks: u64) -> Self {
        let mut p = Palette {
            entries,
            asks,
            ..Palette::default()
        };
        p.reset();
        p
    }

    /// Set the composition in progress (empty when none).
    pub(crate) fn set_preedit(&mut self, text: &str) {
        self.preedit = text.to_owned();
    }

    /// The composition in progress.
    pub(crate) fn preedit(&self) -> &str {
        &self.preedit
    }

    /// The query, or the browse's filter.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// The folder shown while browsing, as far as known (D78).
    pub(crate) fn folder(&self) -> Option<&str> {
        let b = self.browse.as_ref()?;
        match &b.load {
            Load::Ready(l) => Some(&l.path),
            _ => b.at.as_deref(),
        }
    }

    /// The last folder request number used.
    pub(crate) fn asks(&self) -> u64 {
        self.asks
    }

    /// Whether the list is a folder's (D78 footer and header).
    pub(crate) fn browsing(&self) -> bool {
        self.browse.is_some()
    }

    /// The first line shown.
    #[cfg(test)]
    pub(crate) fn top(&self) -> usize {
        self.top
    }

    pub(crate) fn selected_line(&self) -> usize {
        self.sel
    }

    /// The server-side rows now (D77: the list updates live); the
    /// selection stays on its row while it is there.
    pub(crate) fn update(&mut self, entries: Vec<Entry>) {
        if self.entries == entries {
            return;
        }
        let kept = self.pick();
        self.entries = entries;
        if self.browse.is_some() {
            return;
        }
        let lines = self.lines();
        let at = kept.and_then(|k| {
            lines
                .iter()
                .position(|l| matches!(l, Line::Item(i) if i.pick == k && i.enabled))
        });
        self.sel = at.unwrap_or_else(|| first(&lines));
        self.settle(&lines);
    }

    /// The lines the list shows now.
    pub(crate) fn lines(&self) -> Vec<Line> {
        match &self.browse {
            Some(b) => self.browse_lines(b),
            None => self.command_lines(),
        }
    }

    fn command_lines(&self) -> Vec<Line> {
        let q = self.query.trim_start();
        let mut groups: Vec<(i32, Group, Vec<Hit>)> = Vec::new();
        for group in Group::ALL {
            let mut hits: Vec<Hit> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.group() == group)
                .filter_map(|(i, e)| matches(q, &e.label).map(|m| (m.score, i, m.at)))
                .collect();
            if hits.is_empty() {
                continue;
            }
            if q.is_empty() {
                hits.truncate(FIRST);
            } else {
                hits.sort_by_key(|(score, i, _)| (-score, *i));
            }
            let best = hits.iter().map(|h| h.0).max().unwrap_or(0);
            groups.push((best, group, hits));
        }
        if !q.is_empty() {
            groups.sort_by_key(|(best, group, _)| (-best, *group as usize));
        }
        if groups.is_empty() {
            return vec![Line::Note(Note {
                title: format!("No match for \"{}\"", self.query),
                hint: Some("Try fewer letters.".into()),
                alert: false,
                spinner: false,
            })];
        }
        let mut lines = Vec::new();
        for (_, group, hits) in groups {
            lines.push(Line::Head(group.title().into()));
            for (_, i, at) in hits {
                let e = &self.entries[i];
                lines.push(Line::Item(Item {
                    pick: Pick::Run(e.id),
                    label: e.label.clone(),
                    detail: e.detail.clone(),
                    at,
                    enabled: e.enabled,
                    current: e.current,
                    folder: false,
                }));
            }
        }
        lines
    }

    fn browse_lines(&self, b: &Browse) -> Vec<Line> {
        let note = |title: String, hint: Option<&str>, alert: bool, spinner: bool| {
            Line::Note(Note {
                title,
                hint: hint.map(Into::into),
                alert,
                spinner,
            })
        };
        let list = match &b.load {
            Load::Reading => {
                let at = b.at.as_deref().unwrap_or("the folder");
                return vec![note(format!("Reading {at}"), None, false, true)];
            }
            Load::Failed(path, why) => {
                let hint = match why.is_empty() {
                    true => "Check the folder exists and you may open it.",
                    false => why.as_str(),
                };
                return vec![note(format!("Can't read {path}"), Some(hint), true, false)];
            }
            Load::Ready(list) => list,
        };
        let head = if b.insert {
            "Folders and files"
        } else {
            "Folders"
        };
        let item = |pick: Pick, label: &str, at: Vec<usize>, folder| {
            let path = match &pick {
                Pick::Here => Some(list.path.clone()),
                Pick::Up => parent(&list.path),
                Pick::Entry(_, name) => Some(join(&list.path, name)),
                Pick::Run(_) => None,
            };
            let typable = path.is_none_or(|p| quote(&p).is_some());
            Line::Item(Item {
                detail: if typable { "" } else { "Can't type this name" }.into(),
                pick,
                label: label.into(),
                at,
                enabled: true,
                current: false,
                folder,
            })
        };
        let mut lines = vec![Line::Head(head.into())];
        if self.query.is_empty() {
            lines.push(item(Pick::Here, "This folder", Vec::new(), false));
            if parent(&list.path).is_some() {
                lines.push(item(Pick::Up, "..", Vec::new(), true));
            }
        }
        let dotted = self.query.starts_with('.');
        let mut shown: Vec<Hit> = list
            .items
            .iter()
            .enumerate()
            .filter(|(_, (kind, name))| {
                (b.insert || *kind == Kind::Dir) && (dotted || !name.starts_with('.'))
            })
            .filter_map(|(i, (_, name))| matches(&self.query, name).map(|m| (m.score, i, m.at)))
            .collect();
        shown.sort_by_key(|(score, i, _)| (-score, *i));
        let total = shown.len();
        for (_, i, at) in shown.into_iter().take(SHOWN) {
            let (kind, name) = &list.items[i];
            let label = if *kind == Kind::Dir {
                format!("{name}/")
            } else {
                name.clone()
            };
            lines.push(item(
                Pick::Entry(*kind, name.clone()),
                &label,
                at,
                *kind == Kind::Dir,
            ));
        }
        if total == 0 {
            let (title, hint) = match self.query.is_empty() {
                true => ("Empty folder".to_owned(), None),
                false => (
                    format!("No match for \"{}\"", self.query),
                    Some("Try fewer letters."),
                ),
            };
            lines.push(note(title, hint, false, false));
        }
        let left = list.hidden() + total.saturating_sub(SHOWN);
        if left > 0 && self.query.is_empty() {
            let few = if list.at_least { "at least " } else { "" };
            lines.push(note(
                format!("{few}{left} more not shown, type to narrow"),
                None,
                false,
                false,
            ));
        }
        lines
    }

    /// What the selected row asks for.
    pub(crate) fn pick(&self) -> Option<Pick> {
        match self.lines().get(self.sel)? {
            Line::Item(i) if i.enabled => Some(i.pick.clone()),
            _ => None,
        }
    }

    /// The pick of line `i`, if it is an enabled row.
    pub(crate) fn pick_at(&self, i: usize) -> Option<Pick> {
        match self.lines().get(i)? {
            Line::Item(item) if item.enabled => Some(item.pick.clone()),
            _ => None,
        }
    }

    fn reset(&mut self) {
        let lines = self.lines();
        self.sel = first(&lines);
        self.top = 0;
        self.settle(&lines);
    }

    /// Keeps the selection in view, and its header with it.
    fn settle(&mut self, lines: &[Line]) {
        if self.sel < self.top {
            self.top = self.sel;
        }
        while self.top < self.sel && !fits(lines, self.top, self.sel) {
            self.top += 1;
        }
        if self.top > 0
            && self.top == self.sel
            && matches!(lines.get(self.top - 1), Some(Line::Head(_)))
        {
            self.top -= 1;
        }
    }

    /// How many lines from `top` fit the list's height.
    pub(crate) fn visible(&self, lines: &[Line]) -> std::ops::Range<usize> {
        let mut used = 0.0;
        let mut end = self.top;
        for line in &lines[self.top.min(lines.len())..] {
            used += line.height();
            if used > PAGE as f32 * 40.0 && end > self.top {
                break;
            }
            end += 1;
        }
        self.top.min(lines.len())..end
    }

    /// Moves the selection `by` rows among the enabled ones, wrapping.
    fn step(&mut self, by: isize) {
        let lines = self.lines();
        let rows: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i].enabled_item())
            .collect();
        if rows.is_empty() {
            return;
        }
        let now = rows.iter().position(|&i| i == self.sel).unwrap_or(0) as isize;
        let n = rows.len() as isize;
        // A page stops at the end; a single step wraps.
        let to = if by.abs() == 1 {
            (now + by).rem_euclid(n)
        } else {
            (now + by).clamp(0, n - 1)
        };
        self.sel = rows[to as usize];
        self.settle(&lines);
    }

    /// A key; the window carries out what it asks.
    pub(crate) fn key(&mut self, key: Key) -> Out {
        match key {
            Key::Esc => Out::Close,
            Key::Up => self.moved(-1),
            Key::Down => self.moved(1),
            Key::PageUp => self.moved(-(PAGE as isize)),
            Key::PageDown => self.moved(PAGE as isize),
            Key::Char(text) => {
                self.query.push_str(&text);
                self.reset();
                Out::Stay
            }
            Key::Backspace if !self.query.is_empty() => {
                self.query.pop();
                self.reset();
                Out::Stay
            }
            Key::Backspace => self.up(),
            Key::Enter => self.pick().map_or(Out::Stay, |p| self.choose(p)),
            Key::Tab => self.pick().map_or(Out::Stay, |p| self.open_folder(&p)),
        }
    }

    fn moved(&mut self, by: isize) -> Out {
        self.step(by);
        Out::Stay
    }

    /// A click on line `i`; `chevron` when on a folder's right end.
    pub(crate) fn click(&mut self, i: usize, chevron: bool) -> Out {
        let Some(pick) = self.pick_at(i) else {
            return Out::Stay;
        };
        self.sel = i;
        match chevron {
            true => self.open_folder(&pick),
            false => self.choose(pick),
        }
    }

    /// Enter on `pick` (D77, D78).
    fn choose(&mut self, pick: Pick) -> Out {
        let Some(b) = &self.browse else {
            return match pick {
                Pick::Run(Id::GoTo) => self.start_browse(false),
                Pick::Run(Id::Insert) => self.start_browse(true),
                Pick::Run(id) => Out::Run(id),
                _ => Out::Stay,
            };
        };
        let Load::Ready(list) = &b.load else {
            return Out::Stay;
        };
        let path = match pick {
            Pick::Here => Some(list.path.clone()),
            Pick::Up => parent(&list.path),
            Pick::Entry(_, name) => Some(join(&list.path, &name)),
            Pick::Run(_) => None,
        };
        let Some(path) = path else {
            return Out::Stay;
        };
        let Some(quoted) = quote(&path) else {
            return Out::Stay;
        };
        Out::Type(match b.insert {
            true => quoted,
            false => format!("cd {quoted}"),
        })
    }

    /// Tab, or a click on the chevron: into a folder, or up for "..".
    fn open_folder(&mut self, pick: &Pick) -> Out {
        let Some(Load::Ready(list)) = self.browse.as_ref().map(|b| &b.load) else {
            return Out::Stay;
        };
        let path = match pick {
            Pick::Up => parent(&list.path),
            Pick::Entry(Kind::Dir, name) => Some(join(&list.path, name)),
            _ => None,
        };
        path.map_or(Out::Stay, |p| self.read(Some(p)))
    }

    /// Backspace on an empty filter: up one level (D78).
    fn up(&mut self) -> Out {
        let here = match self.browse.as_ref() {
            Some(Browse {
                load: Load::Ready(list),
                ..
            }) => Some(list.path.clone()),
            // A refused folder still goes up from where it was asked.
            Some(Browse {
                load: Load::Failed(..),
                at,
                ..
            }) => at.clone(),
            _ => None,
        };
        here.and_then(|h| parent(&h))
            .map_or(Out::Stay, |p| self.read(Some(p)))
    }

    fn start_browse(&mut self, insert: bool) -> Out {
        self.browse = Some(Browse {
            insert,
            ask: 0,
            at: None,
            load: Load::Reading,
        });
        self.read(None)
    }

    /// Asks for `path`, clearing the filter.
    fn read(&mut self, path: Option<String>) -> Out {
        self.asks += 1;
        let ask = self.asks;
        self.query.clear();
        if let Some(b) = &mut self.browse {
            b.ask = ask;
            b.at.clone_from(&path);
            b.load = Load::Reading;
        }
        self.reset();
        Out::List { ask, path }
    }

    /// The answer to a [`Out::List`]; an old one is ignored.
    pub(crate) fn listed(&mut self, ask: u64, listed: Listed) {
        let Some(b) = self.browse.as_mut().filter(|b| b.ask == ask) else {
            return;
        };
        b.load = match listed.and_then(|text| Listing::parse(&text)) {
            Ok(list) => Load::Ready(list),
            Err(why) => Load::Failed(b.at.clone().unwrap_or_else(|| "the folder".into()), why),
        };
        self.reset();
    }
}

/// The first line a key can pick, else 0.
fn first(lines: &[Line]) -> usize {
    lines.iter().position(Line::enabled_item).unwrap_or(0)
}

/// Whether line `to` ends within the list's height from line `from`.
fn fits(lines: &[Line], from: usize, to: usize) -> bool {
    let used: f32 = lines[from..=to].iter().map(Line::height).sum();
    used <= PAGE as f32 * 40.0
}

/// Whether `path` is a Windows path: a drive (`X:` then a separator or
/// the end) or a UNC share.
fn windows(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with("\\\\")
        || (b.len() >= 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b.get(2), None | Some(b'\\' | b'/')))
}

/// `path` without a verbatim prefix: `\\?\C:\x` as `C:\x` and
/// `\\?\UNC\h\s` as `\\h\s`, which `cd` and [`parent`] understand.
fn plain_path(path: String) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{rest}");
    }
    match path.strip_prefix("\\\\?\\") {
        Some(rest) if windows(rest) => rest.to_owned(),
        _ => path,
    }
}

fn sep(path: &str) -> char {
    if windows(path) { '\\' } else { '/' }
}

/// `path`'s folder, or `None` at the root.
fn parent(path: &str) -> Option<String> {
    let s = sep(path);
    let trimmed = path.trim_end_matches(s);
    let at = trimmed.rfind(s)?;
    if !windows(path) {
        return Some(if at == 0 { "/" } else { &trimmed[..at] }.to_owned());
    }
    match at {
        // `C:\x` goes to `C:\`; `\\host` has no folder above it.
        2 if trimmed.as_bytes()[1] == b':' => Some(trimmed[..=at].to_owned()),
        0 | 1 => None,
        at => Some(trimmed[..at].to_owned()),
    }
}

/// `name` in the folder `path`.
fn join(path: &str, name: &str) -> String {
    let s = sep(path);
    match path.ends_with(s) {
        true => format!("{path}{name}"),
        false => format!("{path}{s}{name}"),
    }
}

/// `path` as one shell word for the host's path style (D78): single
/// quotes on POSIX when it has more than plain characters, double quotes
/// on Windows when it has a space or a character cmd reads. `None` when
/// no quoting is safe without knowing the shell: any control character
/// (it would reach the pty as a key), or on Windows one of `$`, a
/// backtick, `%`, `!` or `"`, which PowerShell, Git Bash or cmd read
/// inside double quotes.
pub(crate) fn quote(path: &str) -> Option<String> {
    if path.chars().any(char::is_control) {
        return None;
    }
    if windows(path) {
        if path.contains(['$', '`', '%', '!', '"']) {
            return None;
        }
        let plain = !path.contains([' ', '&', '(', ')', '^', ',', ';', '=', '\'']);
        return Some(match plain {
            true => path.to_owned(),
            false => format!("\"{path}\""),
        });
    }
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    Some(match plain {
        true => path.to_owned(),
        false => format!("'{}'", path.replace('\'', "'\\''")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(cmd: Cmd, label: &str) -> Entry {
        Entry {
            id: Id::Cmd(cmd),
            label: label.into(),
            detail: String::new(),
            enabled: true,
            current: false,
        }
    }

    fn entries() -> Vec<Entry> {
        vec![
            cmd(Cmd::SplitRight, "Split right"),
            cmd(Cmd::SplitDown, "Split down"),
            cmd(Cmd::Settings, "Settings"),
            Entry {
                id: Id::Session(1),
                label: "work".into(),
                detail: "2 windows".into(),
                enabled: true,
                current: true,
            },
            Entry {
                id: Id::Window(7),
                label: "build-logs".into(),
                detail: String::new(),
                enabled: true,
                current: false,
            },
            Entry {
                id: Id::GoTo,
                label: "Go to directory".into(),
                detail: String::new(),
                enabled: true,
                current: false,
            },
            Entry {
                id: Id::Insert,
                label: "Insert path".into(),
                detail: String::new(),
                enabled: true,
                current: false,
            },
        ]
    }

    fn type_in(p: &mut Palette, text: &str) {
        for c in text.chars() {
            p.key(Key::Char(c.to_string()));
        }
    }

    fn labels(p: &Palette) -> Vec<String> {
        p.lines()
            .iter()
            .filter_map(|l| match l {
                Line::Item(i) => Some(i.label.clone()),
                _ => None,
            })
            .collect()
    }

    /// As the server sorts: folders first, then names without regard to case.
    const LISTING: &str = "/home/me\nd\t.git\nd\tdocs\nd\tsrc\nf\t.env\nf\tREADME.md\n+\t3\n!\t1";

    fn select(p: &mut Palette, label: &str) {
        for _ in 0..30 {
            if matches!(p.lines().get(p.selected_line()), Some(Line::Item(i)) if i.label == label) {
                return;
            }
            p.key(Key::Down);
        }
        panic!("no row {label}");
    }

    fn browsing(insert: bool) -> Palette {
        let mut p = Palette::open(entries(), 0);
        let id = if insert { "Insert" } else { "Go" };
        type_in(&mut p, id);
        let Out::List { ask, path } = p.key(Key::Enter) else {
            panic!("no listing asked");
        };
        assert_eq!(path, None);
        p.listed(ask, Ok(LISTING.into()));
        p
    }

    #[test]
    fn a_prefix_beats_a_word_beats_scattered_letters() {
        let prefix = matches("spl", "Split right").expect("prefix");
        let word = matches("rig", "Split right").expect("word");
        let loose = matches("sgt", "Split right").expect("loose");
        assert!(prefix.score > word.score && word.score > loose.score);
        assert_eq!(prefix.at, [0, 1, 2]);
        assert_eq!(word.at, [6, 7, 8]);
        assert_eq!(matches("zq", "Split right"), None);
        assert_eq!(matches("SPLIT", "split").map(|m| m.at.len()), Some(5));
    }

    #[test]
    fn words_split_on_dashes_underscores_and_slashes() {
        let word = |q, l| matches(q, l).is_some_and(|m| m.score == 2000);
        assert!(word("log", "build-logs"));
        assert!(word("b", "a_b"));
        assert!(word("src", "x/src"));
    }

    #[test]
    fn an_empty_query_shows_every_group_in_order() {
        let p = Palette::open(entries(), 0);
        let lines = p.lines();
        let heads: Vec<_> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Head(h) => Some(h.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(heads, ["Actions", "Sessions", "Windows", "Folders"]);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Cmd(Cmd::SplitRight))));
    }

    #[test]
    fn an_empty_query_shows_five_rows_a_group() {
        let many = (0..9)
            .map(|i| Entry {
                id: Id::Window(i),
                label: format!("win {i}"),
                detail: String::new(),
                enabled: true,
                current: false,
            })
            .collect();
        assert_eq!(labels(&Palette::open(many, 0)).len(), FIRST);
    }

    #[test]
    fn equal_matches_keep_their_list_order() {
        let mut p = Palette::open(entries(), 0);
        type_in(&mut p, "spl");
        assert_eq!(labels(&p), ["Split right", "Split down"]);
    }

    #[test]
    fn typing_filters_and_the_best_group_goes_first() {
        let mut p = Palette::open(entries(), 0);
        type_in(&mut p, "logs");
        assert_eq!(labels(&p), ["build-logs"]);
        assert!(matches!(&p.lines()[0], Line::Head(h) if h == "Windows"));
        assert_eq!(p.pick(), Some(Pick::Run(Id::Window(7))));
    }

    #[test]
    fn no_match_says_so_and_picks_nothing() {
        let mut p = Palette::open(entries(), 0);
        type_in(&mut p, "zzz");
        assert!(matches!(&p.lines()[..], [Line::Note(n)] if n.title == "No match for \"zzz\""));
        assert_eq!(p.pick(), None);
        assert_eq!(p.key(Key::Enter), Out::Stay);
        p.key(Key::Backspace);
        p.key(Key::Backspace);
        p.key(Key::Backspace);
        assert!(p.pick().is_some());
    }

    #[test]
    fn up_and_down_wrap_and_skip_disabled_rows() {
        let mut list = entries();
        list[1].enabled = false;
        let mut p = Palette::open(list, 0);
        p.key(Key::Down);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Cmd(Cmd::Settings))));
        p.key(Key::Up);
        p.key(Key::Up);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Insert)));
        p.key(Key::Down);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Cmd(Cmd::SplitRight))));
    }

    #[test]
    fn enter_runs_an_action_and_esc_closes() {
        let mut p = Palette::open(entries(), 0);
        assert_eq!(p.key(Key::Enter), Out::Run(Id::Cmd(Cmd::SplitRight)));
        assert_eq!(p.key(Key::Esc), Out::Close);
    }

    #[test]
    fn an_update_keeps_the_selection_on_its_row() {
        let mut p = Palette::open(entries(), 0);
        p.key(Key::Down);
        let mut next = entries();
        next.insert(0, cmd(Cmd::NewWindow, "New window"));
        p.update(next);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Cmd(Cmd::SplitDown))));
        let mut gone = entries();
        gone.remove(1);
        p.update(gone);
        assert_eq!(p.pick(), Some(Pick::Run(Id::Cmd(Cmd::SplitRight))));
    }

    #[test]
    fn go_to_directory_asks_for_the_folder_then_lists_it() {
        let p = browsing(false);
        let lines = p.lines();
        assert!(matches!(&lines[0], Line::Head(h) if h == "Folders"));
        assert_eq!(
            labels(&p),
            ["This folder", "..", "docs/", "src/"],
            "folders only, dot entries hidden, This folder and .. first"
        );
        assert_eq!(p.pick(), Some(Pick::Here));
        assert_eq!(p.folder(), Some("/home/me"));
        let last = lines.last().expect("a last line");
        assert!(
            matches!(last, Line::Note(n) if n.title == "4 more not shown, type to narrow"),
            "{last:?}"
        );
    }

    #[test]
    fn insert_path_lists_files_and_dots_show_after_a_dot() {
        let mut p = browsing(true);
        assert!(matches!(&p.lines()[0], Line::Head(h) if h == "Folders and files"));
        assert!(labels(&p).contains(&"README.md".into()));
        assert!(!labels(&p).contains(&".env".into()));
        p.key(Key::Char(".".into()));
        assert_eq!(labels(&p)[..2], [".git/", ".env"]);
    }

    #[test]
    fn enter_types_the_path_and_tab_goes_in() {
        let mut p = browsing(false);
        assert_eq!(p.key(Key::Enter), Out::Type("cd /home/me".into()));
        p.key(Key::Down);
        p.key(Key::Down);
        assert_eq!(p.pick(), Some(Pick::Entry(Kind::Dir, "docs".into())));
        assert_eq!(p.key(Key::Enter), Out::Type("cd /home/me/docs".into()));
        let Out::List { ask, path } = p.key(Key::Tab) else {
            panic!("no listing asked");
        };
        assert_eq!(path.as_deref(), Some("/home/me/docs"));
        assert!(matches!(&p.lines()[..], [Line::Note(n)] if n.spinner));
        p.listed(ask, Ok("/home/me/docs\nf\ta b.txt".into()));
        assert_eq!(p.folder(), Some("/home/me/docs"));
        assert_eq!(labels(&p), ["This folder", ".."], "no folders in it");
    }

    #[test]
    fn insert_quotes_and_never_presses_enter() {
        let mut p = browsing(true);
        select(&mut p, "README.md");
        assert_eq!(p.pick(), Some(Pick::Entry(Kind::File, "README.md".into())));
        assert_eq!(p.key(Key::Enter), Out::Type("/home/me/README.md".into()));
        let q = |p| quote(p).expect("typable");
        assert_eq!(q("/a b/it's"), "'/a b/it'\\''s'");
        assert_eq!(q("/plain/dir-1.txt"), "/plain/dir-1.txt");
        assert_eq!(q("/x*"), "'/x*'");
        assert_eq!(q("/$(touch x)"), "'/$(touch x)'");
        assert_eq!(q("C:\\Program Files\\x"), "\"C:\\Program Files\\x\"");
        assert_eq!(q("C:\\dev"), "C:\\dev");
    }

    #[test]
    fn backspace_on_an_empty_filter_goes_up_and_an_old_answer_is_ignored() {
        let mut p = browsing(false);
        let Out::List { ask, path } = p.key(Key::Backspace) else {
            panic!("no listing asked");
        };
        assert_eq!(path.as_deref(), Some("/home"));
        p.listed(ask - 1, Ok("/elsewhere".into()));
        assert!(matches!(&p.lines()[..], [Line::Note(n)] if n.spinner));
        p.listed(ask, Ok("/home\nd\tme".into()));
        assert_eq!(p.folder(), Some("/home"));
    }

    #[test]
    fn the_root_has_no_up_row() {
        let mut p = browsing(false);
        p.listed(p.asks, Ok("/\nd\tusr".into()));
        assert_eq!(labels(&p), ["This folder", "usr/"]);
        assert_eq!(p.key(Key::Backspace), Out::Stay);
    }

    #[test]
    fn a_refused_listing_shows_the_alert_and_backspace_still_goes_up() {
        let mut p = browsing(false);
        p.key(Key::Down);
        p.key(Key::Down);
        let Out::List { ask, path } = p.key(Key::Tab) else {
            panic!("no listing asked");
        };
        assert_eq!(path.as_deref(), Some("/home/me/docs"));
        p.listed(ask, Err("terminal.list-dir: no such folder".into()));
        let lines = p.lines();
        assert!(
            matches!(&lines[..], [Line::Note(n)] if n.alert && n.title == "Can't read /home/me/docs")
        );
    }

    #[test]
    fn backspace_leaves_a_refused_folder_for_its_parent() {
        let mut p = browsing(false);
        select(&mut p, "docs/");
        let Out::List { ask, .. } = p.key(Key::Tab) else {
            panic!("no listing asked");
        };
        p.listed(ask, Err("permission denied".into()));
        let Out::List { path, .. } = p.key(Key::Backspace) else {
            panic!("backspace stayed");
        };
        assert_eq!(path.as_deref(), Some("/home/me"));
    }

    #[test]
    fn a_new_palette_numbers_its_requests_after_the_last() {
        let mut first = Palette::open(entries(), 0);
        type_in(&mut first, "Go");
        let Out::List { ask: old, .. } = first.key(Key::Enter) else {
            panic!("no listing asked");
        };
        let mut second = Palette::open(entries(), first.asks());
        type_in(&mut second, "Go");
        let Out::List { ask: new, .. } = second.key(Key::Enter) else {
            panic!("no listing asked");
        };
        assert_ne!(old, new);
        second.listed(old, Ok("/stale".into()));
        assert!(matches!(&second.lines()[..], [Line::Note(n)] if n.spinner));
    }

    #[test]
    fn an_empty_folder_says_so() {
        let mut p = browsing(false);
        p.listed(p.asks, Ok("/home/me/empty".into()));
        let lines = p.lines();
        assert!(
            matches!(&lines[..], [Line::Head(_), Line::Item(here), Line::Item(up), Line::Note(n)]
                if here.pick == Pick::Here && up.pick == Pick::Up && n.title == "Empty folder"),
            "{lines:?}"
        );
        let mut p = browsing(false);
        p.listed(p.asks, Ok("/".into()));
        assert!(
            matches!(&p.lines()[..], [Line::Head(_), Line::Item(_), Line::Note(n)]
            if n.title == "Empty folder")
        );
    }

    #[test]
    fn a_listing_is_read_with_its_counts_and_escapes() {
        let list = Listing::parse("/a\\\\b\nd\tx\\ty\nf\tz\n>\t20000\n+\t5\n!\t2\nzz\tskip\n")
            .expect("parsed");
        assert_eq!(list.path, "/a\\b");
        assert_eq!(
            list.items,
            [(Kind::Dir, "x\ty".to_owned()), (Kind::File, "z".to_owned())]
        );
        assert_eq!(
            (list.more, list.at_least, list.unreadable),
            (20005, true, 2)
        );
        assert_eq!(unescape("a\\u{1b}b").as_deref(), Some("a\u{1b}b"));
        assert_eq!(unescape("x\\u{61"), None, "unterminated escape");
        assert!(Listing::parse("").is_err());
    }

    #[test]
    fn a_name_that_would_run_or_press_a_key_is_never_typed() {
        for bad in [
            "/x\rtouch pwned\r",
            "/x\ny",
            "/x\u{1b}[31m",
            "/x\u{7f}",
            "/x\u{85}",
            "/x\ty",
            "C:\\x\\$(calc)",
            "C:\\x\\a`id`",
            "C:\\x\\%PATH%",
            "C:\\x\\a!b",
            "C:\\x\\a\"b",
        ] {
            assert_eq!(quote(bad), None, "{bad:?}");
        }
        assert_eq!(quote("C:\\x\\a'b").as_deref(), Some("\"C:\\x\\a'b\""));
    }

    #[test]
    fn a_hostile_listed_name_is_marked_and_enter_types_nothing() {
        let mut p = browsing(true);
        p.listed(p.asks, Ok("/h\nf\tx\\rtouch pwned\\r\nd\tok".into()));
        select(&mut p, "x\rtouch pwned\r");
        let lines = p.lines();
        assert!(
            matches!(&lines[p.selected_line()], Line::Item(i) if i.detail == "Can't type this name")
        );
        assert_eq!(p.key(Key::Enter), Out::Stay);
        select(&mut p, "ok/");
        assert_eq!(p.key(Key::Enter), Out::Type("/h/ok".into()));
    }

    #[test]
    fn a_posix_path_with_a_colon_second_is_not_a_drive() {
        assert!(!windows("/:proj"));
        assert!(!windows("a:b"));
        assert!(windows("C:"));
        assert!(windows("c:/x"));
        assert!(windows("\\\\host\\share"));
        assert_eq!(join("/:proj", "x"), "/:proj/x");
        assert_eq!(parent("/:proj/x"), Some("/:proj".into()));
    }

    #[test]
    fn a_verbatim_windows_path_is_read_plain() {
        // As the server writes it: each backslash doubled.
        let wire = |p: &str| format!("{}\nd\ty", p.replace('\\', "\\\\"));
        let list = Listing::parse(&wire(r"\\?\C:\dev\x")).expect("parsed");
        assert_eq!(list.path, r"C:\dev\x");
        let unc = Listing::parse(&wire(r"\\?\UNC\h\s\d")).expect("parsed");
        assert_eq!(unc.path, r"\\h\s\d");
        assert_eq!(parent(&list.path), Some(r"C:\dev".into()));
    }

    #[test]
    fn a_bad_escape_or_count_is_one_unreadable_entry_not_a_swallowed_line() {
        let list =
            Listing::parse("/a\nd\tx\\uZZ\nd\tok\n!\t18446744073709551615\n+\t5").expect("parsed");
        assert_eq!(list.items, [(Kind::Dir, "ok".to_owned())]);
        assert_eq!(list.unreadable, usize::MAX);
        assert_eq!(list.hidden(), usize::MAX);
        assert!(Listing::parse("/a\\q").is_err());
    }

    #[test]
    fn a_refusal_shows_the_hosts_reason() {
        let mut p = browsing(false);
        p.listed(p.asks, Err("terminal.list-dir: permission denied".into()));
        assert!(matches!(&p.lines()[..], [Line::Note(n)]
            if n.hint.as_deref() == Some("terminal.list-dir: permission denied")));
    }

    #[test]
    fn parents_and_joins_follow_the_path_style() {
        assert_eq!(parent("/a/b"), Some("/a".into()));
        assert_eq!(parent("/a"), Some("/".into()));
        assert_eq!(parent("/"), None);
        assert_eq!(parent("C:\\a\\b"), Some("C:\\a".into()));
        assert_eq!(parent("C:\\a"), Some("C:\\".into()));
        assert_eq!(parent("C:\\"), None);
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("C:\\", "a"), "C:\\a");
        assert_eq!(join("C:\\a", "b"), "C:\\a\\b");
    }

    #[test]
    fn a_click_picks_and_the_chevron_opens() {
        let mut p = browsing(false);
        let at = p
            .lines()
            .iter()
            .position(|l| matches!(l, Line::Item(i) if i.label == "docs/"))
            .expect("docs");
        assert!(matches!(p.click(at, true), Out::List { .. }));
        let mut p = browsing(false);
        assert_eq!(p.click(at, false), Out::Type("cd /home/me/docs".into()));
        assert_eq!(p.click(0, false), Out::Stay, "a header picks nothing");
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_in_view() {
        let many: Vec<Entry> = (0..30)
            .map(|i| cmd(Cmd::Settings, &format!("row {i}")))
            .collect();
        let mut p = Palette::open(many, 0);
        type_in(&mut p, "row");
        for _ in 0..12 {
            p.key(Key::Down);
        }
        let lines = p.lines();
        let seen = p.visible(&lines);
        assert!(seen.contains(&p.selected_line()), "{seen:?}");
        assert!(p.top() > 0);
        p.key(Key::PageUp);
        assert!(p.visible(&p.lines()).contains(&p.selected_line()));
    }
}
