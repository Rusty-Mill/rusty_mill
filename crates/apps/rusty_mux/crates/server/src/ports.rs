//! The TCP ports a block's programs listen on, for `terminal.ports` (C80):
//! the listening sockets of the processes in the block's session, read from
//! `/proc` on Linux.

#[cfg(target_os = "linux")]
use crate::process::Identity;
use std::collections::BTreeSet;
#[cfg(target_os = "linux")]
use std::collections::HashSet;

/// The state column of a listening socket.
const LISTEN: &str = "0A";

/// One listening socket and the process that holds it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Listener {
    pub(crate) port: u16,
    pub(crate) pid: u32,
    pub(crate) name: String,
}

/// The `(port, inode)` of every row in the LISTEN state in one
/// `/proc/net/tcp` or `tcp6` table. A row it cannot read is skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_table(table: &str) -> Vec<(u16, u64)> {
    table.lines().skip(1).filter_map(parse_row).collect()
}

/// One table row: `sl local rem st tx:rx tr:when retrnsmt uid timeout
/// inode ...`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_row(line: &str) -> Option<(u16, u64)> {
    let mut fields = line.split_whitespace();
    let local = fields.nth(1)?;
    let state = fields.nth(1)?;
    let inode = fields.nth(5)?;
    if state != LISTEN {
        return None;
    }
    let port = u16::from_str_radix(local.rsplit_once(':')?.1, 16).ok()?;
    Some((port, inode.parse().ok()?))
}

/// One process of the block, and the sockets it has open.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) struct Member {
    pub(crate) pid: u32,
    pub(crate) name: String,
    pub(crate) sockets: BTreeSet<u64>,
}

/// The listeners among `table` that a member holds, by port, then pid.
/// One that listens on IPv4 and IPv6 at once, which are two rows, is listed
/// once.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn assemble(members: &[Member], table: &[(u16, u64)]) -> Vec<Listener> {
    let found: BTreeSet<Listener> = members
        .iter()
        .flat_map(|m| {
            table
                .iter()
                .filter(|(_, inode)| m.sockets.contains(inode))
                .map(|&(port, _)| Listener {
                    port,
                    pid: m.pid,
                    name: m.name.clone(),
                })
        })
        .collect();
    found.into_iter().collect()
}

/// `listeners` as `terminal.ports` replies: `PORT<TAB>PID<TAB>NAME` per
/// line, the name [`escape`](crate::process::escape)d as in
/// `terminal.process`.
pub(crate) fn text(listeners: &[Listener]) -> String {
    listeners
        .iter()
        .map(|l| format!("{}\t{}\t{}", l.port, l.pid, crate::process::escape(&l.name)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sockets the processes of the session `child` leads listen on.
#[cfg(target_os = "linux")]
pub(crate) fn listening(child: u32) -> Vec<Listener> {
    let table: Vec<(u16, u64)> = ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|t| parse_table(&t))
        .collect();
    if table.is_empty() {
        return Vec::new();
    }
    let wanted: HashSet<u64> = table.iter().map(|&(_, inode)| inode).collect();
    assemble(&session(child, &wanted), &table)
}

/// Nothing: no sockets are read here, and the feature is not offered.
#[cfg(not(target_os = "linux"))]
pub(crate) fn listening(_child: u32) -> Vec<Listener> {
    Vec::new()
}

/// The name of the process `after` reads, if it is the process `before`
/// read: the same start time. A pid reused while its descriptors were
/// read is another process, which may belong to another block.
#[cfg(target_os = "linux")]
fn same_process(before: &Identity, after: Identity) -> Option<String> {
    (before.start == after.start && before.session == after.session).then_some(after.comm)
}

/// The processes of the session `child` leads that hold any of the
/// sockets `wanted`.
#[cfg(target_os = "linux")]
fn session(child: u32, wanted: &HashSet<u64>) -> Vec<Member> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            // Stat only: no command line is read on this path.
            let before = crate::process::identity(pid)?;
            if before.session != child {
                return None;
            }
            let sockets: BTreeSet<u64> = sockets(pid).filter(|i| wanted.contains(i)).collect();
            if sockets.is_empty() {
                return None;
            }
            let name = same_process(&before, crate::process::identity(pid)?)?;
            Some(Member { pid, name, sockets })
        })
        .collect()
}

/// The inodes of the sockets process `pid` has open; none if it has gone or
/// may not be read.
#[cfg(target_os = "linux")]
fn sockets(pid: u32) -> impl Iterator<Item = u64> {
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok();
    fds.into_iter()
        .flatten()
        .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
        .filter_map(|target| socket_inode(target.to_str()?))
}

/// The inode in a `socket:[12345]` link target.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn socket_inode(target: &str) -> Option<u64> {
    target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0000000000000000 100 0 0 10 0
   1: 0100007F:1F91 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 222 1 0000000000000000 100 0 0 10 0
   2: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 333 1 0000000000000000 100 0 0 10 0
   3: garbage
";

    #[cfg(target_os = "linux")]
    fn id(session: u32, start: u64, comm: &str) -> Identity {
        Identity {
            session,
            start,
            comm: comm.into(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_same_process_keeps_its_name() {
        let name = same_process(&id(7, 100, "node"), id(7, 100, "node"));
        assert_eq!(name.as_deref(), Some("node"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_reused_pid_is_dropped() {
        // Another start time: the pid now names a new process, even in
        // the same session.
        assert_eq!(same_process(&id(7, 100, "node"), id(7, 200, "node")), None);
        // Another session: it belongs to another block.
        assert_eq!(same_process(&id(7, 100, "node"), id(9, 100, "node")), None);
    }

    fn member(pid: u32, name: &str, sockets: &[u64]) -> Member {
        Member {
            pid,
            name: name.into(),
            sockets: sockets.iter().copied().collect(),
        }
    }

    #[test]
    fn only_listening_rows_are_read() {
        // Port 0x1F90 is 8080 and 0x0016 is 22; the established row and
        // the garbage row are skipped.
        assert_eq!(parse_table(TABLE), [(8080, 111), (22, 333)]);
    }

    #[test]
    fn an_empty_or_headerless_table_has_no_listeners() {
        assert!(parse_table("").is_empty());
        assert!(parse_table("  sl  local_address").is_empty());
    }

    #[test]
    fn a_listener_is_a_row_whose_socket_a_member_holds() {
        let table = parse_table(TABLE);
        let found = assemble(&[member(7, "vite", &[111, 999])], &table);
        assert_eq!(
            found,
            [Listener {
                port: 8080,
                pid: 7,
                name: "vite".into()
            }]
        );
        // Socket 333 belongs to a process outside the block.
        assert!(assemble(&[member(7, "vite", &[999])], &table).is_empty());
    }

    #[test]
    fn two_rows_of_one_process_and_port_are_listed_once() {
        let table = [(3000, 1), (3000, 2)];
        let found = assemble(&[member(7, "node", &[1, 2])], &table);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn listeners_are_ordered_by_port_then_pid() {
        let table = [(9000, 1), (80, 2), (80, 3)];
        let members = [member(9, "b", &[3, 1]), member(4, "a", &[2])];
        let ports: Vec<(u16, u32)> = assemble(&members, &table)
            .iter()
            .map(|l| (l.port, l.pid))
            .collect();
        assert_eq!(ports, [(80, 4), (80, 9), (9000, 9)]);
    }

    #[test]
    fn the_reply_is_one_escaped_line_per_listener() {
        let listeners = [
            Listener {
                port: 80,
                pid: 4,
                name: "a\tb".into(),
            },
            Listener {
                port: 81,
                pid: 5,
                name: "c".into(),
            },
        ];
        assert_eq!(text(&listeners), "80\t4\ta\\tb\n81\t5\tc");
        assert_eq!(text(&[]), "");
    }

    #[test]
    fn a_socket_link_names_its_inode() {
        assert_eq!(socket_inode("socket:[12345]"), Some(12345));
        assert_eq!(socket_inode("pipe:[12345]"), None);
        assert_eq!(socket_inode("socket:[x]"), None);
    }
}
