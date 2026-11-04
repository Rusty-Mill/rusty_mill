//! `bbp verify`: check a task log's hash chain (#722) and print its head.
//!
//! Reads the one log under the directory lock and never changes it, so it
//! works when `FsStore::open` refuses the directory, and a cut last line is
//! reported, not repaired away. `--expect H` passes when `H` is the hash of
//! any line: a head saved earlier still verifies after legitimate appends,
//! while a truncation or rewrite that drops it fails.

use rusty_bbp::fs_store::verify_log;
use rusty_bbp::*;
use std::path::Path;

/// The chain as JSON. `Err` (after printing the JSON) when the log has a
/// partial last line or does not contain `expect`; `Err` alone when the
/// chain is broken or the log can't be read.
pub fn json(dir: &Path, task: &TaskId, expect: Option<&str>) -> Result<String, String> {
    let expect = expect
        .map(|h| Sha256::from_hex(h).ok_or("--expect must be 64 hex digits"))
        .transpose()?;
    let chain = verify_log(dir, task).map_err(|e| e.to_string())?;
    let found = expect.is_none_or(|h| chain.hashes.contains(&h));
    let out = format!(
        "{{\"lines\":{},\"unchained\":{},\"head\":\"{}\",\"partial\":{},\"expect_found\":{found}}}",
        chain.hashes.len(),
        chain.unchained,
        chain.head().hex(),
        chain.partial,
    );
    if chain.partial || !found {
        println!("{out}");
        return Err(match chain.partial {
            true => "the log ends in a partial line".into(),
            false => "--expect is not a line of this log".into(),
        });
    }
    Ok(out)
}
