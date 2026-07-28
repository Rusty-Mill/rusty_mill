//! Walks a compiled [`Node`] tree, evaluating expressions against a
//! `rusty_json::Value` context and producing the rendered string.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rusty_json::{Map, Number, Value};

use crate::ast::{BinOp, Expr, Node};
use crate::template::JinjaError;

type Scope = BTreeMap<String, Value>;

/// Renders `nodes` against `context` (typically a JSON object holding
/// `messages`, `add_generation_prompt`, `bos_token`, etc.).
pub fn render(nodes: &[Node], context: &Value) -> Result<String, JinjaError> {
    let mut scopes: Vec<Scope> = alloc::vec![Scope::new()];
    let mut out = String::new();
    render_nodes(nodes, context, &mut scopes, &mut out)?;
    Ok(out)
}

fn render_nodes(nodes: &[Node], context: &Value, scopes: &mut Vec<Scope>, out: &mut String) -> Result<(), JinjaError> {
    for node in nodes {
        match node {
            Node::Text(s) => out.push_str(s),
            Node::Output(expr) => out.push_str(&display(&eval(expr, context, scopes)?)),
            Node::Set { var, value } => {
                let v = eval(value, context, scopes)?;
                scopes.last_mut().expect("render always keeps at least one scope").insert(var.clone(), v);
            }
            Node::If { branches, else_branch } => {
                let mut matched = false;
                for (cond, body) in branches {
                    if truthy(&eval(cond, context, scopes)?) {
                        render_nodes(body, context, scopes, out)?;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    if let Some(else_body) = else_branch {
                        render_nodes(else_body, context, scopes, out)?;
                    }
                }
            }
            Node::For { var, iterable, body } => {
                let items = eval_iterable(iterable, context, scopes)?;
                let len = items.len();
                for (i, item) in items.into_iter().enumerate() {
                    let mut scope = Scope::new();
                    scope.insert(var.clone(), item);
                    let mut loop_obj = Map::new();
                    loop_obj.insert("index".into(), Value::Number(Number::from((i + 1) as i64)));
                    loop_obj.insert("index0".into(), Value::Number(Number::from(i as i64)));
                    loop_obj.insert("first".into(), Value::Bool(i == 0));
                    loop_obj.insert("last".into(), Value::Bool(i + 1 == len));
                    loop_obj.insert("length".into(), Value::Number(Number::from(len as i64)));
                    scope.insert("loop".into(), Value::Object(loop_obj));
                    scopes.push(scope);
                    let result = render_nodes(body, context, scopes, out);
                    scopes.pop();
                    result?;
                }
            }
        }
    }
    Ok(())
}

fn eval_iterable(expr: &Expr, context: &Value, scopes: &[Scope]) -> Result<Vec<Value>, JinjaError> {
    match eval(expr, context, scopes)? {
        Value::Array(items) => Ok(items),
        Value::Null => Ok(Vec::new()),
        _ => Err(JinjaError::Expression("'for' target is not iterable")),
    }
}

fn lookup_var(name: &str, context: &Value, scopes: &[Scope]) -> Option<Value> {
    for scope in scopes.iter().rev() {
        if let Some(v) = scope.get(name) {
            return Some(v.clone());
        }
    }
    context.get(name).cloned()
}

fn is_defined(expr: &Expr, context: &Value, scopes: &[Scope]) -> bool {
    match expr {
        Expr::Var(name) => lookup_var(name, context, scopes).is_some(),
        Expr::Attr(base, name) => eval(base, context, scopes).map(|v| v.get(name).is_some()).unwrap_or(false),
        _ => true,
    }
}

fn eval(expr: &Expr, context: &Value, scopes: &[Scope]) -> Result<Value, JinjaError> {
    match expr {
        Expr::Str(s) => Ok(Value::String(s.clone())),
        Expr::Num(n) => Ok(f64_to_value(*n)),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::None => Ok(Value::Null),
        Expr::Var(name) => Ok(lookup_var(name, context, scopes).unwrap_or(Value::Null)),
        Expr::Attr(base, name) => {
            let base_val = eval(base, context, scopes)?;
            Ok(base_val.get(name).cloned().unwrap_or(Value::Null))
        }
        Expr::Index(base, idx) => {
            let base_val = eval(base, context, scopes)?;
            let idx_val = eval(idx, context, scopes)?;
            let i = num(&idx_val) as i64;
            if i < 0 {
                return Ok(Value::Null);
            }
            Ok(base_val.get_index(i as usize).cloned().unwrap_or(Value::Null))
        }
        Expr::Not(e) => Ok(Value::Bool(!truthy(&eval(e, context, scopes)?))),
        Expr::Concat(a, b) => {
            let mut s = display(&eval(a, context, scopes)?);
            s.push_str(&display(&eval(b, context, scopes)?));
            Ok(Value::String(s))
        }
        Expr::BinOp(op, a, b) => eval_binop(op, a, b, context, scopes),
        Expr::Filter(base, name, args) => {
            let base_val = eval(base, context, scopes)?;
            let arg_vals: Vec<Value> =
                args.iter().map(|a| eval(a, context, scopes)).collect::<Result<_, _>>()?;
            apply_filter(name, base_val, &arg_vals)
        }
        Expr::Test(base, name, negate) => {
            let result = match name.as_str() {
                "defined" => is_defined(base, context, scopes),
                "none" => matches!(eval(base, context, scopes)?, Value::Null),
                "string" => eval(base, context, scopes)?.is_string(),
                "number" => eval(base, context, scopes)?.is_number(),
                "mapping" => eval(base, context, scopes)?.is_object(),
                "iterable" => matches!(
                    eval(base, context, scopes)?,
                    Value::Array(_) | Value::String(_) | Value::Object(_)
                ),
                _ => return Err(JinjaError::Expression("unknown 'is' test")),
            };
            Ok(Value::Bool(result != *negate))
        }
        Expr::In(needle, haystack, negate) => {
            let n = eval(needle, context, scopes)?;
            let h = eval(haystack, context, scopes)?;
            let result = match &h {
                Value::Array(items) => items.contains(&n),
                Value::String(s) => n.as_str().map(|needle_s| s.contains(needle_s)).unwrap_or(false),
                Value::Object(map) => n.as_str().map(|k| map.get(k).is_some()).unwrap_or(false),
                _ => false,
            };
            Ok(Value::Bool(result != *negate))
        }
    }
}

fn eval_binop(op: &BinOp, a: &Expr, b: &Expr, context: &Value, scopes: &[Scope]) -> Result<Value, JinjaError> {
    if *op == BinOp::And {
        let av = eval(a, context, scopes)?;
        if !truthy(&av) {
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(truthy(&eval(b, context, scopes)?)));
    }
    if *op == BinOp::Or {
        let av = eval(a, context, scopes)?;
        if truthy(&av) {
            return Ok(Value::Bool(true));
        }
        return Ok(Value::Bool(truthy(&eval(b, context, scopes)?)));
    }

    let av = eval(a, context, scopes)?;
    let bv = eval(b, context, scopes)?;
    Ok(match op {
        BinOp::Add => match (&av, &bv) {
            (Value::String(sa), Value::String(sb)) => Value::String(format!("{sa}{sb}")),
            _ => f64_to_value(num(&av) + num(&bv)),
        },
        BinOp::Sub => f64_to_value(num(&av) - num(&bv)),
        BinOp::Eq => Value::Bool(values_eq(&av, &bv)),
        BinOp::Ne => Value::Bool(!values_eq(&av, &bv)),
        BinOp::Lt => Value::Bool(compare(&av, &bv) == core::cmp::Ordering::Less),
        BinOp::Le => Value::Bool(compare(&av, &bv) != core::cmp::Ordering::Greater),
        BinOp::Gt => Value::Bool(compare(&av, &bv) == core::cmp::Ordering::Greater),
        BinOp::Ge => Value::Bool(compare(&av, &bv) != core::cmp::Ordering::Less),
        BinOp::And | BinOp::Or => unreachable!("handled above"),
    })
}

fn values_eq(a: &Value, b: &Value) -> bool {
    if let (Some(x), Some(y)) = (a.as_str(), b.as_str()) {
        return x == y;
    }
    if a.is_number() && b.is_number() {
        return num(a) == num(b);
    }
    a == b
}

fn compare(a: &Value, b: &Value) -> core::cmp::Ordering {
    if let (Some(x), Some(y)) = (a.as_str(), b.as_str()) {
        return x.cmp(y);
    }
    num(a).partial_cmp(&num(b)).unwrap_or(core::cmp::Ordering::Equal)
}

fn apply_filter(name: &str, value: Value, args: &[Value]) -> Result<Value, JinjaError> {
    match name {
        "trim" | "strip" => Ok(Value::String(display(&value).trim().to_string())),
        "upper" => Ok(Value::String(display(&value).to_uppercase())),
        "lower" => Ok(Value::String(display(&value).to_lowercase())),
        "title" => Ok(Value::String(title_case(&display(&value)))),
        "string" => Ok(Value::String(display(&value))),
        "length" | "count" => Ok(Value::Number(Number::from(value_length(&value) as i64))),
        "first" => Ok(match value {
            Value::Array(items) => items.into_iter().next().unwrap_or(Value::Null),
            Value::String(s) => s.chars().next().map(|c| Value::String(c.to_string())).unwrap_or(Value::Null),
            _ => Value::Null,
        }),
        "last" => Ok(match value {
            Value::Array(items) => items.into_iter().next_back().unwrap_or(Value::Null),
            Value::String(s) => s.chars().next_back().map(|c| Value::String(c.to_string())).unwrap_or(Value::Null),
            _ => Value::Null,
        }),
        "join" => {
            let sep = args.first().map(display).unwrap_or_default();
            match value {
                Value::Array(items) => Ok(Value::String(
                    items.iter().map(display).collect::<Vec<_>>().join(&sep),
                )),
                other => Ok(Value::String(display(&other))),
            }
        }
        "default" => Ok(match value {
            Value::Null => args.first().cloned().unwrap_or(Value::Null),
            other => other,
        }),
        "list" => Ok(value),
        _ => Err(JinjaError::Expression("unknown filter")),
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c.is_whitespace() {
            capitalize_next = true;
            out.push(c);
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

fn value_length(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::String(s) => s.chars().count(),
        Value::Object(map) => map.len(),
        _ => 0,
    }
}

fn num(value: &Value) -> f64 {
    value.as_f64().or_else(|| value.as_i64().map(|n| n as f64)).unwrap_or(0.0)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(_) => num(value) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// How a value renders inside `{{ }}` output or string concatenation —
/// Jinja/Python-style (no quotes around strings, `True`/`False`/`None`
/// capitalized to match what real chat templates that echo booleans back
/// would expect, though chat templates rarely do this).
fn display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(_) => num(value).to_string_trimmed(),
        Value::Array(items) => format!("[{}]", items.iter().map(display).collect::<Vec<_>>().join(", ")),
        Value::Object(_) => "{...}".to_string(),
    }
}

fn f64_to_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::Number(Number::from(n as i64))
    } else {
        Value::Number(Number::from_f64(n).unwrap_or_else(|| Number::from(0i64)))
    }
}

trait TrimmedNumberDisplay {
    fn to_string_trimmed(self) -> String;
}

impl TrimmedNumberDisplay for f64 {
    fn to_string_trimmed(self) -> String {
        if self.fract() == 0.0 && self.abs() < 1e15 {
            format!("{}", self as i64)
        } else {
            format!("{self}")
        }
    }
}
