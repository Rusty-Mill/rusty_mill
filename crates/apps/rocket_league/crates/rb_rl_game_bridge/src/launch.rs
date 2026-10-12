//! Starting Rocket League so that it dials RLBot.
//!
//! Evidence: `RLBotCS/ManagerTools/LaunchManager/RocketLeagueLauncher.cs:14-20` (`GetRLBotArgs`)
//! builds exactly these four arguments for every launcher (Steam, Epic, legendary, Heroic);
//! `Epic.Windows.cs:50-62` runs the game executable directly with them.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// The port RLBot prefers for the game's controller socket.
pub const IDEAL_GAME_PORT: u16 = 23233;

/// The port RLBot falls back to when it cannot probe for a free one.
pub const DEFAULT_GAME_PORT: u16 = 50000;

/// The port bots connect to (RLBot core's own server, not the game's).
pub const RLBOT_SOCKETS_PORT: u16 = 23234;

/// How often the game is asked to send state, per second. RLBot asks for twice the 120 Hz
/// physics rate, which is why a game can deliver more than one message per physics tick.
pub const PACKET_SEND_RATE: u32 = 240;

/// The command-line arguments that make Rocket League connect out to `127.0.0.1:port`.
pub fn rlbot_args(port: u16) -> [String; 4] {
    [
        "-rlbot".into(),
        format!("RLBot_ControllerURL=127.0.0.1:{port}"),
        format!("RLBot_PacketSendRate={PACKET_SEND_RATE}"),
        "-nomovie".into(),
    ]
}

/// A game executable and the controller port it should dial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameLaunch {
    pub executable: PathBuf,
    pub port: u16,
}

impl GameLaunch {
    pub fn new(executable: impl AsRef<Path>, port: u16) -> GameLaunch {
        GameLaunch {
            executable: executable.as_ref().to_path_buf(),
            port,
        }
    }

    /// The command, not yet started. Output is discarded: the game prints a great deal and
    /// nothing here reads it.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .args(rlbot_args(self.port))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    /// Starts the game. Launching through Steam or Epic, which RLBotCS also supports, needs
    /// those launchers' own protocols and is not done here: this starts the executable.
    pub fn spawn(&self) -> io::Result<Child> {
        self.command().spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_arguments_are_rlbots() {
        assert_eq!(
            rlbot_args(23233),
            [
                "-rlbot",
                "RLBot_ControllerURL=127.0.0.1:23233",
                "RLBot_PacketSendRate=240",
                "-nomovie"
            ]
        );
    }

    #[test]
    fn the_command_carries_them_in_order() {
        let command = GameLaunch::new("RocketLeague.exe", 5000).command();
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy()).collect();
        assert_eq!(args[0], "-rlbot");
        assert_eq!(args[1], "RLBot_ControllerURL=127.0.0.1:5000");
    }

    #[test]
    fn a_missing_executable_is_an_error_not_a_panic() {
        let launch = GameLaunch::new("definitely/not/RocketLeague.exe", 1);
        assert!(launch.spawn().is_err());
    }
}
