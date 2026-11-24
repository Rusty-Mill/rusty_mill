//! `terminal.upload` (C82): a file sent to the server's host a chunk at a
//! time, into a folder kept for one block.
//!
//! A file grows under `part/` and moves to the folder itself on commit, so a
//! half-sent file is never taken for a whole one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// The most bytes one chunk carries.
pub(crate) const CHUNK_MAX: usize = 1 << 20;
/// The most bytes a block keeps; the oldest files go first.
pub(crate) const BLOCK_MAX: u64 = 2 << 30;
/// Unfinished files a block keeps at most.
const PARTIALS_MAX: usize = 16;
/// How long an unfinished file may sit untouched before a new upload clears
/// it: its sender is gone.
const STALE: Duration = Duration::from_secs(600);
/// The longest file name kept, in bytes.
const NAME_MAX: usize = 200;
/// Where a file under way grows.
const PART: &str = "part";
/// Device names Windows refuses as a file's name, whatever its extension.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Whether `c` is a control character or a format one that draws nothing
/// (a zero-width space, a bidirectional override), either of which would
/// make a name read as something else in a listing or a prompt.
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

/// `raw` as a file name that is safe to make and to type: its last path
/// part, with invisible characters dropped, the characters Windows refuses
/// turned into `_`, no leading or trailing dot or space, and no device name.
/// `None` when nothing is left.
pub(crate) fn clean_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = last
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    while name.len() > NAME_MAX {
        name.pop();
    }
    let name = name.trim_matches(['.', ' ']);
    let stem = name.split('.').next().unwrap_or_default();
    let reserved = RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem));
    (!name.is_empty() && !reserved).then(|| name.to_owned())
}

/// What `terminal.upload` did with one chunk.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Received {
    /// The file has this many bytes so far.
    Partial(u64),
    /// The file is whole, at this absolute path.
    Done(PathBuf),
}

/// What a block's folder may hold, and for how long an unfinished file may
/// sit untouched.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) block_max: u64,
    pub(crate) partials: usize,
    pub(crate) stale: Duration,
}

/// The limits uploads keep.
pub(crate) const LIMITS: Limits = Limits {
    block_max: BLOCK_MAX,
    partials: PARTIALS_MAX,
    stale: STALE,
};

/// Appends `data` to `name` at `offset` in `dir`; with `done` commits it.
/// `offset` must be the bytes received so far, and 0 starts the file again.
/// The folder's bytes, this chunk included, are held under the limit before
/// anything is written, the oldest other files making room.
pub(crate) fn receive(
    dir: &Path,
    name: &str,
    offset: u64,
    data: &[u8],
    done: bool,
) -> Result<Received, String> {
    receive_within(dir, name, offset, data, done, LIMITS)
}

fn receive_within(
    dir: &Path,
    name: &str,
    offset: u64,
    data: &[u8],
    done: bool,
    limits: Limits,
) -> Result<Received, String> {
    let name = clean_name(name).ok_or("the file name is empty or not usable")?;
    if data.len() > CHUNK_MAX {
        return Err(format!("a chunk is at most {CHUNK_MAX} bytes"));
    }
    let end = offset + data.len() as u64;
    if end > limits.block_max {
        return Err(format!("a file is at most {} bytes", limits.block_max));
    }
    let io = |e: io::Error| format!("{name}: {e}");
    let part_dir = dir.join(PART);
    fs::create_dir_all(&part_dir).map_err(io)?;
    let part = part_dir.join(&name);
    check_offset(&part, offset)?;
    if offset == 0 {
        // A new file: what its senders left unfinished long ago goes, and
        // the rest is bounded.
        clear_stale(&part_dir, &part, limits.stale).map_err(io)?;
        if count_others(&part_dir, &part).map_err(io)? >= limits.partials {
            return Err("too many unfinished uploads for this block".into());
        }
    }
    make_room(dir, &part, end, limits.block_max).map_err(io)?;
    let mut file = open(&part, offset).map_err(io)?;
    file.write_all(data).map_err(io)?;
    if !done {
        return Ok(Received::Partial(end));
    }
    file.sync_all().map_err(io)?;
    drop(file);
    let target = unique(dir, &name);
    fs::rename(&part, &target).map_err(io)?;
    std::path::absolute(&target).map(Received::Done).map_err(io)
}

/// Deletes a closed block's upload folder on a thread of its own.
pub(crate) fn discard(dir: PathBuf) {
    std::thread::spawn(move || match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            eprintln!("rusty-mux-server: removing {}: {e}", dir.display());
        }
        _ => {}
    });
}

/// One block's upload authority: its transfers run one at a time, so the
/// offsets, the byte budget and the final names are decided by one party,
/// and once the block closes none runs again. A job that holds the slot as
/// the block closes finishes, then clears the folder itself.
#[derive(Default)]
pub(crate) struct Slot {
    closing: AtomicBool,
    busy: Mutex<()>,
}

impl Slot {
    /// Runs `work` alone, unless the block closed or `deadline` passed
    /// while this job waited for its turn.
    pub(crate) fn run<T>(
        &self,
        dir: &Path,
        deadline: Instant,
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let turn = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        let out = if self.closing.load(Ordering::SeqCst) {
            Err("the block was closed".to_owned())
        } else if Instant::now() >= deadline {
            Err("timed out waiting for the block's earlier transfers".to_owned())
        } else {
            work()
        };
        drop(turn);
        // The block may have closed while `work` ran.
        if self.closing.load(Ordering::SeqCst) {
            self.clear(dir);
        }
        out
    }

    /// The block closed: no job runs after this. Its folder goes now, or
    /// when the job in flight ends.
    ///
    /// The deletion runs on a thread of its own, never on the caller's.
    pub(crate) fn close(self: &Arc<Self>, dir: PathBuf) {
        self.closing.store(true, Ordering::SeqCst);
        let slot = Arc::clone(self);
        std::thread::spawn(move || {
            if slot.busy.try_lock().is_ok() {
                slot.clear(&dir);
            }
        });
    }

    fn clear(&self, dir: &Path) {
        let _turn = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        match fs::remove_dir_all(dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                eprintln!("rusty-mux-server: removing {}: {e}", dir.display());
            }
            _ => {}
        }
    }
}

/// Whether `offset` is where `part` left off; 0 always is.
fn check_offset(part: &Path, offset: u64) -> Result<(), String> {
    let have = fs::metadata(part).map_or(0, |m| m.len());
    match offset {
        0 => Ok(()),
        _ if have == offset => Ok(()),
        _ => Err(format!(
            "the chunk starts at {offset} but {have} bytes were received"
        )),
    }
}

/// `part` opened to append (at 0, to start over).
fn open(part: &Path, offset: u64) -> io::Result<File> {
    match offset {
        0 => File::create(part),
        _ => OpenOptions::new().append(true).open(part),
    }
}

/// Removes the unfinished files in `part_dir`, but `keep`, that nobody has
/// written to for `stale`.
fn clear_stale(part_dir: &Path, keep: &Path, stale: Duration) -> io::Result<()> {
    let now = SystemTime::now();
    for entry in fs::read_dir(part_dir)?.filter_map(Result::ok) {
        let meta = entry.metadata()?;
        let idle = now.duration_since(meta.modified()?).unwrap_or_default();
        if meta.is_file() && entry.path() != keep && idle >= stale {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// The unfinished files in `part_dir` but `keep`.
fn count_others(part_dir: &Path, keep: &Path) -> io::Result<usize> {
    Ok(fs::read_dir(part_dir)?
        .filter_map(Result::ok)
        .filter(|e| e.path() != keep)
        .count())
}

/// `name` in `dir`, or the first of `stem-1.ext`, `stem-2.ext`, ... that is
/// free, so an upload never replaces an earlier one.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    (1u32..)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Removes the oldest files of `dir` (and of its `part/`) until `keep_len`
/// more bytes fit under `max`; `keep` is not touched or counted.
fn make_room(dir: &Path, keep: &Path, keep_len: u64, max: u64) -> io::Result<()> {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for folder in [dir.to_path_buf(), dir.join(PART)] {
        // A folder that cannot be read leaves the budget unknown: nothing
        // is written. Only a missing one is empty.
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_file() && entry.path() != keep {
                files.push((meta.modified()?, meta.len(), entry.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum::<u64>() + keep_len;
    files.sort();
    for (_, len, path) in files {
        if total <= max {
            break;
        }
        fs::remove_file(&path)?;
        total -= len;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc;

    fn scratch() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusty-mux-upload-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("a.txt").as_deref(), Some("a.txt"));
        assert_eq!(clean_name("/etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(clean_name("..\\..\\x.sh").as_deref(), Some("x.sh"));
        assert_eq!(
            clean_name("a\u{202e}b\u{200b}.txt\n").as_deref(),
            Some("ab.txt")
        );
        assert_eq!(clean_name("a:b?.txt").as_deref(), Some("a_b_.txt"));
        assert_eq!(clean_name(" .hidden. ").as_deref(), Some("hidden"));
    }

    #[test]
    fn unusable_names_are_refused() {
        for raw in ["", ".", "..", "dir/", "\u{200b}", "NUL", "con.txt"] {
            assert_eq!(clean_name(raw), None, "{raw:?}");
        }
        assert_eq!(clean_name(&"x".repeat(500)).unwrap().len(), NAME_MAX);
    }

    #[test]
    fn chunks_make_one_file() {
        let dir = scratch();
        assert_eq!(
            receive(&dir, "f.bin", 0, b"abc", false),
            Ok(Received::Partial(3))
        );
        assert!(!dir.join("f.bin").exists(), "not whole yet");
        let done = receive(&dir, "f.bin", 3, b"def", true).unwrap();
        let Received::Done(path) = done else {
            panic!("{done:?}")
        };
        assert!(path.is_absolute());
        assert_eq!(fs::read(path).unwrap(), b"abcdef");
        assert!(!dir.join(PART).join("f.bin").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_file_of_the_same_name_keeps_the_first() {
        let dir = scratch();
        for body in [&b"one"[..], b"two"] {
            receive(&dir, "n.txt", 0, body, true).unwrap();
        }
        assert_eq!(fs::read(dir.join("n.txt")).unwrap(), b"one");
        assert_eq!(fs::read(dir.join("n-1.txt")).unwrap(), b"two");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_chunk_out_of_place_is_refused() {
        let dir = scratch();
        receive(&dir, "f", 0, b"abc", false).unwrap();
        let err = receive(&dir, "f", 2, b"x", false).unwrap_err();
        assert!(err.contains("3 bytes were received"), "{err}");
        let err = receive(&dir, "never", 5, b"x", false).unwrap_err();
        assert!(err.contains("0 bytes were received"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_oversize_chunk_or_file_is_refused() {
        let dir = scratch();
        let big = vec![0u8; CHUNK_MAX + 1];
        assert!(receive(&dir, "f", 0, &big, false).is_err());
        assert!(receive(&dir, "f", BLOCK_MAX, b"x", false).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    const SMALL: Limits = Limits {
        block_max: 100,
        partials: 2,
        stale: Duration::from_secs(600),
    };

    fn age(path: &Path, secs: u64) {
        let then = SystemTime::now() - Duration::from_secs(secs);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(then)
            .unwrap();
    }

    #[test]
    fn unfinished_files_count_against_the_limit_before_they_are_written() {
        let dir = scratch();
        receive_within(&dir, "a", 0, &[1; 60], false, SMALL).unwrap();
        age(&dir.join(PART).join("a"), 30);
        // A second 60 bytes cannot sit beside the first: the older goes.
        receive_within(&dir, "b", 0, &[2; 60], false, SMALL).unwrap();
        assert!(!dir.join(PART).join("a").exists());
        let err = receive_within(&dir, "a", 60, &[1; 10], false, SMALL).unwrap_err();
        assert!(err.contains("0 bytes were received"), "{err}");
        // Growing the one that is left past the limit removes nothing it needs.
        receive_within(&dir, "b", 60, &[2; 40], false, SMALL).unwrap();
        assert_eq!(fs::metadata(dir.join(PART).join("b")).unwrap().len(), 100);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unfinished_files_are_bounded_and_abandoned_ones_cleared() {
        let dir = scratch();
        for name in ["a", "b"] {
            receive_within(&dir, name, 0, b"x", false, SMALL).unwrap();
        }
        let err = receive_within(&dir, "c", 0, b"x", false, SMALL).unwrap_err();
        assert!(err.contains("too many unfinished"), "{err}");
        // A sender gone for longer than the stale time frees its place.
        age(&dir.join(PART).join("a"), 3600);
        receive_within(&dir, "c", 0, b"x", false, SMALL).unwrap();
        assert!(!dir.join(PART).join("a").exists());
        // The one being continued is never counted against itself.
        receive_within(&dir, "b", 1, b"y", false, SMALL).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_oldest_files_go_when_the_folder_is_full() {
        let dir = scratch();
        fs::write(dir.join("old"), b"1234").unwrap();
        // Counting `keep_len` as the whole allowance leaves no room for it.
        make_room(&dir, &dir.join("keep"), BLOCK_MAX, BLOCK_MAX).unwrap();
        assert!(!dir.join("old").exists());
        fs::write(dir.join("new"), b"1234").unwrap();
        make_room(&dir, &dir.join("keep"), 0, BLOCK_MAX).unwrap();
        assert!(dir.join("new").exists(), "room enough: nothing removed");
        fs::remove_dir_all(dir).unwrap();
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    #[test]
    fn a_blocks_transfers_take_turns() {
        let dir = scratch();
        let slot = Arc::new(Slot::default());
        let inside = Arc::new(AtomicU32::new(0));
        let most = Arc::new(AtomicU32::new(0));
        let jobs: Vec<_> = (0..8)
            .map(|_| {
                let (slot, inside, most, dir) =
                    (slot.clone(), inside.clone(), most.clone(), dir.clone());
                std::thread::spawn(move || {
                    slot.run(&dir, soon(), || {
                        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                        inside.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                })
            })
            .collect();
        for j in jobs {
            j.join().unwrap().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn closing_while_a_transfer_runs_clears_the_folder_after_it() {
        let dir = scratch();
        let slot = Arc::new(Slot::default());
        let (started_tx, started) = mpsc::channel();
        let (go, go_rx) = mpsc::channel::<()>();
        let job = {
            let (slot, dir) = (slot.clone(), dir.clone());
            std::thread::spawn(move || {
                slot.run(&dir, soon(), || {
                    fs::write(dir.join("f"), b"x").unwrap();
                    started_tx.send(()).unwrap();
                    go_rx.recv().unwrap();
                    fs::write(dir.join("g"), b"y").unwrap();
                    Ok(())
                })
            })
        };
        started.recv().unwrap();
        slot.close(dir.clone());
        assert!(
            dir.join("f").exists(),
            "the transfer in flight is not cut off"
        );
        go.send(()).unwrap();
        job.join().unwrap().unwrap();
        assert!(!dir.exists(), "the folder goes once the transfer ends");
        let mut ran = false;
        let late = slot.run(&dir, soon(), || {
            ran = true;
            Ok(())
        });
        assert!(late.is_err() && !ran, "nothing runs after the close");
    }

    #[test]
    fn a_transfer_whose_deadline_passed_while_it_waited_does_not_run() {
        let dir = scratch();
        let slot = Slot::default();
        let mut ran = false;
        let out = slot.run(&dir, Instant::now(), || {
            ran = true;
            Ok(())
        });
        assert!(out.is_err() && !ran);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn closing_an_idle_slot_clears_the_folder_off_the_callers_thread() {
        let dir = scratch();
        fs::write(dir.join("f"), b"x").unwrap();
        let slot = Arc::new(Slot::default());
        slot.close(dir.clone());
        for _ in 0..200 {
            if !dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the folder was not cleared");
    }

    #[test]
    fn an_unreadable_folder_stops_the_budget_check() {
        let dir = scratch();
        // `part` is a file: reading it as a folder fails, which is not
        // the same as there being nothing in it.
        fs::write(dir.join(PART), b"x").unwrap();
        assert!(make_room(&dir, &dir.join("keep"), 0, BLOCK_MAX).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
