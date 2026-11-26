//! The workspace tree (card #1573, D337): a read-only tree of a block's
//! workspace, built from `terminal.workspace-list` replies. This module is
//! pure: it parses replies, holds the tree and its cursor, and renders text
//! rows. It sends nothing; a caller fetches the path [`Tree::enter`] names
//! and hands the reply to [`Tree::load`].
//!
//! Rendering clips by terminal cell width (`char_width`); combining marks and
//! grapheme clusters are not joined, which the server's escaping makes rare.

use rusty_term_core::char_width;
use std::collections::{HashMap, HashSet};

/// What a name is, as the server's `KIND` column says. A link is never
/// followed, so only a directory can be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Link,
    Other,
}

/// How a listing was cut short, from the server's trailer lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cut {
    /// Entries read and left out (`+`).
    pub left_out: usize,
    /// Entries left out because the directory is bigger than the server
    /// reads (`>`): at least this many, counting ones it never read.
    pub more: usize,
    /// Entries it could not read or whose names are not UTF-8 (`!`).
    pub unreadable: usize,
}

impl Cut {
    fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// One parsed reply: the directory's entries, as the server sorted them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The path under the workspace the reply names (`.` for the root).
    pub path: String,
    pub entries: Vec<(Kind, String)>,
    pub cut: Cut,
}

/// A reply that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed(pub String);

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unreadable workspace listing: {}", self.0)
    }
}

impl std::error::Error for Malformed {}

/// A name or path part the client will not draw or fetch: empty, `.`, `..`,
/// with a `/`, or with any control character (the server escapes those, so a
/// raw one means a server this client should not trust).
fn bad_name(raw: &str) -> bool {
    let name = unescape(raw);
    matches!(name.as_str(), "" | "." | "..")
        || name.contains('/')
        || raw.chars().any(char::is_control)
        || name.contains('\0')
}

/// Reads a `terminal.workspace-list` reply. Names stay as the server wrote
/// them, escaped, so they are safe to draw; [`unescape`] gives the real one.
///
/// # Errors
/// [`Malformed`] on an empty reply, an unknown kind, a bad trailer count or
/// a path or name that is unsafe to draw or fetch.
pub fn parse(reply: &str) -> Result<Listing, Malformed> {
    let mut lines = reply.lines();
    let path = lines
        .next()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Malformed("no path line".into()))?;
    if path != "." && path.split('/').any(bad_name) || path.chars().any(char::is_control) {
        return Err(Malformed(format!("bad path `{}`", path.escape_debug())));
    }
    let mut listing = Listing {
        path: path.to_owned(),
        entries: Vec::new(),
        cut: Cut::default(),
    };
    for line in lines {
        let (head, rest) = line
            .split_once('\t')
            .ok_or_else(|| Malformed(format!("no tab in `{line}`")))?;
        let count = |rest: &str| {
            rest.parse::<usize>()
                .map_err(|_| Malformed(format!("bad count `{rest}`")))
        };
        if matches!(head, "d" | "f" | "l" | "o") && bad_name(rest) {
            return Err(Malformed(format!("bad name `{}`", rest.escape_debug())));
        }
        match head {
            "d" => listing.entries.push((Kind::Dir, rest.to_owned())),
            "f" => listing.entries.push((Kind::File, rest.to_owned())),
            "l" => listing.entries.push((Kind::Link, rest.to_owned())),
            "o" => listing.entries.push((Kind::Other, rest.to_owned())),
            "+" => listing.cut.left_out = count(rest)?,
            ">" => listing.cut.more = count(rest)?,
            "!" => listing.cut.unreadable = count(rest)?,
            other => return Err(Malformed(format!("unknown kind `{other}`"))),
        }
    }
    Ok(listing)
}

/// The inverse of the server's escaping: `\\`, `\t`, `\n`, `\r` and
/// `\u{HEX}`. Text that is not an escape is kept as it is.
#[must_use]
pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let braced = chars
                    .as_str()
                    .strip_prefix('{')
                    .and_then(|r| r.split_once('}'));
                let Some((hex, rest)) = braced else {
                    out.push_str("\\u");
                    continue;
                };
                match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => out.push_str(&format!("\\u{{{hex}}}")),
                }
                chars = rest.chars();
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// What `terminal.workspace-stat` says of one entry, as the tree shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub size: u64,
    /// Seconds since it was modified, when the reply was read.
    pub age: u64,
}

impl Stat {
    /// "4.2 KiB, 3 h ago".
    fn line(self) -> String {
        let age = match self.age {
            0..=59 => "now".to_owned(),
            60..=3599 => format!("{} min ago", self.age / 60),
            3600..=86399 => format!("{} h ago", self.age / 3600),
            _ => format!("{} d ago", self.age / 86400),
        };
        let mut size = self.size as f64;
        let mut unit = 0;
        while size >= 1024.0 && unit < 4 {
            size /= 1024.0;
            unit += 1;
        }
        let size = match unit {
            0 => format!("{} B", self.size),
            _ => format!("{size:.1} {}", ["", "KiB", "MiB", "GiB", "TiB"][unit]),
        };
        format!("{size}, {age}")
    }
}

/// Reads a `terminal.workspace-stat` reply (`KIND<TAB>SIZE<TAB>MTIME`) at
/// time `now`, in seconds since the epoch.
///
/// # Errors
/// [`Malformed`] when it is not three tab-separated fields with numbers
/// for the last two.
pub fn parse_stat(reply: &str, now: u64) -> Result<Stat, Malformed> {
    let mut f = reply.trim_end().split('\t');
    let num = |f: Option<&str>| {
        f.and_then(|t| t.parse::<u64>().ok())
            .ok_or_else(|| Malformed(format!("bad stat `{}`", reply.escape_debug())))
    };
    let _kind = f.next();
    let size = num(f.next())?;
    let mtime = num(f.next())?;
    Ok(Stat {
        size,
        age: now.saturating_sub(mtime),
    })
}

/// What changed under the workspace, from `terminal.workspace-touched`:
/// the real path of each entry git reports, and every directory above one.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// Path (empty for the workspace) to its one-letter status.
    exact: HashMap<String, char>,
    /// Directories holding a change.
    within: HashSet<String>,
    /// Paths git named that are not UTF-8.
    pub unreadable: usize,
}

/// One letter for git's two status letters (a space shown as `.`): `?` for
/// untracked, else the work tree's, else the index's.
fn letter(code: &str) -> char {
    let mut c = code.chars();
    match (c.next(), c.next()) {
        (Some('?'), _) => '?',
        (_, Some(y)) if y != '.' => y,
        (Some(x), _) => x,
        _ => '?',
    }
}

/// Reads a `terminal.workspace-touched` reply.
///
/// # Errors
/// [`Malformed`] on a first line that is not `.`, a line without a two
/// letter code and a tab, or a path that is unsafe to draw.
pub fn parse_touched(reply: &str) -> Result<Touched, Malformed> {
    let mut lines = reply.lines();
    if lines.next() != Some(".") {
        return Err(Malformed("touched reply does not start with `.`".into()));
    }
    let mut out = Touched::default();
    for line in lines {
        let (code, path) = line
            .split_once('\t')
            .ok_or_else(|| Malformed(format!("no tab in `{}`", line.escape_debug())))?;
        if code == "!" {
            out.unreadable = path
                .parse()
                .map_err(|_| Malformed(format!("bad count `{path}`")))?;
            continue;
        }
        if code.chars().count() != 2 {
            return Err(Malformed(format!("bad status `{}`", code.escape_debug())));
        }
        let path = path.trim_end_matches('/');
        if path != "." && path.split('/').any(bad_name) || path.chars().any(char::is_control) {
            return Err(Malformed(format!("bad path `{}`", path.escape_debug())));
        }
        let real = if path == "." {
            String::new()
        } else {
            unescape(path)
        };
        let mut above = String::new();
        out.within.insert(above.clone());
        for part in real.split('/').filter(|p| !p.is_empty()) {
            if !above.is_empty() {
                above.push('/');
            }
            above.push_str(part);
            out.within.insert(above.clone());
        }
        out.exact.insert(real, letter(code));
    }
    Ok(out)
}

impl Touched {
    /// The letter drawn beside `path` (real names, `/`-joined): its own
    /// status, or `*` for a directory with a change below it.
    fn mark(&self, path: &str, dir: bool) -> Option<char> {
        self.exact
            .get(path)
            .copied()
            .or_else(|| (dir && self.within.contains(path)).then_some('*'))
    }
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    /// Escaped, as drawn.
    name: String,
    /// `None` until a listing arrives; only a directory has any.
    children: Option<Vec<Node>>,
    cut: Cut,
    open: bool,
}

impl Node {
    fn of((kind, name): &(Kind, String)) -> Self {
        Self {
            kind: *kind,
            name: name.clone(),
            children: None,
            cut: Cut::default(),
            open: false,
        }
    }
}

/// What a key on the cursor's row asks of the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// Nothing, or the tree changed on its own.
    Nothing,
    /// List this path (relative to the workspace, real names, `/`-joined;
    /// empty for the root) and pass the reply to [`Tree::load`].
    Fetch(String),
}

/// The tree and its cursor.
#[derive(Debug)]
pub struct Tree {
    root: Node,
    touched: Touched,
    /// Index into the selectable rows (everything but cut summaries).
    cursor: usize,
}

/// One drawn row: depth, the node's place and what to show.
struct Row {
    depth: usize,
    /// Child indices from the root.
    at: Vec<usize>,
    text: String,
    /// The cut-summary line: drawn, never under the cursor.
    summary: bool,
}

impl Tree {
    /// A tree holding the root listing a first reply gave.
    ///
    /// # Errors
    /// [`Malformed`] when the listing is not the root (`.`): paths would
    /// then be built from the wrong place.
    pub fn new(root: &Listing) -> Result<Self, Malformed> {
        if root.path != "." {
            return Err(Malformed(format!(
                "first listing is `{}`, not the root",
                root.path
            )));
        }
        let mut node = Node {
            kind: Kind::Dir,
            name: root.path.clone(),
            children: None,
            cut: Cut::default(),
            open: true,
        };
        fill(&mut node, root);
        Ok(Self {
            root: node,
            touched: Touched::default(),
            cursor: 0,
        })
    }

    /// Puts `listing` under the directory it names. A path that is no longer
    /// in the tree (a stale reply) is ignored.
    pub fn load(&mut self, listing: &Listing) {
        // The reply's path is escaped like its names: decode each part.
        let names: Vec<String> = match listing.path.as_str() {
            "." => Vec::new(),
            p => p.split('/').map(unescape).collect(),
        };
        let held = self
            .selectable()
            .into_iter()
            .nth(self.cursor)
            .map(|r| r.at)
            .unwrap_or_default();
        let mut node = &mut self.root;
        for name in &names {
            let Some(next) = node
                .children
                .as_mut()
                .and_then(|c| c.iter_mut().find(|n| unescape(&n.name) == *name))
            else {
                return;
            };
            node = next;
        }
        // A repeat reply must not rebuild a subtree already read.
        if node.kind != Kind::Dir || node.children.is_some() {
            return;
        }
        fill(node, listing);
        node.open = true;
        self.reseat(held);
    }

    /// Puts the cursor back on the node at `held`, or else on its nearest
    /// surviving ancestor, so a late reply never moves the selection.
    fn reseat(&mut self, mut held: Vec<usize>) {
        let rows = self.selectable();
        loop {
            if let Some(i) = rows.iter().position(|r| r.at == held) {
                self.cursor = i;
                return;
            }
            if held.pop().is_none() {
                self.cursor = self.cursor.min(rows.len().saturating_sub(1));
                return;
            }
        }
    }

    /// Moves the cursor `by` rows, held to the first and last.
    pub fn step(&mut self, by: isize) {
        let last = self.selectable().len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Opens the directory under the cursor: a loaded one toggles, one not
    /// yet read asks for its listing. Files and links do nothing.
    pub fn enter(&mut self) -> Want {
        let Some(at) = self.selectable().into_iter().nth(self.cursor).map(|r| r.at) else {
            return Want::Nothing;
        };
        let path = self.path_of(&at);
        let Some(node) = self.node_mut(&at) else {
            return Want::Nothing;
        };
        match (node.kind, node.children.is_some()) {
            (Kind::Dir, true) => {
                node.open = !node.open;
                Want::Nothing
            }
            (Kind::Dir, false) => Want::Fetch(path),
            _ => Want::Nothing,
        }
    }

    /// Closes the open directory under the cursor, or else moves to its
    /// parent.
    pub fn leave(&mut self) {
        let Some(row) = self.selectable().into_iter().nth(self.cursor) else {
            return;
        };
        let Some(node) = self.node_mut(&row.at) else {
            return;
        };
        if node.open && !row.at.is_empty() {
            node.open = false;
        } else if let Some((_, parent)) = row.at.split_last() {
            let target = parent.to_vec();
            if let Some(i) = self.selectable().iter().position(|r| r.at == target) {
                self.cursor = i;
            }
        }
    }

    /// Marks what changed, as `terminal.workspace-touched` reported.
    pub fn set_touched(&mut self, touched: Touched) {
        self.touched = touched;
    }

    /// The path of the row under the cursor, as [`Want::Fetch`] writes it.
    #[must_use]
    pub fn selected_path(&self) -> String {
        self.selectable()
            .into_iter()
            .nth(self.cursor)
            .map(|r| self.path_of(&r.at))
            .unwrap_or_default()
    }

    /// The rows to draw in `height` lines of `width` columns, scrolled to
    /// keep the cursor in view. The cursor row starts with `>`.
    #[must_use]
    pub fn render(&self, width: usize, height: usize) -> Vec<String> {
        let rows = self.rows();
        // The cursor's place among all rows, to scroll by.
        let mut seen = 0;
        let mut here = 0;
        let marked: Vec<bool> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let on = !r.summary && seen == self.cursor;
                if !r.summary {
                    seen += 1;
                }
                if on {
                    here = i;
                }
                on
            })
            .collect();
        // Keep the cut summary right under the cursor in view, so a long
        // listing never looks complete.
        let end = here + 1 + rows[here + 1..].iter().take_while(|r| r.summary).count();
        // The selected row first, then as many summaries as fit.
        let top = end.saturating_sub(height.max(1)).min(here);
        rows.iter()
            .zip(marked)
            .skip(top)
            .take(height)
            .map(|(r, on)| {
                let mark = if on { '>' } else { ' ' };
                clip(&format!("{mark}{}{}", "  ".repeat(r.depth), r.text), width)
            })
            .collect()
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        walk(&self.root, 0, &mut Vec::new(), "", &self.touched, &mut rows);
        rows
    }

    fn selectable(&self) -> Vec<Row> {
        self.rows().into_iter().filter(|r| !r.summary).collect()
    }

    fn node_mut(&mut self, at: &[usize]) -> Option<&mut Node> {
        let mut node = &mut self.root;
        for i in at {
            node = node.children.as_mut()?.get_mut(*i)?;
        }
        Some(node)
    }

    fn path_of(&self, at: &[usize]) -> String {
        let mut node = &self.root;
        let mut parts = Vec::new();
        for i in at {
            let Some(next) = node.children.as_ref().and_then(|c| c.get(*i)) else {
                break;
            };
            parts.push(unescape(&next.name));
            node = next;
        }
        parts.join("/")
    }
}

/// `line` cut to `width` terminal cells, never splitting a wide character.
fn clip(line: &str, width: usize) -> String {
    let mut used = 0;
    line.chars()
        .take_while(|c| {
            used += char_width(*c);
            used <= width
        })
        .collect()
}

fn fill(node: &mut Node, listing: &Listing) {
    node.children = Some(listing.entries.iter().map(Node::of).collect());
    node.cut = listing.cut;
}

fn walk(
    node: &Node,
    depth: usize,
    at: &mut Vec<usize>,
    path: &str,
    touched: &Touched,
    rows: &mut Vec<Row>,
) {
    let sign = match (node.kind, node.open) {
        (Kind::Dir, true) => "v ",
        (Kind::Dir, false) => "+ ",
        (Kind::Link, _) => "@ ",
        (Kind::Other, _) => "? ",
        (Kind::File, _) => "  ",
    };
    let slash = if node.kind == Kind::Dir { "/" } else { "" };
    let mark = touched
        .mark(path, node.kind == Kind::Dir)
        .map_or_else(String::new, |c| format!("  {c}"));
    rows.push(Row {
        depth,
        at: at.clone(),
        text: format!("{sign}{}{slash}{mark}", node.name),
        summary: false,
    });
    if !node.open {
        return;
    }
    for (i, child) in node.children.iter().flatten().enumerate() {
        at.push(i);
        let name = unescape(&child.name);
        let below = if path.is_empty() {
            name
        } else {
            format!("{path}/{name}")
        };
        walk(child, depth + 1, at, &below, touched, rows);
        at.pop();
    }
    if !node.cut.is_empty() {
        rows.push(Row {
            depth: depth + 1,
            at: at.clone(),
            text: cut_line(node.cut),
            summary: true,
        });
    }
}

/// "... 3 not shown, 1 unreadable" for a cut listing.
fn cut_line(cut: Cut) -> String {
    let mut parts = Vec::new();
    if cut.left_out > 0 {
        parts.push(format!("{} not shown", cut.left_out));
    }
    if cut.more > 0 {
        parts.push(format!("{}+ not shown", cut.more));
    }
    if cut.unreadable > 0 {
        parts.push(format!("{} unreadable", cut.unreadable));
    }
    format!("... {}", parts.join(", "))
}

/// What a key batch did to the open [`Explorer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Still open; draw it again.
    Open,
    /// Closed.
    Leave,
    /// Still open; list this path (see [`Want::Fetch`]) and pass the reply to
    /// [`Explorer::load`], or the refusal to [`Explorer::refuse`].
    Fetch(String),
}

/// What the overlay wants asked besides listings, once per ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// `terminal.workspace-touched`, for the change marks.
    Touched,
    /// `terminal.workspace-stat` of this path (`.` for the workspace).
    Stat(String),
}

/// The overlay's state: the tree once the root listing arrives, one listing
/// in flight at most, the stats read, and the last refusal.
#[derive(Debug, Default)]
pub struct Explorer {
    tree: Option<Tree>,
    waiting: bool,
    note: Option<String>,
    /// Whether the server answers [`Call`]s.
    status: bool,
    touched_asked: bool,
    /// The path whose stat is in flight.
    stat_wait: Option<String>,
    /// Each stat read, or `None` for one refused: asked once.
    stats: HashMap<String, Option<Stat>>,
}

impl Explorer {
    /// An explorer that has asked for the root and shows nothing yet.
    /// `status`: the server answers stat and touched calls.
    #[must_use]
    pub fn opened(status: bool) -> Self {
        Self {
            waiting: true,
            status,
            ..Self::default()
        }
    }

    /// Takes a listing reply: the first one is the root.
    pub fn load(&mut self, reply: &str) {
        self.waiting = false;
        let listing = match parse(reply) {
            Ok(listing) => listing,
            Err(e) => return self.note = Some(e.to_string()),
        };
        self.note = None;
        match self.tree.as_mut() {
            Some(tree) => tree.load(&listing),
            None => match Tree::new(&listing) {
                Ok(tree) => self.tree = Some(tree),
                Err(e) => self.note = Some(e.to_string()),
            },
        }
    }

    /// The next call to make, if any: the change marks once the tree
    /// stands, then the stat of the row under the cursor while none is in
    /// flight and it has not been asked for. Call it after each key batch
    /// and reply; it marks what it returns as asked.
    pub fn wants(&mut self) -> Option<Call> {
        let tree = self.tree.as_ref().filter(|_| self.status)?;
        if !self.touched_asked {
            self.touched_asked = true;
            return Some(Call::Touched);
        }
        if self.stat_wait.is_some() {
            return None;
        }
        let path = match tree.selected_path() {
            p if p.is_empty() => ".".to_owned(),
            p => p,
        };
        if self.stats.contains_key(&path) {
            return None;
        }
        self.stat_wait = Some(path.clone());
        Some(Call::Stat(path))
    }

    /// Takes the change marks. A bad reply leaves the tree unmarked.
    pub fn load_touched(&mut self, reply: &str) {
        match (parse_touched(reply), self.tree.as_mut()) {
            (Ok(touched), Some(tree)) => {
                if touched.unreadable > 0 {
                    self.note = Some(format!(
                        "{} changed paths are not UTF-8",
                        touched.unreadable
                    ));
                }
                tree.set_touched(touched);
            }
            (Err(e), _) => self.note = Some(e.to_string()),
            _ => {}
        }
    }

    /// Takes the stat of the path asked for, read at `now`; `None` for a
    /// refusal or a reply it cannot read, which are not asked for again.
    pub fn load_stat(&mut self, reply: Option<&str>, now: u64) {
        let Some(path) = self.stat_wait.take() else {
            return;
        };
        let stat = reply.and_then(|r| parse_stat(r, now).ok());
        self.stats.insert(path, stat);
    }

    /// The size and age of the row under the cursor, once read.
    fn stat_line(&self) -> Option<String> {
        let path = match self.tree.as_ref()?.selected_path() {
            p if p.is_empty() => ".".to_owned(),
            p => p,
        };
        self.stats.get(&path).copied().flatten().map(Stat::line)
    }

    /// Takes a refusal in place of a reply; one line, since the server's
    /// text may carry anything.
    pub fn refuse(&mut self, why: &str) {
        self.waiting = false;
        let line: String = why
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.note = Some(line);
    }

    /// Whether a request is in flight.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// Acts on the keys in `bytes`: `j`/`k` or the arrows move, Enter, `l`
    /// or right opens, `h` or left goes up, `q` or a lone Esc leaves. Stops
    /// at the first key that asks for a listing.
    pub fn keys(&mut self, bytes: &[u8]) -> Step {
        if bytes == b"\x1b" {
            return Step::Leave;
        }
        let mut i = 0;
        while i < bytes.len() {
            let key = match bytes[i..] {
                [0x1b, b'[' | b'O', last, ..] => {
                    i += 2;
                    last
                }
                [b, ..] => b,
                [] => break,
            };
            i += 1;
            match key {
                b'q' => return Step::Leave,
                b'j' | b'B' => self.tree.iter_mut().for_each(|t| t.step(1)),
                b'k' | b'A' => self.tree.iter_mut().for_each(|t| t.step(-1)),
                b'h' | b'D' => self.tree.iter_mut().for_each(Tree::leave),
                b'l' | b'C' | b'\r' | b'\n' if !self.waiting => {
                    if let Some(Want::Fetch(path)) = self.tree.as_mut().map(Tree::enter) {
                        self.waiting = true;
                        return Step::Fetch(path);
                    }
                }
                _ => {}
            }
        }
        Step::Open
    }

    /// `height` lines of `width` cells: a title, the tree and, last, the
    /// refusal or a wait notice.
    #[must_use]
    pub fn render(&self, width: usize, height: usize) -> Vec<String> {
        let mut lines = vec![clip(
            "workspace: j/k move, Enter open, h up, q leave",
            width,
        )];
        let status = match (&self.note, self.waiting) {
            (Some(note), _) => Some(note.as_str()),
            (None, true) => Some("reading..."),
            (None, false) => None,
        };
        let stat = self.stat_line();
        let foot = usize::from(status.is_some()) + usize::from(stat.is_some());
        let room = height.saturating_sub(1 + foot);
        if let Some(tree) = &self.tree {
            lines.extend(tree.render(width, room));
        }
        lines.truncate(height.saturating_sub(foot));
        lines.extend(stat.map(|s| clip(&s, width)));
        lines.extend(status.map(|s| clip(s, width)));
        lines.truncate(height);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(l: &Listing) -> Tree {
        Tree::new(l).unwrap()
    }

    fn root() -> Listing {
        parse(".\nd\tsrc\nf\tREADME.md\nl\tlink\no\tsock").unwrap()
    }

    #[test]
    fn parses_entries_and_trailers() {
        let l = parse("sub\nd\ta\nf\tb\n+\t3\n>\t2\n!\t1").unwrap();
        assert_eq!(l.path, "sub");
        assert_eq!(
            l.entries,
            [(Kind::Dir, "a".into()), (Kind::File, "b".into())]
        );
        assert_eq!(
            l.cut,
            Cut {
                left_out: 3,
                more: 2,
                unreadable: 1
            }
        );
    }

    #[test]
    fn refuses_a_reply_it_cannot_read() {
        assert!(parse("").is_err());
        assert!(parse(".\nx\tname").is_err());
        assert!(parse(".\n+\tmany").is_err());
        assert!(parse(".\nno tab").is_err());
    }

    #[test]
    fn unescape_undoes_the_servers_escaping() {
        assert_eq!(unescape(r"a\\b\tc\u{1b}d"), "a\\b\tc\u{1b}d");
        assert_eq!(unescape(r"plain"), "plain");
        assert_eq!(unescape(r"bad\u{zz}"), r"bad\u{zz}");
        assert_eq!(unescape("end\\"), "end\\");
    }

    #[test]
    fn draws_kinds_and_the_cursor() {
        let t = tree(&root());
        assert_eq!(
            t.render(40, 10),
            [
                ">v ./",
                "   + src/",
                "     README.md",
                "   @ link",
                "   ? sock"
            ]
        );
    }

    #[test]
    fn only_a_directory_opens_and_it_asks_for_its_listing_once() {
        let mut t = tree(&root());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("src".into()));
        t.load(&parse("src\nf\tlib.rs").unwrap());
        assert_eq!(t.render(40, 10)[2], "       lib.rs");
        // Loaded: Enter now toggles, with no second fetch.
        assert_eq!(t.enter(), Want::Nothing);
        assert_eq!(t.render(40, 10).len(), 5);
        t.step(1);
        t.step(1);
        assert_eq!(t.enter(), Want::Nothing, "a file does nothing");
        t.step(1);
        assert_eq!(t.enter(), Want::Nothing, "a link is never followed");
    }

    #[test]
    fn fetch_paths_use_real_names_and_nest() {
        let mut t = tree(&parse(".\nd\ta\\tb").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a\tb".into()));
        t.load(&parse("a\\tb\nd\tin").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a\tb/in".into()));
        assert_eq!(t.selected_path(), "a\tb/in");
    }

    #[test]
    fn escaped_names_attach_their_replies() {
        // Exactly as the server writes a tab and a backslash in a name.
        let mut t = tree(&parse(".\nd\ta\\tb\nd\tc\\\\d").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a\tb".into()));
        t.load(&parse("a\\tb\nf\tin").unwrap());
        assert_eq!(t.render(40, 10)[2], "       in");
        t.step(2);
        assert_eq!(t.enter(), Want::Fetch("c\\d".into()));
        t.load(&parse("c\\\\d\nf\tx").unwrap());
        assert_eq!(t.render(40, 10).len(), 5);
    }

    #[test]
    fn a_late_reply_does_not_move_the_selection() {
        let mut t = tree(&parse(".\nd\ta\nd\tb").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("a".into()));
        t.step(1);
        t.load(&parse("a\nf\tx\nf\ty").unwrap());
        assert_eq!(t.selected_path(), "b");
        assert!(t.render(40, 10).iter().any(|r| r.starts_with(">  + b/")));
    }

    #[test]
    fn the_cut_row_is_never_under_the_cursor() {
        let mut t = tree(&parse(".\nd\ta\n+\t2").unwrap());
        t.step(5);
        assert_eq!(t.selected_path(), "a");
        let rows = t.render(40, 10);
        assert!(rows[2].starts_with(' ') && rows[2].contains("not shown"));
        assert_eq!(t.enter(), Want::Fetch("a".into()), "not the root toggle");
    }

    #[test]
    fn wide_characters_are_clipped_by_cells() {
        let t = tree(&parse(".\nf\t\u{4e2d}\u{6587}").unwrap());
        // Five columns of mark, indent and sign, then two 2-cell characters:
        // 8 cells fit one, and the second is not split.
        assert_eq!(t.render(8, 10)[1], "     \u{4e2d}");
    }

    #[test]
    fn leave_closes_then_goes_to_the_parent() {
        let mut t = tree(&root());
        t.step(1);
        t.enter();
        t.load(&parse("src\nf\tlib.rs").unwrap());
        t.step(1);
        t.leave();
        assert_eq!(t.selected_path(), "src");
        t.leave();
        assert_eq!(t.render(40, 10).len(), 5, "src is closed");
    }

    #[test]
    fn a_cut_listing_says_so() {
        let t = tree(&parse(".\nf\ta\n+\t2\n!\t1").unwrap());
        assert_eq!(
            t.render(40, 10).last().unwrap(),
            "   ... 2 not shown, 1 unreadable"
        );
    }

    #[test]
    fn a_stale_reply_is_ignored_and_the_cursor_is_held() {
        let mut t = tree(&root());
        t.load(&parse("gone\nf\tx").unwrap());
        assert_eq!(t.render(40, 10).len(), 5);
        t.step(100);
        assert_eq!(t.selected_path(), "sock");
        t.step(-100);
        assert_eq!(t.selected_path(), "");
    }

    #[test]
    fn long_trees_scroll_to_the_cursor_and_lines_are_clipped() {
        let mut t = tree(&parse(".\nf\t1\nf\t2\nf\t3\nf\t4").unwrap());
        t.step(4);
        let rows = t.render(5, 3);
        assert_eq!(rows.len(), 3);
        assert!(rows[2].starts_with('>'));
        assert!(rows.iter().all(|r| r.chars().count() <= 5));
    }

    #[test]
    fn unsafe_names_and_paths_are_refused() {
        for bad in [
            ".\nd\t\u{1b}]0;x",
            ".\nd\t\u{9b}2J",
            ".\nf\t",
            ".\nf\t..",
            ".\nf\ta/b",
            ".\nf\t\\u{2f}",
            "\u{1b}\nf\ta",
            "a//b\nf\ta",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert!(parse("a/b\nf\t.hidden").is_ok());
    }

    #[test]
    fn a_first_listing_must_be_the_root() {
        assert!(Tree::new(&parse("src\nf\tlib.rs").unwrap()).is_err());
    }

    #[test]
    fn a_repeat_reply_keeps_what_was_read() {
        let mut t = tree(&parse(".\nd\ta").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a\nd\tb").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a/b\nf\tc").unwrap());
        t.load(&parse("a\nd\tb").unwrap());
        assert_eq!(t.render(40, 10).len(), 4, "b stays open with c");
    }

    #[test]
    fn the_more_trailer_is_not_called_unread() {
        let t = tree(&parse(".\nf\ta\n>\t19001").unwrap());
        assert_eq!(t.render(40, 10).last().unwrap(), "   ... 19001+ not shown");
    }

    #[test]
    fn unescape_wants_braces() {
        assert_eq!(unescape(r"\uX41}"), r"\uX41}");
        assert_eq!(unescape(r"\u{41"), r"\u{41");
    }

    #[test]
    fn the_cut_summary_is_reachable_in_a_short_view() {
        let mut t = tree(&parse(".\nf\ta\nf\tb\nf\tc\n+\t2").unwrap());
        t.step(10);
        let rows = t.render(40, 3);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].starts_with('>') && rows[2].contains("2 not shown"));
    }

    #[test]
    fn the_selection_stays_in_view_before_summaries() {
        let mut t = tree(&parse(".\nf\ta\nf\tb\n+\t2").unwrap());
        t.step(10);
        assert_eq!(t.render(40, 1), [">    b"]);
        // Adjacent summaries of nested cut listings: two of them under `x`.
        let mut t = tree(&parse(".\nd\ta\n+\t1").unwrap());
        t.step(1);
        t.enter();
        t.load(&parse("a\nf\tx\n+\t3").unwrap());
        t.step(1);
        assert_eq!(t.selected_path(), "a/x");
        let two = t.render(40, 2);
        assert!(two[0].starts_with('>') && two[1].contains("3 not shown"));
        let three = t.render(40, 3);
        assert!(three[0].starts_with('>') && three[2].contains("1 not shown"));
        assert_eq!(t.render(40, 1).len(), 1);
        assert!(t.render(40, 1)[0].starts_with('>'));
    }

    #[test]
    fn the_explorer_reads_the_root_then_asks_for_a_directory_once() {
        let mut e = Explorer::opened(true);
        assert!(e.waiting());
        e.load(".\nd\tsrc\nf\ta");
        assert!(!e.waiting());
        assert_eq!(e.keys(b"j"), Step::Open);
        assert_eq!(e.keys(b"\r"), Step::Fetch("src".into()));
        // Waiting: a second Enter asks for nothing.
        assert_eq!(e.keys(b"\r"), Step::Open);
        e.load("src\nf\tlib.rs");
        assert!(e.render(40, 10).iter().any(|r| r.ends_with("lib.rs")));
    }

    #[test]
    fn keys_move_leave_and_read_arrows() {
        let mut e = Explorer::opened(true);
        e.load(".\nf\ta\nf\tb");
        assert_eq!(e.keys(b"\x1b[B\x1b[B"), Step::Open);
        assert!(e.render(40, 10).iter().any(|r| r.starts_with(">    b")));
        assert_eq!(e.keys(b"k"), Step::Open);
        assert_eq!(e.keys(b"\x1b"), Step::Leave);
        assert_eq!(e.keys(b"q"), Step::Leave);
    }

    #[test]
    fn a_refusal_is_one_clean_line_and_frees_the_next_request() {
        let mut e = Explorer::opened(true);
        e.refuse("not listed\u{1b}[2J\n");
        assert!(!e.waiting());
        let lines = e.render(40, 4);
        assert_eq!(lines.last().unwrap().trim_end(), "not listed [2J");
        assert!(lines.iter().all(|l| !l.chars().any(char::is_control)));
    }

    #[test]
    fn a_bad_first_reply_leaves_no_tree_and_says_why() {
        let mut e = Explorer::opened(true);
        e.load("src\nf\ta");
        assert!(e.render(100, 3)[1].contains("not the root"));
        e.load("garbage");
        assert!(e.render(60, 3).len() <= 3);
    }

    #[test]
    fn render_fits_its_height() {
        let mut e = Explorer::opened(true);
        e.load(".\nf\ta\nf\tb\nf\tc");
        for h in 0..6 {
            assert!(e.render(30, h).len() <= h);
        }
    }

    #[test]
    fn reads_a_stat_and_words_it() {
        let s = parse_stat("f\t4300\t1000", 1000 + 3 * 3600).unwrap();
        assert_eq!(s.line(), "4.2 KiB, 3 h ago");
        assert_eq!(parse_stat("d\t12\t5", 5).unwrap().line(), "12 B, now");
        assert!(parse_stat("f\tbig\t1", 1).is_err());
        assert!(parse_stat("f", 1).is_err());
    }

    #[test]
    fn marks_touched_files_and_the_directories_above_them() {
        let mut t = tree(&parse(".\nd\tsrc\nd\tdocs\nf\tnew").unwrap());
        t.step(1);
        assert_eq!(t.enter(), Want::Fetch("src".into()));
        t.load(&parse("src\nf\tlib.rs\nf\tok.rs").unwrap());
        t.set_touched(parse_touched(".\n.M\tsrc/lib.rs\n??\tnew\nM.\tdocs/").unwrap());
        assert_eq!(
            t.render(40, 10),
            [
                " v ./  *",
                ">  v src/  *",
                "       lib.rs  M",
                "       ok.rs",
                "   + docs/  M",
                "     new  ?"
            ]
        );
    }

    #[test]
    fn a_touched_reply_that_cannot_be_trusted_is_refused() {
        assert!(parse_touched("").is_err());
        assert!(parse_touched("x\n.M\ta").is_err());
        assert!(parse_touched(".\nM\ta").is_err());
        assert!(parse_touched(".\n.M\t../a").is_err());
        assert!(parse_touched(".\n.M\ta\u{1b}b").is_err());
        let t = parse_touched(".\n??\t.\n!\t2").unwrap();
        assert_eq!(t.unreadable, 2);
        assert_eq!(t.mark("", true), Some('?'), "the workspace itself");
    }

    #[test]
    fn wants_marks_once_then_each_cursor_rows_stat_once() {
        let mut e = Explorer::opened(true);
        assert_eq!(e.wants(), None, "no tree yet");
        e.load(".\nd\tsrc\nf\ta");
        assert_eq!(e.wants(), Some(Call::Touched));
        assert_eq!(e.wants(), Some(Call::Stat(".".into())));
        assert_eq!(e.wants(), None, "one stat in flight");
        e.load_stat(Some("d\t0\t0"), 120);
        assert_eq!(e.wants(), None, "the root's is read");
        assert_eq!(e.stat_line().as_deref(), Some("0 B, 2 min ago"));
        e.keys(b"j");
        assert_eq!(e.stat_line(), None);
        assert_eq!(e.wants(), Some(Call::Stat("src".into())));
        e.load_stat(None, 0);
        assert_eq!(e.wants(), None, "a refusal is not asked again");
        assert!(!Explorer::opened(false).wants().is_some());
    }
}
