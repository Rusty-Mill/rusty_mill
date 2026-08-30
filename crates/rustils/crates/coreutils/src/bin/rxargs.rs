//! `rxargs` — build and execute command lines from standard input

use std::env;
use std::io::{self, BufRead};
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut cmd_name = "echo".to_string();
    let mut base_args = Vec::new();

    let non_opts: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    if !non_opts.is_empty() {
        cmd_name = non_opts[0].clone();
        base_args = non_opts[1..].iter().map(|s| (*s).clone()).collect();
    }

    let stdin = io::stdin();
    let mut items = Vec::new();

    for line in stdin.lock().lines() {
        let l = line?;
        for word in l.split_whitespace() {
            items.push(word.to_string());
        }
    }

    if items.is_empty() {
        return Ok(());
    }

    let mut full_args = base_args;
    full_args.extend(items);

    let status = Command::new(cmd_name).args(&full_args).status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
