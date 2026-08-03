//! The snippet under a search result.
//!
//! A result that shows the first eighty characters of a two-minute utterance
//! makes the user open every hit to find out which one they wanted. The snippet
//! has to be centred on the *match*, and it has to say which part matched.
//!
//! Offsets are returned rather than markup: the frontend renders the highlight,
//! so this never has to think about escaping, and a snippet cannot smuggle HTML
//! out of a transcript into the UI.

use serde::Serialize;

/// A snippet with the matched ranges marked.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub text: String,
    /// `(start, end)` character offsets into `text`, not bytes — the frontend
    /// indexes by character, and a byte offset would land mid-glyph on any
    /// transcript with an accent or an em dash in it.
    pub spans: Vec<(usize, usize)>,
    /// Whether text was dropped from the front, so the UI can show an ellipsis.
    pub truncated_start: bool,
    pub truncated_end: bool,
}

use super::COMMON_WORDS;

/// Terms worth highlighting, lowercased.
fn terms(query: &str) -> Vec<String> {
    let all: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();

    let meaningful: Vec<String> = all
        .iter()
        .filter(|t| !COMMON_WORDS.contains(&t.as_str()))
        .cloned()
        .collect();

    // A query of nothing but common words still has to highlight something, or
    // the result looks like it matched for no reason.
    if meaningful.is_empty() {
        all
    } else {
        meaningful
    }
}

/// Every character range in `text` matching any term.
///
/// Prefix matching, to agree with the `*` the FTS query puts on its last term —
/// otherwise typing "migra" highlights nothing in the result it just found.
fn matches(text: &str, terms: &[String]) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let lowered: Vec<char> = text.to_lowercase().chars().collect();
    // Case folding can change length (ß → ss), which would misalign every
    // offset after it. Falling back to the original text is a worse highlight
    // and a correct one.
    let haystack = if lowered.len() == chars.len() {
        lowered
    } else {
        chars.iter().map(|c| c.to_ascii_lowercase()).collect()
    };

    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < haystack.len() {
        // Only start a match at a word boundary, so "art" does not highlight the
        // middle of "start".
        let at_boundary = index == 0 || !haystack[index - 1].is_alphanumeric();
        if at_boundary {
            for term in terms {
                let needle: Vec<char> = term.chars().collect();
                if index + needle.len() <= haystack.len()
                    && haystack[index..index + needle.len()] == needle[..]
                {
                    // Extend to the end of the word: matching "migra" should
                    // highlight "migration", not five letters of it.
                    let mut end = index + needle.len();
                    while end < haystack.len() && haystack[end].is_alphanumeric() {
                        end += 1;
                    }
                    spans.push((index, end));
                    break;
                }
            }
        }
        index += 1;
    }
    spans
}

/// A snippet of `text` centred on the first match.
///
/// `max_chars` is a budget, not a promise: a match wider than the budget is
/// never cut in half, because a highlight that stops mid-word looks like a bug.
pub fn preview(text: &str, query: &str, max_chars: usize) -> Preview {
    let chars: Vec<char> = text.chars().collect();
    let found = matches(text, &terms(query));

    if chars.len() <= max_chars {
        return Preview {
            text: text.to_string(),
            spans: found,
            truncated_start: false,
            truncated_end: false,
        };
    }

    // Centre on the first match; with no match at all, show the opening.
    let focus = found.first().map(|(start, _)| *start).unwrap_or(0);
    let half = max_chars / 2;
    let mut start = focus.saturating_sub(half);
    let mut end = (start + max_chars).min(chars.len());
    // Re-widen backwards when the match sits near the end, or the snippet would
    // be shorter than the budget for no reason.
    start = end.saturating_sub(max_chars).min(start);

    // Snap to word boundaries so the snippet does not begin or end mid-word —
    // but only a little. A transcript containing a long URL is one unbroken
    // "word", and an unbounded snap walks the whole snippet away: the end
    // pointer marches down to zero and the subtraction underflows. A hard cut
    // through a hundred-character link is the better failure.
    const SNAP_LIMIT: usize = 15;

    let snap_start_limit = start + SNAP_LIMIT;
    while start > 0
        && start < snap_start_limit
        && start < end
        && chars[start].is_alphanumeric()
        && chars[start - 1].is_alphanumeric()
    {
        start += 1;
    }

    let snap_end_floor = end.saturating_sub(SNAP_LIMIT).max(start + 1);
    while end > snap_end_floor
        && end < chars.len()
        && chars[end - 1].is_alphanumeric()
        && chars[end].is_alphanumeric()
    {
        end -= 1;
    }

    let snippet: String = chars[start..end].iter().collect();
    let spans = found
        .into_iter()
        .filter(|(s, e)| *s >= start && *e <= end)
        .map(|(s, e)| (s - start, e - start))
        .collect();

    Preview {
        text: snippet,
        spans,
        truncated_start: start > 0,
        truncated_end: end < chars.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_line_is_shown_whole() {
        let out = preview("the deadline is Thursday", "deadline", 200);
        assert_eq!(out.text, "the deadline is Thursday");
        assert!(!out.truncated_start && !out.truncated_end);
    }

    #[test]
    fn the_match_is_marked() {
        let out = preview("the deadline is Thursday", "deadline", 200);
        assert_eq!(out.spans, vec![(4, 12)]);
        let (start, end) = out.spans[0];
        let marked: String = out.text.chars().skip(start).take(end - start).collect();
        assert_eq!(marked, "deadline");
    }

    #[test]
    fn matching_is_case_insensitive() {
        let out = preview("The Deadline is Thursday", "deadline", 200);
        assert_eq!(out.spans.len(), 1);
    }

    #[test]
    fn every_term_is_marked() {
        let out = preview("the deadline is Thursday", "deadline thursday", 200);
        assert_eq!(out.spans.len(), 2);
    }

    #[test]
    fn a_prefix_highlights_the_whole_word() {
        // The FTS query makes the last term a prefix, so typing "migra" finds a
        // line containing "migration". Highlighting five letters of it would
        // look like a rendering bug.
        let out = preview("the migration is late", "migra", 200);
        let (start, end) = out.spans[0];
        let marked: String = out.text.chars().skip(start).take(end - start).collect();
        assert_eq!(marked, "migration");
    }

    #[test]
    fn a_term_inside_another_word_is_not_marked() {
        // "art" must not highlight the middle of "start" — the result would look
        // arbitrary and wrong.
        let out = preview("we start on Monday", "art", 200);
        assert!(out.spans.is_empty(), "{:?}", out.spans);
    }

    #[test]
    fn a_long_line_is_centred_on_the_match() {
        let text = format!("{} needle {}", "a ".repeat(200), "b ".repeat(200));
        let out = preview(&text, "needle", 80);

        assert!(out.text.contains("needle"));
        assert!(out.truncated_start && out.truncated_end);
        assert!(
            out.text.chars().count() <= 80,
            "{}",
            out.text.chars().count()
        );
    }

    #[test]
    fn the_span_offsets_are_relative_to_the_snippet() {
        // The frontend slices `text` by these; an offset into the original
        // would highlight the wrong words or panic.
        let text = format!("{} needle tail", "a ".repeat(200));
        let out = preview(&text, "needle", 60);
        let (start, end) = out.spans[0];
        let marked: String = out.text.chars().skip(start).take(end - start).collect();
        assert_eq!(marked, "needle");
    }

    #[test]
    fn a_match_near_the_end_still_fills_the_budget() {
        // Clamping naively would return a stub snippet when the match is in the
        // last few characters.
        let text = format!("{}needle", "a ".repeat(200));
        let out = preview(&text, "needle", 80);
        assert!(out.text.chars().count() > 40, "{}", out.text);
        assert!(out.text.contains("needle"));
    }

    #[test]
    fn a_long_line_with_no_match_shows_its_opening() {
        let text = "one two three ".repeat(60);
        let out = preview(&text, "nothing", 50);
        assert!(out.text.chars().count() <= 50);
        assert!(!out.text.is_empty());
        assert!(!out.truncated_start);
        assert!(out.truncated_end);
        assert!(out.spans.is_empty());
    }

    #[test]
    fn one_unbroken_word_longer_than_the_budget_is_cut_not_erased() {
        // A URL in a transcript is a single "word". Snapping to a boundary that
        // does not exist walked the snippet to nothing and underflowed.
        let text = "x".repeat(500);
        let out = preview(&text, "nothing", 50);
        assert!(!out.text.is_empty(), "the snippet was erased");
        assert!(out.text.chars().count() <= 50);
    }

    #[test]
    fn a_long_url_around_the_match_does_not_erase_the_snippet() {
        let text = format!(
            "see https://example.com/{} for the deadline",
            "a".repeat(300)
        );
        let out = preview(&text, "deadline", 60);
        assert!(!out.text.is_empty());
        assert!(out.text.contains("deadline"));
    }

    #[test]
    fn multibyte_text_is_sliced_by_character_not_byte() {
        // A byte offset lands mid-glyph and either panics or renders mojibake.
        let text = "café — the déjà vu of migration déjà vu again and again";
        let out = preview(text, "migration", 30);
        assert!(out.text.contains("migration"));

        let (start, end) = out.spans[0];
        let marked: String = out.text.chars().skip(start).take(end - start).collect();
        assert_eq!(marked, "migration");
    }

    #[test]
    fn common_words_are_matched_but_not_highlighted() {
        // Terms are ORed, so "the" is a legitimate match — painting it in every
        // snippet just buries the word the user cares about.
        let out = preview(
            "we should cut the release scope in half",
            "shrink the scope",
            200,
        );
        let marked: Vec<String> = out
            .spans
            .iter()
            .map(|(s, e)| out.text.chars().skip(*s).take(e - s).collect())
            .collect();
        assert_eq!(marked, vec!["scope"]);
    }

    #[test]
    fn a_query_of_only_common_words_still_highlights_them() {
        // Otherwise the result appears to have matched for no reason at all.
        let out = preview("the thing", "the", 200);
        assert_eq!(out.spans.len(), 1);
    }

    #[test]
    fn an_empty_query_marks_nothing_and_does_not_panic() {
        let out = preview("some text", "", 200);
        assert!(out.spans.is_empty());
        assert_eq!(out.text, "some text");
    }

    #[test]
    fn empty_text_is_harmless() {
        let out = preview("", "query", 200);
        assert_eq!(out.text, "");
        assert!(out.spans.is_empty());
    }

    #[test]
    fn spans_never_point_outside_the_snippet() {
        // The invariant the frontend depends on. A span past the end would throw
        // when slicing.
        let text = format!("{} needle {}", "word ".repeat(60), "tail ".repeat(60));
        let out = preview(&text, "needle word tail", 60);
        let length = out.text.chars().count();
        for (start, end) in &out.spans {
            assert!(*start <= *end, "inverted span");
            assert!(*end <= length, "span {end} past end {length}");
        }
    }
}
