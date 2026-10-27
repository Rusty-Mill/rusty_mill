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
            name: String::new(),
            command: String::new(),
        }
    }
}

/// The foreground process `foreground`, then its ancestors, then the
/// block's program `child`, which is always last. The walk stops at a
/// process `lookup` cannot find, or at one whose parent has gone (reparented
/// to pid 1), so only the program is listed then.
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
        let Some(proc) = lookup(pid) else { break };
        next = proc.parent.filter(|&p| p != child && p > 1);
        found.push(proc);
    }
    found.push(lookup(child).unwrap_or_else(|| Proc::unknown(child)));
    found
}

/// `chain` as `terminal.process` replies: `PID<TAB>NAME<TAB>COMMAND` per
/// line.
pub(crate) fn text(chain: &[Proc]) -> String {
    chain
        .iter()
        .map(|p| format!("{}\t{}\t{}", p.pid, p.name, p.command))
        .collect::<Vec<_>>()
        .join("\n")
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
    let parent = tail.split_whitespace().nth(1)?.parse().ok();
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
        name,
        command: args.join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: u32, name: &str) -> Proc {
        Proc {
            pid,
            parent: Some(parent),
            name: name.into(),
            command: name.into(),
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

    #[test]
    fn proc_files_are_parsed() {
        let stat = b"42 (my (odd) prog) S 7 42 42 34816 42 4194304";
        let parsed = parse(42, stat, b"/usr/bin/vim\0-u\0a b\0").unwrap();
        assert_eq!(
            parsed,
            Proc {
                pid: 42,
                parent: Some(7),
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
