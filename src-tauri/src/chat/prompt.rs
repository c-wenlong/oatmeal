//! What the model is told when answering a question over meetings.
//!
//! The citation instruction is blunt for the same reason it is blunt in the
//! panel prompt: models cite enthusiastically and inaccurately, and saying
//! plainly that an uncited sentence is acceptable makes honesty cheaper than
//! fabrication. The validator catches what gets through, but a prompt that
//! invites guessing means the validator throws away most of the answer.

use super::ContextLine;

pub fn system_prompt() -> String {
    r#"You answer questions about meetings the user recorded, using only the
transcript lines provided.

Lines are labelled `[#id]`. That id is how you cite.

Rules:
- Answer only from the lines below. If they do not contain the answer, say so.
- Break your answer into short claims. One fact per claim.
- Cite every claim with the ids of the lines that support it.
- Only cite ids that literally appear below. Never guess an id, never invent
  one, and never cite a line you did not use.
- If you cannot support a claim with a line, still make it, with an empty
  `citations` array. An honest uncited sentence is better than a wrong citation.
- Do not repeat the question. Do not add a preamble.

Reply with JSON only."#
        .to_string()
}

/// JSON schema, for providers that enforce one.
pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "claims": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "citations": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "utteranceId": { "type": "integer" }
                                },
                                "required": ["utteranceId"]
                            }
                        }
                    },
                    "required": ["text", "citations"]
                }
            }
        },
        "required": ["claims"]
    })
}

/// Formats the evidence, grouped by meeting.
///
/// Grouped and dated because a question over a folder is usually comparative —
/// "what did we commit to across these calls" — and a flat list of lines gives
/// the model no way to tell which conversation each came from.
pub fn format_context(context: &[ContextLine]) -> String {
    let mut out = String::new();
    let mut current: Option<&str> = None;

    for line in context {
        if current != Some(line.meeting_id.as_str()) {
            if current.is_some() {
                out.push('\n');
            }
            let title = line.meeting_title.as_deref().unwrap_or("Untitled meeting");
            out.push_str(&format!("## {title}\n"));
            current = Some(&line.meeting_id);
        }
        out.push_str(&format!("[#{}] {}\n", line.utterance_id, line.text));
    }
    out
}

pub fn build_user_prompt(question: &str, context: &[ContextLine]) -> String {
    format!(
        "Question: {}\n\nTranscript lines:\n{}",
        question.trim(),
        format_context(context)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i64, meeting: &str, title: &str, text: &str) -> ContextLine {
        ContextLine {
            utterance_id: id,
            meeting_id: meeting.into(),
            meeting_title: Some(title.into()),
            start_ms: 0,
            text: text.into(),
        }
    }

    #[test]
    fn the_system_prompt_forbids_guessing_ids() {
        // The single most load-bearing sentence in the prompt.
        let prompt = system_prompt();
        assert!(prompt.contains("Never guess an id"));
        assert!(prompt.contains("uncited"));
    }

    #[test]
    fn lines_carry_the_id_the_model_must_cite() {
        let text = format_context(&[line(42, "m1", "Standup", "we ship thursday")]);
        assert!(text.contains("[#42] we ship thursday"));
    }

    #[test]
    fn lines_are_grouped_under_their_meeting() {
        // A folder question is comparative; a flat list loses which call each
        // line came from.
        let text = format_context(&[
            line(1, "m1", "Standup", "a"),
            line(2, "m1", "Standup", "b"),
            line(3, "m2", "Retro", "c"),
        ]);
        assert_eq!(text.matches("## Standup").count(), 1);
        assert_eq!(text.matches("## Retro").count(), 1);
        assert!(text.find("## Standup") < text.find("## Retro"));
    }

    #[test]
    fn an_untitled_meeting_still_gets_a_heading() {
        let mut without = line(1, "m1", "x", "a");
        without.meeting_title = None;
        let text = format_context(&[without]);
        assert!(text.contains("## Untitled meeting"));
    }

    #[test]
    fn the_prompt_carries_both_question_and_evidence() {
        let prompt = build_user_prompt(
            "  what did we commit to?  ",
            &[line(1, "m1", "Standup", "we will ship")],
        );
        assert!(prompt.contains("what did we commit to?"));
        assert!(!prompt.contains("  what"), "question was not trimmed");
        assert!(prompt.contains("[#1] we will ship"));
    }

    #[test]
    fn the_schema_requires_a_citations_array() {
        // Providers that enforce a schema will otherwise omit the field and
        // every claim arrives uncited.
        let schema = schema();
        let required = &schema["properties"]["claims"]["items"]["required"];
        assert!(required.to_string().contains("citations"));
    }
}
