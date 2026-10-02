//! `rusty_baseline`: measures each product in `products.txt` and prints one
//! Markdown table — binary size, dependency closure, clean and incremental
//! release build, startup, idle and peak RSS.
//!
//! ```text
//! rusty_baseline [--products PATH] [--work DIR] [--runs N] [--settle SECS]
//!                [--prebuilt TARGET_DIR] [BIN...]
//! ```
//!
//! Each product builds clean into its own target directory under `--work`,
//! which is deleted once measured, so peak disk use is one product's build.
//! `--prebuilt` skips the builds and runs binaries from an existing target
//! directory instead. Naming `BIN`s measures only those products.

mod cargo;
mod measure;
mod products;
mod report;
mod sys;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use products::Product;
use report::Row;

struct Options {
    products: PathBuf,
    work: PathBuf,
    runs: usize,
    settle: Duration,
    prebuilt: Option<PathBuf>,
    only: Vec<String>,
}

fn main() -> ExitCode {
    match options(std::env::args().skip(1)).and_then(|options| baseline(&options)) {
        Ok(table) => {
            print!("{table}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rusty_baseline: {error}");
            ExitCode::FAILURE
        }
    }
}

fn options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        products: Path::new(env!("CARGO_MANIFEST_DIR")).join("products.txt"),
        work: std::env::temp_dir().join("rusty_baseline"),
        runs: 5,
        settle: Duration::from_secs(3),
        prebuilt: None,
        only: Vec::new(),
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--products" => options.products = value()?.into(),
            "--work" => options.work = value()?.into(),
            "--runs" => options.runs = number(&value()?)?,
            "--settle" => options.settle = Duration::from_secs(number(&value()?)? as u64),
            "--prebuilt" => options.prebuilt = Some(value()?.into()),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            bin => options.only.push(bin.to_owned()),
        }
    }
    Ok(options)
}

fn number(text: &str) -> Result<usize, String> {
    text.parse()
        .map_err(|_| format!("`{text}` is not a whole number"))
}

fn baseline(options: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&options.products)
        .map_err(|error| format!("reading {}: {error}", options.products.display()))?;
    let products: Vec<Product> = products::parse(&text)?
        .into_iter()
        .filter(|product| options.only.is_empty() || options.only.contains(&product.bin))
        .collect();
    if products.is_empty() {
        return Err("no products selected".to_owned());
    }
    let eligible: Vec<Product> = products
        .iter()
        .filter(|product| product.supports_os(std::env::consts::OS))
        .cloned()
        .collect();
    let entry_points = if eligible.is_empty() {
        Vec::new()
    } else {
        cargo::entry_points(&eligible)?
    };
    let home = options.work.join("home");
    if !eligible.is_empty() {
        std::fs::create_dir_all(&home)
            .map_err(|error| format!("creating {}: {error}", home.display()))?;
    }

    let mut rows = Vec::with_capacity(products.len());
    let mut entry_points = entry_points.iter();
    for product in &products {
        let Some(platform) = product
            .platform
            .filter(|_| !product.supports_os(std::env::consts::OS))
        else {
            let entry_point = entry_points
                .next()
                .expect("one entry point per eligible product");
            eprintln!("rusty_baseline: measuring {}", product.bin);
            rows.push(measure_one(options, product, entry_point, &home));
            continue;
        };
        eprintln!(
            "rusty_baseline: skipping {}: unsupported on {} (allowed: {})",
            product.bin,
            std::env::consts::OS,
            platform.name()
        );
        rows.push(Row::Unsupported {
            bin: product.bin.clone(),
            current: std::env::consts::OS.to_owned(),
            allowed: platform.name().to_owned(),
        });
    }
    let floor = sys::floor();
    Ok(format!(
        "{}\n{}",
        environment(floor),
        report::render(&rows, floor)
    ))
}

fn measure_one(options: &Options, product: &Product, entry_point: &Path, home: &Path) -> Row {
    let closure = cargo::closure(product);
    let (target_dir, build) = match &options.prebuilt {
        Some(target_dir) => (target_dir.clone(), Ok(None)),
        None => {
            let target_dir = options.work.join("target").join(&product.bin);
            let build = cargo::build(product, &target_dir, entry_point).map(Some);
            (target_dir, build)
        }
    };
    let binary = cargo::binary(&target_dir, &product.bin);
    let size = std::fs::metadata(&binary)
        .ok()
        .map(|metadata| metadata.len());
    let run = match size {
        Some(_) => measure::run(product, &binary, home, options.runs, options.settle),
        None => Err(format!("no binary at {}", binary.display())),
    };
    if options.prebuilt.is_none() {
        if let Err(error) = std::fs::remove_dir_all(&target_dir) {
            eprintln!("rusty_baseline: leaving {}: {error}", target_dir.display());
        }
    }
    Row::Measured {
        bin: product.bin.clone(),
        closure,
        build,
        size,
        run,
    }
}

/// One line naming what the numbers were measured on.
fn environment(floor: Option<u64>) -> String {
    let probe = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_else(|| "unknown".to_owned())
    };
    let cpus = std::thread::available_parallelism().map_or(0, usize::from);
    let floor = floor.map_or_else(String::new, |bytes| {
        format!(
            " An exiting product's peak RSS cannot read below {:.1} MiB here: Linux counts the spawner's resident set at `exec` (measured on `true`). Servers' peaks are read from the live process, so have no floor.",
            bytes as f64 / 1_048_576.0
        )
    });
    format!(
        "{}-{}, {cpus} CPUs, {}, commit {}. Release profile; startup is the median of the timed runs.{floor}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        probe("rustc", &["-V"]),
        probe("git", &["rev-parse", "--short", "HEAD"]),
    )
}
