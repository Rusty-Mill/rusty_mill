//! The archive of dismissed blocks (C84, card #1570, rusty_term D339): a
//! held block's transcript and outcome are kept as text files under the
//! server's state directory, owner-only like the rest of it and never
//! loaded at a start. A record is `name: value` header lines, a blank line,
//! then the transcript. A record whose text looks like it holds a secret
//! says `sensitive: yes`; nothing is redacted (D339).

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// The longest `card`, `label` or `why`.
pub(crate) const MAX_TEXT: usize = 256;

/// The largest transcript archived; a longer one is refused, never cut.
/// `archive-show` answers a record in one `Text` reply, which the server
/// caps at 4 MiB, so a record must fit there with its header (under 2 KiB).
pub(crate) const MAX_TRANSCRIPT: usize = 3 << 20;

/// The folder of the archive inside the state directory.
pub(crate) const DIR: &str = "archive";

/// What a record says besides its transcript.
pub(crate) struct Record<'a> {
    pub(crate) id: &'a str,
    /// Unix seconds when it was archived.
    pub(crate) when: u64,
    /// How the program ended: `code:N`, `signal:N` or `unknown`.
    pub(crate) exit: &'a str,
    pub(crate) card: &'a str,
    pub(crate) label: &'a str,
    pub(crate) why: &'a str,
    /// Who archived it: a principal id.
    pub(crate) by: &'a str,
    pub(crate) block: &'a str,
    pub(crate) program: &'a str,
    pub(crate) sensitive: bool,
}

/// `text` as a header value, or why not: at most [`MAX_TEXT`] bytes and no
/// control character, which would end a header line early.
pub(crate) fn check_text(name: &str, text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT {
        return Err(format!("{name} is over {MAX_TEXT} bytes"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("{name} holds a control character"));
    }
    Ok(())
}

/// Writes `record` and `transcript` as `<id>.txt` in the archive folder of
/// `state`, made owner-only if missing, through a temporary file renamed
/// into place so a reader never sees half a record. Never replaces one.
pub(crate) fn write(state: &Path, record: &Record<'_>, transcript: &str) -> io::Result<()> {
    let dir = state.join(DIR);
    make_dir(&dir)?;
    let path = dir.join(format!("{}.txt", record.id));
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the archive has this record already",
        ));
    }
    let tmp = dir.join(format!("{}.tmp", record.id));
    let mut file = create_private(&tmp)?;
    let header = format!(
        "id: {}\nwhen: {}\nexit: {}\nsensitive: {}\ncard: {}\nlabel: {}\nwhy: {}\nby: {}\nblock: {}\nprogram: {}\n\n",
        record.id,
        record.when,
        record.exit,
        if record.sensitive { "yes" } else { "no" },
        record.card,
        record.label,
        record.why,
        record.by,
        record.block,
        record.program,
    );
    file.write_all(header.as_bytes())?;
    file.write_all(transcript.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, &path)
}

#[cfg(unix)]
fn make_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn make_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// The header of one record file: its lines, read up to the blank one.
fn header(path: &Path) -> io::Result<Vec<(String, String)>> {
    let mut lines = BufReader::new(fs::File::open(path)?).lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next().transpose()? {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(": ") {
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    Ok(out)
}

/// The archived records, newest first, one per line as
/// `ID<TAB>WHEN<TAB>EXIT<TAB>SENSITIVE<TAB>CARD<TAB>LABEL`, only those of
/// `card` when given. A file that is not a record is skipped.
pub(crate) fn list(state: &Path, card: Option<&str>) -> io::Result<String> {
    let dir = state.join(DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let mut rows: Vec<(u64, String)> = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        let Ok(head) = header(&path) else { continue };
        let get = |name: &str| {
            head.iter()
                .find(|(n, _)| n == name)
                .map_or("", |(_, v)| v.as_str())
        };
        if card.is_some_and(|c| c != get("card")) || get("id").is_empty() {
            continue;
        }
        let when = get("when").parse().unwrap_or(0);
        rows.push((
            when,
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                get("id"),
                get("when"),
                get("exit"),
                get("sensitive"),
                get("card"),
                get("label")
            ),
        ));
    }
    rows.sort_by(|a, b| b.cmp(a));
    Ok(rows
        .into_iter()
        .map(|(_, row)| row)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Record `id`, whole, or why not. An id is lowercase hex, digits and
/// hyphens, so it can never name another path.
pub(crate) fn show(state: &Path, id: &str) -> io::Result<String> {
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f' | '-'));
    if !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that is not a record id",
        ));
    }
    fs::read_to_string(state.join(DIR).join(format!("{id}.txt")))
}

/// Whether `text` looks like it holds a secret: a private key, a token of a
/// known shape, or a `password`, `token` or similar name given a value. It
/// errs toward yes: a false hit only marks a record (D339).
pub(crate) fn sensitive(text: &str) -> bool {
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    let token = |c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/'));
    text.split(token).any(token_is_secret) || text.lines().any(assigns_a_secret)
}

fn token_is_secret(t: &str) -> bool {
    let upper = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    let prefixed = |p: &str, min: usize| t.starts_with(p) && t.len() >= min;
    (t.len() == 20 && (t.starts_with("AKIA") || t.starts_with("ASIA")) && upper(t))
        || ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
            .iter()
            .any(|p| prefixed(p, 36))
        || prefixed("github_pat_", 40)
        || prefixed("sk-", 24)
        || (t.len() >= 20
            && t.starts_with("xox")
            && t[3..].starts_with(['b', 'a', 'p', 'r', 's'])
            && t[4..].starts_with('-'))
        || (t.len() >= 40 && t.starts_with("eyJ") && t.matches('.').count() == 2)
}

/// A line that gives a secret-sounding name a value of 8 or more
/// characters: `password=hunter2222`, `"api_key": "..."`, `Authorization:
/// Bearer ...`.
fn assigns_a_secret(line: &str) -> bool {
    const NAMES: [&str; 7] = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "authorization",
    ];
    let lower = line.to_ascii_lowercase();
    NAMES.iter().any(|name| {
        lower.match_indices(name).any(|(at, _)| {
            let after = lower[at + name.len()..].trim_start_matches(['"', '\'', ' ']);
            let Some(value) = after.strip_prefix([':', '=']) else {
                return false;
            };
            let value = value.trim_start_matches(['"', '\'', ' ']);
            // `Authorization: Bearer <token>`: the token is the value.
            let value = value
                .strip_prefix("bearer ")
                .or_else(|| value.strip_prefix("basic "))
                .unwrap_or(value)
                .trim_start();
            value.chars().take_while(|c| !c.is_whitespace()).count() >= 8
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<'a>(id: &'a str, when: u64, card: &'a str) -> Record<'a> {
        Record {
            id,
            when,
            exit: "code:0",
            card,
            label: "good",
            why: "done",
            by: "uid:1000",
            block: "b",
            program: "sh",
            sensitive: false,
        }
    }

    fn state() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-archive-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_are_listed_newest_first_by_card_and_shown_whole() {
        let dir = state();
        write(&dir, &record("00a-1", 10, "c1"), "first\nrun\n").unwrap();
        write(&dir, &record("00b-2", 20, "c2"), "second\n").unwrap();
        let all = list(&dir, None).unwrap();
        let ids: Vec<&str> = all.lines().map(|l| l.split('\t').next().unwrap()).collect();
        assert_eq!(ids, ["00b-2", "00a-1"]);
        assert_eq!(list(&dir, Some("c1")).unwrap().lines().count(), 1);
        let shown = show(&dir, "00a-1").unwrap();
        assert!(shown.starts_with("id: 00a-1\n") && shown.ends_with("\n\nfirst\nrun\n"));
        // Never replaced, and never named by a path.
        assert!(write(&dir, &record("00a-1", 11, "c1"), "x").is_err());
        assert!(show(&dir, "../sessions").is_err());
        assert!(show(&dir, "zz").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_archive_lists_nothing() {
        let dir = state();
        assert_eq!(list(&dir, None).unwrap(), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn header_text_is_bounded_and_one_line() {
        assert!(check_text("card", "abc").is_ok());
        assert!(check_text("card", &"x".repeat(MAX_TEXT + 1)).is_err());
        assert!(check_text("why", "a\nb").is_err());
    }

    #[test]
    fn secret_looking_text_is_marked() {
        for text in [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
            "key AKIAABCDEFGHIJKLMNOP here",
            "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "export password=hunter2hunter2",
            "\"api_key\": \"abcd1234efgh\"",
            "Authorization: Bearer abcdefghijkl",
            "sk-abcdefghijklmnopqrstuvwxyz",
        ] {
            assert!(sensitive(text), "{text}");
        }
    }

    #[test]
    fn ordinary_output_is_not_marked() {
        for text in [
            "cargo build\n   Compiling foo v0.1.0\n    Finished dev",
            "the password is asked for below",
            "token: short",
            "ls -la /home/user",
        ] {
            assert!(!sensitive(text), "{text}");
        }
    }
}
