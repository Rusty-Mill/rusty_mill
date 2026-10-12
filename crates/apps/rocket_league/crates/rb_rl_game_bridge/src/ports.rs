//! Choosing the game's controller port, and noticing a game that is already running.
//!
//! Evidence: `RLBotCS/ManagerTools/LaunchManager/ProcessUtils.cs`, `FindUsableGamePort`
//! (lines 86-113) and `IsRocketLeagueRunningWithArgs` (170-182). A game that was started with
//! the RLBot arguments already names its port, so that one is reused; otherwise the first free
//! port from 23233 up, skipping RLBot's own 23234, is taken.

use std::io;
use std::net::TcpListener;

use crate::launch::{DEFAULT_GAME_PORT, IDEAL_GAME_PORT};

/// The port in a game's `RLBot_ControllerURL=host:port` argument, if its command line has one.
pub fn controller_port(command_line: &str) -> Option<u16> {
    command_line
        .split(' ')
        .find(|arg| arg.contains("RLBot_ControllerURL"))
        .and_then(|arg| arg.rsplit(':').next())
        .and_then(|port| port.trim_end_matches('"').parse().ok())
}

/// Whether one of the game processes was started in RLBot mode (RLBotCS looks for the text
/// `rlbot` anywhere in the command line).
pub fn any_started_with_rlbot(command_lines: &[String]) -> bool {
    command_lines
        .iter()
        .any(|line| !line.is_empty() && line.contains("rlbot"))
}

/// The controller port to use.
///
/// `running` are the command lines of running Rocket League processes; the first that names a
/// port wins. Otherwise ports from [`IDEAL_GAME_PORT`] up are tried with `is_free`, skipping
/// `sockets_port`, and [`DEFAULT_GAME_PORT`] is the answer if none is free.
pub fn find_usable_game_port(
    sockets_port: u16,
    running: &[String],
    mut is_free: impl FnMut(u16) -> bool,
) -> u16 {
    running
        .iter()
        .find_map(|line| controller_port(line))
        .or_else(|| (IDEAL_GAME_PORT..u16::MAX).find(|&p| p != sockets_port && is_free(p)))
        .unwrap_or(DEFAULT_GAME_PORT)
}

/// Whether nothing holds `port`. RLBotCS binds `IPAddress.Any` and stops again straight away;
/// that alone is not enough here, because on Windows a wildcard bind succeeds while another
/// socket holds `127.0.0.1:port`, and `127.0.0.1` is exactly where [`crate::GameListener`]
/// listens. So both are tried (found by a test that bound the listener and still saw the port
/// as free).
pub fn tcp_port_is_free(port: u16) -> bool {
    ["0.0.0.0", "127.0.0.1"]
        .iter()
        .all(|host| TcpListener::bind((*host, port)).is_ok())
}

/// Where the command lines of running processes come from; a trait so the decisions above can
/// be tested without a game.
pub trait ProcessSource {
    /// The command lines of every running process whose name contains `RocketLeague`.
    fn rocket_league_command_lines(&self) -> io::Result<Vec<String>>;
}

/// The operating system's process table.
///
/// Windows has no safe std way to read another process's command line, so this asks
/// PowerShell's `Get-CimInstance Win32_Process`; Linux reads `/proc/<pid>/cmdline`, the same
/// file RLBotCS reads.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcesses;

impl ProcessSource for SystemProcesses {
    #[cfg(windows)]
    fn rocket_league_command_lines(&self) -> io::Result<Vec<String>> {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process | Where-Object { $_.Name -like '*RocketLeague*' } \
                 | ForEach-Object { $_.CommandLine }",
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "powershell exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    #[cfg(not(windows))]
    fn rocket_league_command_lines(&self) -> io::Result<Vec<String>> {
        let mut lines = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            let path = entry?.path();
            let Ok(name) = std::fs::read_to_string(path.join("comm")) else {
                continue; // The process ended while we were looking.
            };
            if !name.contains("RocketLeague") {
                continue;
            }
            if let Ok(raw) = std::fs::read(path.join("cmdline")) {
                // Arguments are NUL-separated.
                lines.push(
                    String::from_utf8_lossy(&raw)
                        .replace('\0', " ")
                        .trim()
                        .into(),
                );
            }
        }
        Ok(lines)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_port_comes_from_the_controller_url_argument() {
        let cmd = r#"RocketLeague.exe -rlbot "RLBot_ControllerURL=127.0.0.1:23999" RLBot_PacketSendRate=240"#;
        assert_eq!(controller_port(cmd), Some(23999));
        assert_eq!(controller_port("RocketLeague.exe -windowed"), None);
        assert_eq!(
            controller_port("RLBot_ControllerURL=127.0.0.1:notaport"),
            None
        );
    }

    #[test]
    fn a_running_games_port_is_reused() {
        let running = lines(&["RocketLeague.exe -rlbot RLBot_ControllerURL=127.0.0.1:40000"]);
        assert_eq!(find_usable_game_port(23234, &running, |_| true), 40000);
    }

    #[test]
    fn otherwise_the_first_free_port_from_23233_skipping_the_sockets_port() {
        assert_eq!(find_usable_game_port(23234, &[], |_| true), 23233);
        // 23233 taken: 23234 is RLBot's own, so the next is 23235.
        assert_eq!(find_usable_game_port(23234, &[], |p| p != 23233), 23235);
        // A game with no RLBot argument does not block the search.
        let plain = lines(&["RocketLeague.exe"]);
        assert_eq!(find_usable_game_port(23234, &plain, |_| true), 23233);
    }

    #[test]
    fn with_nothing_free_the_default_is_50000() {
        assert_eq!(find_usable_game_port(23234, &[], |_| false), 50000);
    }

    #[test]
    fn rlbot_mode_is_a_substring_of_the_command_line() {
        assert!(any_started_with_rlbot(&lines(&["a -rlbot b"])));
        assert!(!any_started_with_rlbot(&lines(&["RocketLeague.exe"])));
        assert!(!any_started_with_rlbot(&lines(&[""])));
        assert!(!any_started_with_rlbot(&[]));
    }

    #[test]
    fn a_bound_port_is_not_free() {
        let held = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!tcp_port_is_free(port));
        drop(held);
    }
}
