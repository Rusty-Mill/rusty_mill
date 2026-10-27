//! What runs in a block, for `terminal.process` and `process-changed`: the
//! terminal's foreground process and its ancestors up to the program the
//! block started, read from the platform's process table.

/// Most processes listed above the block's own program, so a cycle in a
/// racing process table cannot loop.
const DEPTH: usize = 64;

/// One process, as `terminal.process` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proc {
    pub(crate) pid: u32,
    /// Its parent, if known.
    pub(crate) parent: Option<u32>,
    /// The session it belongs to, if known. A block's program leads its
    /// own, so the session id is the program's pid.
    pub(crate) session: Option<u32>,
    /// Its program's file name; empty if unknown.
    pub(crate) name: String,
    /// Its arguments joined by spaces; empty if unknown.
    pub(crate) command: String,
}

impl Proc {
    /// A process the platform tells nothing about.
    fn unknown(pid: u32) -> Self {
        Self {
            pid,
            parent: None,
            session: None,
            name: String::new(),
            command: String::new(),
        }
    }
}

/// The foreground process `foreground`, then its ancestors, then the
/// block's program `child`, which is always last. Only processes of the
/// block's session (the one `child` leads) are listed: the walk stops at a
/// process `lookup` cannot find or that belongs elsewhere, such as the
/// subreaper or init a process was reparented to, or a pid reused by
/// another program meanwhile.
pub(crate) fn chain(
    child: u32,
    foreground: Option<u32>,
    lookup: impl Fn(u32) -> Option<Proc>,
) -> Vec<Proc> {
    let mut found = Vec::new();
    let mut next = foreground.filter(|&pid| pid != child);
    while let Some(pid) = next {
        if found.len() == DEPTH {
            break;
        }
        let Some(proc) = lookup(pid).filter(|p| p.session == Some(child)) else {
            break;
        };
        next = proc.parent.filter(|&p| p != child);
        found.push(proc);
    }
    found.push(lookup(child).unwrap_or_else(|| Proc::unknown(child)));
    found
}

/// The foreground process group a block last announced, and its leader's
/// program name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Foreground {
    pub(crate) pid: u32,
    pub(crate) name: String,
}

impl Foreground {
    /// Takes the group `pid` that has the terminal now; whether it or its
    /// name differs from the last taken, and so is worth announcing. The
    /// name is looked up when the group changed, and on every `refresh`
    /// even when it did not: a job's leader takes the terminal before it
    /// execs its program, so its first name may be the shell's, or unknown.
    pub(crate) fn observe(
        &mut self,
        pid: u32,
        refresh: bool,
        lookup: impl Fn(u32) -> Option<Proc>,
    ) -> bool {
        if pid == self.pid && !refresh {
            return false;
        }
        let name = lookup(pid).map(|p| p.name).unwrap_or_default();
        if pid == self.pid && name == self.name {
            return false;
        }
        *self = Self { pid, name };
        true
    }
}

/// `chain` as `terminal.process` replies: `PID<TAB>NAME<TAB>COMMAND` per
/// line, the name and command [`escape`]d so neither can add a field or a
/// line.
pub(crate) fn text(chain: &[Proc]) -> String {
    chain
        .iter()
        .map(|p| format!("{}\t{}\t{}", p.pid, escape(&p.name), escape(&p.command)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` with a backslash as `\\`, a tab, newline or carriage return as
/// `\t`, `\n` or `\r`, and any other control character, such as ESC or a
/// C1 control, as `\u{HEX}`, the notation `rusty-mux-ctl` prints text
/// fields in. Reversible, and plain commands read as they are.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// Process `pid` from `/proc`; `None` if it has gone.
#[cfg(target_os = "linux")]
pub(crate) fn lookup(pid: u32) -> Option<Proc> {
    let dir = std::path::Path::new("/proc").join(pid.to_string());
    let stat = std::fs::read(dir.join("stat")).ok()?;
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    parse(pid, &stat, &cmdline)
}

/// Nothing: only Linux's process table is read.
#[cfg(not(target_os = "linux"))]
pub(crate) fn lookup(_pid: u32) -> Option<Proc> {
    None
}

/// A process from its `/proc` `stat` and `cmdline`. The name is the first
/// argument's file name, else the kernel's `comm`, as for a kernel thread
/// or a zombie, whose command line is empty.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse(pid: u32, stat: &[u8], cmdline: &[u8]) -> Option<Proc> {
    let stat = String::from_utf8_lossy(stat);
    // `comm` is in parentheses and may hold any byte but NUL, so the
    // fields after it start at the last `)`.
    let (head, tail) = stat.rsplit_once(')')?;
    let comm = head.split_once('(')?.1;
    // After `comm`: state, parent, process group, session.
    let mut fields = tail.split_whitespace().skip(1);
    let parent = fields.next()?.parse().ok();
    let session = fields.nth(1)?.parse().ok();
    let args: Vec<String> = cmdline
        .split(|&b| b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let name = args
        .first()
        .and_then(|a| a.rsplit('/').next())
        .filter(|n| !n.is_empty())
        .unwrap_or(comm)
        .to_owned();
    Some(Proc {
        pid,
        parent,
        session,
        name,
        command: args.join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block's program in these tests.
    const CHILD: u32 = 10;

    /// A process of the block's session.
    fn proc(pid: u32, parent: u32, name: &str) -> Proc {
        Proc {
            pid,
            parent: Some(parent),
            session: Some(CHILD),
            name: name.into(),
            command: name.into(),
        }
    }

    /// A process of another session.
    fn foreign(pid: u32, parent: u32, name: &str) -> Proc {
        Proc {
            session: Some(5),
            ..proc(pid, parent, name)
        }
    }

    /// A process table of `procs`.
    fn table(procs: &[Proc]) -> impl Fn(u32) -> Option<Proc> + '_ {
        |pid| procs.iter().find(|p| p.pid == pid).cloned()
    }

    fn pids(chain: &[Proc]) -> Vec<u32> {
        chain.iter().map(|p| p.pid).collect()
    }

    #[test]
    fn the_foreground_comes_first_and_the_program_last() {
        let procs = [proc(10, 1, "sh"), proc(20, 10, "make"), proc(30, 20, "cc")];
        assert_eq!(pids(&chain(10, Some(30), table(&procs))), [30, 20, 10]);
        assert_eq!(pids(&chain(10, Some(10), table(&procs))), [10], "idle");
        assert_eq!(pids(&chain(10, None, table(&procs))), [10], "unknown");
    }

    #[test]
    fn a_broken_walk_still_ends_with_the_program() {
        let orphan = [proc(10, 1, "sh"), proc(30, 1, "daemon")];
        assert_eq!(pids(&chain(10, Some(30), table(&orphan))), [30, 10]);
        let gone = [proc(10, 1, "sh"), proc(30, 20, "cc")];
        assert_eq!(pids(&chain(10, Some(30), table(&gone))), [30, 10]);
        let nothing = chain(10, Some(30), |_| None);
        assert_eq!(nothing, [Proc::unknown(10)], "no process table");
        assert_eq!(text(&nothing), "10\t\t");
    }

    #[test]
    fn only_the_blocks_session_is_listed() {
        let subreaper = [
            proc(10, 1, "sh"),
            proc(30, 5, "job"),
            foreign(5, 1, "supervisor"),
        ];
        assert_eq!(pids(&chain(10, Some(30), table(&subreaper))), [30, 10]);
        let reused = [proc(10, 1, "sh"), foreign(30, 1, "unrelated")];
        assert_eq!(pids(&chain(10, Some(30), table(&reused))), [10]);
        let racing = [proc(10, 1, "sh"), proc(30, 20, "cc"), foreign(20, 1, "x")];
        assert_eq!(pids(&chain(10, Some(30), table(&racing))), [30, 10]);
    }

    fn named(name: &str) -> impl Fn(u32) -> Option<Proc> + '_ {
        move |pid| Some(proc(pid, CHILD, name))
    }

    #[test]
    fn a_job_that_execs_after_taking_the_terminal_is_announced_again() {
        let mut fg = Foreground {
            pid: CHILD,
            name: "sh".into(),
        };
        assert!(!fg.observe(CHILD, true, named("sh")), "idle shell");
        assert!(fg.observe(30, false, named("sh")), "forked, not yet exec'd");
        assert!(
            !fg.observe(30, false, |_| panic!("no lookup")),
            "cheap check"
        );
        assert!(fg.observe(30, true, named("sleep")), "exec'd");
        assert_eq!(fg.name, "sleep");
        assert!(!fg.observe(30, true, named("sleep")), "announced once");
        assert!(fg.observe(CHILD, false, named("sh")), "back to the shell");
    }

    #[test]
    fn an_unknown_name_is_announced_once_known() {
        let mut fg = Foreground {
            pid: CHILD,
            name: "sh".into(),
        };
        assert!(fg.observe(30, false, |_| None));
        assert_eq!(fg.name, "");
        assert!(!fg.observe(30, true, |_| None), "still unknown");
        assert!(fg.observe(30, true, named("vim")));
        assert_eq!(fg.name, "vim");
    }

    #[test]
    fn a_cycle_stops_at_the_depth_limit() {
        let cycle = [proc(10, 1, "sh"), proc(20, 30, "a"), proc(30, 20, "b")];
        assert_eq!(chain(10, Some(20), table(&cycle)).len(), DEPTH + 1);
    }

    #[test]
    fn text_has_one_tab_separated_line_per_process() {
        let procs = [proc(10, 1, "sh"), proc(20, 10, "vim")];
        assert_eq!(
            text(&chain(10, Some(20), table(&procs))),
            "20\tvim\tvim\n10\tsh\tsh"
        );
    }

    /// Undoes [`escape`], as a reader of `terminal.process` would.
    fn unescape(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('u') => {
                    let hex: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                other => out.extend(other),
            }
        }
        out
    }

    #[test]
    fn hostile_fields_stay_in_their_row_and_column() {
        let hostile = Proc {
            pid: 20,
            parent: Some(10),
            session: None,
            name: "a\tb\nc".into(),
            command: "x\ty\nz\r\\n \u{1b}]2;pwned\u{7} \u{9b}31m \u{7f}".into(),
        };
        let reply = text(&[hostile.clone(), proc(10, 1, "sh")]);
        let rows: Vec<Vec<&str>> = reply.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(rows.len(), 2, "{reply}");
        assert!(rows.iter().all(|r| r.len() == 3), "{reply}");
        assert!(
            !reply
                .chars()
                .any(|c| c.is_control() && c != '\t' && c != '\n')
        );
        assert_eq!(rows[0][1], "a\\tb\\nc");
        assert_eq!(
            rows[0][2],
            "x\\ty\\nz\\r\\\\n \\u{1b}]2;pwned\\u{7} \\u{9b}31m \\u{7f}"
        );
        assert_eq!(unescape(rows[0][1]), hostile.name);
        assert_eq!(unescape(rows[0][2]), hostile.command);
    }

    #[test]
    fn ordinary_commands_read_as_they_are() {
        let plain = Proc {
            pid: 7,
            parent: Some(1),
            session: None,
            name: "vim".into(),
            command: "/usr/bin/vim -u ~/.vimrc 'a b' \u{4e2d}.txt".into(),
        };
        assert_eq!(
            text(std::slice::from_ref(&plain)),
            format!("7\tvim\t{}", plain.command)
        );
    }

    #[test]
    fn proc_files_are_parsed() {
        let stat = b"42 (my (odd) prog) S 7 42 42 34816 42 4194304";
        let parsed = parse(42, stat, b"/usr/bin/vim\0-u\0a b\0").unwrap();
        assert_eq!(
            parsed,
            Proc {
                pid: 42,
                parent: Some(7),
                session: Some(42),
                name: "vim".into(),
                command: "/usr/bin/vim -u a b".into(),
            }
        );
        let zombie = parse(42, stat, b"").unwrap();
        assert_eq!(
            (zombie.name.as_str(), zombie.command.as_str()),
            ("my (odd) prog", "")
        );
        assert_eq!(parse(42, b"42 garbage", b""), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_can_be_looked_up() {
        let me = lookup(std::process::id()).unwrap();
        assert_eq!(
            me.parent,
            Some(rusty_libc::process::getppid().cast_unsigned())
        );
        assert!(!me.name.is_empty());
    }
}
