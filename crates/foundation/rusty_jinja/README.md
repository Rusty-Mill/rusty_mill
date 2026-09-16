# rusty_jinja

A `#![no_std]` + `alloc` sovereign, real (subset, honestly-documented)
Jinja2 template engine — for rendering the actual Jinja chat-template
source a GGUF model embeds, the same job `rusty_llama` currently asks the
real `minijinja` crate to do.

**Cross-validated against a real Jinja2 engine**, not just its own tests:
one test uses the exact template, message content, and expected output from
`rusty_llama`'s own `render_jinja_llama3_shaped_template` test (which runs
the same input through real `minijinja`) — this engine produces
byte-identical output.

## Implemented

- `{{ output }}` expressions
- `{% if/elif/else/endif %}`
- `{% for x in y %}...{% endfor %}` with `loop.index`/`loop.index0`/
  `loop.first`/`loop.last`/`loop.length`
- `{% set x = expr %}`
- `{%-`/`-%}`/`{{-`/`-}}` whitespace trimming
- Attribute/index access: `a.b`, `a['b']`, `a[0]`
- Comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`), `and`/`or`/`not`,
  `in`/`not in`
- `is`/`is not` tests: `defined`, `none`, `string`, `number`, `mapping`,
  `iterable`
- String concatenation: `~` and Python-style `+` (numeric addition when
  both sides are numbers)
- Filters/methods (`expr | name(args)` or `expr.name(args)`, identical
  either way): `trim`/`strip`, `upper`, `lower`, `title`, `string`,
  `length`/`count`, `first`, `last`, `join`, `default`, `list`

## Known, deliberate gaps

No user-defined macros/`{% macro %}`, no `{% include %}`/`{% extends %}`,
no arithmetic beyond `+`/`-`, no `range()`/other builtin functions, no
dict/list literals in expression position (only as context values), and
`for` only iterates arrays (not object keys) — all narrower than full
Jinja2, but covering the control flow real LLM chat templates (ChatML,
Llama, Mistral, Zephyr, Qwen style) actually use.

## Example

```rust
use rusty_jinja::Template;
use rusty_json::{Map, Value};

let template = Template::compile(
    "{% for m in messages %}{{ m['role'] }}: {{ m['content'] }}\n{% endfor %}"
).unwrap();

let mut msg = Map::new();
msg.insert("role".into(), Value::String("user".into()));
msg.insert("content".into(), Value::String("hi".into()));
let mut context = Map::new();
context.insert("messages".into(), Value::Array(vec![Value::Object(msg)]));

assert_eq!(template.render(&Value::Object(context)).unwrap(), "user: hi\n");
```

The `ChatMessage`/`TemplateEnvironment` convenience wrapper still exists for
the simple "just render a message list as ChatML" case, now backed by this
real engine instead of hand-formatted strings.

## Testing

```
cargo test
```
