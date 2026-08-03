//! Running the retention policy against the database and disk.

use rusqlite::{params, Connection};

use super::{is_expired, SweepReport};
use crate::db::DbError;

/// A meeting whose audio may be due for deletion.
#[derive(Debug, Clone)]
pub struct Expiring {
    pub meeting_id: String,
    pub audio_path: String,
    pub expires_at: Option<i64>,
}

/// Every meeting that still has an audio file recorded against it.
pub fn with_audio(conn: &Connection) -> Result<Vec<Expiring>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, audio_path, audio_expires_at
         FROM meetings
         WHERE audio_path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Expiring {
            meeting_id: row.get(0)?,
            audio_path: row.get(1)?,
            expires_at: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Forgets a meeting's audio: removes the file, clears the columns.
///
/// The row is **updated, never deleted** — the transcript, notes, panels and
/// links all hang off it, and they are the durable record. Only the pointer to
/// the sound goes.
fn forget_audio(conn: &Connection, meeting_id: &str) -> Result<(), DbError> {
    conn.execute(
        "UPDATE meetings SET audio_path = NULL, audio_expires_at = NULL WHERE id = ?1",
        params![meeting_id],
    )?;
    Ok(())
}

/// Deletes one file and clears its row.
///
/// Order matters: the file goes first, then the database. Interrupted between
/// the two, the next sweep finds a row pointing at a missing file and clears
/// it — which is why a missing file counts as success. The other order would
/// leave an orphaned recording on disk that nothing ever points at again.
fn delete_one(conn: &Connection, entry: &Expiring, report: &mut SweepReport) {
    let path = std::path::Path::new(&entry.audio_path);
    match std::fs::metadata(path) {
        Ok(meta) => {
            if std::fs::remove_file(path).is_ok() {
                report.deleted += 1;
                report.freed_bytes += meta.len();
            } else {
                // Could not remove it — leave the row alone so a later sweep
                // tries again rather than losing track of the file.
                return;
            }
        }
        Err(_) => report.already_missing += 1,
    }
    let _ = forget_audio(conn, &entry.meeting_id);
}

/// Deletes audio past its expiry.
///
/// Run at launch, per G27's done-when: audio older than the window is gone on
/// next launch, transcripts untouched.
pub fn sweep(conn: &Connection, now_ms: i64) -> Result<SweepReport, DbError> {
    let mut report = SweepReport::default();
    for entry in with_audio(conn)? {
        if is_expired(entry.expires_at, now_ms) {
            delete_one(conn, &entry, &mut report);
        }
    }
    Ok(report)
}

/// Deletes every audio file, whatever its expiry.
///
/// The "purge all audio" button. Transcripts survive, which the UI has to say
/// out loud — otherwise it reads like it deletes the meetings.
pub fn purge_all(conn: &Connection) -> Result<SweepReport, DbError> {
    let mut report = SweepReport::default();
    for entry in with_audio(conn)? {
        delete_one(conn, &entry, &mut report);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{repo, Database};
    use crate::retention::DAY_MS;

    /// A meeting with a real file on disk.
    fn meeting_with_audio(
        conn: &Connection,
        dir: &std::path::Path,
        id: &str,
        expires_at: Option<i64>,
    ) -> std::path::PathBuf {
        repo::insert_meeting(conn, id, "Standup", 0).unwrap();
        repo::insert_utterance(conn, id, 0, "system", "we ship thursday", 0, 1_000, None).unwrap();

        let path = dir.join(format!("{id}.m4a"));
        std::fs::write(&path, vec![0u8; 2_048]).unwrap();
        repo::finish_meeting(conn, id, 1_000, Some(&path.to_string_lossy()), expires_at).unwrap();
        path
    }

    #[test]
    fn expired_audio_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let path = meeting_with_audio(conn, dir.path(), "m1", Some(1_000));

        let report = sweep(conn, 2_000).unwrap();

        assert_eq!(report.deleted, 1);
        assert_eq!(report.freed_bytes, 2_048);
        assert!(!path.exists(), "the file survived the sweep");
    }

    #[test]
    fn the_transcript_survives_the_sweep() {
        // The rule the whole module exists to protect. Audio is a re-listening
        // aid; the transcript is the record.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        meeting_with_audio(conn, dir.path(), "m1", Some(1_000));

        sweep(conn, 2_000).unwrap();

        assert_eq!(repo::meeting_utterances(conn, "m1").unwrap().len(), 1);
        assert_eq!(repo::list_meetings(conn, 10).unwrap().len(), 1);
    }

    #[test]
    fn the_row_stops_pointing_at_the_deleted_file() {
        // Otherwise the UI offers a play button for a file that is gone.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        meeting_with_audio(conn, dir.path(), "m1", Some(1_000));

        sweep(conn, 2_000).unwrap();

        assert_eq!(repo::list_meetings(conn, 10).unwrap()[0].audio_path, None);
    }

    #[test]
    fn audio_inside_the_window_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let path = meeting_with_audio(conn, dir.path(), "m1", Some(10 * DAY_MS));

        let report = sweep(conn, DAY_MS).unwrap();

        assert_eq!(report.deleted, 0);
        assert!(path.exists());
    }

    #[test]
    fn keep_forever_is_never_swept() {
        // A null expiry means keep — either the user chose forever, or the row
        // predates retention.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let path = meeting_with_audio(conn, dir.path(), "m1", None);

        let report = sweep(conn, i64::MAX).unwrap();

        assert_eq!(report.deleted, 0);
        assert!(path.exists(), "audio marked keep-forever was deleted");
    }

    #[test]
    fn a_file_already_gone_is_not_an_error() {
        // An interrupted sweep, or a user who deleted it themselves. Erroring
        // would leave the row pointing at nothing forever.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let path = meeting_with_audio(conn, dir.path(), "m1", Some(1_000));
        std::fs::remove_file(&path).unwrap();

        let report = sweep(conn, 2_000).unwrap();

        assert_eq!(report.deleted, 0);
        assert_eq!(report.already_missing, 1);
        // And the stale pointer is cleared, so it is not retried forever.
        assert_eq!(repo::list_meetings(conn, 10).unwrap()[0].audio_path, None);
    }

    #[test]
    fn sweeping_twice_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        meeting_with_audio(conn, dir.path(), "m1", Some(1_000));

        sweep(conn, 2_000).unwrap();
        let second = sweep(conn, 2_000).unwrap();
        assert_eq!(second.touched(), 0, "a second sweep found work to redo");
    }

    #[test]
    fn purge_takes_everything_including_keep_forever() {
        // The explicit "delete all my audio" button means all of it.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        let kept = meeting_with_audio(conn, dir.path(), "m1", None);
        let expiring = meeting_with_audio(conn, dir.path(), "m2", Some(i64::MAX));

        let report = purge_all(conn).unwrap();

        assert_eq!(report.deleted, 2);
        assert!(!kept.exists() && !expiring.exists());
    }

    #[test]
    fn purge_keeps_every_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        meeting_with_audio(conn, dir.path(), "m1", None);

        purge_all(conn).unwrap();

        assert_eq!(repo::meeting_utterances(conn, "m1").unwrap().len(), 1);
    }

    #[test]
    fn a_meeting_with_no_audio_is_not_visited() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "No audio", 0).unwrap();

        assert!(with_audio(conn).unwrap().is_empty());
        assert_eq!(sweep(conn, i64::MAX).unwrap().touched(), 0);
    }
}
