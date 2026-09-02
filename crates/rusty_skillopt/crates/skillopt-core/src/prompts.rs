use crate::types::{AggregatedFeedback, Reflection, Skill};

/// System prompt injecting the current skill ahead of an executor rollout.
pub fn executor_system_prompt(skill: &Skill) -> String {
    format!(
        "You are an agent completing a task. Follow the guidance below exactly.\n\n\
         --- SKILL ---\n{}\n--- END SKILL ---\n\n\
         Respond with only your final answer, no preamble.",
        skill.text
    )
}

/// Prompt asking the reflector model for a short qualitative critique of one
/// trajectory. The score itself comes from the environment's programmatic
/// scorer, not from this call.
pub fn reflect_prompt(input: &str, expected: &str, output: &str, score: f64) -> String {
    format!(
        "A task was attempted by an agent following a skill document.\n\n\
         Task input: {input}\n\
         Expected answer: {expected}\n\
         Agent output: {output}\n\
         Programmatic score (0=wrong, 1=correct): {score}\n\n\
         In 1-2 sentences, explain the likely cause (if score < 1) or the key \
         reason it succeeded (if score == 1), from the perspective of what the \
         skill document should say to make this more reliable. Be specific and \
         actionable. Do not repeat the task input/output back verbatim."
    )
}

/// Prompt asking the optimizer model to propose a bounded skill edit given
/// aggregated, selected feedback from the current batch.
pub fn optimize_prompt(skill: &Skill, feedback: &AggregatedFeedback, max_ops: usize) -> String {
    let mut highlighted = String::new();
    for r in &feedback.highlighted {
        highlighted.push_str(&format!(
            "- example {} (score {:.2}): {}\n",
            r.example_id, r.score, r.critique
        ));
    }
    if highlighted.is_empty() {
        highlighted.push_str("(none)\n");
    }

    let mut rejected = String::new();
    for (i, r) in feedback.rejected_edit_summaries.iter().enumerate() {
        rejected.push_str(&format!("{}. {}\n", i + 1, r));
    }
    if rejected.is_empty() {
        rejected.push_str("(none)\n");
    }

    format!(
        "You are optimizing a skill document (a markdown file of instructions \
         given to a frozen agent). Current mean batch score: {mean_score:.2}.\n\n\
         --- CURRENT SKILL ---\n{skill}\n--- END SKILL ---\n\n\
         Feedback from the worst-scoring examples this batch:\n{highlighted}\n\
         Previously rejected edits (validation did not improve, do not repeat \
         these verbatim):\n{rejected}\n\
         Propose AT MOST {max_ops} bounded edit operations to the skill document \
         that address the feedback above. Each op is one of:\n\
         - add: insert a new line after an exact existing line (\"anchor\"), or at \
           the end if anchor is null\n\
         - delete: remove the exact existing line given by \"anchor\"\n\
         - replace: replace the exact existing line given by \"anchor\" with \"content\"\n\n\
         \"anchor\" must be an exact, verbatim line from the current skill above \
         (or null for add-at-end). \"content\" may contain multiple lines.\n\n\
         Reply with ONLY a JSON object, no markdown fences, no commentary, matching \
         exactly this shape:\n\
         {{\"ops\": [{{\"op\": \"add\", \"anchor\": \"...\" | null, \"content\": \"...\"}}, \
         {{\"op\": \"delete\", \"anchor\": \"...\"}}, \
         {{\"op\": \"replace\", \"anchor\": \"...\", \"content\": \"...\"}}], \
         \"rationale\": \"one sentence explaining the change\"}}",
        mean_score = feedback.mean_score,
        skill = skill.text,
        max_ops = max_ops,
    )
}

/// Best-effort extraction of a JSON object from a model response that may
/// be wrapped in prose or markdown code fences.
pub fn extract_json_object(text: &str) -> &str {
    let start = text.find('{');
    let end = text.rfind('}');
    match (start, end) {
        (Some(s), Some(e)) if e >= s => &text[s..=e],
        _ => text,
    }
}

pub fn select_feedback(
    reflections: &[Reflection],
    highlight_count: usize,
    rejected_edit_summaries: Vec<String>,
) -> AggregatedFeedback {
    let mean_score = if reflections.is_empty() {
        0.0
    } else {
        reflections.iter().map(|r| r.score).sum::<f64>() / reflections.len() as f64
    };

    let mut sorted: Vec<Reflection> = reflections.to_vec();
    sorted.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
    sorted.truncate(highlight_count);

    AggregatedFeedback {
        mean_score,
        highlighted: sorted,
        rejected_edit_summaries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_response() {
        let raw = "Sure, here you go:\n```json\n{\"ops\": [], \"rationale\": \"x\"}\n```\nhope that helps";
        assert_eq!(
            extract_json_object(raw),
            "{\"ops\": [], \"rationale\": \"x\"}"
        );
    }

    #[test]
    fn select_feedback_picks_lowest_scores() {
        let reflections = vec![
            Reflection {
                example_id: "a".into(),
                score: 1.0,
                critique: "ok".into(),
            },
            Reflection {
                example_id: "b".into(),
                score: 0.0,
                critique: "bad".into(),
            },
            Reflection {
                example_id: "c".into(),
                score: 0.5,
                critique: "meh".into(),
            },
        ];
        let fb = select_feedback(&reflections, 2, vec![]);
        assert_eq!(fb.highlighted.len(), 2);
        assert_eq!(fb.highlighted[0].example_id, "b");
        assert_eq!(fb.highlighted[1].example_id, "c");
        assert!((fb.mean_score - 0.5).abs() < 1e-9);
    }
}
