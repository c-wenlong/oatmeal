//! SQLite data layer.
//!
//! Local-first and account-free (SPEC section 2), so this file is the whole
//! persistence story: schema, migrations, and the queries the rest of the app
//! builds on.

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;

pub mod repo;

/// Dimension of stored embedding vectors. Must match `0002_embeddings.sql`;
/// changing it requires a new migration that rebuilds the table.
pub const EMBEDDING_DIM: usize = 384;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        version: i32,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("expected {expected} dimensions, got {actual}")]
    EmbeddingDimension { expected: usize, actual: usize },
}

pub type Result<T> = std::result::Result<T, DbError>;

struct Migration {
    version: i32,
    name: &'static str,
    sql: &'static str,
}

/// Applied in order; `user_version` records how far we got. Append only —
/// never edit a migration that has shipped.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "embeddings",
        sql: include_str!("migrations/0002_embeddings.sql"),
    },
    Migration {
        version: 3,
        name: "note_block_identity",
        sql: include_str!("migrations/0003_note_block_identity.sql"),
    },
];

/// Latest schema version this build knows about.
pub fn target_version() -> i32 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

static REGISTER_VEC: Once = Once::new();

/// Registers sqlite-vec as an auto-extension so every subsequent connection —
/// including ones rusqlite opens internally — gets `vec0`.
///
/// Must run before the first `Connection::open`. Idempotent.
fn register_vec_extension() {
    REGISTER_VEC.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
}

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (creating if needed) and migrates to [`target_version`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        register_vec_extension();
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// In-memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        register_vec_extension();
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // WAL keeps the UI readable while a recording writes utterances.
        // Ignored (and harmless) for in-memory databases.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA synchronous = NORMAL;",
        )?;

        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn schema_version(&self) -> Result<i32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// Applies every migration newer than the stored `user_version`.
    ///
    /// Each runs in its own transaction, so a failure part-way leaves the
    /// database at the last version that fully succeeded rather than in a
    /// half-migrated state. Re-running on an up-to-date database is a no-op,
    /// which is what makes relaunch safe.
    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()?;

        for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(migration.sql)
                .map_err(|source| DbError::Migration {
                    version: migration.version,
                    name: migration.name,
                    source,
                })?;
            // PRAGMA can't be parameterised.
            tx.pragma_update(None, "user_version", migration.version)?;
            tx.commit()?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_empty_database_to_target_version() {
        let db = Database::open_in_memory().expect("open");
        assert_eq!(db.schema_version().unwrap(), target_version());
    }

    #[test]
    fn migration_is_idempotent_across_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oatmeal.sqlite");

        {
            let db = Database::open(&path).expect("first open");
            assert_eq!(db.schema_version().unwrap(), target_version());
        }

        // Reopening must not re-run migrations — re-running 0001 would fail on
        // "table already exists", so a clean second open proves the guard works.
        {
            let db = Database::open(&path).expect("second open");
            assert_eq!(db.schema_version().unwrap(), target_version());
        }

        // And a third, to catch anything that only breaks after WAL files exist.
        let db = Database::open(&path).expect("third open");
        assert_eq!(db.schema_version().unwrap(), target_version());
    }

    #[test]
    fn every_expected_table_exists() {
        let db = Database::open_in_memory().unwrap();
        let mut stmt = db
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view')")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        for expected in [
            "folders",
            "calendar_events",
            "meetings",
            "utterances",
            "note_blocks",
            "note_links",
            "templates",
            "panels",
            "panel_citations",
            "providers",
            "detection_rules",
            "settings",
            "utterances_fts",
            "note_blocks_fts",
            "panels_fts",
            "embeddings",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing table {expected}"
            );
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Database::open_in_memory().unwrap();
        // An utterance pointing at a meeting that doesn't exist must be rejected;
        // otherwise orphaned transcript rows accumulate invisibly.
        let result = db.connection().execute(
            "INSERT INTO utterances (meeting_id, seq, source, text, start_ms, end_ms)
             VALUES ('no-such-meeting', 0, 'mic', 'hello', 0, 10)",
            [],
        );
        assert!(result.is_err(), "foreign key was not enforced");
    }

    #[test]
    fn utterance_source_is_constrained_to_the_two_streams() {
        let db = Database::open_in_memory().unwrap();
        repo::insert_meeting(db.connection(), "m1", "Test", 0).unwrap();

        let bad = db.connection().execute(
            "INSERT INTO utterances (meeting_id, seq, source, text, start_ms, end_ms)
             VALUES ('m1', 0, 'speaker_3', 'hello', 0, 10)",
            [],
        );
        assert!(bad.is_err(), "source CHECK constraint missing");
    }
}
