//! Queries over the schema in `migrations/`.
//!
//! Deliberately free functions taking `&Connection` rather than methods on
//! `Database`, so they compose inside a transaction the caller owns.

use rusqlite::{params, Connection, OptionalExtension};
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

#[allow(clippy::too_many_arguments)]
pub fn insert_note_block(
    conn: &Connection,
    meeting_id: &str,
    block_id: &str,
    seq: i64,
    text: &str,
    first_typed_at_ms: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO note_blocks
             (meeting_id, block_id, seq, text, first_typed_at_ms, last_edited_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![meeting_id, block_id, seq, text, first_typed_at_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

/// One block of the notepad, as the editor knows it.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteBlock {
    /// Editor-assigned, stable for the life of the block.
    pub block_id: String,
    pub seq: i64,
    pub text: String,
    /// Milliseconds from meeting start to the first keystroke in this block.
    pub first_typed_at_ms: Option<i64>,
    pub last_edited_at_ms: Option<i64>,
}

/// Replaces a meeting's notes with `blocks`, preserving each block's identity.
///
/// The rule this exists to enforce: **`first_typed_at_ms` is written once and
/// never rewritten.** It is the anchor the temporal linker keys on (SPEC
/// section 7), so letting an edit move it would silently re-point a note at a
/// different moment in the transcript. Text, order and last-edited all update
/// freely; the anchor does not.
///
/// Runs in one transaction so an autosave interrupted half-way cannot leave the
/// notepad partially rewritten.
pub fn save_note_blocks(
    conn: &mut Connection,
    meeting_id: &str,
    blocks: &[NoteBlock],
) -> Result<()> {
    let tx = conn.transaction()?;

    {
        // Anything the editor no longer has was deleted by the user.
        let keep: Vec<String> = blocks.iter().map(|b| b.block_id.clone()).collect();
        let mut stmt = tx.prepare("SELECT block_id FROM note_blocks WHERE meeting_id = ?1")?;
        let existing: Vec<String> = stmt
            .query_map(params![meeting_id], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);

        for gone in existing.iter().filter(|id| !keep.contains(id)) {
            tx.execute(
                "DELETE FROM note_blocks WHERE meeting_id = ?1 AND block_id = ?2",
                params![meeting_id, gone],
            )?;
        }

        for block in blocks {
            // `excluded.first_typed_at_ms` is deliberately absent from the SET
            // clause — see the doc comment.
            tx.execute(
                "INSERT INTO note_blocks
                     (meeting_id, block_id, seq, text,
                      first_typed_at_ms, last_edited_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (meeting_id, block_id) DO UPDATE SET
                     seq = excluded.seq,
                     text = excluded.text,
                     last_edited_at_ms = excluded.last_edited_at_ms",
                params![
                    meeting_id,
                    block.block_id,
                    block.seq,
                    block.text,
                    block.first_typed_at_ms,
                    block.last_edited_at_ms
                ],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn meeting_notes(conn: &Connection, meeting_id: &str) -> Result<Vec<NoteBlock>> {
    let mut stmt = conn.prepare(
        "SELECT block_id, seq, text, first_typed_at_ms, last_edited_at_ms
         FROM note_blocks WHERE meeting_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(NoteBlock {
            block_id: row.get(0)?,
            seq: row.get(1)?,
            text: row.get(2)?,
            first_typed_at_ms: row.get(3)?,
            last_edited_at_ms: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
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

pub fn rename_meeting(conn: &Connection, meeting_id: &str, title: &str) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET title = ?2 WHERE id = ?1",
        params![meeting_id, title],
    )?;
    Ok(())
}

/// Deletes a meeting and everything hanging off it.
///
/// Returns the audio path, if any, so the caller can remove the file — the
/// cascade clears the rows but knows nothing about the filesystem, and an
/// orphaned recording is exactly the data a user asked to be rid of.
pub fn delete_meeting(conn: &Connection, meeting_id: &str) -> Result<Option<String>> {
    let audio_path: Option<String> = conn
        .query_row(
            "SELECT audio_path FROM meetings WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    // `embeddings` is a `vec0` virtual table, so it carries no foreign key and
    // the cascade that clears utterances and note blocks cannot reach it. Its
    // rows have to go first, while the ids that name them still exist —
    // afterwards there is no way to tell which vectors belonged to this meeting.
    conn.execute(
        "DELETE FROM embeddings WHERE owner_type = 'utterance' AND owner_id IN (
             SELECT CAST(id AS TEXT) FROM utterances WHERE meeting_id = ?1
         )",
        params![meeting_id],
    )?;
    conn.execute(
        "DELETE FROM embeddings WHERE owner_type = 'note_block' AND owner_id IN (
             SELECT ?1 || ':' || block_id FROM note_blocks WHERE meeting_id = ?1
         )",
        params![meeting_id],
    )?;

    conn.execute("DELETE FROM meetings WHERE id = ?1", params![meeting_id])?;
    Ok(audio_path)
}

/// Ensures the built-in templates exist as rows.
///
/// `panels.template_id` is a foreign key, so a panel cannot reference a
/// template that has no row. The prompt text stays authoritative in code and is
/// refreshed here on every launch, so improving a built-in prompt does not
/// require a migration — but user-defined templates still live in the same
/// table and are never touched.
pub fn ensure_builtin_templates(
    conn: &Connection,
    templates: &[(&str, &str, &str)],
    now: i64,
) -> Result<()> {
    for (id, name, prompt) in templates {
        conn.execute(
            "INSERT INTO templates (id, name, prompt, is_builtin, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT (id) DO UPDATE SET
                 name = excluded.name,
                 prompt = excluded.prompt
             WHERE templates.is_builtin = 1",
            params![id, name, prompt, now],
        )?;
    }
    Ok(())
}

/// A generated view over a meeting. Regenerating adds a new one rather than
/// replacing the old — per the G15 decision gate, edits are never destroyed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Panel {
    pub id: String,
    pub template_id: Option<String>,
    pub content_json: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub generated_at: i64,
}

#[allow(clippy::too_many_arguments)]
pub fn insert_panel(
    conn: &Connection,
    id: &str,
    meeting_id: &str,
    template_id: &str,
    content_json: &str,
    plaintext: &str,
    provider: &str,
    model: &str,
    generated_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO panels
             (id, meeting_id, template_id, content_json, plaintext,
              provider, model, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            meeting_id,
            template_id,
            content_json,
            plaintext,
            provider,
            model,
            generated_at
        ],
    )?;
    Ok(())
}

/// Panels for a meeting, newest first.
pub fn meeting_panels(conn: &Connection, meeting_id: &str) -> Result<Vec<Panel>> {
    let mut stmt = conn.prepare(
        "SELECT id, template_id, content_json, provider, model, generated_at
         FROM panels WHERE meeting_id = ?1 ORDER BY generated_at DESC, rowid DESC",
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(Panel {
            id: row.get(0)?,
            template_id: row.get(1)?,
            content_json: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            generated_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn delete_panel(conn: &Connection, panel_id: &str) -> Result<()> {
    conn.execute("DELETE FROM panels WHERE id = ?1", params![panel_id])?;
    Ok(())
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

/// Writes an embedding, replacing any existing one for the same owner.
///
/// Re-embedding happens whenever a transcript or note is revised, and `vec0`
/// has no upsert — a plain insert would silently accumulate duplicate vectors
/// for one owner and skew every nearest-neighbour query afterwards.
pub fn replace_embedding(
    conn: &Connection,
    owner_type: &str,
    owner_id: &str,
    vector: &[f32],
) -> Result<()> {
    // Encode first: on a width mismatch this leaves the existing vector alone
    // rather than deleting it and failing to write a replacement.
    let blob = encode(vector)?;
    conn.execute(
        "DELETE FROM embeddings WHERE owner_type = ?1 AND owner_id = ?2",
        params![owner_type, owner_id],
    )?;
    conn.execute(
        "INSERT INTO embeddings (owner_type, owner_id, embedding) VALUES (?1, ?2, ?3)",
        params![owner_type, owner_id, blob],
    )?;
    Ok(())
}

fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Reads back stored vectors for specific owners, keyed by `owner_id`.
///
/// The linker needs the vectors themselves, not nearest-neighbour results — it
/// scores every in-window candidate against one note rather than asking the
/// index for a global top-k. Scoped to the ids asked for, because one meeting's
/// linking pass has no use for every vector in the library.
pub fn embeddings_for(
    conn: &Connection,
    owner_type: &str,
    owner_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<f32>>> {
    let mut out = std::collections::HashMap::new();
    if owner_ids.is_empty() {
        return Ok(out);
    }

    // Chunked because SQLite caps variables per statement (999 by default) and a
    // long meeting has more utterances than that.
    for chunk in owner_ids.chunks(500) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT owner_id, embedding FROM embeddings
             WHERE owner_type = ? AND owner_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
        values.push(&owner_type);
        for id in chunk {
            values.push(id);
        }
        let rows = stmt.query_map(values.as_slice(), |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, decode(&blob)))
        })?;
        for row in rows {
            let (id, vector) = row?;
            out.insert(id, vector);
        }
    }
    Ok(out)
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

/// A link as it comes back out of the database, keyed by the editor's block id
/// rather than the rowid the table stores.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredLink {
    pub note_block_id: String,
    pub utterance_id: i64,
    pub method: String,
    pub score: f64,
}

/// Replaces every link for a meeting.
///
/// Linking is a pure function of the transcript and the notes, so it is
/// recomputed wholesale rather than patched — that keeps a re-run from leaving
/// links behind that point at text the user has since deleted.
///
/// `note_links.note_block_id` is the rowid, but callers work in the editor's
/// stable string ids; the translation happens here so no caller has to know.
/// Links naming a block this meeting does not have are skipped rather than
/// failing the batch: an LLM citation is the one input that can name anything.
pub fn replace_note_links(
    conn: &mut Connection,
    meeting_id: &str,
    links: &[(String, i64, String, f64)],
) -> Result<usize> {
    let tx = conn.transaction()?;

    let mut rowids = std::collections::HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT block_id, id FROM note_blocks WHERE meeting_id = ?1")?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (block_id, rowid) = row?;
            rowids.insert(block_id, rowid);
        }
    }

    tx.execute(
        "DELETE FROM note_links
         WHERE note_block_id IN (SELECT id FROM note_blocks WHERE meeting_id = ?1)",
        params![meeting_id],
    )?;

    let mut written = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO note_links (note_block_id, utterance_id, method, score)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (block_id, utterance_id, method, score) in links {
            let Some(rowid) = rowids.get(block_id) else {
                continue;
            };
            written += stmt.execute(params![rowid, utterance_id, method, score])?;
        }
    }

    tx.commit()?;
    Ok(written)
}

pub fn meeting_links(conn: &Connection, meeting_id: &str) -> Result<Vec<StoredLink>> {
    let mut stmt = conn.prepare(
        "SELECT b.block_id, l.utterance_id, l.method, l.score
         FROM note_links l
         JOIN note_blocks b ON b.id = l.note_block_id
         WHERE b.meeting_id = ?1
         ORDER BY b.seq, l.score DESC",
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(StoredLink {
            note_block_id: row.get(0)?,
            utterance_id: row.get(1)?,
            method: row.get(2)?,
            score: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// -------------------------------------------------------------------- folders

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub meeting_count: i64,
}

pub fn list_folders(conn: &Connection) -> Result<Vec<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.name, f.parent_id,
                (SELECT COUNT(*) FROM meetings m WHERE m.folder_id = f.id)
         FROM folders f
         ORDER BY f.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Folder {
            id: row.get(0)?,
            name: row.get(1)?,
            parent_id: row.get(2)?,
            meeting_count: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn create_folder(
    conn: &Connection,
    name: &str,
    parent_id: Option<&str>,
    now: i64,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO folders (id, name, parent_id, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, name.trim(), parent_id, now],
    )?;
    Ok(id)
}

pub fn rename_folder(conn: &Connection, folder_id: &str, name: &str) -> Result<()> {
    conn.execute(
        "UPDATE folders SET name = ?2 WHERE id = ?1",
        params![folder_id, name.trim()],
    )?;
    Ok(())
}

/// Deletes a folder. Meetings inside are **kept** and become unfiled.
///
/// `meetings.folder_id` is `ON DELETE SET NULL` precisely so this cannot destroy
/// recordings: deleting a folder is an organisational act, and losing an hour of
/// transcript to one would be indefensible.
pub fn delete_folder(conn: &Connection, folder_id: &str) -> Result<()> {
    conn.execute("DELETE FROM folders WHERE id = ?1", params![folder_id])?;
    Ok(())
}

/// Files a meeting, or unfiles it with `None`.
pub fn set_meeting_folder(
    conn: &Connection,
    meeting_id: &str,
    folder_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET folder_id = ?2 WHERE id = ?1",
        params![meeting_id, folder_id],
    )?;
    Ok(())
}

/// Meetings in a folder, or unfiled ones when `folder_id` is `None`.
pub fn meetings_in_folder(
    conn: &Connection,
    folder_id: Option<&str>,
    limit: i64,
) -> Result<Vec<MeetingSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.title, m.started_at, m.ended_at, m.status, m.audio_path,
                (SELECT COUNT(*) FROM utterances u WHERE u.meeting_id = m.id)
         FROM meetings m
         WHERE (?1 IS NULL AND m.folder_id IS NULL) OR m.folder_id = ?1
         ORDER BY m.started_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![folder_id, limit], |row| {
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
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One utterance, with what search needs to rank and preview it.
#[derive(Debug, Clone)]
pub struct SearchRow {
    pub id: i64,
    pub meeting_id: String,
    pub text: String,
    pub start_ms: i64,
}

/// Full-text hits, best first, optionally confined to a folder.
pub fn search_rows_fts(
    conn: &Connection,
    fts_query: &str,
    folder_id: Option<&str>,
    limit: i64,
) -> Result<Vec<SearchRow>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.meeting_id, u.text, u.start_ms
         FROM utterances_fts f
         JOIN utterances u ON u.id = f.rowid
         JOIN meetings m ON m.id = u.meeting_id
         WHERE utterances_fts MATCH ?1
           AND (?2 IS NULL OR m.folder_id = ?2)
         ORDER BY f.rank
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![fts_query, folder_id, limit], |row| {
        Ok(SearchRow {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            text: row.get(2)?,
            start_ms: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Loads specific utterances by id, for the semantic half of a search.
pub fn utterances_by_id(conn: &Connection, ids: &[i64]) -> Result<Vec<SearchRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, meeting_id, text, start_ms FROM utterances WHERE id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let values: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(values.as_slice(), |row| {
        Ok(SearchRow {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            text: row.get(2)?,
            start_ms: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Title and start time for a set of meetings, for grouping search results.
pub fn meeting_headers(
    conn: &Connection,
    ids: &[String],
) -> Result<std::collections::HashMap<String, (Option<String>, i64)>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT id, title, started_at FROM meetings WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let values: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(values.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (id, header) = row?;
        out.insert(id, header);
    }
    Ok(out)
}

// ------------------------------------------------------------------ detection

/// A per-app detection rule as stored.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionRule {
    pub bundle_id: String,
    pub app_name: Option<String>,
    pub mode: String,
}

pub fn detection_rules(conn: &Connection) -> Result<Vec<DetectionRule>> {
    let mut stmt = conn.prepare(
        "SELECT bundle_id, app_name, mode FROM detection_rules ORDER BY app_name, bundle_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(DetectionRule {
            bundle_id: row.get(0)?,
            app_name: row.get(1)?,
            mode: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Writes a rule, replacing any previous answer for the same app.
///
/// Upsert rather than insert: the settings screen lets someone change their
/// mind, and a second answer must replace the first rather than collide with
/// the unique constraint.
pub fn set_detection_rule(
    conn: &Connection,
    bundle_id: &str,
    app_name: Option<&str>,
    mode: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO detection_rules (id, bundle_id, app_name, mode, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (bundle_id) DO UPDATE SET
             mode = excluded.mode,
             app_name = COALESCE(excluded.app_name, detection_rules.app_name)",
        params![
            uuid::Uuid::new_v4().to_string(),
            bundle_id,
            app_name,
            mode,
            now
        ],
    )?;
    Ok(())
}

/// Removes a rule, returning the app to "ask once".
pub fn clear_detection_rule(conn: &Connection, bundle_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM detection_rules WHERE bundle_id = ?1",
        params![bundle_id],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

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
        insert_note_block(conn, "m1", "b0", 0, "deadline = 14th", Some(16_000)).unwrap();

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

    fn block(block_id: &str, seq: i64, text: &str, first: i64) -> NoteBlock {
        NoteBlock {
            block_id: block_id.into(),
            seq,
            text: text.into(),
            first_typed_at_ms: Some(first),
            last_edited_at_ms: Some(first),
        }
    }

    fn seeded_meeting() -> Database {
        let db = Database::open_in_memory().unwrap();
        insert_meeting(db.connection(), "m1", "Test", 0).unwrap();
        ensure_builtin_templates(
            db.connection(),
            &[
                ("default", "Summary", "prompt"),
                ("one-on-one", "1:1", "prompt"),
            ],
            0,
        )
        .unwrap();
        db
    }

    #[test]
    fn seeding_builtin_templates_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let seed = |name: &str| {
            ensure_builtin_templates(db.connection(), &[("default", name, "p")], 0).unwrap()
        };
        seed("Summary");
        seed("Summary renamed");

        let (count, name): (i64, String) = db
            .connection()
            .query_row(
                "SELECT COUNT(*), MAX(name) FROM templates WHERE id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "seeding duplicated a built-in");
        // Code stays the source of truth for built-in prompts.
        assert_eq!(name, "Summary renamed");
    }

    #[test]
    fn seeding_never_overwrites_a_user_template() {
        let db = Database::open_in_memory().unwrap();
        db.connection()
            .execute(
                "INSERT INTO templates (id, name, prompt, is_builtin, created_at)
                 VALUES (\'mine\', \'My template\', \'my prompt\', 0, 0)",
                [],
            )
            .unwrap();

        ensure_builtin_templates(db.connection(), &[("mine", "Hijacked", "other")], 0).unwrap();

        let name: String = db
            .connection()
            .query_row("SELECT name FROM templates WHERE id = 'mine'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "My template", "a user template was overwritten");
    }

    #[test]
    fn notes_round_trip_in_display_order() {
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b1", 0, "first", 1_000),
                block("b2", 1, "second", 2_000),
            ],
        )
        .unwrap();

        let notes = meeting_notes(db.connection(), "m1").unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].text, "first");
        assert_eq!(notes[1].text, "second");
    }

    #[test]
    fn editing_a_block_never_moves_its_first_typed_anchor() {
        // The single most important rule in the notepad. `first_typed_at_ms` is
        // what the temporal linker keys on, so if an edit moved it the note
        // would silently re-point at a different moment in the transcript.
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[block("b1", 0, "deadline", 5_000)],
        )
        .unwrap();

        let mut edited = block("b1", 0, "deadline is the 14th", 999_999);
        edited.last_edited_at_ms = Some(60_000);
        save_note_blocks(db.connection_mut(), "m1", &[edited]).unwrap();

        let notes = meeting_notes(db.connection(), "m1").unwrap();
        assert_eq!(notes[0].text, "deadline is the 14th");
        assert_eq!(
            notes[0].first_typed_at_ms,
            Some(5_000),
            "the linker anchor was rewritten by an edit"
        );
        assert_eq!(notes[0].last_edited_at_ms, Some(60_000));
    }

    #[test]
    fn inserting_a_block_does_not_steal_its_neighbours_anchors() {
        // The bug that made `block_id` necessary: keyed by `seq` alone, adding a
        // line in the middle shifts everything below and each block inherits the
        // previous occupant's timestamp.
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b1", 0, "first", 1_000),
                block("b2", 1, "second", 2_000),
            ],
        )
        .unwrap();

        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b1", 0, "first", 1_000),
                block("b3", 1, "inserted", 9_000),
                block("b2", 2, "second", 2_000),
            ],
        )
        .unwrap();

        let notes = meeting_notes(db.connection(), "m1").unwrap();
        let by_id: std::collections::HashMap<_, _> =
            notes.iter().map(|n| (n.block_id.as_str(), n)).collect();
        assert_eq!(by_id["b1"].first_typed_at_ms, Some(1_000));
        assert_eq!(by_id["b2"].first_typed_at_ms, Some(2_000));
        assert_eq!(by_id["b3"].first_typed_at_ms, Some(9_000));
        // ...and the new order is reflected.
        assert_eq!(by_id["b2"].seq, 2);
    }

    #[test]
    fn reordering_blocks_keeps_every_anchor() {
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b1", 0, "first", 1_000),
                block("b2", 1, "second", 2_000),
            ],
        )
        .unwrap();

        // Swapped.
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b2", 0, "second", 2_000),
                block("b1", 1, "first", 1_000),
            ],
        )
        .unwrap();

        let notes = meeting_notes(db.connection(), "m1").unwrap();
        assert_eq!(notes[0].block_id, "b2");
        assert_eq!(notes[0].first_typed_at_ms, Some(2_000));
        assert_eq!(notes[1].first_typed_at_ms, Some(1_000));
    }

    #[test]
    fn deleting_a_block_removes_it() {
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[
                block("b1", 0, "keep", 1_000),
                block("b2", 1, "delete me", 2_000),
            ],
        )
        .unwrap();

        save_note_blocks(db.connection_mut(), "m1", &[block("b1", 0, "keep", 1_000)]).unwrap();

        let notes = meeting_notes(db.connection(), "m1").unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].block_id, "b1");
    }

    #[test]
    fn clearing_the_notepad_removes_every_block() {
        let mut db = seeded_meeting();
        save_note_blocks(db.connection_mut(), "m1", &[block("b1", 0, "text", 1_000)]).unwrap();
        save_note_blocks(db.connection_mut(), "m1", &[]).unwrap();
        assert!(meeting_notes(db.connection(), "m1").unwrap().is_empty());
    }

    #[test]
    fn notes_are_scoped_to_their_meeting() {
        let mut db = seeded_meeting();
        insert_meeting(db.connection(), "m2", "Other", 0).unwrap();
        save_note_blocks(db.connection_mut(), "m1", &[block("b1", 0, "mine", 1_000)]).unwrap();
        save_note_blocks(
            db.connection_mut(),
            "m2",
            &[block("b1", 0, "theirs", 1_000)],
        )
        .unwrap();

        // Same block_id in both meetings must not collide or overwrite.
        assert_eq!(
            meeting_notes(db.connection(), "m1").unwrap()[0].text,
            "mine"
        );
        assert_eq!(
            meeting_notes(db.connection(), "m2").unwrap()[0].text,
            "theirs"
        );
    }

    #[test]
    fn saving_notes_keeps_them_searchable() {
        let mut db = seeded_meeting();
        save_note_blocks(
            db.connection_mut(),
            "m1",
            &[block("b1", 0, "deadline for the migration", 1_000)],
        )
        .unwrap();

        // The FTS table was rebuilt in migration 3; its triggers must still fire.
        let hits = search_note_blocks(db.connection(), "migrate", 10).unwrap();
        assert_eq!(hits.len(), 1, "note FTS index is not being maintained");

        save_note_blocks(db.connection_mut(), "m1", &[]).unwrap();
        assert!(search_note_blocks(db.connection(), "migrate", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn panels_come_back_newest_first() {
        let db = seeded_meeting();
        let conn = db.connection();
        insert_panel(
            conn, "p1", "m1", "default", "{}", "older", "Ollama", "llama", 1_000,
        )
        .unwrap();
        insert_panel(
            conn,
            "p2",
            "m1",
            "one-on-one",
            "{}",
            "newer",
            "Ollama",
            "llama",
            2_000,
        )
        .unwrap();

        let panels = meeting_panels(conn, "m1").unwrap();
        assert_eq!(panels[0].id, "p2");
        assert_eq!(panels[1].id, "p1");
    }

    #[test]
    fn regenerating_adds_a_panel_rather_than_replacing_one() {
        // The decision-gate default: edits are never destroyed.
        let db = seeded_meeting();
        let conn = db.connection();
        insert_panel(
            conn, "p1", "m1", "default", "{}", "first", "Ollama", "l", 1_000,
        )
        .unwrap();
        insert_panel(
            conn, "p2", "m1", "default", "{}", "second", "Ollama", "l", 2_000,
        )
        .unwrap();
        assert_eq!(meeting_panels(conn, "m1").unwrap().len(), 2);
    }

    #[test]
    fn a_panel_records_which_model_produced_it() {
        // The privacy panel (G27) reports local-vs-cloud per generation, not
        // per app, so this has to be stored with the panel itself.
        let db = seeded_meeting();
        insert_panel(
            db.connection(),
            "p1",
            "m1",
            "default",
            "{}",
            "text",
            "Anthropic",
            "claude-sonnet-5",
            1_000,
        )
        .unwrap();

        let panel = &meeting_panels(db.connection(), "m1").unwrap()[0];
        assert_eq!(panel.provider.as_deref(), Some("Anthropic"));
        assert_eq!(panel.model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn deleting_a_panel_leaves_the_transcript_and_notes_alone() {
        let mut db = seeded_meeting();
        append_utterance(db.connection(), "m1", "mic", "said aloud", 0, 1, None).unwrap();
        save_note_blocks(db.connection_mut(), "m1", &[block("b1", 0, "noted", 1_000)]).unwrap();
        insert_panel(
            db.connection(),
            "p1",
            "m1",
            "default",
            "{}",
            "text",
            "Ollama",
            "l",
            1_000,
        )
        .unwrap();

        delete_panel(db.connection(), "p1").unwrap();

        assert!(meeting_panels(db.connection(), "m1").unwrap().is_empty());
        assert_eq!(meeting_utterances(db.connection(), "m1").unwrap().len(), 1);
        assert_eq!(meeting_notes(db.connection(), "m1").unwrap().len(), 1);
    }

    #[test]
    fn panels_are_searchable_by_their_plaintext() {
        let db = seeded_meeting();
        insert_panel(
            db.connection(),
            "p1",
            "m1",
            "default",
            "{}",
            "deadline for the migration",
            "Ollama",
            "l",
            1_000,
        )
        .unwrap();

        let found: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM panels_fts WHERE panels_fts MATCH 'migrate'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "panel text is not reaching the FTS index");
    }

    #[test]
    fn deleting_a_meeting_takes_its_panels() {
        let db = seeded_meeting();
        insert_panel(
            db.connection(),
            "p1",
            "m1",
            "default",
            "{}",
            "t",
            "Ollama",
            "l",
            1_000,
        )
        .unwrap();
        delete_meeting(db.connection(), "m1").unwrap();
        assert!(meeting_panels(db.connection(), "m1").unwrap().is_empty());
    }

    #[test]
    fn renaming_a_meeting_keeps_everything_else() {
        let db = seeded_meeting();
        append_utterance(db.connection(), "m1", "mic", "hello", 0, 1, None).unwrap();

        rename_meeting(db.connection(), "m1", "Quarterly planning").unwrap();

        let meeting = &list_meetings(db.connection(), 10).unwrap()[0];
        assert_eq!(meeting.title.as_deref(), Some("Quarterly planning"));
        assert_eq!(meeting.utterance_count, 1);
    }

    #[test]
    fn deleting_a_meeting_takes_its_transcript_and_notes_with_it() {
        let mut db = seeded_meeting();
        append_utterance(db.connection(), "m1", "mic", "hello", 0, 1, None).unwrap();
        save_note_blocks(db.connection_mut(), "m1", &[block("b1", 0, "note", 1_000)]).unwrap();

        delete_meeting(db.connection(), "m1").unwrap();

        assert!(list_meetings(db.connection(), 10).unwrap().is_empty());
        assert!(meeting_utterances(db.connection(), "m1")
            .unwrap()
            .is_empty());
        assert!(meeting_notes(db.connection(), "m1").unwrap().is_empty());
    }

    #[test]
    fn deleting_returns_the_audio_path_so_the_file_can_go_too() {
        // The cascade clears rows but knows nothing about the filesystem; an
        // orphaned recording is exactly what the user asked to be rid of.
        let db = seeded_meeting();
        finish_meeting(db.connection(), "m1", 2_000, Some("/tmp/a.m4a"), None).unwrap();

        let path = delete_meeting(db.connection(), "m1").unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/a.m4a"));
    }

    #[test]
    fn deleting_a_meeting_with_no_audio_reports_no_path() {
        let db = seeded_meeting();
        assert_eq!(delete_meeting(db.connection(), "m1").unwrap(), None);
    }

    #[test]
    fn deleting_a_meeting_that_does_not_exist_is_not_an_error() {
        // Two windows, or a double click, must not produce a hard failure.
        let db = seeded_meeting();
        assert_eq!(delete_meeting(db.connection(), "nope").unwrap(), None);
    }

    #[test]
    fn deleting_one_meeting_leaves_the_others_alone() {
        let db = seeded_meeting();
        insert_meeting(db.connection(), "m2", "Keep me", 0).unwrap();
        append_utterance(db.connection(), "m2", "mic", "survives", 0, 1, None).unwrap();

        delete_meeting(db.connection(), "m1").unwrap();

        let remaining = list_meetings(db.connection(), 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "m2");
        assert_eq!(remaining[0].utterance_count, 1);
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

    #[test]
    fn a_detection_rule_round_trips() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        set_detection_rule(conn, "us.zoom.xos", Some("Zoom"), "allow", 0).unwrap();

        let rules = detection_rules(conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].bundle_id, "us.zoom.xos");
        assert_eq!(rules[0].mode, "allow");
        assert_eq!(rules[0].app_name.as_deref(), Some("Zoom"));
    }

    #[test]
    fn answering_again_replaces_the_earlier_answer() {
        // The settings screen lets someone change their mind; a second answer
        // must not collide with the unique constraint on bundle_id.
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        set_detection_rule(conn, "us.zoom.xos", Some("Zoom"), "allow", 0).unwrap();
        set_detection_rule(conn, "us.zoom.xos", None, "ignore", 1).unwrap();

        let rules = detection_rules(conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].mode, "ignore");
        // The name survives an update that did not carry one.
        assert_eq!(rules[0].app_name.as_deref(), Some("Zoom"));
    }

    #[test]
    fn clearing_a_rule_returns_the_app_to_being_asked_about() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        set_detection_rule(conn, "com.example.app", None, "ignore", 0).unwrap();
        clear_detection_rule(conn, "com.example.app").unwrap();
        assert!(detection_rules(conn).unwrap().is_empty());
    }

    #[test]
    fn an_unrecognised_mode_is_refused_by_the_schema() {
        // The CHECK constraint is the last line of defence against a typo
        // silently disabling detection for an app.
        let db = Database::open_in_memory().unwrap();
        assert!(set_detection_rule(db.connection(), "x", None, "maybe", 0).is_err());
    }

    #[test]
    fn settings_round_trip_and_overwrite() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        assert_eq!(get_setting(conn, "lead").unwrap(), None);
        set_setting(conn, "lead", "90").unwrap();
        set_setting(conn, "lead", "120").unwrap();
        assert_eq!(get_setting(conn, "lead").unwrap().as_deref(), Some("120"));
    }

    // MARK: folders

    #[test]
    fn a_folder_round_trips_with_its_meeting_count() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let id = create_folder(conn, "Vendors", None, 0).unwrap();
        insert_meeting(conn, "m1", "Review", 0).unwrap();
        set_meeting_folder(conn, "m1", Some(&id)).unwrap();

        let folders = list_folders(conn).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Vendors");
        assert_eq!(folders[0].meeting_count, 1);
    }

    #[test]
    fn deleting_a_folder_keeps_its_meetings() {
        // `folder_id` is ON DELETE SET NULL precisely so an organisational act
        // cannot destroy an hour of transcript.
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let id = create_folder(conn, "Temp", None, 0).unwrap();
        insert_meeting(conn, "m1", "Review", 0).unwrap();
        set_meeting_folder(conn, "m1", Some(&id)).unwrap();

        delete_folder(conn, &id).unwrap();

        assert!(list_folders(conn).unwrap().is_empty());
        assert_eq!(
            list_meetings(conn, 10).unwrap().len(),
            1,
            "meeting was lost"
        );
        // And it is unfiled, not orphaned into a folder that no longer exists.
        assert_eq!(meetings_in_folder(conn, None, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_meeting_can_be_unfiled() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let id = create_folder(conn, "Vendors", None, 0).unwrap();
        insert_meeting(conn, "m1", "Review", 0).unwrap();
        set_meeting_folder(conn, "m1", Some(&id)).unwrap();
        set_meeting_folder(conn, "m1", None).unwrap();

        assert!(meetings_in_folder(conn, Some(&id), 10).unwrap().is_empty());
        assert_eq!(meetings_in_folder(conn, None, 10).unwrap().len(), 1);
    }

    #[test]
    fn renaming_a_folder_keeps_its_contents() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let id = create_folder(conn, "Old", None, 0).unwrap();
        insert_meeting(conn, "m1", "Review", 0).unwrap();
        set_meeting_folder(conn, "m1", Some(&id)).unwrap();

        rename_folder(conn, &id, "New").unwrap();
        let folders = list_folders(conn).unwrap();
        assert_eq!(folders[0].name, "New");
        assert_eq!(folders[0].meeting_count, 1);
    }

    #[test]
    fn folder_names_are_trimmed() {
        // A name of spaces renders as an unclickable blank row.
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        create_folder(conn, "  Vendors  ", None, 0).unwrap();
        assert_eq!(list_folders(conn).unwrap()[0].name, "Vendors");
    }

    #[test]
    fn deleting_a_parent_folder_takes_its_children() {
        // `folders.parent_id` cascades; a child pointing at a missing parent
        // would never render.
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let parent = create_folder(conn, "Clients", None, 0).unwrap();
        create_folder(conn, "Acme", Some(&parent), 0).unwrap();

        delete_folder(conn, &parent).unwrap();
        assert!(list_folders(conn).unwrap().is_empty());
    }

    #[test]
    fn unfiled_meetings_are_listable() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "Loose", 0).unwrap();
        assert_eq!(meetings_in_folder(conn, None, 10).unwrap().len(), 1);
    }

    #[test]
    fn meeting_headers_ignores_ids_that_no_longer_exist() {
        // A meeting deleted between a search and its grouping must not error.
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        insert_meeting(conn, "m1", "Here", 5).unwrap();
        let headers = meeting_headers(conn, &["m1".to_string(), "gone".to_string()]).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers["m1"].1, 5);
    }
}
