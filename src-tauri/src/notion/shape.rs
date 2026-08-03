//! Turning a meeting into the JSON Notion expects.
//!
//! Kept separate from the HTTP client so the mapping — which is where the bugs
//! live — can be tested without a token, a workspace, or a network.
//!
//! Two Notion constraints shape everything here, and both are silent failures
//! rather than errors if you get them wrong:
//!
//! - A rich-text value is capped at **2000 characters**. Over that, the API
//!   rejects the whole request, so a long transcript line has to be split.
//! - A page can carry **100 blocks per request**. A two-hour transcript is
//!   thousands of lines, so the append has to be chunked.

use serde_json::{json, Value};

use crate::db::repo::Utterance;
use crate::panel::content::PanelContent;

/// Notion's per-rich-text-value ceiling.
pub const RICH_TEXT_LIMIT: usize = 2000;

/// Notion's per-request block ceiling.
pub const BLOCKS_PER_REQUEST: usize = 100;

/// Splits text into chunks Notion will accept.
///
/// Splits on a character boundary near the limit rather than mid-word where it
/// can, because the result is read by a person.
pub fn chunk_text(text: &str, limit: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return vec![text.to_string()];
    }

    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        if end < chars.len() {
            // Walk back to whitespace, but not far — a chunk of one long token
            // (a URL) has no space to find, and an unbounded search would make
            // no progress at all.
            let floor = end.saturating_sub(80).max(start + 1);
            let mut candidate = end;
            while candidate > floor && !chars[candidate - 1].is_whitespace() {
                candidate -= 1;
            }
            if candidate > floor {
                end = candidate;
            }
        }
        out.push(chars[start..end].iter().collect());
        start = end;
    }
    out
}

/// One rich-text run.
fn rich_text(content: &str) -> Value {
    json!({ "type": "text", "text": { "content": content } })
}

/// Rich text for a value that may exceed the limit.
fn rich_text_runs(text: &str) -> Vec<Value> {
    chunk_text(text, RICH_TEXT_LIMIT)
        .iter()
        .map(|chunk| rich_text(chunk))
        .collect()
}

/// The page properties, mapped onto a database's own column names.
///
/// Only properties the target database actually has are sent. Notion rejects
/// the entire request for one unknown property name, so a database missing
/// "Duration" would otherwise make export fail outright rather than export
/// what it can.
pub fn properties(
    title_property: &str,
    available: &[String],
    title: &str,
    started_at_ms: i64,
    duration_ms: Option<i64>,
    folder: Option<&str>,
    attendees: &[String],
) -> Value {
    let mut props = serde_json::Map::new();

    props.insert(
        title_property.to_string(),
        json!({ "title": [rich_text(title)] }),
    );

    let has = |name: &str| available.iter().any(|p| p == name);

    if has("Date") {
        props.insert(
            "Date".into(),
            json!({ "date": { "start": iso8601(started_at_ms) } }),
        );
    }
    if has("Duration") {
        if let Some(ms) = duration_ms {
            // Minutes: a number column showing 5400000 helps nobody.
            props.insert(
                "Duration".into(),
                json!({ "number": (ms as f64 / 60_000.0).round() }),
            );
        }
    }
    if has("Folder") {
        if let Some(folder) = folder {
            props.insert("Folder".into(), json!({ "select": { "name": folder } }));
        }
    }
    if has("Attendees") && !attendees.is_empty() {
        props.insert(
            "Attendees".into(),
            json!({
                "multi_select": attendees
                    .iter()
                    .map(|name| json!({ "name": name }))
                    .collect::<Vec<_>>()
            }),
        );
    }

    Value::Object(props)
}

/// Milliseconds since the epoch as an ISO-8601 date.
///
/// Hand-rolled rather than pulling in a date crate for one format: Notion
/// accepts a bare `YYYY-MM-DD`, and that is all the Date property needs.
pub fn iso8601(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's days-from-civil, inverted. Correct for any proleptic
/// Gregorian date, which is more than enough for a meeting.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn heading(text: &str) -> Value {
    json!({
        "object": "block",
        "type": "heading_2",
        "heading_2": { "rich_text": [rich_text(text)] }
    })
}

fn bullet(text: &str) -> Value {
    json!({
        "object": "block",
        "type": "bulleted_list_item",
        "bulleted_list_item": { "rich_text": rich_text_runs(text) }
    })
}

fn paragraph(text: &str) -> Value {
    json!({
        "object": "block",
        "type": "paragraph",
        "paragraph": { "rich_text": rich_text_runs(text) }
    })
}

/// The page body: the summary, and optionally the transcript.
pub fn blocks(panel: &PanelContent, transcript: Option<&[Utterance]>) -> Vec<Value> {
    let mut out = Vec::new();

    for section in &panel.sections {
        out.push(heading(&section.heading));
        for item in &section.bullets {
            // Citations are deliberately not carried across. A `#42` chip means
            // nothing in Notion — it cannot be clicked, and it resolves against
            // a database the reader does not have. The transcript below is the
            // evidence there.
            out.push(bullet(&item.text));
        }
    }

    if let Some(lines) = transcript {
        out.push(heading("Transcript"));
        for line in lines {
            let speaker = if line.source == "mic" { "You" } else { "Them" };
            out.push(paragraph(&format!(
                "[{}] {speaker}: {}",
                timecode(line.start_ms),
                line.text
            )));
        }
    }

    out
}

fn timecode(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// Splits blocks into requests Notion will accept.
pub fn batches(blocks: Vec<Value>) -> Vec<Vec<Value>> {
    blocks
        .chunks(BLOCKS_PER_REQUEST)
        .map(|chunk| chunk.to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::content::{Bullet, Section};

    fn panel() -> PanelContent {
        PanelContent {
            sections: vec![Section {
                heading: "Decisions".into(),
                bullets: vec![Bullet {
                    text: "Ship on Thursday".into(),
                    source_utterances: vec![7],
                    from_note: Some("b1".into()),
                }],
            }],
        }
    }

    fn utterance(id: i64, source: &str, text: &str, start_ms: i64) -> Utterance {
        Utterance {
            id,
            seq: id,
            source: source.into(),
            text: text.into(),
            start_ms,
            end_ms: start_ms + 1_000,
            confidence: None,
        }
    }

    #[test]
    fn a_short_value_is_one_chunk() {
        assert_eq!(chunk_text("hello", RICH_TEXT_LIMIT), vec!["hello"]);
    }

    #[test]
    fn a_long_value_is_split_under_the_limit() {
        // Over 2000 characters Notion rejects the whole request, so this is the
        // difference between exporting and failing.
        let text = "word ".repeat(1000);
        let chunks = chunk_text(&text, RICH_TEXT_LIMIT);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= RICH_TEXT_LIMIT);
        }
        assert_eq!(chunks.join(""), text, "text was lost or duplicated");
    }

    #[test]
    fn splitting_prefers_a_word_boundary() {
        // Ten words of five characters; a limit of 32 falls mid-word unless the
        // split backs off to the space before it.
        let text = "alpha bravo charl delta echo1 foxtr golfx hotel india juliet";
        let chunks = chunk_text(text, 32);
        assert!(chunks.len() > 1);
        assert!(
            !chunks[1].starts_with(|c: char| c.is_alphanumeric()) || chunks[0].ends_with(' '),
            "split mid-word: {:?} | {:?}",
            chunks[0],
            chunks[1]
        );
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn one_unbroken_token_longer_than_the_limit_still_terminates() {
        // A URL has no whitespace to back off to; an unbounded search would
        // make no progress and loop forever.
        let text = "x".repeat(5_000);
        let chunks = chunk_text(&text, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn multibyte_text_splits_on_characters_not_bytes() {
        let text = "é".repeat(3_000);
        let chunks = chunk_text(&text, 2000);
        assert_eq!(chunks.join(""), text);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 2000);
        }
    }

    #[test]
    fn the_title_property_is_always_sent() {
        let props = properties("Name", &[], "Standup", 0, None, None, &[]);
        assert!(props["Name"]["title"].is_array());
    }

    #[test]
    fn the_title_property_can_be_named_anything() {
        // Notion databases name their title column whatever the user chose.
        let props = properties("Meeting", &[], "Standup", 0, None, None, &[]);
        assert!(props["Meeting"]["title"].is_array());
    }

    #[test]
    fn only_properties_the_database_has_are_sent() {
        // Notion rejects the entire request for one unknown property name, so a
        // database without "Duration" would otherwise fail outright rather than
        // exporting what it can.
        let props = properties(
            "Name",
            &["Date".to_string()],
            "Standup",
            0,
            Some(600_000),
            Some("Clients"),
            &["Ada".into()],
        );
        assert!(props.get("Date").is_some());
        assert!(props.get("Duration").is_none());
        assert!(props.get("Folder").is_none());
        assert!(props.get("Attendees").is_none());
    }

    #[test]
    fn duration_is_reported_in_minutes() {
        // A number column showing 5400000 helps nobody.
        let props = properties(
            "Name",
            &["Duration".to_string()],
            "Standup",
            0,
            Some(5_400_000),
            None,
            &[],
        );
        assert_eq!(props["Duration"]["number"], 90.0);
    }

    #[test]
    fn an_absent_duration_is_omitted_rather_than_zero() {
        // Zero would read as "a meeting that took no time".
        let props = properties("Name", &["Duration".to_string()], "S", 0, None, None, &[]);
        assert!(props.get("Duration").is_none());
    }

    #[test]
    fn dates_are_formatted_the_way_notion_expects() {
        assert_eq!(iso8601(0), "1970-01-01");
        // Cross-checked against `date -u -r 1785000000`.
        assert_eq!(iso8601(1_785_000_000_000), "2026-07-25");
        assert_eq!(iso8601(1_700_000_000_000), "2023-11-14");
    }

    #[test]
    fn a_date_before_the_epoch_does_not_panic() {
        assert_eq!(iso8601(-86_400_000), "1969-12-31");
    }

    #[test]
    fn the_summary_becomes_headings_and_bullets() {
        let out = blocks(&panel(), None);
        assert_eq!(out[0]["type"], "heading_2");
        assert_eq!(
            out[0]["heading_2"]["rich_text"][0]["text"]["content"],
            "Decisions"
        );
        assert_eq!(out[1]["type"], "bulleted_list_item");
    }

    #[test]
    fn citations_are_not_carried_into_notion() {
        // A `#42` chip cannot be clicked there and resolves against a database
        // the reader does not have.
        let out = blocks(&panel(), None);
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("#7"), "a citation leaked into the page");
    }

    #[test]
    fn the_transcript_is_optional() {
        assert_eq!(blocks(&panel(), None).len(), 2);
        let with = blocks(&panel(), Some(&[utterance(1, "mic", "hello", 0)]));
        assert_eq!(with.len(), 4, "heading plus one line");
    }

    #[test]
    fn transcript_lines_carry_speaker_and_timecode() {
        let out = blocks(&panel(), Some(&[utterance(1, "mic", "we ship", 65_000)]));
        let line = &out[3]["paragraph"]["rich_text"][0]["text"]["content"];
        assert_eq!(line, "[01:05] You: we ship");
    }

    #[test]
    fn the_other_speaker_is_them() {
        let out = blocks(&panel(), Some(&[utterance(1, "system", "hi", 0)]));
        let line = out[3]["paragraph"]["rich_text"][0]["text"]["content"]
            .as_str()
            .unwrap();
        assert!(line.contains("Them:"), "{line}");
    }

    #[test]
    fn a_very_long_transcript_line_is_split_into_runs() {
        let long = "word ".repeat(1000);
        let out = blocks(&panel(), Some(&[utterance(1, "mic", &long, 0)]));
        let runs = out[3]["paragraph"]["rich_text"].as_array().unwrap();
        assert!(runs.len() > 1, "a 5000-character line was sent as one run");
    }

    #[test]
    fn blocks_are_batched_under_the_request_limit() {
        // A two-hour transcript is thousands of lines; one request would be
        // rejected outright.
        let many: Vec<Value> = (0..250).map(|i| bullet(&format!("line {i}"))).collect();
        let batched = batches(many);
        assert_eq!(batched.len(), 3);
        assert!(batched.iter().all(|b| b.len() <= BLOCKS_PER_REQUEST));
        assert_eq!(batched.iter().map(|b| b.len()).sum::<usize>(), 250);
    }

    #[test]
    fn an_empty_body_produces_no_batches() {
        assert!(batches(Vec::new()).is_empty());
    }
}
