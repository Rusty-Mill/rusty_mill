//! End-to-end throughput of our console host against ConPTY: the Phase B
//! gate (design §9.2). Each workload from `rusty_term/bench/gen_workloads.py`
//! is written to a console by a client, and the master is read to EOF; the
//! time runs from spawn to EOF plus the child's exit.
//!
//! ```text
//! conhost_bench [options] <workload.vt>...
//!   --reps <n>             runs per host and workload (default 5); the median counts
//!   --max-slowdown <pct>   fail when our host is slower than ConPTY by more (default 10)
//!   --write <console|raw>  how the client writes (default console)
//!   --size <rows>x<cols>   the console size (default 30x120)
//! conhost_bench cat <console|raw> <file>   the client
//! ```
//!
//! `console` writes through std's stdout, which is WriteConsoleW on a
//! console: code-page independent, as most Rust and .NET tools write.
//! `raw` is WriteFile, as C stdio does; its client runs under
//! `cmd /c chcp 65001` so both hosts decode UTF-8 (ConPTY starts on the
//! OEM code page, our host on 65001, V4).
//!
//! The runs alternate hosts and order, so drift (thermal, background work)
//! falls on both. Exit 0 when every workload is within the limit, 1 when
//! one is not, 2 on a usage or run error.

// Off Windows only the report's tests use the report.
#![cfg_attr(not(windows), allow(dead_code))]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("conhost_bench: {e}");
            2
        }
    };
    ExitCode::from(code)
}

/// `Ok(false)` when the gate fails.
fn run(args: &[String]) -> Result<bool, String> {
    if args.first().map(String::as_str) == Some("cat") {
        let write = Write::parse(args.get(1).ok_or("cat: missing writer")?)?;
        let file = args.get(2).ok_or("cat: missing file")?;
        client::cat(write, file)?;
        return Ok(true);
    }
    let opts = Options::parse(args)?;
    bench::run(&opts)
}

/// How the client writes its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Write {
    /// WriteConsoleW, through std's stdout.
    Console,
    /// WriteFile, read in the output code page.
    Raw,
}

impl Write {
    fn parse(s: &str) -> Result<Write, String> {
        match s {
            "console" => Ok(Write::Console),
            "raw" => Ok(Write::Raw),
            other => Err(format!("unknown writer {other:?}: console or raw")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Write::Console => "console",
            Write::Raw => "raw",
        }
    }
}

#[derive(Debug, PartialEq)]
struct Options {
    reps: usize,
    max_slowdown_pct: f64,
    write: Write,
    rows: u16,
    cols: u16,
    files: Vec<String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut opts = Options {
            reps: 5,
            max_slowdown_pct: 10.0,
            write: Write::Console,
            rows: 30,
            cols: 120,
            files: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let mut value = || it.next().ok_or(format!("{arg}: missing value"));
            match arg.as_str() {
                "--reps" => opts.reps = number(value()?)?,
                "--max-slowdown" => opts.max_slowdown_pct = number(value()?)?,
                "--write" => opts.write = Write::parse(value()?)?,
                "--size" => (opts.rows, opts.cols) = size(value()?)?,
                flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
                file => opts.files.push(file.to_owned()),
            }
        }
        if opts.files.is_empty() {
            return Err("no workload files".to_owned());
        }
        if opts.reps == 0 {
            return Err("--reps must be at least 1".to_owned());
        }
        if opts.max_slowdown_pct.is_nan() || opts.max_slowdown_pct < 0.0 {
            return Err("--max-slowdown must be 0 or more".to_owned());
        }
        Ok(opts)
    }
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("not a number: {s:?}"))
}

fn size(s: &str) -> Result<(u16, u16), String> {
    let (rows, cols) = s.split_once('x').ok_or(format!("size {s:?}: <rows>x<cols>"))?;
    let (rows, cols): (u16, u16) = (number(rows)?, number(cols)?);
    if rows == 0 || cols == 0 {
        return Err(format!("size {s:?}: rows and columns must be at least 1"));
    }
    Ok((rows, cols))
}

/// The verdict for one workload, from each host's run times in seconds.
#[derive(Debug, PartialEq)]
struct Row {
    conpty_s: f64,
    own_s: f64,
}

impl Row {
    fn new(conpty: &[f64], own: &[f64]) -> Row {
        Row {
            conpty_s: median(conpty),
            own_s: median(own),
        }
    }

    /// Our time over ConPTY's: above 1 is slower.
    fn ratio(&self) -> f64 {
        self.own_s / self.conpty_s
    }

    fn passes(&self, max_slowdown_pct: f64) -> bool {
        self.ratio() <= 1.0 + max_slowdown_pct / 100.0
    }
}

/// The median; the mean of the middle two for an even count.
fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// The geometric mean of the ratios: the suite's one number.
fn geomean(ratios: &[f64]) -> f64 {
    let logs: f64 = ratios.iter().map(|r| r.ln()).sum();
    (logs / ratios.len() as f64).exp()
}

fn mb_per_s(bytes: u64, secs: f64) -> f64 {
    bytes as f64 / 1e6 / secs
}

#[cfg(windows)]
mod client {
    use std::io::Write as _;

    use super::Write;

    /// Write `file` to stdout, then flush.
    pub(crate) fn cat(write: Write, file: &str) -> Result<(), String> {
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        match write {
            Write::Console => {
                let mut out = std::io::stdout().lock();
                out.write_all(&bytes).map_err(|e| e.to_string())?;
                out.flush().map_err(|e| e.to_string())
            }
            Write::Raw => raw(&bytes),
        }
    }

    /// `WriteFile` on stdout's handle, which the console serves as a raw
    /// write; std's console stdout would use WriteConsoleW instead.
    #[allow(unsafe_code)]
    fn raw(bytes: &[u8]) -> Result<(), String> {
        use std::os::windows::io::FromRawHandle as _;

        use platform_windows::ffi::win32_surface as w;

        // SAFETY: GetStdHandle takes a constant and returns a handle value;
        // nothing is dereferenced.
        let handle = unsafe { w::GetStdHandle(w::STD_OUTPUT_HANDLE) };
        // SAFETY: stdout's handle stays open for the process's life;
        // ManuallyDrop keeps the File from closing it.
        let file = unsafe { std::fs::File::from_raw_handle(handle) };
        let mut file = std::mem::ManuallyDrop::new(file);
        file.write_all(bytes).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod bench {
    use std::time::Instant;

    use platform::process::{Child, Command, ExitStatus};
    use platform::pty::PtyMaster;
    use platform::term::WinSize;
    use platform_windows::WindowsPty;
    use rusty_conhost::OwnPty;

    use super::{Options, Row, Write, geomean, mb_per_s};

    #[derive(Clone, Copy, Debug)]
    enum Host {
        ConPty,
        Own,
    }

    pub(crate) fn run(opts: &Options) -> Result<bool, String> {
        let size = WinSize {
            rows: opts.rows,
            cols: opts.cols,
        };
        println!(
            "conhost_bench: {} reps, {} writer, {}x{}, limit +{}%",
            opts.reps,
            opts.write.name(),
            opts.rows,
            opts.cols,
            opts.max_slowdown_pct
        );
        println!(
            "{:<22} {:>10} {:>12} {:>12} {:>9}  verdict",
            "workload", "bytes", "conpty MB/s", "own MB/s", "own/conpty"
        );
        let mut ratios = Vec::new();
        let mut pass = true;
        for file in &opts.files {
            let bytes = std::fs::metadata(file)
                .map_err(|e| format!("{file}: {e}"))?
                .len();
            let cmd = client(opts.write, file)?;
            let (mut conpty, mut own) = (Vec::new(), Vec::new());
            for rep in 0..opts.reps {
                // Alternate who goes first, so drift falls on both.
                let order = if rep % 2 == 0 {
                    [Host::ConPty, Host::Own]
                } else {
                    [Host::Own, Host::ConPty]
                };
                for host in order {
                    let secs = time(host, &cmd, size)?;
                    match host {
                        Host::ConPty => conpty.push(secs),
                        Host::Own => own.push(secs),
                    }
                }
            }
            let row = Row::new(&conpty, &own);
            let ok = row.passes(opts.max_slowdown_pct);
            pass &= ok;
            ratios.push(row.ratio());
            let name = std::path::Path::new(file)
                .file_stem()
                .map_or(file.clone(), |s| s.to_string_lossy().into_owned());
            println!(
                "{:<22} {:>10} {:>12.1} {:>12.1} {:>9.3}  {}",
                name,
                bytes,
                mb_per_s(bytes, row.conpty_s),
                mb_per_s(bytes, row.own_s),
                row.ratio(),
                if ok { "ok" } else { "SLOWER" }
            );
        }
        println!(
            "geomean own/conpty {:.3}: {}",
            geomean(&ratios),
            if pass { "PASS" } else { "FAIL" }
        );
        Ok(pass)
    }

    /// The client command: this binary's `cat`, under `cmd /c chcp 65001`
    /// for the raw writer.
    fn client(write: Write, file: &str) -> Result<Command, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().into_owned();
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let argv: Vec<String> = match write {
            Write::Console => vec![exe, "cat".into(), "console".into(), file.into()],
            Write::Raw => vec![
                "cmd.exe".into(),
                "/c".into(),
                format!("chcp 65001 >nul && \"{exe}\" cat raw \"{file}\""),
            ],
        };
        let (program, rest) = argv.split_first().ok_or("empty command")?;
        let mut cmd = Command::new(program, cwd);
        cmd.argv = rest.iter().map(Into::into).collect();
        Ok(cmd)
    }

    /// Seconds from spawn to EOF and the client's exit.
    fn time(host: Host, cmd: &Command, size: WinSize) -> Result<f64, String> {
        let start = Instant::now();
        let status = match host {
            Host::ConPty => {
                let (master, child) = WindowsPty
                    .spawn_concrete(cmd, size)
                    .map_err(|e| format!("ConPTY spawn: {e}"))?;
                drain(&master)?;
                Box::new(child).wait()
            }
            Host::Own => {
                let (master, child) = OwnPty
                    .spawn_concrete(cmd, size)
                    .map_err(|e| format!("own spawn: {e}"))?;
                drain(&master)?;
                Box::new(child).wait()
            }
        }
        .map_err(|e| format!("{host:?}: wait: {e}"))?;
        let secs = start.elapsed().as_secs_f64();
        match status {
            ExitStatus::Code(0) => Ok(secs),
            other => Err(format!("{host:?}: the client ended with {other:?}")),
        }
    }

    /// Read to EOF, discarding the bytes, as a terminal that keeps up.
    fn drain(master: &impl PtyMaster) -> Result<(), String> {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match master.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(e) => return Err(format!("read: {e}")),
            }
        }
    }
}

#[cfg(not(windows))]
mod client {
    pub(crate) fn cat(_: super::Write, _: &str) -> Result<(), String> {
        Err("the console bench runs on Windows only".to_owned())
    }
}

#[cfg(not(windows))]
mod bench {
    pub(crate) fn run(_: &super::Options) -> Result<bool, String> {
        Err("the console bench runs on Windows only".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_default_and_override() {
        let o = Options::parse(&args("a.vt")).unwrap();
        assert_eq!((o.reps, o.max_slowdown_pct, o.write), (5, 10.0, Write::Console));
        assert_eq!((o.rows, o.cols, o.files), (30, 120, vec!["a.vt".to_owned()]));
        let o = Options::parse(&args("--reps 3 --max-slowdown 5 --write raw --size 50x200 a b"))
            .unwrap();
        assert_eq!((o.reps, o.max_slowdown_pct, o.write), (3, 5.0, Write::Raw));
        assert_eq!((o.rows, o.cols, o.files.len()), (50, 200, 2));
    }

    #[test]
    fn options_refuse_bad_input() {
        for bad in [
            "",
            "--reps 0 a",
            "--reps x a",
            "--size 0x10 a",
            "--size 10 a",
            "--write pipe a",
            "--max-slowdown -1 a",
            "--max-slowdown NaN a",
            "--bogus a",
            "a --reps",
        ] {
            assert!(Options::parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn writer_names_round_trip() {
        for w in [Write::Console, Write::Raw] {
            assert_eq!(Write::parse(w.name()), Ok(w));
        }
    }

    #[test]
    fn median_of_odd_and_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[7.0]), 7.0);
    }

    #[test]
    fn gate_is_inclusive_at_the_limit() {
        let row = Row::new(&[1.0], &[1.1]);
        assert!(row.passes(10.0));
        assert!(!row.passes(9.9));
        // Faster always passes.
        assert!(Row::new(&[2.0], &[1.0]).passes(0.0));
    }

    #[test]
    fn geomean_balances_ratios() {
        assert!((geomean(&[2.0, 0.5]) - 1.0).abs() < 1e-12);
        assert!((geomean(&[1.21, 1.0, 1.0]) - 1.21f64.powf(1.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn throughput_in_mb_per_s() {
        assert_eq!(mb_per_s(4_000_000, 2.0), 2.0);
    }
}
