//! Running a search against the database.
//!
//! Where [`super::fuse`] decides *how* two ranked lists combine, this decides
//! what goes into them: full text from FTS5, meaning from the vector index, and
//! the meeting rows needed to group and preview the result.
//!
//! The semantic half is optional at every step. A user with no embedding model
//! running still gets keyword search, degraded rather than broken — the same
//! rule the linker follows.

use rusqlite::Connection;

use super::{fuse, group, preview, Hit, MeetingHits, Preview};
use crate::db::{repo, DbError};
use crate::embed::Embedder;

/// How many candidates each index contributes before fusion.
///
/// Wider than the number shown: fusion can only promote something that at least
/// one index surfaced, so a stingy candidate pool caps quality no matter how
/// good the ranking is.
const CANDIDATES: i64 = 50;

/// Characters of context around a match.
const PREVIEW_CHARS: usize = 160;

/// A search result: a meeting, its best moment, and why it matched.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub meeting: MeetingHits,
    /// Snippets parallel to `meeting.hits`.
    pub previews: Vec<Preview>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    /// False when the embedder was unreachable, so the UI can say the search was
    /// keyword-only rather than silently returning worse answers.
    pub semantic: bool,
}

/// Searches transcripts, optionally within one folder.
pub async fn search(
    conn: &Connection,
    query: &str,
    folder_id: Option<&str>,
    embedder: &impl Embedder,
    limit: usize,
) -> Result<SearchResponse, DbError> {
    let Some(fts_query) = super::to_fts_query(query) else {
        return Ok(SearchResponse {
            results: Vec::new(),
            semantic: false,
        });
    };

    // Keyword half. A malformed MATCH is treated as "no keyword hits" rather
    // than an error: the query is built from user typing, and a search box that
    // shows a SQL error has failed at its job.
    let keyword_rows =
        repo::search_rows_fts(conn, &fts_query, folder_id, CANDIDATES).unwrap_or_default();

    // Semantic half.
    let mut semantic_rows = Vec::new();
    let mut semantic_ok = false;
    if let Ok(vectors) = embedder.embed(&[query.to_string()]).await {
        if let Some(vector) = vectors.first() {
            semantic_ok = true;
            let neighbours = repo::nearest_embeddings(conn, vector, CANDIDATES).unwrap_or_default();
            let ids: Vec<i64> = neighbours
                .iter()
                .filter(|hit| hit.owner_type == "utterance")
                .filter_map(|hit| hit.owner_id.parse().ok())
                .collect();

            // `nearest_embeddings` returns ids in distance order; the lookup does
            // not preserve it, so the order is restored here. Fusion is by rank,
            // so losing the order would silently flatten the semantic signal.
            let fetched = repo::utterances_by_id(conn, &ids)?;
            for id in &ids {
                if let Some(row) = fetched.iter().find(|row| row.id == *id) {
                    // Folder scoping is applied here rather than in SQL: the
                    // vector index has no join to meetings.
                    semantic_rows.push(row.clone());
                }
            }
        }
    }

    if let Some(folder) = folder_id {
        let allowed = repo::meetings_in_folder(conn, Some(folder), 10_000)?;
        semantic_rows.retain(|row| allowed.iter().any(|m| m.id == row.meeting_id));
    }

    let keyword_ids: Vec<i64> = keyword_rows.iter().map(|row| row.id).collect();
    let semantic_ids: Vec<i64> = semantic_rows.iter().map(|row| row.id).collect();
    let fused = fuse(&keyword_ids, &semantic_ids);

    let mut hits: Vec<Hit> = Vec::new();
    for (id, score, kind) in fused.into_iter().take(limit * 4) {
        let Some(row) = keyword_rows
            .iter()
            .chain(semantic_rows.iter())
            .find(|row| row.id == id)
        else {
            continue;
        };
        hits.push(Hit {
            utterance_id: row.id,
            meeting_id: row.meeting_id.clone(),
            text: row.text.clone(),
            start_ms: row.start_ms,
            kind,
            score,
        });
    }

    let meeting_ids: Vec<String> = {
        let mut ids: Vec<String> = hits.iter().map(|hit| hit.meeting_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let headers = repo::meeting_headers(conn, &meeting_ids)?;

    let mut groups = group(hits, &headers);
    groups.truncate(limit);

    let results = groups
        .into_iter()
        .map(|meeting| {
            let previews = meeting
                .hits
                .iter()
                .map(|hit| preview(&hit.text, query, PREVIEW_CHARS))
                .collect();
            SearchResult { meeting, previews }
        })
        .collect();

    Ok(SearchResponse {
        results,
        semantic: semantic_ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::embed::{EmbedError, HashEmbedder};

    struct DeadEmbedder;

    impl Embedder for DeadEmbedder {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Err(EmbedError::Unreachable {
                url: "http://localhost:11434/v1".into(),
                detail: "connection refused".into(),
            })
        }
    }

    /// Two meetings, three weeks apart, on overlapping topics.
    fn seeded() -> Database {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();

        repo::insert_meeting(conn, "m1", "Vendor review", 1_000).unwrap();
        repo::insert_meeting(conn, "m2", "Standup", 2_000).unwrap();

        let lines = [
            ("m1", 0, "the vendor wants a two year commitment"),
            ("m1", 1, "and they will not move on the price at all"),
            ("m1", 2, "who is bringing lunch tomorrow"),
            ("m2", 0, "the migration deadline is the fourteenth"),
            ("m2", 1, "I will own the rollback plan myself"),
        ];
        for (meeting, seq, text) in lines.iter() {
            let start = seq * 10_000;
            let id = repo::insert_utterance(
                conn,
                meeting,
                *seq,
                "system",
                text,
                start,
                start + 3_000,
                None,
            )
            .unwrap();
            // Embed everything, so the semantic half has something to find.
            repo::replace_embedding(
                conn,
                "utterance",
                &id.to_string(),
                &HashEmbedder::vector(text),
            )
            .unwrap();
        }
        db
    }

    #[tokio::test]
    async fn an_exact_phrase_finds_its_meeting_and_moment() {
        // G24's done-when, in miniature: the phrase leads to the right meeting
        // and to the right point in it.
        let db = seeded();
        let response = search(db.connection(), "rollback plan", None, &HashEmbedder, 10)
            .await
            .unwrap();

        assert!(!response.results.is_empty());
        let top = &response.results[0];
        assert_eq!(top.meeting.meeting_id, "m2");
        assert_eq!(top.meeting.best_at_ms, 10_000, "wrong moment");
        assert!(top.previews[0].text.contains("rollback"));
        assert!(!top.previews[0].spans.is_empty(), "nothing highlighted");
    }

    #[tokio::test]
    async fn a_half_remembered_phrase_still_finds_the_meeting() {
        // Nobody said "supplier"; the vector half has to carry this.
        let db = seeded();
        let response = search(
            db.connection(),
            "vendor commitment",
            None,
            &HashEmbedder,
            10,
        )
        .await
        .unwrap();
        assert_eq!(response.results[0].meeting.meeting_id, "m1");
    }

    #[tokio::test]
    async fn search_still_works_with_no_embedder() {
        // Degraded, not broken — the same rule the linker follows.
        let db = seeded();
        let response = search(db.connection(), "rollback", None, &DeadEmbedder, 10)
            .await
            .unwrap();

        assert!(
            !response.results.is_empty(),
            "keyword search stopped working"
        );
        assert!(!response.semantic, "should have reported keyword-only");
    }

    #[tokio::test]
    async fn a_folder_scopes_the_results() {
        let db = seeded();
        let conn = db.connection();
        let folder = repo::create_folder(conn, "Vendors", None, 0).unwrap();
        repo::set_meeting_folder(conn, "m1", Some(&folder)).unwrap();

        // "the" appears in both meetings; only the filed one may come back.
        let response = search(conn, "the", Some(&folder), &HashEmbedder, 10)
            .await
            .unwrap();
        assert!(!response.results.is_empty());
        assert!(
            response
                .results
                .iter()
                .all(|result| result.meeting.meeting_id == "m1"),
            "a meeting outside the folder leaked in"
        );
    }

    #[tokio::test]
    async fn an_empty_query_returns_nothing_rather_than_everything() {
        let db = seeded();
        let response = search(db.connection(), "   ", None, &HashEmbedder, 10)
            .await
            .unwrap();
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn punctuation_does_not_produce_an_error() {
        // FTS5 would throw on this unquoted; a search box must not.
        let db = seeded();
        for query in ["don't", "\"", "a -- b", "()"] {
            assert!(
                search(db.connection(), query, None, &HashEmbedder, 10)
                    .await
                    .is_ok(),
                "query {query:?} errored"
            );
        }
    }

    #[tokio::test]
    async fn results_carry_a_preview_per_hit() {
        // The frontend zips these; a length mismatch would misalign every
        // snippet under its line.
        let db = seeded();
        let response = search(db.connection(), "the", None, &HashEmbedder, 10)
            .await
            .unwrap();
        for result in &response.results {
            assert_eq!(result.previews.len(), result.meeting.hits.len());
        }
    }

    #[tokio::test]
    async fn a_word_nobody_said_finds_nothing() {
        let db = seeded();
        let response = search(db.connection(), "zzzz", None, &DeadEmbedder, 10)
            .await
            .unwrap();
        assert!(response.results.is_empty());
    }
}
