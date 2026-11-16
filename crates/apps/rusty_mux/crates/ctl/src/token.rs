//! `token create|rotate|revoke NAME --file TOKENS` (#506, D237): edits the
//! token file the server's `--tokens` names, with no server running or asked.
//! The new token is printed once, as 64 hex digits; only its SHA-256 is kept.
//! The file is written whole to a sibling and renamed over, mode 0600, so a
//! server reading it never sees half of one. The server learns of the change
//! on its next SIGHUP, which closes only that token's connections.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusty_mux_script::token::{self, Tokens};

use crate::Exit;
use crate::cli::{TokenCommand, TokenVerb};

pub(crate) fn run(
    command: &TokenCommand,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), (Exit, String)> {
    let failed = |e: String| (Exit::Failed, e);
    let file = Path::new(&command.file);
    let mut tokens = match std::fs::read_to_string(file) {
        Ok(text) => Tokens::parse(&text).map_err(|e| failed(format!("{}: {e}", command.file)))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && command.verb == TokenVerb::Create => {
            Tokens::default()
        }
        Err(e) => return Err(failed(format!("{}: {e}", command.file))),
    };
    let name = command.name.as_str();
    let shown = match command.verb {
        TokenVerb::Revoke => {
            tokens.revoke(name).map_err(failed)?;
            None
        }
        verb => {
            let (bytes, shown) = token::generate().map_err(failed)?;
            match verb {
                TokenVerb::Create => tokens.create(name, &bytes),
                _ => tokens.rotate(name, &bytes),
            }
            .map_err(failed)?;
            Some(shown)
        }
    };
    save(file, &tokens.text()).map_err(|e| failed(format!("{}: {e}", command.file)))?;
    let line = match (shown, json) {
        (Some(shown), true) => format!(r#"{{"name":"{name}","token":"{shown}"}}"#),
        (Some(shown), false) => shown,
        (None, true) => format!(r#"{{"name":"{name}","revoked":true}}"#),
        (None, false) => format!("revoked {name}"),
    };
    writeln!(out, "{line}").map_err(|e| failed(e.to_string()))
}

/// Writes `text` to `file` whole: a sibling, then a rename.
fn save(file: &Path, text: &str) -> std::io::Result<()> {
    let mut temp = file.as_os_str().to_owned();
    temp.push(".new");
    let temp = std::path::PathBuf::from(temp);
    let mut options = std::fs::OpenOptions::new();
    // A stale sibling (or a link planted there) is removed, and the new one
    // must be created by us, so it has our mode and is no one's link.
    match std::fs::remove_file(&temp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut f = options.open(&temp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&temp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rusty-mux-ctl-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn run_in(file: &Path, verb: TokenVerb, name: &str) -> Result<String, (Exit, String)> {
        let command = TokenCommand {
            verb,
            name: name.into(),
            file: file.display().to_string(),
        };
        let mut out = Vec::new();
        run(&command, false, &mut out)?;
        Ok(String::from_utf8(out).expect("utf8").trim().to_owned())
    }

    #[test]
    fn create_rotate_and_revoke_edit_the_file_and_show_the_token_once() {
        let file = dir().join("tokens-a");
        let _ = std::fs::remove_file(&file);
        let first = run_in(&file, TokenVerb::Create, "laptop").expect("created");
        let tokens = Tokens::parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            tokens.verify(&token::from_hex(&first).unwrap()),
            Some("laptop")
        );
        assert!(!std::fs::read_to_string(&file).unwrap().contains(&first));
        run_in(&file, TokenVerb::Create, "laptop").expect_err("a name in use");
        let second = run_in(&file, TokenVerb::Rotate, "laptop").expect("rotated");
        let tokens = Tokens::parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(tokens.verify(&token::from_hex(&first).unwrap()), None);
        assert_eq!(
            tokens.verify(&token::from_hex(&second).unwrap()),
            Some("laptop")
        );
        assert_eq!(
            run_in(&file, TokenVerb::Revoke, "laptop").unwrap(),
            "revoked laptop"
        );
        assert!(run_in(&file, TokenVerb::Revoke, "laptop").is_err());
        assert!(run_in(&file, TokenVerb::Rotate, "laptop").is_err());
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&file).unwrap().permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_missing_file_is_made_only_by_create() {
        let file = dir().join("tokens-missing");
        let _ = std::fs::remove_file(&file);
        assert!(run_in(&file, TokenVerb::Rotate, "a").is_err());
        assert!(run_in(&file, TokenVerb::Revoke, "a").is_err());
        assert!(!file.exists());
    }
}
