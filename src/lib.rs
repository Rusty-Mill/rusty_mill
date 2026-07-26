//! `rusty_text`: Stream editing (sed) and pattern scanning/processing (awk) engines powered by `rusty_regx`.

use rusty_regx::Regex;

/// Sed substitute command definition.
pub struct SedSubst {
    pub pattern: Regex,
    pub replacement: String,
    pub global: bool,
}

impl SedSubst {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let trimmed = expr.trim();
        if !trimmed.starts_with("s/") {
            return Err("Sed expression must start with s/".to_string());
        }

        let parts: Vec<&str> = trimmed[2..].split('/').collect();
        if parts.len() < 2 {
            return Err("Invalid sed substitution format s/pattern/replacement/flags".to_string());
        }

        let pat_str = parts[0];
        let rep_str = parts[1];
        let flags = if parts.len() > 2 { parts[2] } else { "" };

        let pattern = Regex::new(pat_str).map_err(|e| format!("Regex error: {:?}", e))?;
        let global = flags.contains('g');

        Ok(SedSubst {
            pattern,
            replacement: rep_str.to_string(),
            global,
        })
    }

    pub fn apply(&self, line: &str) -> String {
        if !self.global {
            if let Some(m) = self.pattern.find(line) {
                let mut out = String::new();
                out.push_str(&line[..m.start()]);
                out.push_str(&self.replacement);
                out.push_str(&line[m.end()..]);
                return out;
            }
            line.to_string()
        } else {
            let mut out = String::new();
            let mut last_end = 0;
            for m in self.pattern.find_iter(line) {
                out.push_str(&line[last_end..m.start()]);
                out.push_str(&self.replacement);
                last_end = m.end();
            }
            out.push_str(&line[last_end..]);
            out
        }
    }
}

/// Simple Awk line processor engine.
pub struct AwkProcessor {
    pub field_sep: String,
    pub print_fields: Vec<usize>, // 0 for $0, 1 for $1, etc.
}

impl AwkProcessor {
    pub fn new(field_sep: &str, print_fields: Vec<usize>) -> Self {
        AwkProcessor {
            field_sep: field_sep.to_string(),
            print_fields,
        }
    }

    pub fn process_line(&self, line: &str) -> String {
        let fields: Vec<&str> = if self.field_sep == " " || self.field_sep.is_empty() {
            line.split_whitespace().collect()
        } else {
            line.split(&self.field_sep).collect()
        };

        let mut output_parts = Vec::new();
        for &idx in &self.print_fields {
            if idx == 0 {
                output_parts.push(line);
            } else if idx <= fields.len() {
                output_parts.push(fields[idx - 1]);
            } else {
                output_parts.push("");
            }
        }

        output_parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sed_subst() {
        let sed = SedSubst::parse("s/foo/bar/g").expect("parse failed");
        let result = sed.apply("foo test foo");
        assert_eq!(result, "bar test bar");
    }

    #[test]
    fn test_awk_processor() {
        let awk = AwkProcessor::new(" ", vec![1, 3]);
        let result = awk.process_line("apple banana cherry durian");
        assert_eq!(result, "apple cherry");
    }
}
