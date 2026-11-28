//! `lel-treegen plan`: writes the manifest of a seeded tree (no tree bytes yet).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lel_treegen::{Profile, manifest, plan::plan};

#[derive(Parser)]
#[command(
    name = "lel-treegen",
    about = "Seeded synthetic dedup test tree (LEL #1319)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Plan the tree and write `manifest.jsonl` and `manifest.sha256` into a new directory.
    Plan {
        #[arg(long)]
        seed: u64,
        /// 1.0 is the 500 GB tree; 0.1 the 50 GB slice.
        #[arg(long)]
        scale: f64,
        /// A `lel-drive-stats/2` JSON file; the built-in default profile otherwise.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Harness-only directory to create; it must not exist yet.
        #[arg(long)]
        manifest: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lel-treegen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let Command::Plan {
        seed,
        scale,
        profile,
        manifest: dir,
    } = cli.command;
    let profile = match profile {
        Some(path) => Profile::from_stats(&fs::read_to_string(&path)?)?,
        None => Profile::default(),
    };
    let tree = plan(seed, scale, &profile)?;
    let bytes = manifest::render(&tree, &profile)?;
    let digest = manifest::digest(&bytes);
    fs::create_dir(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_new(&dir.join("manifest.jsonl"), &bytes)?;
    write_new(
        &dir.join("manifest.sha256"),
        format!("{digest}  manifest.jsonl\n").as_bytes(),
    )?;
    println!(
        "{} entries, {} groups, {} bytes; manifest sha256 {digest}",
        tree.entries.len(),
        tree.groups.len(),
        tree.bytes()
    );
    Ok(())
}

/// Creates `path`, refusing to overwrite an existing file.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}
