//! Turning a meeting into a prompt.
//!
//! The differentiator lives here (SPEC section 1): the model receives the
//! transcript *and* the user's notes, and is told the notes are the signal for
//! what mattered. Everyone else summarises the transcript alone, which is why
//! their output reads generically.
//!
//! Both are labelled with stable ids so the model can cite them — and so
//! `content::validate` can check every citation afterwards.

use crate::db::repo::{NoteBlock, Utterance};

/// A named output format. Built-ins ship with the app; users can add their own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    /// What this template should produce. Appended to the shared instructions
    /// rather than replacing them, so a custom template cannot accidentally
    /// discard the citation rules.
    pub prompt: String,
    pub is_builtin: bool,
}

pub fn builtin_templates() -> Vec<Template> {
    vec![
        Template {
            id: "default".into(),
            name: "Summary".into(),
            prompt: "Produce sections: Decisions, Action items, Open questions. \
                     Omit a section entirely if the meeting contained nothing for it — \
                     do not invent filler."
                .into(),
            is_builtin: true,
        },
        Template {
            id: "one-on-one".into(),
            name: "1:1".into(),
            prompt: "Produce sections: Updates, Blockers, Feedback, Follow-ups. \
                     Attribute follow-ups to whoever committed to them."
                .into(),
            is_builtin: true,
        },
        Template {
            id: "standup".into(),
            name: "Standup".into(),
            prompt: "Produce sections: Done, In progress, Blocked. Keep bullets to one line."
                .into(),
            is_builtin: true,
        },
        Template {
            id: "sales-call".into(),
            name: "Sales call".into(),
            prompt: "Produce sections: Pain points, Requirements, Objections, Next steps. \
                     Quote the prospect's own words for pain points where possible."
                .into(),
            is_builtin: true,
        },
        Template {
            id: "interview".into(),
            name: "Interview".into(),
            prompt: "Produce sections: Background, Signals, Concerns, Recommendation. \
                     Keep the recommendation to what was actually evidenced."
                .into(),
            is_builtin: true,
        },
    ]
}

/// Rules every template inherits.
///
/// The citation instruction is deliberately blunt. Models will cite
/// enthusiastically and inaccurately if merely invited to; telling them an
/// uncited bullet is acceptable makes fabrication less attractive than honesty.
/// It is still not trusted — `content::validate` checks every id regardless.
const SYSTEM: &str = "\
You turn meeting transcripts into structured notes.

The transcript is labelled by speaker: `You` is the person whose notes these are,
`Them` is everyone else. Every line has an id like [#12].

The user's own notes are included, each with an id like [b3]. Those notes are the
strongest signal for what mattered in this meeting — they wrote them down while it
was happening. Give what they flagged more weight than the transcript alone would
suggest, and expand on it rather than repeating it verbatim.

Rules:
- Cite the transcript line ids that support each bullet in `sourceUtterances`.
- Only cite ids that literally appear in the transcript below. Never guess an id.
- If a claim is a fair summary but you cannot point at specific lines, return an
  empty `sourceUtterances`. An honest uncited bullet is better than a wrong citation.
- When a bullet came from something the user noted, set `fromNote` to that note's id.
- Write about what was actually said. Do not add advice, next steps, or context
  that nobody mentioned.
- Respond with JSON only.";

/// Formats the transcript with the ids the model must cite.
pub fn format_transcript(utterances: &[Utterance]) -> String {
    if utterances.is_empty() {
        return "(no transcript was captured)".to_string();
    }

    utterances
        .iter()
        .map(|u| {
            let speaker = if u.source == "mic" { "You" } else { "Them" };
            format!(
                "[#{}] {} {}: {}",
                u.id,
                timecode(u.start_ms),
                speaker,
                u.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Formats the notes with their block ids.
pub fn format_notes(notes: &[NoteBlock]) -> String {
    let written: Vec<&NoteBlock> = notes.iter().filter(|n| !n.text.trim().is_empty()).collect();
    if written.is_empty() {
        return "(the user took no notes)".to_string();
    }

    written
        .iter()
        .map(|n| {
            // The anchor tells the model *when* the note was written, which is
            // what makes it a pointer into the transcript rather than a
            // free-floating remark.
            match n.first_typed_at_ms {
                Some(ms) => format!("[{}] {} {}", n.block_id, timecode(ms), n.text.trim()),
                None => format!("[{}] {}", n.block_id, n.text.trim()),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn timecode(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// The full user-side prompt.
pub fn build_user_prompt(
    template: &Template,
    utterances: &[Utterance],
    notes: &[NoteBlock],
) -> String {
    format!(
        "{}\n\n## The user's notes\n\n{}\n\n## Transcript\n\n{}",
        template.prompt,
        format_notes(notes),
        format_transcript(utterances)
    )
}

pub fn system_prompt() -> &'static str {
    SYSTEM
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utterance(id: i64, source: &str, text: &str, start_ms: i64) -> Utterance {
        Utterance {
            id,
            seq: id,
            source: source.into(),
            text: text.into(),
            start_ms,
            end_ms: start_ms + 1000,
            confidence: None,
        }
    }

    fn note(block_id: &str, text: &str, first: Option<i64>) -> NoteBlock {
        NoteBlock {
            block_id: block_id.into(),
            seq: 0,
            text: text.into(),
            first_typed_at_ms: first,
            last_edited_at_ms: first,
        }
    }

    #[test]
    fn every_builtin_template_is_usable() {
        for template in builtin_templates() {
            assert!(!template.id.is_empty());
            assert!(!template.name.is_empty());
            assert!(!template.prompt.is_empty(), "{} has no prompt", template.id);
            assert!(template.is_builtin);
        }
    }

    #[test]
    fn builtin_template_ids_are_unique() {
        let templates = builtin_templates();
        let ids: std::collections::HashSet<_> = templates.iter().map(|t| &t.id).collect();
        assert_eq!(ids.len(), templates.len());
    }

    #[test]
    fn the_transcript_carries_the_ids_the_model_must_cite() {
        let text = format_transcript(&[utterance(12, "system", "the deadline is Thursday", 5_000)]);
        assert!(text.contains("[#12]"), "no citable id: {text}");
        assert!(text.contains("the deadline is Thursday"));
    }

    #[test]
    fn the_transcript_names_the_speakers_rather_than_the_streams() {
        // "mic"/"system" is an implementation detail; the model should reason
        // about who was talking.
        let text = format_transcript(&[
            utterance(1, "mic", "I'll own it", 0),
            utterance(2, "system", "great", 1_000),
        ]);
        assert!(text.contains("You:"));
        assert!(text.contains("Them:"));
        assert!(!text.contains("mic"));
    }

    #[test]
    fn transcript_lines_carry_timecodes() {
        let text = format_transcript(&[utterance(1, "mic", "hello", 65_000)]);
        assert!(text.contains("01:05"), "no timecode: {text}");
    }

    #[test]
    fn an_empty_transcript_says_so_rather_than_being_blank() {
        // A blank section invites the model to fill the gap with invention.
        let text = format_transcript(&[]);
        assert!(!text.trim().is_empty());
        assert!(text.contains("no transcript"));
    }

    #[test]
    fn notes_carry_their_block_ids_and_anchors() {
        let text = format_notes(&[note("b3", "deadline = 14th", Some(65_000))]);
        assert!(text.contains("[b3]"));
        assert!(text.contains("01:05"), "no anchor: {text}");
        assert!(text.contains("deadline = 14th"));
    }

    #[test]
    fn empty_note_blocks_are_left_out() {
        // Blank lines are editor artifacts, not signal.
        let text = format_notes(&[
            note("b1", "real note", Some(0)),
            note("b2", "   ", Some(1_000)),
        ]);
        assert!(text.contains("real note"));
        assert!(!text.contains("[b2]"));
    }

    #[test]
    fn a_meeting_with_no_notes_says_so() {
        let text = format_notes(&[]);
        assert!(text.contains("no notes"));
    }

    #[test]
    fn a_note_without_an_anchor_still_appears() {
        let text = format_notes(&[note("b1", "typed before recording", None)]);
        assert!(text.contains("[b1]"));
        assert!(text.contains("typed before recording"));
    }

    #[test]
    fn the_prompt_contains_both_notes_and_transcript() {
        // The whole differentiator: notes are an input, not decoration.
        let prompt = build_user_prompt(
            &builtin_templates()[0],
            &[utterance(1, "mic", "the deadline is Thursday", 0)],
            &[note("b1", "deadline!", Some(500))],
        );
        assert!(prompt.contains("deadline!"), "notes missing from prompt");
        assert!(prompt.contains("[#1]"), "transcript missing from prompt");
        assert!(prompt.contains("Decisions"), "template missing from prompt");
    }

    #[test]
    fn the_system_prompt_forbids_guessing_ids_and_permits_uncited_bullets() {
        // Inviting citation without permitting honesty is what produces
        // confident, fabricated ids.
        let system = system_prompt();
        assert!(system.contains("Never guess an id"));
        assert!(system.to_lowercase().contains("empty `sourceutterances`"));
    }

    #[test]
    fn the_system_prompt_tells_the_model_notes_carry_weight() {
        assert!(system_prompt().contains("strongest signal"));
    }
}
