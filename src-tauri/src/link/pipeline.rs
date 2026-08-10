//! Turning a finished meeting into embeddings and links.
//!
//! This is where G16 and G17 meet the database. The linker itself is pure — it
//! takes notes and utterances and returns links — so everything stateful lives
//! here: deciding what still needs embedding, batching it, and writing the
//! result down.
//!
//! **Linking never depends on the embedder being up.** If no local model is
//! running, the semantic layer is simply absent and `link_layered` degrades to
//! the temporal baseline. A user without Ollama installed still gets links;
//! they are just the ones the clock alone can justify.

use rusqlite::Connection;

use super::{link_layered, Link, LinkParams, NoteInput, UtteranceInput};
use crate::db::{repo, DbError};
use crate::embed::Embedder;

/// Batch size for embedding calls.
///
/// Large enough that a long meeting is a handful of round trips, small enough
/// that one oversized request does not trip a server's body limit.
const BATCH: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    /// Texts embedded this run. Already-embedded rows are skipped.
    pub embedded: usize,
    /// Links written.
    pub links: usize,
    /// Set when the embedder could not be reached and linking fell back to
    /// timestamps alone. Not an error — a degraded but working result.
    pub degraded: Option<String>,
}

/// Owner id for a note block's vector.
///
/// Block ids are only unique within a meeting, so they are qualified. Utterance
/// ids are database rowids and already global.
fn note_owner_id(meeting_id: &str, block_id: &str) -> String {
    format!("{meeting_id}:{block_id}")
}

/// Embeds anything in `texts` that has no vector yet, and returns the full map.
async fn ensure_embedded(
    conn: &Connection,
    owner_type: &str,
    items: &[(String, String)],
    embedder: &impl Embedder,
) -> Result<(std::collections::HashMap<String, Vec<f32>>, usize), String> {
    let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
    let mut known = repo::embeddings_for(conn, owner_type, &ids).map_err(|e| e.to_string())?;

    let missing: Vec<&(String, String)> = items
        .iter()
        .filter(|(id, text)| !known.contains_key(id) && !text.trim().is_empty())
        .collect();

    let mut embedded = 0usize;
    for chunk in missing.chunks(BATCH) {
        let texts: Vec<String> = chunk.iter().map(|(_, text)| text.clone()).collect();
        let vectors = embedder.embed(&texts).await.map_err(|e| e.to_string())?;
        for ((id, _), vector) in chunk.iter().zip(vectors) {
            repo::replace_embedding(conn, owner_type, id, &vector).map_err(|e| e.to_string())?;
            known.insert(id.clone(), vector);
            embedded += 1;
        }
    }

    Ok((known, embedded))
}

/// Embeds a meeting's transcript and notes, links them, and stores the result.
///
/// Safe to run repeatedly: embeddings already present are reused, and links are
/// replaced wholesale so a re-run after the user edits their notes cannot leave
/// a stale link pointing at deleted text.
pub async fn index_meeting(
    conn: &mut Connection,
    meeting_id: &str,
    embedder: &impl Embedder,
    params: &LinkParams,
) -> Result<IndexReport, DbError> {
    let utterances = repo::meeting_utterances(conn, meeting_id)?;
    let notes = repo::meeting_notes(conn, meeting_id)?;

    if notes.is_empty() || utterances.is_empty() {
        // Nothing to link. Still clear any links left over from a previous run
        // whose notes have since been deleted.
        let written = repo::replace_note_links(conn, meeting_id, &[])?;
        return Ok(IndexReport {
            embedded: 0,
            links: written,
            degraded: None,
        });
    }

    let utterance_items: Vec<(String, String)> = utterances
        .iter()
        .map(|u| (u.id.to_string(), u.text.clone()))
        .collect();
    let note_items: Vec<(String, String)> = notes
        .iter()
        .map(|n| (note_owner_id(meeting_id, &n.block_id), n.text.clone()))
        .collect();

    let mut degraded = None;
    let mut embedded = 0usize;
    let mut utterance_vectors = std::collections::HashMap::new();
    let mut note_vectors = std::collections::HashMap::new();

    match ensure_embedded(conn, "utterance", &utterance_items, embedder).await {
        Ok((map, count)) => {
            utterance_vectors = map;
            embedded += count;
        }
        Err(err) => degraded = Some(err),
    }

    if degraded.is_none() {
        match ensure_embedded(conn, "note_block", &note_items, embedder).await {
            Ok((map, count)) => {
                note_vectors = map;
                embedded += count;
            }
            Err(err) => degraded = Some(err),
        }
    }

    // On a partial failure, drop the semantic layer entirely rather than linking
    // some notes semantically and others temporally — a silently mixed basis is
    // the kind of thing that makes tuning impossible to reason about later.
    if degraded.is_some() {
        utterance_vectors.clear();
        note_vectors.clear();
    }

    let note_inputs: Vec<NoteInput> = notes
        .iter()
        .map(|n| NoteInput {
            block_id: n.block_id.clone(),
            first_typed_at_ms: n.first_typed_at_ms,
            text: n.text.clone(),
            embedding: note_vectors
                .get(&note_owner_id(meeting_id, &n.block_id))
                .cloned(),
        })
        .collect();

    let utterance_inputs: Vec<UtteranceInput> = utterances
        .iter()
        .map(|u| UtteranceInput {
            id: u.id,
            start_ms: u.start_ms,
            end_ms: u.end_ms,
            text: u.text.clone(),
            embedding: utterance_vectors.get(&u.id.to_string()).cloned(),
        })
        .collect();

    let links = link_layered(&note_inputs, &utterance_inputs, params);
    let written = repo::replace_note_links(conn, meeting_id, &to_rows(&links))?;

    Ok(IndexReport {
        embedded,
        links: written,
        degraded,
    })
}

fn to_rows(links: &[Link]) -> Vec<(String, i64, String, f64)> {
    links
        .iter()
        .map(|l| {
            (
                l.note_block_id.clone(),
                l.utterance_id,
                l.method.as_str().to_string(),
                l.score as f64,
            )
        })
        .collect()
}

/// Re-indexes every meeting that has notes but no links.
///
/// Needed because embeddings arrived in Phase 4: meetings recorded before this
/// existed have transcripts and notes but nothing joining them, and migration
/// 0004 dropped whatever vectors did exist. Returns per-meeting reports.
pub async fn backfill(
    conn: &mut Connection,
    embedder: &impl Embedder,
    params: &LinkParams,
    limit: i64,
) -> Result<Vec<(String, IndexReport)>, DbError> {
    let ids = unlinked_meetings(conn, limit)?;
    let mut out = Vec::new();
    for id in ids {
        let report = index_meeting(conn, &id, embedder, params).await?;
        out.push((id, report));
    }
    Ok(out)
}

fn unlinked_meetings(conn: &Connection, limit: i64) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT b.meeting_id
         FROM note_blocks b
         WHERE NOT EXISTS (
             SELECT 1 FROM note_links l WHERE l.note_block_id = b.id
         )
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![limit], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::embed::{EmbedError, HashEmbedder};

    /// An embedder that is always down, to prove linking survives it.
    struct DeadEmbedder;

    impl Embedder for DeadEmbedder {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Err(EmbedError::Unreachable {
                url: "http://localhost:11434/v1".into(),
                detail: "connection refused".into(),
            })
        }
    }

    /// Counts calls, to prove already-embedded rows are not re-embedded.
    struct CountingEmbedder {
        calls: std::cell::Cell<usize>,
        texts: std::cell::Cell<usize>,
    }

    impl CountingEmbedder {
        fn new() -> Self {
            Self {
                calls: std::cell::Cell::new(0),
                texts: std::cell::Cell::new(0),
            }
        }
    }

    impl Embedder for CountingEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.calls.set(self.calls.get() + 1);
            self.texts.set(self.texts.get() + texts.len());
            Ok(texts.iter().map(|t| HashEmbedder::vector(t)).collect())
        }
    }

    // Safety: the tests are single-threaded and never send these across threads;
    // the Cell counters are what make the auto-impl fail.
    unsafe impl Sync for CountingEmbedder {}

    fn seeded() -> Database {
        let mut db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();

        for (seq, start, text) in [
            (1i64, 2_000i64, "morning everyone lets get started"),
            (2, 18_000, "the deadline for the migration is thursday"),
            (3, 40_000, "who is bringing lunch tomorrow"),
        ] {
            repo::insert_utterance(conn, "m1", seq, "system", text, start, start + 3_000, None)
                .unwrap();
        }

        repo::save_note_blocks(
            db.connection_mut(),
            "m1",
            &[repo::NoteBlock {
                block_id: "b1".into(),
                seq: 1,
                text: "deadline migration".into(),
                first_typed_at_ms: Some(21_000),
                last_edited_at_ms: Some(21_000),
            }],
        )
        .unwrap();
        db
    }

    #[tokio::test]
    async fn indexing_embeds_and_links_a_meeting() {
        let mut db = seeded();
        let report = index_meeting(
            db.connection_mut(),
            "m1",
            &HashEmbedder,
            &LinkParams::default(),
        )
        .await
        .unwrap();

        // 3 utterances + 1 note.
        assert_eq!(report.embedded, 4);
        assert!(report.links >= 1, "no links written");
        assert!(report.degraded.is_none());

        let links = repo::meeting_links(db.connection(), "m1").unwrap();
        let top = links.first().expect("expected a link");
        assert_eq!(top.note_block_id, "b1");
        // The note is about the migration deadline, not about lunch.
        assert_eq!(top.utterance_id, 2);
    }

    #[tokio::test]
    async fn a_second_run_reuses_the_vectors_it_already_has() {
        // Re-embedding an unchanged hour of transcript on every open is the
        // difference between the feature being usable and not.
        let mut db = seeded();
        let embedder = CountingEmbedder::new();

        index_meeting(db.connection_mut(), "m1", &embedder, &LinkParams::default())
            .await
            .unwrap();
        let after_first = embedder.texts.get();
        assert_eq!(after_first, 4);

        let report = index_meeting(db.connection_mut(), "m1", &embedder, &LinkParams::default())
            .await
            .unwrap();

        assert_eq!(embedder.texts.get(), after_first, "re-embedded known text");
        assert_eq!(report.embedded, 0);
    }

    #[tokio::test]
    async fn linking_still_works_with_no_embedder_running() {
        // The whole point of the temporal layer being the baseline.
        let mut db = seeded();
        let report = index_meeting(
            db.connection_mut(),
            "m1",
            &DeadEmbedder,
            &LinkParams::default(),
        )
        .await
        .unwrap();

        assert!(report.degraded.is_some(), "should have reported degrading");
        assert!(
            report.degraded.as_deref().unwrap().contains("11434"),
            "the reason should say what it could not reach: {:?}",
            report.degraded
        );
        assert!(report.links >= 1, "fell back to nothing at all");
    }

    #[tokio::test]
    async fn relinking_drops_links_whose_notes_are_gone() {
        let mut db = seeded();
        index_meeting(
            db.connection_mut(),
            "m1",
            &HashEmbedder,
            &LinkParams::default(),
        )
        .await
        .unwrap();
        assert!(!repo::meeting_links(db.connection(), "m1")
            .unwrap()
            .is_empty());

        // The user deletes the note.
        repo::save_note_blocks(db.connection_mut(), "m1", &[]).unwrap();
        let report = index_meeting(
            db.connection_mut(),
            "m1",
            &HashEmbedder,
            &LinkParams::default(),
        )
        .await
        .unwrap();

        assert_eq!(report.links, 0);
        assert!(repo::meeting_links(db.connection(), "m1")
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_link_naming_an_unknown_block_is_skipped_not_fatal() {
        // LLM citations are the one input that can name anything at all.
        let mut db = seeded();
        let written = repo::replace_note_links(
            db.connection_mut(),
            "m1",
            &[
                ("b1".into(), 2, "temporal".into(), 0.9),
                ("does-not-exist".into(), 2, "llm".into(), 0.9),
            ],
        )
        .unwrap();
        assert_eq!(written, 1, "the unknown block should have been skipped");
    }

    /// G16's done-when: an hour of transcript embeds in well under 30 seconds.
    ///
    /// Ignored by default — it needs a real embedding model running, which CI
    /// does not have. `HashEmbedder` would make this pass instantly and prove
    /// nothing, so this deliberately talks to the actual model:
    ///
    /// ```text
    /// ollama pull nomic-embed-text:v1.5
    /// cargo test --lib an_hour_of_transcript_embeds_within_the_budget -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn an_hour_of_transcript_embeds_within_the_budget() {
        use crate::embed::HttpEmbedder;
        use std::time::Instant;

        // An hour of talking at a realistic utterance rate: roughly one line
        // every five seconds, so 720 lines.
        const LINES: i64 = 720;

        let mut db = Database::open_in_memory().unwrap();
        repo::insert_meeting(db.connection(), "long", "Hour long", 0).unwrap();
        for seq in 0..LINES {
            let start = seq * 5_000;
            repo::insert_utterance(
                db.connection(),
                "long",
                seq,
                if seq % 2 == 0 { "system" } else { "mic" },
                &format!(
                    "so the point I want to make about item number {seq} is that we \
                     should probably revisit the estimate before we commit to it"
                ),
                start,
                start + 4_000,
                None,
            )
            .unwrap();
        }
        repo::save_note_blocks(
            db.connection_mut(),
            "long",
            &[repo::NoteBlock {
                block_id: "b1".into(),
                seq: 1,
                text: "revisit the estimate".into(),
                first_typed_at_ms: Some(60_000),
                last_edited_at_ms: Some(60_000),
            }],
        )
        .unwrap();

        let started = Instant::now();
        let report = index_meeting(
            db.connection_mut(),
            "long",
            &HttpEmbedder::local(),
            &LinkParams::default(),
        )
        .await
        .unwrap();
        let elapsed = started.elapsed();

        assert!(
            report.degraded.is_none(),
            "no embedding model reachable — start one first: {:?}",
            report.degraded
        );
        eprintln!(
            "embedded {} texts and wrote {} links in {:.1}s",
            report.embedded,
            report.links,
            elapsed.as_secs_f32()
        );
        assert_eq!(report.embedded as i64, LINES + 1);
        assert!(
            elapsed.as_secs() < 30,
            "an hour of transcript took {:.1}s to embed, budget is 30s",
            elapsed.as_secs_f32()
        );
    }

    #[tokio::test]
    async fn deleting_a_meeting_takes_its_vectors_with_it() {
        // `embeddings` is a virtual table with no foreign key, so nothing
        // cascades into it. Orphan vectors would keep turning up in every
        // nearest-neighbour search, pointing at rows that no longer exist.
        let mut db = seeded();
        index_meeting(
            db.connection_mut(),
            "m1",
            &HashEmbedder,
            &LinkParams::default(),
        )
        .await
        .unwrap();

        let count = |db: &Database| -> i64 {
            db.connection()
                .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&db), 4);

        repo::delete_meeting(db.connection(), "m1").unwrap();
        assert_eq!(count(&db), 0, "vectors outlived the meeting they described");
    }

    #[tokio::test]
    async fn backfill_only_touches_meetings_that_have_no_links() {
        let mut db = seeded();
        repo::insert_meeting(db.connection(), "m2", "Other", 0).unwrap();

        let done = backfill(
            db.connection_mut(),
            &HashEmbedder,
            &LinkParams::default(),
            10,
        )
        .await
        .unwrap();
        assert_eq!(done.len(), 1, "m2 has no notes and should not be picked up");
        assert_eq!(done[0].0, "m1");

        // Now that m1 is linked it should not come back round again.
        let second = backfill(
            db.connection_mut(),
            &HashEmbedder,
            &LinkParams::default(),
            10,
        )
        .await
        .unwrap();
        assert!(second.is_empty(), "backfill re-did work already done");
    }
}

#[cfg(test)]
mod seed_tools {
    //! A one-off tool, not a test.
    //!
    //! Indexes meetings in a real database so the hover reveal (G35) can be
    //! judged against real language. It lives here because `index_meeting` is
    //! the thing being run and this is the crate that owns it.
    //!
    //! ```text
    //! OATMEAL_DB=~/Library/Application\ Support/com.kaichen.oatmeal/oatmeal.sqlite \
    //!   cargo test --lib seed_tools::index_seeded -- --ignored --nocapture
    //! ```
    use super::*;
    use crate::db::Database;
    use crate::embed::HttpEmbedder;

    #[tokio::test]
    #[ignore]
    async fn index_seeded() {
        let path = std::env::var("OATMEAL_DB").expect("set OATMEAL_DB");
        let mut db = Database::open(path.as_str()).expect("open database");
        let embedder = HttpEmbedder::local();
        let params = LinkParams::default();

        let ids: Vec<String> = {
            let conn = db.connection();
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM meetings WHERE title LIKE '%[seeded]%' ORDER BY started_at",
                )
                .unwrap();
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            rows
        };
        assert!(!ids.is_empty(), "nothing seeded to index");

        for id in ids {
            let started = std::time::Instant::now();
            let report = index_meeting(db.connection_mut(), &id, &embedder, &params)
                .await
                .expect("index");
            eprintln!(
                "{id}: {} embedded, {} links, {:.1}s",
                report.embedded,
                report.links,
                started.elapsed().as_secs_f32()
            );
        }
    }
}
