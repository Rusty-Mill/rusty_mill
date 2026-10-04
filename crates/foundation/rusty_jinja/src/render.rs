//! Walks a compiled [`Node`] tree, evaluating expressions against a
//! `rusty_json::Value` context and producing the rendered string.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rusty_json::{Map, Number, Value};

use crate::RenderLimits;
use crate::ast::{BinOp, Expr, Node};
use crate::template::JinjaError;

type Scope = BTreeMap<String, Value>;

/// Renders `nodes` against `context` (typically a JSON object holding
/// `messages`, `add_generation_prompt`, `bos_token`, etc.).
pub fn render(nodes: &[Node], context: &Value, limits: RenderLimits) -> Result<String, JinjaError> {
    let mut scopes: Vec<Scope> = alloc::vec![Scope::new()];
    let mut out = String::new();
    let mut budget = RenderBudget::new(limits);
    render_nodes(nodes, context, &mut scopes, &mut out, &mut budget)?;
    Ok(out)
}

struct RenderBudget {
    operations_left: u64,
    max_output_bytes: usize,
}

impl RenderBudget {
    fn new(limits: RenderLimits) -> Self {
        Self {
            operations_left: limits.max_operations,
            max_output_bytes: limits.max_output_bytes,
        }
    }

    fn spend(&mut self) -> Result<(), JinjaError> {
        self.operations_left = self
            .operations_left
            .checked_sub(1)
            .ok_or(JinjaError::Limit("render operation limit exceeded"))?;
        Ok(())
    }

    fn push(&self, out: &mut String, value: &str) -> Result<(), JinjaError> {
        let new_len = out
            .len()
            .checked_add(value.len())
            .ok_or(JinjaError::Limit("rendered output exceeds byte limit"))?;
        if new_len > self.max_output_bytes {
            return Err(JinjaError::Limit("rendered output exceeds byte limit"));
        }
        out.push_str(value);
        Ok(())
    }

    fn value_len(&self, left: usize, right: usize) -> Result<usize, JinjaError> {
        let len = left
            .checked_add(right)
            .ok_or(JinjaError::Limit("render value exceeds byte limit"))?;
        if len > self.max_output_bytes {
            return Err(JinjaError::Limit("render value exceeds byte limit"));
        }
        Ok(len)
    }

    fn value_push(&self, out: &mut String, value: &str) -> Result<(), JinjaError> {
        self.value_len(out.len(), value.len())?;
        out.push_str(value);
        Ok(())
    }

    fn clone_value(&self, value: &Value) -> Result<Value, JinjaError> {
        self.check_value(value)?;
        Ok(value.clone())
    }

    fn check_value(&self, value: &Value) -> Result<(), JinjaError> {
        self.value_size(value, 0)?;
        Ok(())
    }

    fn value_size(&self, value: &Value, mut size: usize) -> Result<usize, JinjaError> {
        match value {
            Value::String(s) => self.value_len(size, s.len()),
            Value::Array(items) => {
                for item in items {
                    size = self.value_size(item, size)?;
                }
                Ok(size)
            }
            Value::Object(map) => {
                for (key, value) in map {
                    size = self.value_len(size, key.len())?;
                    size = self.value_size(value, size)?;
                }
                Ok(size)
            }
            _ => Ok(size),
        }
    }
}

fn render_nodes(
    nodes: &[Node],
    context: &Value,
    scopes: &mut Vec<Scope>,
    out: &mut String,
    budget: &mut RenderBudget,
) -> Result<(), JinjaError> {
    for node in nodes {
        budget.spend()?;
        match node {
            Node::Text(s) => budget.push(out, s)?,
            Node::Output(expr) => {
                let value = display(&eval(expr, context, scopes, budget)?, budget)?;
                budget.push(out, &value)?;
            }
            Node::Set { var, value } => {
                let v = eval(value, context, scopes, budget)?;
                scopes
                    .last_mut()
                    .expect("render always keeps at least one scope")
                    .insert(var.clone(), v);
            }
            Node::If {
                branches,
                else_branch,
            } => {
                let mut matched = false;
                for (cond, body) in branches {
                    if truthy(&eval(cond, context, scopes, budget)?) {
                        render_nodes(body, context, scopes, out, budget)?;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    if let Some(else_body) = else_branch {
                        render_nodes(else_body, context, scopes, out, budget)?;
                    }
                }
            }
            Node::For {
                var,
                iterable,
                body,
            } => {
                let items = eval_iterable(iterable, context, scopes, budget)?;
                let len = items.len();
                for (i, item) in items.into_iter().enumerate() {
                    budget.spend()?;
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
                    let result = render_nodes(body, context, scopes, out, budget);
                    scopes.pop();
                    result?;
                }
            }
        }
    }
    Ok(())
}

fn eval_iterable(
    expr: &Expr,
    context: &Value,
    scopes: &[Scope],
    budget: &mut RenderBudget,
) -> Result<Vec<Value>, JinjaError> {
    match eval(expr, context, scopes, budget)? {
        Value::Array(items) => Ok(items),
        Value::Null => Ok(Vec::new()),
        _ => Err(JinjaError::Expression("'for' target is not iterable")),
    }
}

fn lookup_var<'a>(name: &str, context: &'a Value, scopes: &'a [Scope]) -> Option<&'a Value> {
    for scope in scopes.iter().rev() {
        if let Some(v) = scope.get(name) {
            return Some(v);
        }
    }
    context.get(name)
}

fn is_defined(
    expr: &Expr,
    context: &Value,
    scopes: &[Scope],
    budget: &mut RenderBudget,
) -> Result<bool, JinjaError> {
    Ok(match expr {
        Expr::Var(name) => lookup_var(name, context, scopes).is_some(),
        Expr::Attr(base, name) => eval(base, context, scopes, budget)?.get(name).is_some(),
        _ => true,
    })
}

fn eval(
    expr: &Expr,
    context: &Value,
    scopes: &[Scope],
    budget: &mut RenderBudget,
) -> Result<Value, JinjaError> {
    budget.spend()?;
    match expr {
        Expr::Str(s) => {
            budget.value_len(0, s.len())?;
            Ok(Value::String(s.clone()))
        }
        Expr::Num(n) => Ok(f64_to_value(*n)),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::None => Ok(Value::Null),
        Expr::Var(name) => lookup_var(name, context, scopes)
            .map(|value| budget.clone_value(value))
            .unwrap_or(Ok(Value::Null)),
        Expr::Attr(base, name) => {
            let base_val = eval(base, context, scopes, budget)?;
            base_val
                .get(name)
                .map(|value| budget.clone_value(value))
                .unwrap_or(Ok(Value::Null))
        }
        Expr::Index(base, idx) => {
            let base_val = eval(base, context, scopes, budget)?;
            let idx_val = eval(idx, context, scopes, budget)?;
            let i = num(&idx_val) as i64;
            if i < 0 {
                return Ok(Value::Null);
            }
            base_val
                .get_index(i as usize)
                .map(|value| budget.clone_value(value))
                .unwrap_or(Ok(Value::Null))
        }
        Expr::Not(e) => Ok(Value::Bool(!truthy(&eval(e, context, scopes, budget)?))),
        Expr::Concat(a, b) => {
            let mut s = display(&eval(a, context, scopes, budget)?, budget)?;
            let right = display(&eval(b, context, scopes, budget)?, budget)?;
            budget.value_push(&mut s, &right)?;
            Ok(Value::String(s))
        }
        Expr::BinOp(op, a, b) => eval_binop(op, a, b, context, scopes, budget),
        Expr::Filter(base, name, args) => {
            let base_val = eval(base, context, scopes, budget)?;
            let arg_vals: Vec<Value> = args
                .iter()
                .map(|a| eval(a, context, scopes, budget))
                .collect::<Result<_, _>>()?;
            apply_filter(name, base_val, &arg_vals, budget)
        }
        Expr::Test(base, name, negate) => {
            let result = match name.as_str() {
                "defined" => is_defined(base, context, scopes, budget)?,
                "none" => matches!(eval(base, context, scopes, budget)?, Value::Null),
                "string" => eval(base, context, scopes, budget)?.is_string(),
                "number" => eval(base, context, scopes, budget)?.is_number(),
                "mapping" => eval(base, context, scopes, budget)?.is_object(),
                "iterable" => matches!(
                    eval(base, context, scopes, budget)?,
                    Value::Array(_) | Value::String(_) | Value::Object(_)
                ),
                _ => return Err(JinjaError::Expression("unknown 'is' test")),
            };
            Ok(Value::Bool(result != *negate))
        }
        Expr::In(needle, haystack, negate) => {
            let n = eval(needle, context, scopes, budget)?;
            let h = eval(haystack, context, scopes, budget)?;
            let result = match &h {
                Value::Array(items) => items.contains(&n),
                Value::String(s) => n
                    .as_str()
                    .map(|needle_s| s.contains(needle_s))
                    .unwrap_or(false),
                Value::Object(map) => n.as_str().map(|k| map.get(k).is_some()).unwrap_or(false),
                _ => false,
            };
            Ok(Value::Bool(result != *negate))
        }
    }
}

fn eval_binop(
    op: &BinOp,
    a: &Expr,
    b: &Expr,
    context: &Value,
    scopes: &[Scope],
    budget: &mut RenderBudget,
) -> Result<Value, JinjaError> {
    if *op == BinOp::And {
        let av = eval(a, context, scopes, budget)?;
        if !truthy(&av) {
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Bool(truthy(&eval(b, context, scopes, budget)?)));
    }
    if *op == BinOp::Or {
        let av = eval(a, context, scopes, budget)?;
        if truthy(&av) {
            return Ok(Value::Bool(true));
        }
        return Ok(Value::Bool(truthy(&eval(b, context, scopes, budget)?)));
    }

    let av = eval(a, context, scopes, budget)?;
    let bv = eval(b, context, scopes, budget)?;
    Ok(match op {
        BinOp::Add => match (&av, &bv) {
            (Value::String(sa), Value::String(sb)) => {
                let capacity = budget.value_len(sa.len(), sb.len())?;
                let mut value = String::with_capacity(capacity);
                value.push_str(sa);
                value.push_str(sb);
                Value::String(value)
            }
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
    num(a)
        .partial_cmp(&num(b))
        .unwrap_or(core::cmp::Ordering::Equal)
}

fn apply_filter(
    name: &str,
    value: Value,
    args: &[Value],
    budget: &RenderBudget,
) -> Result<Value, JinjaError> {
    match name {
        "trim" | "strip" => {
            let displayed = display(&value, budget)?;
            Ok(Value::String(displayed.trim().to_string()))
        }
        "upper" => map_case(&display(&value, budget)?, budget, true),
        "lower" => map_case(&display(&value, budget)?, budget, false),
        "title" => Ok(Value::String(title_case(
            &display(&value, budget)?,
            budget,
        )?)),
        "string" => Ok(Value::String(display(&value, budget)?)),
        "length" | "count" => Ok(Value::Number(Number::from(value_length(&value) as i64))),
        "first" => Ok(match value {
            Value::Array(items) => items.into_iter().next().unwrap_or(Value::Null),
            Value::String(s) => s
                .chars()
                .next()
                .map(|c| Value::String(c.to_string()))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        }),
        "last" => Ok(match value {
            Value::Array(items) => items.into_iter().next_back().unwrap_or(Value::Null),
            Value::String(s) => s
                .chars()
                .next_back()
                .map(|c| Value::String(c.to_string()))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        }),
        "join" => {
            let sep = args
                .first()
                .map(|value| display(value, budget))
                .transpose()?
                .unwrap_or_default();
            match value {
                Value::Array(items) => {
                    let mut joined = String::new();
                    for (index, item) in items.iter().enumerate() {
                        if index != 0 {
                            budget.value_push(&mut joined, &sep)?;
                        }
                        budget.value_push(&mut joined, &display(item, budget)?)?;
                    }
                    Ok(Value::String(joined))
                }
                other => Ok(Value::String(display(&other, budget)?)),
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

fn title_case(s: &str, budget: &RenderBudget) -> Result<String, JinjaError> {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c.is_whitespace() {
            capitalize_next = true;
            push_char(&mut out, c, budget)?;
        } else if capitalize_next {
            for mapped in c.to_uppercase() {
                push_char(&mut out, mapped, budget)?;
            }
            capitalize_next = false;
        } else {
            for mapped in c.to_lowercase() {
                push_char(&mut out, mapped, budget)?;
            }
        }
    }
    Ok(out)
}

fn map_case(s: &str, budget: &RenderBudget, upper: bool) -> Result<Value, JinjaError> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if upper {
            for mapped in c.to_uppercase() {
                push_char(&mut out, mapped, budget)?;
            }
        } else {
            for mapped in c.to_lowercase() {
                push_char(&mut out, mapped, budget)?;
            }
        }
    }
    Ok(Value::String(out))
}

fn push_char(out: &mut String, c: char, budget: &RenderBudget) -> Result<(), JinjaError> {
    budget.value_len(out.len(), c.len_utf8())?;
    out.push(c);
    Ok(())
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
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|n| n as f64))
        .unwrap_or(0.0)
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
fn display(value: &Value, budget: &RenderBudget) -> Result<String, JinjaError> {
    let mut out = String::new();
    match value {
        Value::Null => {}
        Value::Bool(b) => budget.value_push(&mut out, if *b { "True" } else { "False" })?,
        Value::String(s) => budget.value_push(&mut out, s)?,
        Value::Number(_) => budget.value_push(&mut out, &num(value).to_string_trimmed())?,
        Value::Array(items) => {
            budget.value_push(&mut out, "[")?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    budget.value_push(&mut out, ", ")?;
                }
                budget.value_push(&mut out, &display(item, budget)?)?;
            }
            budget.value_push(&mut out, "]")?;
        }
        Value::Object(_) => budget.value_push(&mut out, "{...}")?,
    }
    Ok(out)
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
