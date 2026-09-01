//! Shared output-formatting helpers for every subcommand: the
//! `--format table|json` enum (CLI-004/010/018/027), ANSI color for
//! standalone status lines (CLI-006/015/020, etc.), and a minimal
//! box-drawn table renderer standing in for the source's Rich
//! `Table`.
//!
//! The table renderer is a deliberate approximation, not a faithful
//! port: Rich's exact box-drawing style and per-cell styling
//! (`header_style="bold cyan"`, bold row labels, ...) has no Rust
//! equivalent in this workspace and nothing in the manifest tests for
//! exact visual byte output (impossible without Rich itself) -- what's
//! testable, and what this renders correctly, is the title, column
//! headers, and cell content each command specifies.

/// Supported output formats for every subcommand (CLI-004/010/018/027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    Table,
    Json,
}

fn sgr(code: u8, text: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}

pub fn red(text: &str) -> String {
    sgr(31, text)
}

pub fn yellow(text: &str) -> String {
    sgr(33, text)
}

pub fn bold(text: &str) -> String {
    sgr(1, text)
}

pub fn cyan_bold(text: &str) -> String {
    format!("\x1b[1;36m{text}\x1b[0m")
}

/// A minimal box-drawn table: a title line, then a bordered grid of
/// headers and rows with column widths sized to their widest cell.
pub struct Table {
    title: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(title: impl Into<String>, headers: &[&str]) -> Self {
        Table {
            title: title.into(),
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn add_row(&mut self, row: Vec<String>) {
        debug_assert_eq!(row.len(), self.headers.len());
        self.rows.push(row);
    }

    pub fn render(&self) -> String {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }

        let border = |left: char, mid: char, right: char| -> String {
            let mut line = String::new();
            line.push(left);
            for (i, w) in widths.iter().enumerate() {
                line.push_str(&"─".repeat(w + 2));
                line.push(if i + 1 == widths.len() { right } else { mid });
            }
            line.push('\n');
            line
        };

        let render_row = |cells: &[String]| -> String {
            let mut line = String::from("│");
            for (cell, w) in cells.iter().zip(&widths) {
                line.push_str(&format!(" {cell:<w$} │"));
            }
            line.push('\n');
            line
        };

        let mut out = String::new();
        out.push_str(&format!("{}\n", cyan_bold(&self.title)));
        out.push_str(&border('┌', '┬', '┐'));
        out.push_str(&render_row(&self.headers));
        out.push_str(&border('├', '┼', '┤'));
        for row in &self.rows {
            out.push_str(&render_row(row));
        }
        out.push_str(&border('└', '┴', '┘'));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_title_headers_and_row_content() {
        let mut table = Table::new("Data Product: orders", &["Field", "Value"]);
        table.add_row(vec!["Name".to_string(), "orders".to_string()]);
        let rendered = table.render();
        assert!(rendered.contains("Data Product: orders"));
        assert!(rendered.contains("Field"));
        assert!(rendered.contains("Value"));
        assert!(rendered.contains("Name"));
        assert!(rendered.contains("orders"));
    }

    #[test]
    fn columns_are_aligned_to_the_widest_cell_in_each() {
        let mut table = Table::new("t", &["A", "B"]);
        table.add_row(vec!["short".to_string(), "x".to_string()]);
        table.add_row(vec!["a-much-longer-value".to_string(), "y".to_string()]);
        let rendered = table.render();
        let lines: Vec<&str> = rendered.lines().collect();
        let widths: Vec<usize> = lines[1..].iter().map(|l| l.chars().count()).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]));
    }

    #[test]
    fn colors_wrap_the_plain_text_so_it_stays_a_contiguous_substring() {
        let line = format!("{} Data product 'x' not found.", red("Error:"));
        assert!(line.contains("Data product 'x' not found."));
        assert!(line.contains("Error:"));
    }
}
