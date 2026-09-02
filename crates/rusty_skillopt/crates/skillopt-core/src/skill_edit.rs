use thiserror::Error;

use crate::types::{EditOp, Skill, SkillEdit};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EditError {
    #[error("anchor not found: {0:?}")]
    AnchorNotFound(String),
    #[error("anchor matched {1} lines, expected exactly 1: {0:?}")]
    AmbiguousAnchor(String, usize),
    #[error("edit has no ops")]
    EmptyEdit,
    #[error("edit exceeds max ops budget: {0} > {1}")]
    TooManyOps(usize, usize),
}

/// Applies a [`SkillEdit`] to a [`Skill`], returning the candidate skill.
/// Ops are applied in order against a running line buffer so later ops see
/// the effect of earlier ones in the same edit.
pub fn apply_edit(skill: &Skill, edit: &SkillEdit, max_ops: usize) -> Result<Skill, EditError> {
    if edit.ops.is_empty() {
        return Err(EditError::EmptyEdit);
    }
    if edit.ops.len() > max_ops {
        return Err(EditError::TooManyOps(edit.ops.len(), max_ops));
    }

    let mut lines: Vec<String> = skill.text.lines().map(str::to_string).collect();

    for op in &edit.ops {
        match op {
            EditOp::Add {
                anchor: None,
                content,
            } => {
                lines.extend(content.lines().map(str::to_string));
            }
            EditOp::Add {
                anchor: Some(anchor),
                content,
            } => {
                let idx = find_unique_line(&lines, anchor)?;
                let insert_at = idx + 1;
                for (offset, new_line) in content.lines().enumerate() {
                    lines.insert(insert_at + offset, new_line.to_string());
                }
            }
            EditOp::Delete { anchor } => {
                let idx = find_unique_line(&lines, anchor)?;
                lines.remove(idx);
            }
            EditOp::Replace { anchor, content } => {
                let idx = find_unique_line(&lines, anchor)?;
                lines.remove(idx);
                for (offset, new_line) in content.lines().enumerate() {
                    lines.insert(idx + offset, new_line.to_string());
                }
            }
        }
    }

    Ok(Skill::new(lines.join("\n")))
}

fn find_unique_line(lines: &[String], anchor: &str) -> Result<usize, EditError> {
    let matches: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == anchor.trim())
        .map(|(i, _)| i)
        .collect();

    match matches.len() {
        0 => Err(EditError::AnchorNotFound(anchor.to_string())),
        1 => Ok(matches[0]),
        n => Err(EditError::AmbiguousAnchor(anchor.to_string(), n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(text: &str) -> Skill {
        Skill::new(text)
    }

    #[test]
    fn add_after_anchor() {
        let s = skill("# Skill\n- rule one\n- rule two");
        let edit = SkillEdit {
            ops: vec![EditOp::Add {
                anchor: Some("- rule one".into()),
                content: "- rule 1.5".into(),
            }],
            rationale: "test".into(),
        };
        let out = apply_edit(&s, &edit, 4).unwrap();
        assert_eq!(out.text, "# Skill\n- rule one\n- rule 1.5\n- rule two");
    }

    #[test]
    fn add_without_anchor_appends() {
        let s = skill("# Skill\n- rule one");
        let edit = SkillEdit {
            ops: vec![EditOp::Add {
                anchor: None,
                content: "- rule two".into(),
            }],
            rationale: "test".into(),
        };
        let out = apply_edit(&s, &edit, 4).unwrap();
        assert_eq!(out.text, "# Skill\n- rule one\n- rule two");
    }

    #[test]
    fn delete_removes_line() {
        let s = skill("# Skill\n- rule one\n- rule two");
        let edit = SkillEdit {
            ops: vec![EditOp::Delete {
                anchor: "- rule one".into(),
            }],
            rationale: "test".into(),
        };
        let out = apply_edit(&s, &edit, 4).unwrap();
        assert_eq!(out.text, "# Skill\n- rule two");
    }

    #[test]
    fn replace_swaps_line() {
        let s = skill("# Skill\n- rule one\n- rule two");
        let edit = SkillEdit {
            ops: vec![EditOp::Replace {
                anchor: "- rule one".into(),
                content: "- rule one (revised)".into(),
            }],
            rationale: "test".into(),
        };
        let out = apply_edit(&s, &edit, 4).unwrap();
        assert_eq!(out.text, "# Skill\n- rule one (revised)\n- rule two");
    }

    #[test]
    fn missing_anchor_errors() {
        let s = skill("# Skill\n- rule one");
        let edit = SkillEdit {
            ops: vec![EditOp::Delete {
                anchor: "- does not exist".into(),
            }],
            rationale: "test".into(),
        };
        assert_eq!(
            apply_edit(&s, &edit, 4).unwrap_err(),
            EditError::AnchorNotFound("- does not exist".into())
        );
    }

    #[test]
    fn ambiguous_anchor_errors() {
        let s = skill("# Skill\n- rule\n- rule");
        let edit = SkillEdit {
            ops: vec![EditOp::Delete {
                anchor: "- rule".into(),
            }],
            rationale: "test".into(),
        };
        assert_eq!(
            apply_edit(&s, &edit, 4).unwrap_err(),
            EditError::AmbiguousAnchor("- rule".into(), 2)
        );
    }

    #[test]
    fn too_many_ops_errors() {
        let s = skill("# Skill");
        let edit = SkillEdit {
            ops: vec![
                EditOp::Add {
                    anchor: None,
                    content: "a".into(),
                },
                EditOp::Add {
                    anchor: None,
                    content: "b".into(),
                },
            ],
            rationale: "test".into(),
        };
        assert_eq!(
            apply_edit(&s, &edit, 1).unwrap_err(),
            EditError::TooManyOps(2, 1)
        );
    }

    #[test]
    fn empty_edit_errors() {
        let s = skill("# Skill");
        let edit = SkillEdit {
            ops: vec![],
            rationale: "test".into(),
        };
        assert_eq!(apply_edit(&s, &edit, 4).unwrap_err(), EditError::EmptyEdit);
    }

    #[test]
    fn sequential_ops_see_prior_effects() {
        let s = skill("# Skill\n- a\n- b");
        let edit = SkillEdit {
            ops: vec![
                EditOp::Add {
                    anchor: Some("- a".into()),
                    content: "- a.5".into(),
                },
                EditOp::Delete {
                    anchor: "- b".into(),
                },
            ],
            rationale: "test".into(),
        };
        let out = apply_edit(&s, &edit, 4).unwrap();
        assert_eq!(out.text, "# Skill\n- a\n- a.5");
    }
}
