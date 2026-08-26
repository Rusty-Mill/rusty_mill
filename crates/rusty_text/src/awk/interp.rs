//! Evaluates a parsed awk [`Program`](super::ast::Program) over input
//! records. Values are awk's usual dynamically-typed number-or-string;
//! comparisons are numeric when both sides look like a full number,
//! string otherwise — the common-case subset of awk's numeric-string
//! rules, not the full standard's edge cases.

use std::collections::HashMap;

use rusty_regx::Regex;

use super::ast::{BinOp, Expr, LValue, Pattern, Program, Rule, Stmt};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    pub fn to_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) => parse_leading_number(s),
        }
    }

    pub fn to_str(&self) -> String {
        match self {
            Value::Num(n) => format_num(*n),
            Value::Str(s) => s.clone(),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
        }
    }

    fn looks_numeric(&self) -> bool {
        match self {
            Value::Num(_) => true,
            Value::Str(s) => s.trim().parse::<f64>().is_ok(),
        }
    }
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn parse_leading_number(s: &str) -> f64 {
    let trimmed = s.trim_start();
    let mut end = 0;
    let bytes = trimmed.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        saw_digit = true;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            saw_digit = true;
        }
    }
    if !saw_digit {
        return 0.0;
    }
    trimmed[..end].parse().unwrap_or(0.0)
}

fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    if a.looks_numeric() && b.looks_numeric() {
        a.to_num().partial_cmp(&b.to_num()).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        a.to_str().cmp(&b.to_str())
    }
}

/// Per-run interpreter state: the current record's fields, `NR`, and
/// user/special variables (`FS`, `OFS`).
pub struct Interp {
    vars: HashMap<String, Value>,
    fields: Vec<String>,
    record: String,
    fs: String,
    ofs: String,
    nr: usize,
}

impl Interp {
    pub fn new(field_sep: &str) -> Self {
        Interp {
            vars: HashMap::new(),
            fields: Vec::new(),
            record: String::new(),
            fs: field_sep.to_string(),
            ofs: " ".to_string(),
            nr: 0,
        }
    }

    fn split_record(&mut self) {
        self.fields = if self.fs == " " || self.fs.is_empty() {
            self.record.split_whitespace().map(str::to_string).collect()
        } else if self.fs.chars().count() == 1 {
            let sep = self.fs.chars().next().unwrap();
            self.record.split(sep).map(str::to_string).collect()
        } else {
            // Multi-char FS: literal substring split. Real awk treats this
            // as an ERE; that's a known, documented scope cut here.
            self.record.split(self.fs.as_str()).map(str::to_string).collect()
        };
    }

    /// Loads a new input record: sets `$0`, re-splits fields, and bumps `NR`.
    pub fn set_record(&mut self, record: &str) {
        self.record = record.to_string();
        self.split_record();
        self.nr += 1;
    }

    fn get_field(&self, n: i64) -> String {
        if n == 0 {
            self.record.clone()
        } else if n >= 1 && (n as usize) <= self.fields.len() {
            self.fields[n as usize - 1].clone()
        } else {
            String::new()
        }
    }

    fn set_field(&mut self, n: i64, value: String) {
        if n == 0 {
            self.record = value;
            self.split_record();
            return;
        }
        if n < 1 {
            return;
        }
        let idx = n as usize - 1;
        if idx >= self.fields.len() {
            self.fields.resize(idx + 1, String::new());
        }
        self.fields[idx] = value;
        self.record = self.fields.join(&self.ofs);
    }

    fn get_var(&self, name: &str) -> Value {
        match name {
            "NR" => Value::Num(self.nr as f64),
            "NF" => Value::Num(self.fields.len() as f64),
            "FS" => Value::Str(self.fs.clone()),
            "OFS" => Value::Str(self.ofs.clone()),
            _ => self.vars.get(name).cloned().unwrap_or(Value::Str(String::new())),
        }
    }

    fn set_var(&mut self, name: &str, value: Value) {
        match name {
            "NF" => {
                let n = value.to_num().max(0.0) as usize;
                self.fields.resize(n, String::new());
                self.record = self.fields.join(&self.ofs);
            }
            "FS" => self.fs = value.to_str(),
            "OFS" => self.ofs = value.to_str(),
            "NR" => self.nr = value.to_num().max(0.0) as usize,
            _ => {
                self.vars.insert(name.to_string(), value);
            }
        }
    }

    fn eval(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Field(inner) => {
                let n = self.eval(inner).to_num() as i64;
                Value::Str(self.get_field(n))
            }
            Expr::Var(name) => self.get_var(name),
            Expr::Concat(a, b) => {
                let mut s = self.eval(a).to_str();
                s.push_str(&self.eval(b).to_str());
                Value::Str(s)
            }
            Expr::BinOp(op, a, b) => self.eval_binop(op, a, b),
            Expr::Not(e) => Value::Num(if self.eval(e).truthy() { 0.0 } else { 1.0 }),
            Expr::Neg(e) => Value::Num(-self.eval(e).to_num()),
            Expr::Match { expr, pattern, negate } => {
                let text = self.eval(expr).to_str();
                // Compiled fresh per evaluation -- a known perf
                // simplification (no compiled-regex cache), fine at the
                // line-processing scale this engine targets.
                let is_match = Regex::new(pattern).map(|re| re.is_match(&text)).unwrap_or(false);
                Value::Num(if is_match != *negate { 1.0 } else { 0.0 })
            }
            Expr::Assign(lvalue, value_expr) => {
                let value = self.eval(value_expr);
                match lvalue {
                    LValue::Var(name) => self.set_var(name, value.clone()),
                    LValue::Field(inner) => {
                        let n = self.eval(inner).to_num() as i64;
                        self.set_field(n, value.to_str());
                    }
                }
                value
            }
        }
    }

    fn eval_binop(&mut self, op: &BinOp, a: &Expr, b: &Expr) -> Value {
        match op {
            BinOp::And => {
                if !self.eval(a).truthy() {
                    return Value::Num(0.0);
                }
                Value::Num(if self.eval(b).truthy() { 1.0 } else { 0.0 })
            }
            BinOp::Or => {
                if self.eval(a).truthy() {
                    return Value::Num(1.0);
                }
                Value::Num(if self.eval(b).truthy() { 1.0 } else { 0.0 })
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let x = self.eval(a).to_num();
                let y = self.eval(b).to_num();
                Value::Num(match op {
                    BinOp::Add => x + y,
                    BinOp::Sub => x - y,
                    BinOp::Mul => x * y,
                    BinOp::Div => x / y,
                    BinOp::Mod => x % y,
                    _ => unreachable!(),
                })
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let x = self.eval(a);
                let y = self.eval(b);
                let ord = compare(&x, &y);
                let result = match op {
                    BinOp::Eq => ord == std::cmp::Ordering::Equal,
                    BinOp::Ne => ord != std::cmp::Ordering::Equal,
                    BinOp::Lt => ord == std::cmp::Ordering::Less,
                    BinOp::Le => ord != std::cmp::Ordering::Greater,
                    BinOp::Gt => ord == std::cmp::Ordering::Greater,
                    BinOp::Ge => ord != std::cmp::Ordering::Less,
                    _ => unreachable!(),
                };
                Value::Num(if result { 1.0 } else { 0.0 })
            }
        }
    }

    fn exec(&mut self, stmt: &Stmt, emit: &mut dyn FnMut(&str)) {
        match stmt {
            Stmt::Print(exprs) => {
                if exprs.is_empty() {
                    emit(&self.get_field(0));
                } else {
                    let parts: Vec<String> = exprs.iter().map(|e| self.eval(e).to_str()).collect();
                    emit(&parts.join(&self.ofs));
                }
            }
            Stmt::Expr(e) => {
                self.eval(e);
            }
            Stmt::If(cond, then_branch, else_branch) => {
                if self.eval(cond).truthy() {
                    self.exec(then_branch, emit);
                } else if let Some(e) = else_branch {
                    self.exec(e, emit);
                }
            }
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.exec(s, emit);
                }
            }
        }
    }

    fn pattern_matches(&mut self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Always => true,
            Pattern::Expr(e) => self.eval(e).truthy(),
            Pattern::Begin | Pattern::End => false, // driven separately
        }
    }

    fn run_action(&mut self, rule: &Rule, emit: &mut dyn FnMut(&str)) {
        match &rule.action {
            Some(stmts) => {
                for s in stmts {
                    self.exec(s, emit);
                }
            }
            None => emit(&self.get_field(0)),
        }
    }

    /// Runs every `BEGIN` rule's action, in order.
    pub fn run_begin(&mut self, program: &Program, emit: &mut dyn FnMut(&str)) {
        for rule in &program.rules {
            if rule.pattern == Pattern::Begin {
                self.run_action(rule, emit);
            }
        }
    }

    /// Runs every non-`BEGIN`/`END` rule whose pattern matches the current
    /// record (set via [`Self::set_record`]).
    pub fn run_main_rules(&mut self, program: &Program, emit: &mut dyn FnMut(&str)) {
        for rule in &program.rules {
            if matches!(rule.pattern, Pattern::Begin | Pattern::End) {
                continue;
            }
            if self.pattern_matches(&rule.pattern) {
                self.run_action(rule, emit);
            }
        }
    }

    /// Runs every `END` rule's action, in order.
    pub fn run_end(&mut self, program: &Program, emit: &mut dyn FnMut(&str)) {
        for rule in &program.rules {
            if rule.pattern == Pattern::End {
                self.run_action(rule, emit);
            }
        }
    }
}
