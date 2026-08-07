//! Reference tool 1: lists a directory and stats each entry through a
//! contract-scoped `FsRoot`. Exercises the filesystem primitive only —
//! no process spawn, no PTY.

use std::path::Path;

use compat::Workspace;
use contract::{Capabilities, FsRoot};

fn main() -> anyhow::Result<()> {
    let target = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let target_path = Path::new(&target);
    let root = target_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target_path
        .file_name()
        .map(Path::new)
        .unwrap_or(Path::new("."));

    let ws = Workspace::open_ambient(root)?;
    let caps = Capabilities::detect();
    println!("capabilities: {caps:?}");

    let meta = ws.stat(name)?;
    if meta.is_dir {
        println!("{:<32} {:>10} {:>6} {:>6}", "name", "bytes", "dir", "link");
        for entry in ws.read_dir(name)? {
            println!(
                "{:<32} {:>10} {:>6} {:>6}",
                entry.name, entry.metadata.len, entry.metadata.is_dir, entry.metadata.is_symlink
            );
        }
    } else {
        println!("{target}: {} bytes, readonly={}", meta.len, meta.readonly);
    }
    Ok(())
}
