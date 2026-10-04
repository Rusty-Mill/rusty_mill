//! Prompts and code extraction. Context is naive, as in AIDE0: every
//! prompt carries the full history, each attempt's code and output.

use crate::agent::{Step, Tree};
use crate::broker::Message;

/// Feedback for a reply that contained no program.
pub const NO_CODE: &str = "The reply contained no ```python code block, so nothing ran.";

const SYSTEM: &str = "You are an expert programmer and data scientist. You \
write one complete Python 3 program, standard library only, that solves the \
task below as well as possible.\n\n";

const FORMAT: &str = "Reply with a short explanation, then exactly one \
```python code block containing the whole program.";

/// The messages for the next step.
pub fn messages(task: &str, tree: &Tree, step: Step) -> Vec<Message> {
    let instruction = match step {
        Step::Draft => "Write a new solution from scratch, different from the \
            attempts above."
            .to_owned(),
        Step::Improve(index) => {
            let score = tree.nodes[index].score.unwrap_or_default();
            format!(
                "Improve attempt {index}, the best so far (score {score}). Make \
                 one focused change that should raise the score."
            )
        }
        Step::Debug(index) => format!(
            "Attempt {index} failed; its output is above. Find the cause and \
             fix it."
        ),
    };
    vec![
        Message {
            role: "system",
            content: format!("{SYSTEM}{task}"),
        },
        Message {
            role: "user",
            content: format!("{}\n{instruction}\n\n{FORMAT}", history(tree)),
        },
    ]
}

fn history(tree: &Tree) -> String {
    if tree.nodes.is_empty() {
        return "# Previous attempts\n\nNone yet.\n".to_owned();
    }
    let mut out = "# Previous attempts\n".to_owned();
    for (index, node) in tree.nodes.iter().enumerate() {
        let origin = match node.step {
            Step::Draft => "draft".to_owned(),
            Step::Improve(parent) => format!("improves {parent}"),
            Step::Debug(parent) => format!("debugs {parent}"),
        };
        let result = node
            .score
            .map_or_else(|| "buggy".to_owned(), |score| format!("score {score}"));
        out.push_str(&format!("\n## Attempt {index} ({origin}): {result}\n\n"));
        if let Some(code) = &node.code {
            out.push_str(&format!("```python\n{}\n```\n\n", code.trim_end()));
        }
        out.push_str(&format!(
            "Output:\n```\n{}\n```\n",
            node.feedback.trim_end()
        ));
    }
    out
}

/// The program in a reply: the first fenced block tagged `python` or `py`,
/// else the first untagged one.
pub fn extract_code(reply: &str) -> Option<String> {
    let mut untagged = None;
    let mut rest = reply;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        let Some(line_end) = after.find('\n') else {
            break;
        };
        let tag = after[..line_end].trim().to_ascii_lowercase();
        let body = &after[line_end + 1..];
        let (code, next) = match body.find("```") {
            Some(close) => (&body[..close], &body[close + 3..]),
            None => (body, ""),
        };
        if !code.trim().is_empty() {
            if tag == "python" || tag == "py" || tag == "python3" {
                return Some(code.to_owned());
            }
            if tag.is_empty() && untagged.is_none() {
                untagged = Some(code.to_owned());
            }
        }
        rest = next;
    }
    untagged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_python_block() {
        let reply =
            "Plan.\n```text\nnot this\n```\n```python\nprint(1)\n```\n```python\nprint(2)\n```";
        assert_eq!(extract_code(reply).as_deref(), Some("print(1)\n"));
        assert_eq!(
            extract_code("```\nprint(3)\n```").as_deref(),
            Some("print(3)\n")
        );
        assert_eq!(
            extract_code("```Python\nprint(4)").as_deref(),
            Some("print(4)"),
            "an unclosed block runs to the end"
        );
        assert_eq!(extract_code("no code"), None);
        assert_eq!(extract_code("```python\n\n```"), None, "empty block");
    }

    #[test]
    fn prompts_carry_the_task_and_the_full_history() {
        let mut tree = Tree::default();
        tree.push(
            Step::Draft,
            Some("print(1)".to_owned()),
            Some(0.25),
            "ok".to_owned(),
        );
        tree.push(Step::Draft, None, None, NO_CODE.to_owned());
        tree.push(
            Step::Debug(1),
            Some("x".to_owned()),
            None,
            "Trace".to_owned(),
        );
        let messages = messages("TASK", &tree, Step::Improve(0));
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.ends_with("TASK"));
        let user = &messages[1].content;
        for needle in [
            "Attempt 0 (draft): score 0.25",
            "print(1)",
            "Attempt 1 (draft): buggy",
            NO_CODE,
            "Attempt 2 (debugs 1): buggy",
            "Trace",
            "Improve attempt 0",
        ] {
            assert!(user.contains(needle), "{needle}");
        }
    }
}
