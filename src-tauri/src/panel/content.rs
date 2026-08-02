//! What a generated panel contains, and the gate every citation passes through.
//!
//! This is the anti-hallucination boundary. A model asked to cite its sources
//! will happily invent utterance ids that look plausible — and a citation chip
//! that scrolls to nothing, or worse to the *wrong* line, is more corrosive than
//! no citation at all, because it looks like proof.
//!
//! So: nothing the model returns is trusted. Every id is checked against what is
//! actually in the database, invalid ones are dropped, and the drop is counted
//! so the UI can say how much of the output was unverifiable.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// One claim in a panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bullet {
    pub text: String,
    /// Transcript lines this claim came from. Empty means unverifiable.
    #[serde(default)]
    pub source_utterances: Vec<i64>,
    /// The note block that flagged this, when the claim came from the user's
    /// own notes rather than only from the transcript.
    #[serde(default)]
    pub from_note: Option<String>,
}

impl Bullet {
    /// Whether this claim can be traced back to the transcript.
    ///
    /// Surfaced in the UI so an uncited bullet reads as "the model's summary"
    /// rather than as established fact.
    pub fn is_cited(&self) -> bool {
        !self.source_utterances.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub heading: String,
    #[serde(default)]
    pub bullets: Vec<Bullet>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelContent {
    #[serde(default)]
    pub sections: Vec<Section>,
}

impl PanelContent {
    /// Flattened text, for FTS and for the Notion export later.
    pub fn plaintext(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            out.push_str(&section.heading);
            out.push('\n');
            for bullet in &section.bullets {
                out.push_str("- ");
                out.push_str(&bullet.text);
                out.push('\n');
            }
        }
        out
    }

    pub fn bullet_count(&self) -> usize {
        self.sections.iter().map(|s| s.bullets.len()).sum()
    }
}

/// What the gate threw away.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    /// Utterance ids the model cited that do not exist.
    pub dropped_utterances: usize,
    /// Note block ids the model cited that do not exist.
    pub dropped_notes: usize,
    /// Bullets left with nothing to cite after the drops.
    pub uncited_bullets: usize,
    pub total_bullets: usize,
}

impl ValidationReport {
    /// True when the model invented anything at all.
    pub fn had_hallucinations(&self) -> bool {
        self.dropped_utterances > 0 || self.dropped_notes > 0
    }
}

/// Strips every citation that does not resolve against the real meeting.
///
/// Bullets survive the loss of their citations — the *text* may still be a fair
/// summary — but they are counted as uncited so the UI can mark them. Removing
/// the bullet entirely would silently delete content the user can see in the
/// transcript themselves.
pub fn validate(
    content: PanelContent,
    valid_utterances: &HashSet<i64>,
    valid_notes: &HashSet<String>,
) -> (PanelContent, ValidationReport) {
    let mut report = ValidationReport::default();

    let sections = content
        .sections
        .into_iter()
        .map(|section| {
            let bullets = section
                .bullets
                .into_iter()
                .map(|mut bullet| {
                    report.total_bullets += 1;

                    let before = bullet.source_utterances.len();
                    bullet
                        .source_utterances
                        .retain(|id| valid_utterances.contains(id));
                    // Duplicated citations render as duplicated chips.
                    bullet.source_utterances.dedup();
                    report.dropped_utterances += before - bullet.source_utterances.len();

                    if let Some(note) = &bullet.from_note {
                        if !valid_notes.contains(note) {
                            bullet.from_note = None;
                            report.dropped_notes += 1;
                        }
                    }

                    if !bullet.is_cited() {
                        report.uncited_bullets += 1;
                    }
                    bullet
                })
                .collect();

            Section {
                heading: section.heading,
                bullets,
            }
        })
        .collect();

    (PanelContent { sections }, report)
}

/// JSON schema handed to providers that support structured output.
///
/// Providers that ignore it are not a problem: `validate` runs regardless, so
/// the schema is an optimisation rather than a guarantee.
pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["sections"],
        "properties": {
            "sections": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["heading", "bullets"],
                    "properties": {
                        "heading": { "type": "string" },
                        "bullets": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["text", "sourceUtterances"],
                                "properties": {
                                    "text": { "type": "string" },
                                    "sourceUtterances": {
                                        "type": "array",
                                        "items": { "type": "integer" }
                                    },
                                    "fromNote": { "type": ["string", "null"] }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Parses a model response into a panel.
///
/// Models wrap JSON in prose or markdown fences even when told not to, so the
/// outermost JSON object is extracted rather than demanding a clean response.
pub fn parse(raw: &str) -> Result<PanelContent, String> {
    let trimmed = raw.trim();

    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else {
        let start = trimmed.find('{').ok_or("no JSON object in the response")?;
        let end = trimmed.rfind('}').ok_or("no JSON object in the response")?;
        if end <= start {
            return Err("no JSON object in the response".into());
        }
        trimmed[start..=end].to_string()
    };

    serde_json::from_str(&candidate).map_err(|e| format!("could not parse the panel: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utterances(ids: &[i64]) -> HashSet<i64> {
        ids.iter().copied().collect()
    }

    fn notes(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn bullet(text: &str, sources: &[i64], note: Option<&str>) -> Bullet {
        Bullet {
            text: text.into(),
            source_utterances: sources.to_vec(),
            from_note: note.map(str::to_string),
        }
    }

    fn panel(bullets: Vec<Bullet>) -> PanelContent {
        PanelContent {
            sections: vec![Section {
                heading: "Summary".into(),
                bullets,
            }],
        }
    }

    #[test]
    fn a_real_citation_survives() {
        let (out, report) = validate(
            panel(vec![bullet("deadline is the 14th", &[12], None)]),
            &utterances(&[12]),
            &notes(&[]),
        );
        assert_eq!(out.sections[0].bullets[0].source_utterances, vec![12]);
        assert!(!report.had_hallucinations());
    }

    #[test]
    fn an_invented_utterance_id_is_dropped() {
        // The whole point: a chip that scrolls nowhere looks like proof.
        let (out, report) = validate(
            panel(vec![bullet("something nobody said", &[999], None)]),
            &utterances(&[12]),
            &notes(&[]),
        );
        assert!(out.sections[0].bullets[0].source_utterances.is_empty());
        assert_eq!(report.dropped_utterances, 1);
        assert!(report.had_hallucinations());
    }

    #[test]
    fn a_bullet_keeps_its_real_citations_when_only_some_are_invented() {
        let (out, report) = validate(
            panel(vec![bullet("mixed", &[12, 999, 13], None)]),
            &utterances(&[12, 13]),
            &notes(&[]),
        );
        assert_eq!(out.sections[0].bullets[0].source_utterances, vec![12, 13]);
        assert_eq!(report.dropped_utterances, 1);
    }

    #[test]
    fn a_bullet_survives_losing_every_citation() {
        // Deleting the bullet would silently remove text the user can verify
        // against the transcript themselves; marking it uncited is honest.
        let (out, report) = validate(
            panel(vec![bullet("plausible claim", &[999], None)]),
            &utterances(&[12]),
            &notes(&[]),
        );
        assert_eq!(out.bullet_count(), 1);
        assert_eq!(out.sections[0].bullets[0].text, "plausible claim");
        assert!(!out.sections[0].bullets[0].is_cited());
        assert_eq!(report.uncited_bullets, 1);
    }

    #[test]
    fn an_invented_note_reference_is_dropped() {
        let (out, report) = validate(
            panel(vec![bullet("from a note", &[12], Some("no-such-block"))]),
            &utterances(&[12]),
            &notes(&["b1"]),
        );
        assert_eq!(out.sections[0].bullets[0].from_note, None);
        assert_eq!(report.dropped_notes, 1);
    }

    #[test]
    fn a_real_note_reference_survives() {
        let (out, report) = validate(
            panel(vec![bullet("from a note", &[12], Some("b1"))]),
            &utterances(&[12]),
            &notes(&["b1"]),
        );
        assert_eq!(out.sections[0].bullets[0].from_note.as_deref(), Some("b1"));
        assert_eq!(report.dropped_notes, 0);
    }

    #[test]
    fn duplicate_citations_are_collapsed() {
        // Otherwise the same chip renders twice.
        let (out, _) = validate(
            panel(vec![bullet("repeated", &[12, 12], None)]),
            &utterances(&[12]),
            &notes(&[]),
        );
        assert_eq!(out.sections[0].bullets[0].source_utterances, vec![12]);
    }

    #[test]
    fn a_meeting_with_no_utterances_invalidates_everything() {
        // Generating against an empty transcript must not produce citations.
        let (out, report) = validate(
            panel(vec![bullet("invented", &[1, 2, 3], None)]),
            &utterances(&[]),
            &notes(&[]),
        );
        assert!(out.sections[0].bullets[0].source_utterances.is_empty());
        assert_eq!(report.dropped_utterances, 3);
    }

    #[test]
    fn the_report_counts_every_bullet() {
        let (_, report) = validate(
            panel(vec![
                bullet("a", &[12], None),
                bullet("b", &[], None),
                bullet("c", &[999], None),
            ]),
            &utterances(&[12]),
            &notes(&[]),
        );
        assert_eq!(report.total_bullets, 3);
        assert_eq!(report.uncited_bullets, 2);
    }

    #[test]
    fn validation_leaves_the_text_untouched() {
        // The gate polices citations, not content.
        let (out, _) = validate(
            panel(vec![bullet("exact wording preserved", &[999], None)]),
            &utterances(&[]),
            &notes(&[]),
        );
        assert_eq!(out.sections[0].bullets[0].text, "exact wording preserved");
    }

    #[test]
    fn plaintext_flattens_headings_and_bullets() {
        let content = PanelContent {
            sections: vec![Section {
                heading: "Decisions".into(),
                bullets: vec![bullet("ship on Thursday", &[], None)],
            }],
        };
        let text = content.plaintext();
        assert!(text.contains("Decisions"));
        assert!(text.contains("ship on Thursday"));
    }

    // ---------------------------------------------------------------- parsing

    #[test]
    fn parses_a_clean_response() {
        let content = parse(r#"{"sections":[{"heading":"Summary","bullets":[]}]}"#).unwrap();
        assert_eq!(content.sections[0].heading, "Summary");
    }

    #[test]
    fn parses_json_wrapped_in_a_markdown_fence() {
        // Local models do this constantly regardless of instructions.
        let raw = "```json\n{\"sections\":[{\"heading\":\"S\",\"bullets\":[]}]}\n```";
        assert_eq!(parse(raw).unwrap().sections[0].heading, "S");
    }

    #[test]
    fn parses_json_surrounded_by_prose() {
        let raw = "Sure! Here is the summary:\n{\"sections\":[]}\nHope that helps.";
        assert!(parse(raw).unwrap().sections.is_empty());
    }

    #[test]
    fn a_response_with_no_json_is_an_error() {
        assert!(parse("I'm sorry, I can't do that.").is_err());
    }

    #[test]
    fn malformed_json_is_an_error_not_an_empty_panel() {
        // An empty panel would look like a meeting where nothing was said.
        assert!(parse(r#"{"sections": [ }"#).is_err());
    }

    #[test]
    fn missing_optional_fields_default_rather_than_failing() {
        // Models omit empty arrays and nulls all the time.
        let content = parse(r#"{"sections":[{"heading":"S","bullets":[{"text":"t"}]}]}"#).unwrap();
        let bullet = &content.sections[0].bullets[0];
        assert!(bullet.source_utterances.is_empty());
        assert_eq!(bullet.from_note, None);
    }

    #[test]
    fn the_schema_requires_text_and_sources_on_every_bullet() {
        let schema = schema();
        let required = &schema["properties"]["sections"]["items"]["properties"]["bullets"]["items"]
            ["required"];
        assert!(required.as_array().unwrap().iter().any(|v| v == "text"));
        assert!(required
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "sourceUtterances"));
    }
}
