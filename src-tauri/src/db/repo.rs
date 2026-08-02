//! Queries over the schema in `migrations/`.
//!
//! Deliberately free functions taking `&Connection` rather than methods on
//! `Database`, so they compose inside a transaction the caller owns.

use rusqlite::{params, Connection};
use serde::Serialize;

use super::{DbError, Result, EMBEDDING_DIM};

// ------------------------------------------------------------------- writes

pub fn insert_meeting(conn: &Connection, id: &str, title: &str, started_at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO meetings (id, title, started_at, status, trigger_source)
         VALUES (?1, ?2, ?3, 'recording', 'manual')",
        params![id, title, started_at],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn insert_utterance(
    conn: &Connection,
    meeting_id: &str,
    seq: i64,
    source: &str,
    text: &str,
    start_ms: i64,
    end_ms: i64,
    confidence: Option<f64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO utterances
             (meeting_id, seq, source, text, start_ms, end_ms, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![meeting_id, seq, source, text, start_ms, end_ms, confidence],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_note_block(
    conn: &Connection,
    meeting_id: &str,
    seq: i64,
    text: &str,
    first_typed_at_ms: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO note_blocks
             (meeting_id, seq, text, first_typed_at_ms, last_edited_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![meeting_id, seq, text, first_typed_at_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Next sequence number for a meeting's transcript.
///
/// Derived from the table rather than tracked in memory so a crash mid-recording
/// can't restart numbering and collide with rows already written.
pub fn next_utterance_seq(conn: &Connection, meeting_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq) + 1, 0) FROM utterances WHERE meeting_id = ?1",
        params![meeting_id],
        |row| row.get(0),
    )?)
}

/// Appends an utterance, allocating its sequence number.
pub fn append_utterance(
    conn: &Connection,
    meeting_id: &str,
    source: &str,
    text: &str,
    start_ms: i64,
    end_ms: i64,
    confidence: Option<f64>,
) -> Result<i64> {
    let seq = next_utterance_seq(conn, meeting_id)?;
    insert_utterance(
        conn, meeting_id, seq, source, text, start_ms, end_ms, confidence,
    )
}

/// Marks a meeting complete and records where its audio landed.
pub fn finish_meeting(
    conn: &Connection,
    meeting_id: &str,
    ended_at: i64,
    audio_path: Option<&str>,
    audio_expires_at: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE meetings
            SET ended_at = ?2, status = 'complete',
                audio_path = ?3, audio_expires_at = ?4
          WHERE id = ?1",
        params![meeting_id, ended_at, audio_path, audio_expires_at],
    )?;
    Ok(())
}

/// Closes out meetings left mid-recording by a crash or a quit.
///
/// `active_meeting` lives in memory and dies with the process, but the row does
/// not — so without this a killed app leaves a meeting stuck in `recording`
/// forever. It blocks the next recording ("already recording"), and it lies to
/// the user about what happened.
///
/// The transcript captured up to the crash is kept; only the status changes.
/// Returns how many were recovered.
pub fn recover_interrupted_meetings(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE meetings
            SET status = 'interrupted', ended_at = COALESCE(ended_at, ?1)
          WHERE status = 'recording'",
        params![now],
    )?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub audio_path: Option<String>,
    pub utterance_count: i64,
}

pub fn list_meetings(conn: &Connection, limit: i64) -> Result<Vec<MeetingSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.title, m.started_at, m.ended_at, m.status, m.audio_path,
                (SELECT COUNT(*) FROM utterances u WHERE u.meeting_id = m.id)
         FROM meetings m
         ORDER BY m.started_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(MeetingSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            started_at: row.get(2)?,
            ended_at: row.get(3)?,
            status: row.get(4)?,
            audio_path: row.get(5)?,
            utterance_count: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Utterance {
    pub id: i64,
    pub seq: i64,
    pub source: String,
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: Option<f64>,
}

pub fn meeting_utterances(conn: &Connection, meeting_id: &str) -> Result<Vec<Utterance>> {
    let mut stmt = conn.prepare(
        "SELECT id, seq, source, text, start_ms, end_ms, confidence
         FROM utterances WHERE meeting_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(Utterance {
            id: row.get(0)?,
            seq: row.get(1)?,
            source: row.get(2)?,
            text: row.get(3)?,
            start_ms: row.get(4)?,
            end_ms: row.get(5)?,
            confidence: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

// -------------------------------------------------------------- full text

#[derive(Debug, Clone, Serialize)]
pub struct TextHit {
    pub id: i64,
    pub meeting_id: String,
    pub text: String,
    /// FTS5 rank; more negative is a better match.
    pub rank: f64,
}

pub fn search_utterances(conn: &Connection, query: &str, limit: i64) -> Result<Vec<TextHit>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.meeting_id, u.text, f.rank
         FROM utterances_fts f
         JOIN utterances u ON u.id = f.rowid
         WHERE utterances_fts MATCH ?1
         ORDER BY f.rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |row| {
        Ok(TextHit {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            text: row.get(2)?,
            rank: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn search_note_blocks(conn: &Connection, query: &str, limit: i64) -> Result<Vec<TextHit>> {
    let mut stmt = conn.prepare(
        "SELECT n.id, n.meeting_id, n.text, f.rank
         FROM note_blocks_fts f
         JOIN note_blocks n ON n.id = f.rowid
         WHERE note_blocks_fts MATCH ?1
         ORDER BY f.rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |row| {
        Ok(TextHit {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            text: row.get(2)?,
            rank: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

// ---------------------------------------------------------------- vectors

fn encode(vector: &[f32]) -> Result<Vec<u8>> {
    if vector.len() != EMBEDDING_DIM {
        return Err(DbError::EmbeddingDimension {
            expected: EMBEDDING_DIM,
            actual: vector.len(),
        });
    }
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

pub fn insert_embedding(
    conn: &Connection,
    owner_type: &str,
    owner_id: &str,
    vector: &[f32],
) -> Result<()> {
    conn.execute(
        "INSERT INTO embeddings (owner_type, owner_id, embedding)
         VALUES (?1, ?2, ?3)",
        params![owner_type, owner_id, encode(vector)?],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorHit {
    pub owner_type: String,
    pub owner_id: String,
    pub distance: f64,
}

pub fn nearest_embeddings(conn: &Connection, vector: &[f32], k: i64) -> Result<Vec<VectorHit>> {
    let mut stmt = conn.prepare(
        "SELECT owner_type, owner_id, distance
         FROM embeddings
         WHERE embedding MATCH ?1 AND k = ?2
         ORDER BY distance",
    )?;
    let rows = stmt.query_map(params![encode(vector)?, k], |row| {
        Ok(VectorHit {
            owner_type: row.get(0)?,
            owner_id: row.get(1)?,
            distance: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

// ------------------------------------------------------------------ stats

/// Surfaced by the `db_status` command so the Phase 0 harness can show that the
/// data layer is real rather than merely compiled.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbStats {
    pub schema_version: i32,
    pub meetings: i64,
    pub utterances: i64,
    pub note_blocks: i64,
    pub panels: i64,
    pub embeddings: i64,
}

pub fn stats(conn: &Connection) -> Result<DbStats> {
    let count = |table: &str| -> Result<i64> {
        Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    };

    Ok(DbStats {
        schema_version: conn.query_row("PRAGMA user_version", [], |r| r.get(0))?,
        meetings: count("meetings")?,
        utterances: count("utterances")?,
        note_blocks: count("note_blocks")?,
        panels: count("panels")?,
        embeddings: count("embeddings")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn seeded() -> Database {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();

        insert_meeting(conn, "m1", "Quarterly planning", 1_700_000_000_000).unwrap();
        insert_utterance(
            conn,
            "m1",
            0,
            "system",
            "So the deadline for the migration is the fourteenth.",
            12_400,
            14_700,
            Some(0.93),
        )
        .unwrap();
        insert_utterance(
            conn,
            "m1",
            1,
            "mic",
            "Got it, I'll own the rollback plan.",
            15_000,
            17_200,
            Some(0.88),
        )
        .unwrap();
        insert_note_block(conn, "m1", 0, "deadline = 14th", Some(16_000)).unwrap();

        db
    }

    #[test]
    fn append_allocates_sequential_numbers() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "Test", 0).unwrap();

        append_utterance(conn, "m1", "mic", "one", 0, 100, None).unwrap();
        append_utterance(conn, "m1", "system", "two", 100, 200, None).unwrap();
        append_utterance(conn, "m1", "mic", "three", 200, 300, None).unwrap();

        let seqs: Vec<i64> = meeting_utterances(conn, "m1")
            .unwrap()
            .iter()
            .map(|u| u.seq)
            .collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn sequence_numbering_survives_a_restart() {
        // `seq` is derived from the table, not held in memory, so a crash
        // mid-recording cannot restart numbering and collide with existing rows.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oatmeal.sqlite");
        {
            let db = Database::open(&path).unwrap();
            insert_meeting(db.connection(), "m1", "Test", 0).unwrap();
            append_utterance(db.connection(), "m1", "mic", "before", 0, 100, None).unwrap();
        }
        let db = Database::open(&path).unwrap();
        append_utterance(db.connection(), "m1", "mic", "after", 100, 200, None).unwrap();

        let utterances = meeting_utterances(db.connection(), "m1").unwrap();
        assert_eq!(utterances.len(), 2);
        assert_eq!(
            utterances[1].seq, 1,
            "numbering restarted and would collide"
        );
    }

    #[test]
    fn sequences_are_per_meeting_not_global() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "One", 0).unwrap();
        insert_meeting(conn, "m2", "Two", 0).unwrap();

        append_utterance(conn, "m1", "mic", "a", 0, 1, None).unwrap();
        append_utterance(conn, "m2", "mic", "b", 0, 1, None).unwrap();

        assert_eq!(meeting_utterances(conn, "m2").unwrap()[0].seq, 0);
    }

    #[test]
    fn finishing_a_meeting_records_where_the_audio_landed() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "Test", 1_000).unwrap();
        finish_meeting(conn, "m1", 5_000, Some("/tmp/a.m4a"), Some(9_000)).unwrap();

        let meeting = &list_meetings(conn, 10).unwrap()[0];
        assert_eq!(meeting.status, "complete");
        assert_eq!(meeting.ended_at, Some(5_000));
        assert_eq!(meeting.audio_path.as_deref(), Some("/tmp/a.m4a"));
    }

    #[test]
    fn startup_recovers_meetings_left_mid_recording() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "crashed", "Interrupted", 1_000).unwrap();
        append_utterance(conn, "crashed", "mic", "said before the crash", 0, 1, None).unwrap();

        let recovered = recover_interrupted_meetings(conn, 9_000).unwrap();
        assert_eq!(recovered, 1);

        let meeting = &list_meetings(conn, 10).unwrap()[0];
        assert_eq!(meeting.status, "interrupted");
        assert_eq!(meeting.ended_at, Some(9_000));
        // Whatever was captured before the crash must survive; only the status
        // is a lie that needs correcting.
        assert_eq!(meeting.utterance_count, 1);
    }

    #[test]
    fn recovery_leaves_completed_meetings_alone() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "done", "Finished", 1_000).unwrap();
        finish_meeting(conn, "done", 2_000, Some("/tmp/a.m4a"), None).unwrap();

        assert_eq!(recover_interrupted_meetings(conn, 9_000).unwrap(), 0);
        let meeting = &list_meetings(conn, 10).unwrap()[0];
        assert_eq!(meeting.status, "complete");
        assert_eq!(
            meeting.ended_at,
            Some(2_000),
            "recovery rewrote a real end time"
        );
    }

    #[test]
    fn recovery_is_idempotent_across_restarts() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "crashed", "Interrupted", 1_000).unwrap();

        assert_eq!(recover_interrupted_meetings(conn, 9_000).unwrap(), 1);
        // A second launch must not keep "recovering" the same meeting and
        // pushing its end time later each time.
        assert_eq!(recover_interrupted_meetings(conn, 20_000).unwrap(), 0);
        assert_eq!(list_meetings(conn, 10).unwrap()[0].ended_at, Some(9_000));
    }

    #[test]
    fn meetings_list_newest_first_with_counts() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "old", "Older", 1_000).unwrap();
        insert_meeting(conn, "new", "Newer", 2_000).unwrap();
        append_utterance(conn, "new", "mic", "hello", 0, 1, None).unwrap();

        let meetings = list_meetings(conn, 10).unwrap();
        assert_eq!(meetings[0].id, "new");
        assert_eq!(meetings[0].utterance_count, 1);
        assert_eq!(meetings[1].utterance_count, 0);
    }

    #[test]
    fn utterances_come_back_in_order_with_attribution() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "Test", 0).unwrap();
        append_utterance(
            conn,
            "m1",
            "system",
            "Deadline is Thursday.",
            0,
            2_000,
            Some(0.9),
        )
        .unwrap();
        append_utterance(conn, "m1", "mic", "Got it.", 2_000, 3_000, Some(0.8)).unwrap();

        let utterances = meeting_utterances(conn, "m1").unwrap();
        assert_eq!(utterances.len(), 2);
        // Attribution is the whole reason for two capture streams; it has to
        // survive the round trip through storage intact.
        assert_eq!(utterances[0].source, "system");
        assert_eq!(utterances[1].source, "mic");
        assert_eq!(utterances[0].confidence, Some(0.9));
    }

    #[test]
    fn round_trips_a_meeting_with_transcript_and_notes() {
        let db = seeded();
        let s = stats(db.connection()).unwrap();
        assert_eq!(s.meetings, 1);
        assert_eq!(s.utterances, 2);
        assert_eq!(s.note_blocks, 1);
    }

    #[test]
    fn full_text_search_finds_utterances() {
        let db = seeded();
        let hits = search_utterances(db.connection(), "deadline", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("deadline"));
        assert_eq!(hits[0].meeting_id, "m1");
    }

    #[test]
    fn full_text_search_stems_via_the_porter_tokenizer() {
        let db = seeded();
        // "migration" is in the transcript; searching "migrate" must still hit,
        // otherwise recall on real queries will be poor.
        let hits = search_utterances(db.connection(), "migrate", 10).unwrap();
        assert_eq!(hits.len(), 1, "porter stemming is not active");
    }

    #[test]
    fn full_text_search_finds_note_blocks() {
        let db = seeded();
        let hits = search_note_blocks(db.connection(), "deadline", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn fts_index_follows_deletes() {
        let db = seeded();
        db.connection()
            .execute("DELETE FROM utterances WHERE seq = 0", [])
            .unwrap();
        let hits = search_utterances(db.connection(), "deadline", 10).unwrap();
        assert!(
            hits.is_empty(),
            "FTS index kept a row the base table dropped"
        );
    }

    #[test]
    fn fts_index_follows_updates() {
        let db = seeded();
        db.connection()
            .execute(
                "UPDATE utterances SET text = 'completely different content' WHERE seq = 0",
                [],
            )
            .unwrap();
        assert!(search_utterances(db.connection(), "deadline", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            search_utterances(db.connection(), "different", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn vector_search_returns_the_nearest_owner() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();

        let mut near = vec![0.0f32; EMBEDDING_DIM];
        near[0] = 1.0;
        let mut far = vec![0.0f32; EMBEDDING_DIM];
        far[1] = 1.0;

        insert_embedding(conn, "utterance", "1", &near).unwrap();
        insert_embedding(conn, "utterance", "2", &far).unwrap();

        let hits = nearest_embeddings(conn, &near, 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].owner_id, "1", "nearest neighbour ordering is wrong");
        assert!(hits[0].distance < hits[1].distance);
        assert_eq!(hits[0].owner_type, "utterance");
    }

    #[test]
    fn wrong_dimension_vectors_are_rejected_before_reaching_sqlite() {
        let db = Database::open_in_memory().unwrap();
        let err = insert_embedding(db.connection(), "utterance", "1", &[0.0; 8]);
        assert!(matches!(err, Err(DbError::EmbeddingDimension { .. })));
    }

    #[test]
    fn deleting_a_meeting_cascades_to_its_transcript_and_notes() {
        let db = seeded();
        db.connection()
            .execute("DELETE FROM meetings WHERE id = 'm1'", [])
            .unwrap();
        let s = stats(db.connection()).unwrap();
        assert_eq!(s.utterances, 0);
        assert_eq!(s.note_blocks, 0);
    }

    #[test]
    fn panels_are_regenerable_without_touching_the_transcript() {
        let db = seeded();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO panels (id, meeting_id, content_json, plaintext, generated_at)
             VALUES ('p1', 'm1', '{}', 'deadline is the 14th', 0)",
            [],
        )
        .unwrap();

        // Regenerating means dropping the panel and writing a new one. The
        // transcript and notes must be untouched by that.
        conn.execute("DELETE FROM panels WHERE id = 'p1'", [])
            .unwrap();

        let s = stats(conn).unwrap();
        assert_eq!(s.panels, 0);
        assert_eq!(
            s.utterances, 2,
            "regenerating a panel damaged the transcript"
        );
        assert_eq!(s.note_blocks, 1, "regenerating a panel damaged the notes");
    }
}
