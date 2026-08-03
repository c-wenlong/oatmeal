//! Finding a moment you half-remember.
//!
//! Two indexes answer the same question badly on their own. FTS5 finds the exact
//! words and misses "the thing about the vendor contract" when nobody said
//! "vendor". Vectors find the topic and rank a vaguely-related sentence above
//! the one containing the phrase you actually typed. The point of this module is
//! to combine them without either one dominating.
//!
//! The scores are not comparable — FTS5 `rank` is a negative BM25 where more
//! negative is better, cosine distance is a small positive where less is better,
//! and neither has a stable range across queries. So they are **not** normalised
//! and blended; they are fused by *rank position*, which is the one thing both
//! lists genuinely agree on the meaning of.

pub mod preview;
pub mod query;

use serde::Serialize;

pub use preview::{preview, Preview};
pub use query::{search, SearchResponse, SearchResult};

/// Which index found a hit. Kept so the UI can say why something matched, and
/// so "is the semantic half doing anything" stays answerable on real data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Only the full-text index found it.
    Keyword,
    /// Only the vector index found it.
    Semantic,
    /// Both. The strongest signal there is.
    Both,
}

/// One matching utterance.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub utterance_id: i64,
    pub meeting_id: String,
    pub text: String,
    pub start_ms: i64,
    pub kind: MatchKind,
    /// Fused score. Comparable only within one result set.
    pub score: f64,
}

/// Hits grouped under the meeting they came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingHits {
    pub meeting_id: String,
    pub title: Option<String>,
    pub started_at: i64,
    /// Best hit's position in the recording — what "jump to the moment" uses.
    pub best_at_ms: i64,
    pub best_utterance_id: i64,
    pub hits: Vec<Hit>,
    /// Sum of the group's hit scores, so a meeting that matched in several
    /// places outranks one that matched once.
    pub score: f64,
}

/// Reciprocal-rank-fusion constant.
///
/// The standard 60. It damps the top of each list so a single index cannot win
/// on its own: rank 1 contributes 1/61 and rank 2 contributes 1/62, a gap far
/// smaller than the gap between "in both lists" and "in one". That is the
/// behaviour we want — agreement between the two indexes should beat a confident
/// answer from either.
pub const RRF_K: f64 = 60.0;

/// Fuses two ranked lists of utterance ids.
///
/// Input order *is* the ranking; the callers hand over whatever their index
/// considered best-first. Returns ids best-first with a fused score and where
/// each came from.
pub fn fuse(keyword: &[i64], semantic: &[i64]) -> Vec<(i64, f64, MatchKind)> {
    let mut scored: Vec<(i64, f64, MatchKind)> = Vec::new();

    let position = |list: &[i64], id: i64| list.iter().position(|candidate| *candidate == id);

    let mut seen = std::collections::HashSet::new();
    for id in keyword.iter().chain(semantic.iter()) {
        if !seen.insert(*id) {
            continue;
        }

        let in_keyword = position(keyword, *id);
        let in_semantic = position(semantic, *id);

        let mut score = 0.0;
        if let Some(rank) = in_keyword {
            score += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        if let Some(rank) = in_semantic {
            score += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        let kind = match (in_keyword, in_semantic) {
            (Some(_), Some(_)) => MatchKind::Both,
            (Some(_), None) => MatchKind::Keyword,
            (None, Some(_)) => MatchKind::Semantic,
            (None, None) => unreachable!("id came from one of the two lists"),
        };
        scored.push((*id, score, kind));
    }

    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored
}

/// Groups hits by meeting, best meeting first.
///
/// Grouping rather than a flat list because five hits from one long meeting
/// would otherwise bury five different meetings that each matched once, and
/// "which conversation was that in" is usually the actual question.
pub fn group(
    hits: Vec<Hit>,
    titles: &std::collections::HashMap<String, (Option<String>, i64)>,
) -> Vec<MeetingHits> {
    let mut groups: Vec<MeetingHits> = Vec::new();

    for hit in hits {
        if let Some(group) = groups.iter_mut().find(|g| g.meeting_id == hit.meeting_id) {
            group.score += hit.score;
            group.hits.push(hit);
        } else {
            let (title, started_at) = titles.get(&hit.meeting_id).cloned().unwrap_or((None, 0));
            groups.push(MeetingHits {
                meeting_id: hit.meeting_id.clone(),
                title,
                started_at,
                // Hits arrive best-first, so the first one seen for a meeting is
                // its best — that is the moment to jump to.
                best_at_ms: hit.start_ms,
                best_utterance_id: hit.utterance_id,
                score: hit.score,
                hits: vec![hit],
            });
        }
    }

    groups.sort_by(|a, b| b.score.total_cmp(&a.score));
    groups
}

/// Words carried by almost every sentence.
///
/// Dropped from the query, not merely from the highlight. Under `AND` they were
/// harmless — they matched everywhere and so filtered nothing. Under `OR` they
/// match everywhere and *earn a rank*, and because fusion is by rank position
/// rather than by score, BM25's sensible decision to weight them near zero never
/// gets a say: a line whose only match is "the" still lands in the ranked list
/// and collects the same reciprocal-rank credit as a real hit.
///
/// Searching "shrink the scope" ranked a vendor meeting above the line that
/// literally said "cut the release scope in half", purely on the strength of
/// "the".
pub const COMMON_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "is", "it", "of",
    "on", "or", "that", "the", "to", "was", "we", "with",
];

/// Turns user input into an FTS5 MATCH expression.
///
/// FTS5 treats punctuation as syntax: an apostrophe, a hyphen or a stray quote
/// is a parse error, and a search box that throws on `don't` is broken. Every
/// term is quoted, and a trailing `*` makes the last one a prefix so results
/// appear while still typing.
///
/// **Terms are joined with `OR`, not FTS5's default `AND`.** This feature exists
/// for a phrase someone remembers imperfectly, so a query is *expected* to
/// contain a word that was never said — and under `AND` a single wrong word
/// makes the whole keyword half match nothing. Searching "shrink the scope" for
/// a line reading "cut the release scope in half" found it only by luck of the
/// embedding, and ranked an unrelated meeting above it. BM25 already weights a
/// rare matched term far above a common one, so `OR` costs precision much less
/// than `AND` costs recall here.
pub fn to_fts_query(input: &str) -> Option<String> {
    let all: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();

    if all.is_empty() {
        return None;
    }

    // Keep the common words only when they are all there is — someone searching
    // literally "the" should still get something.
    let meaningful: Vec<String> = all
        .iter()
        .filter(|t| !COMMON_WORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    let terms = if meaningful.is_empty() {
        all
    } else {
        meaningful
    };

    let last = terms.len() - 1;
    let quoted: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(index, term)| {
            if index == last {
                format!("\"{term}\"*")
            } else {
                format!("\"{term}\"")
            }
        })
        .collect();
    Some(quoted.join(" OR "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_hit_in_both_indexes_beats_a_top_hit_in_one() {
        // The whole reason for fusing by rank. An utterance both indexes found
        // is far better evidence than one that only FTS ranked first.
        let fused = fuse(&[1, 2], &[2, 3]);
        assert_eq!(fused[0].0, 2);
        assert_eq!(fused[0].2, MatchKind::Both);
    }

    #[test]
    fn a_keyword_only_hit_still_appears() {
        // Someone searching an exact phrase must find it even when the embedder
        // is not running and the semantic list is empty.
        let fused = fuse(&[7, 8], &[]);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, 7);
        assert_eq!(fused[0].2, MatchKind::Keyword);
    }

    #[test]
    fn a_semantic_only_hit_still_appears() {
        // The half-remembered case: none of the typed words were said.
        let fused = fuse(&[], &[4]);
        assert_eq!(fused[0].0, 4);
        assert_eq!(fused[0].2, MatchKind::Semantic);
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert!(fuse(&[], &[]).is_empty());
    }

    #[test]
    fn rank_order_within_one_list_is_preserved() {
        let fused = fuse(&[10, 11, 12], &[]);
        assert_eq!(
            fused.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
    }

    #[test]
    fn an_id_repeated_in_one_list_is_not_double_counted() {
        // A defensive case: a malformed index result must not let one row win by
        // appearing twice.
        let fused = fuse(&[5, 5, 6], &[]);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn ties_break_deterministically() {
        // Two ids at the same rank in different lists score identically; without
        // a tiebreak the order would vary between runs and the UI would jitter.
        let first = fuse(&[1], &[2]);
        let second = fuse(&[1], &[2]);
        assert_eq!(first, second);
        assert_eq!(first[0].0, 1, "lower id should win a tie");
    }

    fn hit(id: i64, meeting: &str, score: f64, at_ms: i64) -> Hit {
        Hit {
            utterance_id: id,
            meeting_id: meeting.into(),
            text: format!("line {id}"),
            start_ms: at_ms,
            kind: MatchKind::Keyword,
            score,
        }
    }

    fn titles() -> HashMap<String, (Option<String>, i64)> {
        HashMap::from([
            ("m1".to_string(), (Some("Standup".to_string()), 1_000)),
            ("m2".to_string(), (Some("Retro".to_string()), 2_000)),
        ])
    }

    #[test]
    fn hits_are_grouped_under_their_meeting() {
        let groups = group(
            vec![
                hit(1, "m1", 0.5, 10_000),
                hit(2, "m2", 0.4, 20_000),
                hit(3, "m1", 0.3, 30_000),
            ],
            &titles(),
        );

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].meeting_id, "m1");
        assert_eq!(groups[0].hits.len(), 2);
        assert_eq!(groups[0].title.as_deref(), Some("Standup"));
    }

    #[test]
    fn a_meeting_matching_repeatedly_outranks_one_matching_once() {
        // Two weak mentions of a topic is usually a better answer to "where did
        // we discuss X" than one passing reference.
        let groups = group(
            vec![
                hit(1, "m2", 0.40, 0),
                hit(2, "m1", 0.30, 0),
                hit(3, "m1", 0.30, 0),
            ],
            &titles(),
        );
        assert_eq!(groups[0].meeting_id, "m1");
    }

    #[test]
    fn the_group_points_at_its_best_moment() {
        // "Jump to the right moment" reads these two fields.
        let groups = group(
            vec![hit(9, "m1", 0.9, 42_000), hit(10, "m1", 0.1, 99_000)],
            &titles(),
        );
        assert_eq!(groups[0].best_at_ms, 42_000);
        assert_eq!(groups[0].best_utterance_id, 9);
    }

    #[test]
    fn a_meeting_with_no_title_row_still_groups() {
        // A meeting deleted between the search and the lookup must not panic.
        let groups = group(vec![hit(1, "gone", 0.5, 0)], &titles());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, None);
    }

    #[test]
    fn an_apostrophe_does_not_break_the_query() {
        // FTS5 treats punctuation as syntax; `don't` is a parse error unquoted,
        // and a search box that throws on an ordinary word is broken. The
        // apostrophe splits the word, which is what the porter tokenizer does
        // to the indexed text too, so the halves still match.
        let query = to_fts_query("don't ship").unwrap();
        assert_eq!(query, "\"don\" OR \"t\" OR \"ship\"*");
        assert!(!query.contains('\''), "an apostrophe reached FTS5: {query}");
    }

    #[test]
    fn the_last_term_is_a_prefix_so_results_appear_while_typing() {
        assert_eq!(to_fts_query("migra").unwrap(), "\"migra\"*");
        assert_eq!(
            to_fts_query("migration dead").unwrap(),
            "\"migration\" OR \"dead\"*"
        );
    }

    #[test]
    fn terms_are_ored_so_one_wrong_word_does_not_lose_the_line() {
        // The bug this replaced: under FTS5's default AND, searching
        // "shrink the scope" for a line reading "cut the release scope in half"
        // matched nothing at all, because "shrink" was never said. A feature for
        // half-remembered phrases cannot require every word to be right.
        let query = to_fts_query("shrink the scope").unwrap();
        assert!(query.contains(" OR "), "terms were ANDed: {query}");
        assert_eq!(query, "\"shrink\" OR \"scope\"*");
    }

    #[test]
    fn common_words_are_kept_out_of_the_query() {
        // Under OR they match every line and earn a rank, and fusion is by rank
        // — so BM25 never gets to discount them. "shrink the scope" ranked an
        // unrelated meeting first purely on the strength of "the".
        assert_eq!(to_fts_query("the scope").unwrap(), "\"scope\"*");
    }

    #[test]
    fn a_query_of_only_common_words_still_searches_for_them() {
        // Someone searching literally "the" should get something back.
        assert_eq!(to_fts_query("the").unwrap(), "\"the\"*");
    }

    #[test]
    fn punctuation_only_input_produces_no_query() {
        // Better than an FTS5 syntax error, and better than matching everything.
        assert_eq!(to_fts_query("   "), None);
        assert_eq!(to_fts_query("!!!"), None);
        assert_eq!(to_fts_query(""), None);
    }

    #[test]
    fn quotes_in_the_input_cannot_escape_the_query() {
        // A stray double quote would otherwise terminate the quoted term and
        // turn the rest of the input into FTS5 syntax.
        let query = to_fts_query("say \"hello\" now").unwrap();
        assert!(!query.contains("\"\""), "unbalanced quoting: {query}");
        assert!(query.contains("\"hello\""));
    }
}
