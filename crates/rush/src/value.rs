//! Structured object model and pipeline data stream for PowerShell-like capabilities in rush.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

/// Structured data types carried through object pipelines and variables.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    /// Lookup a path like `name` or `nested.field` within an object or map.
    pub fn get_path(&self, path: &str) -> Option<Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = self.clone();

        for part in parts {
            match current {
                Value::Object(map) => {
                    current = map.get(part)?.clone();
                }
                _ => return None,
            }
        }
        Some(current)
    }

    /// Check truthiness for conditionals (`where`).
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Object(m) => !m.is_empty(),
        }
    }

    /// Convert value to standard text representation for display.
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::String(s) => s.clone(),
            Value::List(l) => {
                let items: Vec<String> = l.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Object(m) => {
                let pairs: Vec<String> = m
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_display_string()))
                    .collect();
                format!("{{{}}}", pairs.join(", "))
            }
        }
    }

    /// Convert to JSON string.
    pub fn to_json(&self, pretty: bool) -> String {
        let mut out = String::new();
        self.write_json(&mut out, pretty, 0);
        out
    }

    fn write_json(&self, out: &mut String, pretty: bool, indent_level: usize) {
        let indent = if pretty {
            "  ".repeat(indent_level)
        } else {
            String::new()
        };
        let next_indent = if pretty {
            "  ".repeat(indent_level + 1)
        } else {
            String::new()
        };
        let newline = if pretty { "\n" } else { "" };
        let space = if pretty { " " } else { "" };

        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Float(f) => out.push_str(&f.to_string()),
            Value::String(s) => {
                out.push('"');
                for ch in s.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Value::List(l) => {
                if l.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                out.push_str(newline);
                for (i, elem) in l.iter().enumerate() {
                    out.push_str(&next_indent);
                    elem.write_json(out, pretty, indent_level + 1);
                    if i + 1 < l.len() {
                        out.push(',');
                    }
                    out.push_str(newline);
                }
                out.push_str(&indent);
                out.push(']');
            }
            Value::Object(map) => {
                if map.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                out.push_str(newline);
                let len = map.len();
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(&next_indent);
                    out.push('"');
                    out.push_str(k);
                    out.push_str("\":");
                    out.push_str(space);
                    v.write_json(out, pretty, indent_level + 1);
                    if i + 1 < len {
                        out.push(',');
                    }
                    out.push_str(newline);
                }
                out.push_str(&indent);
                out.push('}');
            }
        }
    }

    /// Simple hand-rolled recursive descent JSON parser.
    pub fn parse_json(input: &str) -> Result<Value, String> {
        let chars: Vec<char> = input.chars().collect();
        let mut idx = 0;
        skip_ws(&chars, &mut idx);
        let val = parse_json_val(&chars, &mut idx)?;
        skip_ws(&chars, &mut idx);
        if idx < chars.len() {
            return Err(format!("Trailing characters at index {}", idx));
        }
        Ok(val)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_display_string())
    }
}

fn skip_ws(chars: &[char], idx: &mut usize) {
    while *idx < chars.len() && chars[*idx].is_whitespace() {
        *idx += 1;
    }
}

fn parse_json_val(chars: &[char], idx: &mut usize) -> Result<Value, String> {
    skip_ws(chars, idx);
    if *idx >= chars.len() {
        return Err("Unexpected EOF while parsing JSON".to_string());
    }

    match chars[*idx] {
        'n' => {
            expect_str(chars, idx, "null")?;
            Ok(Value::Null)
        }
        't' => {
            expect_str(chars, idx, "true")?;
            Ok(Value::Bool(true))
        }
        'f' => {
            expect_str(chars, idx, "false")?;
            Ok(Value::Bool(false))
        }
        '"' => parse_json_string(chars, idx).map(Value::String),
        '[' => parse_json_list(chars, idx),
        '{' => parse_json_object(chars, idx),
        '-' | '0'..='9' => parse_json_number(chars, idx),
        c => Err(format!("Unexpected character '{}' at index {}", c, idx)),
    }
}

fn expect_str(chars: &[char], idx: &mut usize, target: &str) -> Result<(), String> {
    for ch in target.chars() {
        if *idx >= chars.len() || chars[*idx] != ch {
            return Err(format!("Expected '{}' at index {}", target, idx));
        }
        *idx += 1;
    }
    Ok(())
}

fn parse_json_string(chars: &[char], idx: &mut usize) -> Result<String, String> {
    if *idx >= chars.len() || chars[*idx] != '"' {
        return Err("Expected starting quote".to_string());
    }
    *idx += 1;
    let mut res = String::new();
    while *idx < chars.len() {
        let c = chars[*idx];
        *idx += 1;
        match c {
            '"' => return Ok(res),
            '\\' => {
                if *idx >= chars.len() {
                    return Err("Unterminated escape sequence".to_string());
                }
                let esc = chars[*idx];
                *idx += 1;
                match esc {
                    '"' => res.push('"'),
                    '\\' => res.push('\\'),
                    '/' => res.push('/'),
                    'n' => res.push('\n'),
                    'r' => res.push('\r'),
                    't' => res.push('\t'),
                    c => res.push(c),
                }
            }
            c => res.push(c),
        }
    }
    Err("Unterminated string literal".to_string())
}

fn parse_json_number(chars: &[char], idx: &mut usize) -> Result<Value, String> {
    let start = *idx;
    if *idx < chars.len() && chars[*idx] == '-' {
        *idx += 1;
    }
    let mut is_float = false;
    while *idx < chars.len() {
        match chars[*idx] {
            '0'..='9' => *idx += 1,
            '.' | 'e' | 'E' => {
                is_float = true;
                *idx += 1;
            }
            _ => break,
        }
    }
    let num_str: String = chars[start..*idx].iter().collect();
    if is_float {
        num_str
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|e| e.to_string())
    } else {
        num_str
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|e| e.to_string())
    }
}

fn parse_json_list(chars: &[char], idx: &mut usize) -> Result<Value, String> {
    *idx += 1; // skip '['
    let mut list = Vec::new();
    skip_ws(chars, idx);
    if *idx < chars.len() && chars[*idx] == ']' {
        *idx += 1;
        return Ok(Value::List(list));
    }

    loop {
        let val = parse_json_val(chars, idx)?;
        list.push(val);
        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err("Unterminated JSON array".to_string());
        }
        if chars[*idx] == ']' {
            *idx += 1;
            break;
        } else if chars[*idx] == ',' {
            *idx += 1;
        } else {
            return Err(format!("Expected ',' or ']' at index {}", idx));
        }
    }
    Ok(Value::List(list))
}

fn parse_json_object(chars: &[char], idx: &mut usize) -> Result<Value, String> {
    *idx += 1; // skip '{'
    let mut map = BTreeMap::new();
    skip_ws(chars, idx);
    if *idx < chars.len() && chars[*idx] == '}' {
        *idx += 1;
        return Ok(Value::Object(map));
    }

    loop {
        skip_ws(chars, idx);
        if *idx >= chars.len() || chars[*idx] != '"' {
            return Err(format!("Expected string key in object at index {}", idx));
        }
        let key = parse_json_string(chars, idx)?;
        skip_ws(chars, idx);
        if *idx >= chars.len() || chars[*idx] != ':' {
            return Err(format!("Expected ':' after key at index {}", idx));
        }
        *idx += 1; // skip ':'
        let val = parse_json_val(chars, idx)?;
        map.insert(key, val);
        skip_ws(chars, idx);
        if *idx >= chars.len() {
            return Err("Unterminated JSON object".to_string());
        }
        if chars[*idx] == '}' {
            *idx += 1;
            break;
        } else if chars[*idx] == ',' {
            *idx += 1;
        } else {
            return Err(format!("Expected ',' or '}}' at index {}", idx));
        }
    }
    Ok(Value::Object(map))
}

/// Format a list of objects as a pretty ASCII table (PowerShell `Format-Table` style).
pub fn format_table(items: &[Value]) -> String {
    if items.is_empty() {
        return String::new();
    }

    // Check if items are Objects
    let mut keys = Vec::new();
    for item in items {
        if let Value::Object(map) = item {
            for k in map.keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    }

    if keys.is_empty() {
        // Fallback for non-object lists
        let mut out = String::new();
        for item in items {
            out.push_str(&item.to_display_string());
            out.push('\n');
        }
        return out;
    }

    // Determine column widths
    let mut col_widths: BTreeMap<String, usize> = BTreeMap::new();
    for key in &keys {
        col_widths.insert(key.clone(), key.len());
    }

    for item in items {
        if let Value::Object(map) = item {
            for key in &keys {
                let cell_str = map
                    .get(key)
                    .map(|v| v.to_display_string())
                    .unwrap_or_default();
                let width = col_widths.get_mut(key).unwrap();
                if cell_str.len() > *width {
                    *width = cell_str.len();
                }
            }
        }
    }

    let mut out = String::new();

    // Header
    for (i, key) in keys.iter().enumerate() {
        let width = col_widths[key];
        out.push_str(&format!("{:width$}", key, width = width));
        if i + 1 < keys.len() {
            out.push_str("  ");
        }
    }
    out.push('\n');

    // Divider
    for (i, key) in keys.iter().enumerate() {
        let width = col_widths[key];
        out.push_str(&"-".repeat(width));
        if i + 1 < keys.len() {
            out.push_str("  ");
        }
    }
    out.push('\n');

    // Rows
    for item in items {
        if let Value::Object(map) = item {
            for (i, key) in keys.iter().enumerate() {
                let cell_str = map
                    .get(key)
                    .map(|v| v.to_display_string())
                    .unwrap_or_default();
                let width = col_widths[key];
                out.push_str(&format!("{:width$}", cell_str, width = width));
                if i + 1 < keys.len() {
                    out.push_str("  ");
                }
            }
            out.push('\n');
        }
    }

    out
}

thread_local! {
    /// Holds the pipeline's active object stream input (if previous stage sent objects).
    static OBJECT_INPUT: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// Holds the pipeline's active object stream output (if current stage produces objects).
    static OBJECT_OUTPUT: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// Flag indicating whether the pipeline output produced structured objects.
    static HAS_OBJECT_OUTPUT: RefCell<bool> = const { RefCell::new(false) };
}

/// Set input objects for the next pipeline stage.
pub fn set_pipeline_input(items: Vec<Value>) {
    OBJECT_INPUT.with(|cell| {
        *cell.borrow_mut() = items;
    });
}

/// Take input objects for the current pipeline stage.
pub fn take_pipeline_input() -> Vec<Value> {
    OBJECT_INPUT.with(|cell| std::mem::take(&mut *cell.borrow_mut()))
}

/// Push an output object from the current pipeline stage.
pub fn push_pipeline_output(val: Value) {
    OBJECT_OUTPUT.with(|cell| {
        cell.borrow_mut().push(val);
    });
    HAS_OBJECT_OUTPUT.with(|cell| {
        *cell.borrow_mut() = true;
    });
}

/// Take output objects from the completed pipeline stage.
pub fn take_pipeline_output() -> (Vec<Value>, bool) {
    let items = OBJECT_OUTPUT.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
    let has = HAS_OBJECT_OUTPUT.with(|cell| {
        let val = *cell.borrow();
        *cell.borrow_mut() = false;
        val
    });
    (items, has)
}

/// Clear object stream state between commands.
pub fn reset_pipeline_stream() {
    OBJECT_INPUT.with(|cell| cell.borrow_mut().clear());
    OBJECT_OUTPUT.with(|cell| cell.borrow_mut().clear());
    HAS_OBJECT_OUTPUT.with(|cell| *cell.borrow_mut() = false);
}
