//! `--key value` flags and positionals, no dependency.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    pub flags: HashMap<String, String>,
}

impl Args {
    pub fn parse<I: IntoIterator<Item = String>>(items: I) -> Args {
        let mut out = Args::default();
        let mut it = items.into_iter();
        while let Some(a) = it.next() {
            if let Some(key) = a.strip_prefix("--") {
                let value = it.next().unwrap_or_default();
                out.flags.insert(key.to_owned(), value);
            } else {
                out.positional.push(a);
            }
        }
        out
    }

    pub fn flag(&self, key: &str) -> Result<&str, String> {
        self.flags
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing --{key}"))
    }

    pub fn flag_or_env(&self, key: &str, env: &str) -> Result<String, String> {
        if let Some(v) = self.flags.get(key) {
            return Ok(v.clone());
        }
        std::env::var(env).map_err(|_| format!("missing --{key} or ${env}"))
    }

    pub fn pos(&self, i: usize) -> Result<&str, String> {
        self.positional
            .get(i)
            .map(String::as_str)
            .ok_or_else(|| format!("missing argument {i}"))
    }
}
