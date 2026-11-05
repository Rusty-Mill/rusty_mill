//! C70: a block's history past its in-memory scrollback, spooled to disk
//! (rusty_term D75). Each line the grid evicts arrives encoded alone
//! ([`rusty_term_core::Grid::take_spilled`]) and is appended to the block's
//! spool directory, `<block>.spool/` in the store, as a record: its length
//! as 4 bytes little-endian, then the line. Records fill segment files
//! `<n>.seg` of about [`SEGMENT`] bytes; past [`CAP`] in all the oldest
//! segment is deleted. A record cut short by a crash is cut off when the
//! spool next opens.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The most a block's spool keeps on disk (D75).
pub(crate) const CAP: u64 = 64 << 20;
/// A segment's size before the next one starts.
const SEGMENT: u64 = 4 << 20;
/// The longest record read back: far past any line's encoding (4096
/// cells), so anything longer is damage.
const RECORD_MAX: usize = 1 << 20;
/// The most lines one read returns.
pub(crate) const READ_MAX: u64 = 100_000;

/// One block's spool, appended to as its grid evicts lines.
#[derive(Debug)]
pub(crate) struct Spool {
    dir: PathBuf,
    /// Oldest first.
    segments: VecDeque<Segment>,
    /// The newest segment, open for appending.
    out: Option<fs::File>,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    n: u64,
    lines: u64,
    bytes: u64,
}

impl Spool {
    /// Opens the spool in `dir`, making it if missing and cutting off a
    /// record a crash left half written.
    pub(crate) fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let mut numbers = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let name = entry?.file_name();
            if let Some(n) = name
                .to_str()
                .and_then(|n| n.strip_suffix(".seg"))
                .and_then(|n| n.parse().ok())
            {
                numbers.push(n);
            }
        }
        numbers.sort_unstable();
        let mut segments = VecDeque::with_capacity(numbers.len());
        for n in numbers {
            let path = segment_path(&dir, n);
            let (lines, whole) = count(&fs::read(&path)?);
            fs::OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_len(whole)?;
            segments.push_back(Segment {
                n,
                lines,
                bytes: whole,
            });
        }
        Ok(Self {
            dir,
            segments,
            out: None,
        })
    }

    /// Lines held, oldest to newest.
    pub(crate) fn lines(&self) -> u64 {
        self.segments.iter().map(|s| s.lines).sum()
    }

    /// Appends `lines`, oldest first, starting a segment and dropping the
    /// oldest as the sizes say.
    pub(crate) fn append(&mut self, lines: Vec<Vec<u8>>) -> io::Result<()> {
        let mut batch = Vec::new();
        for line in lines {
            let len =
                u32::try_from(line.len()).map_err(|_| io::Error::other("spilled line too long"))?;
            if self
                .segments
                .back()
                .is_none_or(|s| s.bytes + batch.len() as u64 >= SEGMENT)
            {
                self.write(&batch)?;
                batch.clear();
                self.rotate()?;
            }
            batch.extend_from_slice(&len.to_le_bytes());
            batch.extend_from_slice(&line);
            if let Some(last) = self.segments.back_mut() {
                last.lines += 1;
            }
        }
        self.write(&batch)
    }

    fn write(&mut self, batch: &[u8]) -> io::Result<()> {
        let (Some(last), false) = (self.segments.back_mut(), batch.is_empty()) else {
            return Ok(());
        };
        if self.out.is_none() {
            let path = segment_path(&self.dir, last.n);
            self.out = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?,
            );
        }
        if let Some(out) = &mut self.out {
            out.write_all(batch)?;
        }
        last.bytes += batch.len() as u64;
        Ok(())
    }

    /// Starts the next segment, then drops the oldest ones past [`CAP`].
    fn rotate(&mut self) -> io::Result<()> {
        self.out = None;
        let n = self.segments.back().map_or(0, |s| s.n + 1);
        self.segments.push_back(Segment {
            n,
            lines: 0,
            bytes: 0,
        });
        while self.segments.len() > 1
            && self.segments.iter().map(|s| s.bytes).sum::<u64>() + SEGMENT > CAP
        {
            let Some(old) = self.segments.pop_front() else {
                break;
            };
            match fs::remove_file(segment_path(&self.dir, old.n)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Up to `count` lines (at most [`READ_MAX`]), oldest first, ending
    /// `skip` lines before the newest. Fewer near the oldest kept.
    pub(crate) fn read(&self, skip: u64, count: u64) -> io::Result<Vec<Vec<u8>>> {
        let count = count.min(READ_MAX);
        let total = self.lines();
        let end = total.saturating_sub(skip);
        let start = end.saturating_sub(count);
        let mut out = Vec::new();
        let mut first = 0u64;
        for segment in &self.segments {
            let next = first + segment.lines;
            if next > start && first < end {
                let data = fs::read(segment_path(&self.dir, segment.n))?;
                let records = records(&data).skip(start.saturating_sub(first) as usize);
                out.extend(
                    records
                        .take((end.min(next) - first.max(start)) as usize)
                        .map(<[u8]>::to_vec),
                );
            }
            first = next;
        }
        Ok(out)
    }
}

fn segment_path(dir: &Path, n: u64) -> PathBuf {
    dir.join(format!("{n:010}.seg"))
}

/// The whole records at the front of `data`.
fn records(mut data: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        let (head, rest) = data.split_first_chunk::<4>()?;
        let len = u32::from_le_bytes(*head) as usize;
        if len > RECORD_MAX || len > rest.len() {
            return None;
        }
        let (record, rest) = rest.split_at(len);
        data = rest;
        Some(record)
    })
}

/// How many whole records `data` starts with, and how many bytes they take.
fn count(data: &[u8]) -> (u64, u64) {
    records(data).fold((0, 0), |(n, bytes), r| (n + 1, bytes + 4 + r.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-spool-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn line(n: u64, len: usize) -> Vec<u8> {
        let mut line = n.to_le_bytes().to_vec();
        line.resize(len, b'.');
        line
    }

    fn numbers(lines: &[Vec<u8>]) -> Vec<u64> {
        lines
            .iter()
            .map(|l| u64::from_le_bytes(l[..8].try_into().unwrap()))
            .collect()
    }

    #[test]
    fn reads_back_any_window_across_segments_and_reopens() {
        let dir = scratch("window");
        let mut spool = Spool::open(dir.clone()).unwrap();
        // ~1.2 segments' worth of 4 KiB lines.
        let lines: Vec<_> = (0..1200).map(|n| line(n, 4096)).collect();
        spool.append(lines).unwrap();
        assert!(spool.segments.len() >= 2);
        assert_eq!(spool.lines(), 1200);
        assert_eq!(numbers(&spool.read(0, 3).unwrap()), [1197, 1198, 1199]);
        assert_eq!(numbers(&spool.read(1199, 5).unwrap()), [0]);
        let across = numbers(&spool.read(1200 - 1030, 40).unwrap());
        assert_eq!(across, (990..1030).collect::<Vec<_>>());
        assert!(spool.read(5000, 10).unwrap().is_empty());
        drop(spool);
        let spool = Spool::open(dir.clone()).unwrap();
        assert_eq!(spool.lines(), 1200);
        assert_eq!(numbers(&spool.read(0, 1).unwrap()), [1199]);
        drop(spool);
        fs::remove_dir_all(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn keeps_at_most_the_cap_dropping_the_oldest() {
        let dir = scratch("cap");
        let mut spool = Spool::open(dir.clone()).unwrap();
        let per = 64 << 10;
        let n = (CAP / per as u64) * 2;
        for chunk in (0..n).collect::<Vec<_>>().chunks(64) {
            spool
                .append(chunk.iter().map(|&i| line(i, per)).collect())
                .unwrap();
        }
        let on_disk: u64 = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(on_disk <= CAP, "{on_disk}");
        assert!(on_disk > CAP / 2, "{on_disk}");
        let newest = numbers(&spool.read(0, 1).unwrap());
        assert_eq!(newest, [n - 1]);
        let oldest = numbers(&spool.read(spool.lines() - 1, 1).unwrap())[0];
        assert!(oldest > 0, "oldest dropped");
        drop(spool);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_half_written_record_is_cut_off() {
        let dir = scratch("torn");
        let mut spool = Spool::open(dir.clone()).unwrap();
        spool.append(vec![line(1, 16), line(2, 16)]).unwrap();
        drop(spool);
        let seg = segment_path(&dir, 0);
        let mut data = fs::read(&seg).unwrap();
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(b"short");
        fs::write(&seg, data).unwrap();
        let mut spool = Spool::open(dir.clone()).unwrap();
        assert_eq!(spool.lines(), 2);
        spool.append(vec![line(3, 16)]).unwrap();
        assert_eq!(numbers(&spool.read(0, 10).unwrap()), [1, 2, 3]);
        drop(spool);
        fs::remove_dir_all(&dir).unwrap();
    }
}
