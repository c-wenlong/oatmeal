//! Exporting a meeting, and re-exporting it without making a second page.
//!
//! G26's done-when is entirely about the second export: regenerating a summary
//! and exporting again must *update* the page. That is the difference between a
//! connector someone uses weekly and one that quietly fills their database with
//! duplicates until they turn it off.

use rusqlite::{params, Connection};
use serde::Serialize;

use super::shape;
use super::{Notion, NotionError};
use crate::db::{repo, DbError};
use crate::panel::content::PanelContent;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Notion(#[from] NotionError),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("this meeting has no summary to export yet")]
    NothingToExport,
    #[error("the summary could not be read: {0}")]
    Malformed(String),
}

/// A previous export.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub meeting_id: String,
    pub database_id: String,
    pub page_id: String,
    pub panel_id: Option<String>,
    pub exported_at: i64,
}

pub fn record_for(conn: &Connection, meeting_id: &str) -> Result<Option<ExportRecord>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT meeting_id, database_id, page_id, panel_id, exported_at
         FROM notion_exports WHERE meeting_id = ?1",
    )?;
    let mut rows = stmt.query_map(params![meeting_id], |row| {
        Ok(ExportRecord {
            meeting_id: row.get(0)?,
            database_id: row.get(1)?,
            page_id: row.get(2)?,
            panel_id: row.get(3)?,
            exported_at: row.get(4)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

fn remember(
    conn: &Connection,
    meeting_id: &str,
    database_id: &str,
    page_id: &str,
    panel_id: Option<&str>,
    now: i64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO notion_exports (meeting_id, database_id, page_id, panel_id, exported_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (meeting_id) DO UPDATE SET
             database_id = excluded.database_id,
             page_id = excluded.page_id,
             panel_id = excluded.panel_id,
             exported_at = excluded.exported_at",
        params![meeting_id, database_id, page_id, panel_id, now],
    )?;
    Ok(())
}

pub fn forget(conn: &Connection, meeting_id: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM notion_exports WHERE meeting_id = ?1",
        params![meeting_id],
    )?;
    Ok(())
}

/// Whether an export should update an existing page or make a new one.
///
/// Pure, because it is the whole of G26's done-when and the one thing worth
/// being certain about. A record for a *different* database means the user
/// repointed the integration; updating a page in the old database would write
/// into a place they have stopped using, and against a schema that may no
/// longer match.
pub fn target<'a>(existing: Option<&'a ExportRecord>, database_id: &str) -> Option<&'a str> {
    existing
        .filter(|record| record.database_id == database_id)
        .map(|record| record.page_id.as_str())
}

/// What an export did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub page_id: String,
    /// False when an existing page was updated.
    pub created: bool,
    pub blocks: usize,
}

/// Everything the export needs, gathered before any network call.
#[derive(Debug)]
pub struct ExportInput {
    pub meeting_id: String,
    pub title: String,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub folder: Option<String>,
    pub panel_id: String,
    pub panel: PanelContent,
    pub transcript: Option<Vec<repo::Utterance>>,
}

/// Reads a meeting into an [`ExportInput`].
pub fn gather(
    conn: &Connection,
    meeting_id: &str,
    include_transcript: bool,
) -> Result<ExportInput, ExportError> {
    let panels = repo::meeting_panels(conn, meeting_id)?;
    // The newest panel: exporting a stale summary after a regeneration is
    // exactly the surprise this feature must not produce.
    let panel = panels.first().ok_or(ExportError::NothingToExport)?;
    let content: PanelContent = serde_json::from_str(&panel.content_json)
        .map_err(|e| ExportError::Malformed(e.to_string()))?;

    let meeting = repo::list_meetings(conn, 10_000)?
        .into_iter()
        .find(|m| m.id == meeting_id)
        .ok_or(ExportError::NothingToExport)?;

    let folder: Option<String> = conn
        .query_row(
            "SELECT f.name FROM meetings m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )
        .ok();

    Ok(ExportInput {
        meeting_id: meeting_id.to_string(),
        title: meeting
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled meeting".to_string()),
        started_at: meeting.started_at,
        duration_ms: meeting.ended_at.map(|end| end - meeting.started_at),
        folder,
        panel_id: panel.id.clone(),
        panel: content,
        transcript: if include_transcript {
            Some(repo::meeting_utterances(conn, meeting_id)?)
        } else {
            None
        },
    })
}

/// Creates or updates the meeting's page.
pub async fn export(
    notion: &Notion,
    database: &super::client::Database,
    input: &ExportInput,
    existing: Option<&ExportRecord>,
) -> Result<ExportResult, ExportError> {
    let properties = shape::properties(
        &database.title_property,
        &database.properties,
        &input.title,
        input.started_at,
        input.duration_ms,
        input.folder.as_deref(),
        // Attendees come from the calendar event; not wired yet, so nothing is
        // sent rather than an empty column claiming nobody was there.
        &[],
    );

    let body = shape::blocks(&input.panel, input.transcript.as_deref());
    let total = body.len();
    let mut batches = shape::batches(body).into_iter();

    match target(existing, &database.id) {
        Some(page_id) => {
            notion.update_properties(page_id, properties).await?;

            // Replace rather than append: the summary was regenerated, and
            // appending would leave the old version above the new one on the
            // same page, which is worse than a duplicate page because it looks
            // like the meeting was discussed twice.
            for block in notion.children(page_id).await? {
                notion.delete_block(&block).await?;
            }
            for batch in batches {
                notion.append_children(page_id, batch).await?;
            }
            Ok(ExportResult {
                page_id: page_id.to_string(),
                created: false,
                blocks: total,
            })
        }
        None => {
            // The first batch goes with the page creation; the rest append.
            let first = batches.next().unwrap_or_default();
            let page_id = notion.create_page(&database.id, properties, first).await?;
            for batch in batches {
                notion.append_children(&page_id, batch).await?;
            }
            Ok(ExportResult {
                page_id,
                created: true,
                blocks: total,
            })
        }
    }
}

/// Records a completed export.
pub fn save(
    conn: &Connection,
    input: &ExportInput,
    database_id: &str,
    result: &ExportResult,
    now: i64,
) -> Result<(), DbError> {
    remember(
        conn,
        &input.meeting_id,
        database_id,
        &result.page_id,
        Some(&input.panel_id),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(database_id: &str, page_id: &str) -> ExportRecord {
        ExportRecord {
            meeting_id: "m1".into(),
            database_id: database_id.into(),
            page_id: page_id.into(),
            panel_id: Some("p1".into()),
            exported_at: 0,
        }
    }

    #[test]
    fn a_first_export_has_no_page_to_update() {
        assert_eq!(target(None, "db-1"), None);
    }

    #[test]
    fn a_second_export_updates_the_same_page() {
        // G26's done-when, reduced to one decision.
        let existing = record("db-1", "page-1");
        assert_eq!(target(Some(&existing), "db-1"), Some("page-1"));
    }

    #[test]
    fn changing_database_makes_a_new_page() {
        // The user repointed the integration. Updating a page in the old
        // database writes into a place they have stopped using, against a
        // schema that may no longer match.
        let existing = record("db-old", "page-1");
        assert_eq!(target(Some(&existing), "db-new"), None);
    }

    #[test]
    fn a_record_round_trips() {
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();

        assert!(record_for(conn, "m1").unwrap().is_none());
        remember(conn, "m1", "db-1", "page-1", Some("p1"), 10).unwrap();

        let found = record_for(conn, "m1").unwrap().unwrap();
        assert_eq!(found.page_id, "page-1");
        assert_eq!(found.panel_id.as_deref(), Some("p1"));
    }

    #[test]
    fn re_exporting_replaces_the_record_rather_than_colliding() {
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();

        remember(conn, "m1", "db-1", "page-1", Some("p1"), 10).unwrap();
        remember(conn, "m1", "db-1", "page-1", Some("p2"), 20).unwrap();

        let found = record_for(conn, "m1").unwrap().unwrap();
        assert_eq!(found.panel_id.as_deref(), Some("p2"));
        assert_eq!(found.exported_at, 20);
    }

    #[test]
    fn deleting_a_meeting_forgets_its_export() {
        // Otherwise a new meeting reusing the id would update a stranger's page.
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();
        remember(conn, "m1", "db-1", "page-1", None, 0).unwrap();

        repo::delete_meeting(conn, "m1").unwrap();
        assert!(record_for(conn, "m1").unwrap().is_none());
    }

    #[test]
    fn a_meeting_with_no_panel_cannot_be_exported() {
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();

        let err = gather(conn, "m1", false).unwrap_err();
        assert!(matches!(err, ExportError::NothingToExport));
    }

    #[test]
    fn gather_takes_the_newest_panel() {
        // Exporting a stale summary after a regeneration is exactly the
        // surprise this must not produce.
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();
        repo::ensure_builtin_templates(conn, &[("t", "T", "p")], 0).unwrap();

        let content = r#"{"sections":[{"heading":"H","bullets":[]}]}"#;
        repo::insert_panel(conn, "p-old", "m1", "t", content, "old", "ollama", "m", 10).unwrap();
        repo::insert_panel(conn, "p-new", "m1", "t", content, "new", "ollama", "m", 20).unwrap();

        let input = gather(conn, "m1", false).unwrap();
        assert_eq!(input.panel_id, "p-new");
    }

    #[test]
    fn gather_reads_the_folder_name_when_there_is_one() {
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "Standup", 0).unwrap();
        repo::ensure_builtin_templates(conn, &[("t", "T", "p")], 0).unwrap();
        repo::insert_panel(
            conn,
            "p1",
            "m1",
            "t",
            r#"{"sections":[]}"#,
            "",
            "ollama",
            "m",
            0,
        )
        .unwrap();

        let folder = repo::create_folder(conn, "Clients", None, 0).unwrap();
        repo::set_meeting_folder(conn, "m1", Some(&folder)).unwrap();

        assert_eq!(
            gather(conn, "m1", false).unwrap().folder.as_deref(),
            Some("Clients")
        );
    }

    #[test]
    fn an_untitled_meeting_gets_a_title_for_notion() {
        // Notion pages need a title; an empty one is an unreadable row.
        use crate::db::Database;
        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();
        repo::insert_meeting(conn, "m1", "   ", 0).unwrap();
        repo::ensure_builtin_templates(conn, &[("t", "T", "p")], 0).unwrap();
        repo::insert_panel(
            conn,
            "p1",
            "m1",
            "t",
            r#"{"sections":[]}"#,
            "",
            "ollama",
            "m",
            0,
        )
        .unwrap();

        assert_eq!(gather(conn, "m1", false).unwrap().title, "Untitled meeting");
    }

    // MARK: round trip against a stand-in Notion

    /// A minimal Notion, enough to prove create-then-update.
    ///
    /// Hand-rolled for the same reason the download tests were: what matters is
    /// *which* requests arrive and in what order, and a polished mock server
    /// makes that harder to assert, not easier.
    struct FakeNotion {
        addr: std::net::SocketAddr,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        _handle: std::thread::JoinHandle<()>,
    }

    impl FakeNotion {
        fn start() -> Self {
            use std::io::{BufRead, BufReader, Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = log.clone();

            let handle = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());

                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() {
                        continue;
                    }
                    let mut length = 0usize;
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                        line.clear();
                    }
                    let mut body = vec![0u8; length];
                    let _ = reader.read_exact(&mut body);

                    let request = request_line.trim().to_string();
                    sink.lock().unwrap().push(request.clone());

                    // Enough of a response for each endpoint the export uses.
                    let json = if request.starts_with("POST /v1/pages") {
                        r#"{"id":"page-1"}"#.to_string()
                    } else if request.contains("/children") && request.starts_with("GET") {
                        r#"{"results":[{"id":"block-old"}]}"#.to_string()
                    } else {
                        r#"{"ok":true}"#.to_string()
                    };

                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                        json.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            });

            Self {
                addr,
                log,
                _handle: handle,
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn requests(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    fn database() -> super::super::client::Database {
        super::super::client::Database {
            id: "db-1".into(),
            title: "Meetings".into(),
            title_property: "Name".into(),
            properties: vec!["Name".into(), "Date".into()],
        }
    }

    fn input() -> ExportInput {
        use crate::panel::content::{Bullet, Section};
        ExportInput {
            meeting_id: "m1".into(),
            title: "Standup".into(),
            started_at: 0,
            duration_ms: Some(600_000),
            folder: None,
            panel_id: "p1".into(),
            panel: PanelContent {
                sections: vec![Section {
                    heading: "Decisions".into(),
                    bullets: vec![Bullet {
                        text: "Ship Thursday".into(),
                        source_utterances: vec![],
                        from_note: None,
                    }],
                }],
            },
            transcript: None,
        }
    }

    #[tokio::test]
    async fn a_first_export_creates_a_page() {
        let server = FakeNotion::start();
        let notion = Notion::with_base_url(server.url(), "token");

        let result = export(&notion, &database(), &input(), None).await.unwrap();

        assert!(result.created);
        assert_eq!(result.page_id, "page-1");
        assert!(server
            .requests()
            .iter()
            .any(|r| r.starts_with("POST /v1/pages")));
    }

    #[tokio::test]
    async fn a_second_export_updates_the_same_page_and_creates_nothing() {
        // G26's done-when, end to end: regenerate, re-export, one page.
        let server = FakeNotion::start();
        let notion = Notion::with_base_url(server.url(), "token");
        let existing = record("db-1", "page-1");

        let result = export(&notion, &database(), &input(), Some(&existing))
            .await
            .unwrap();

        assert!(!result.created, "a second page was created");
        assert_eq!(result.page_id, "page-1");

        let requests = server.requests();
        assert!(
            !requests.iter().any(|r| r.starts_with("POST /v1/pages")),
            "the update path created a page: {requests:?}"
        );
        assert!(requests
            .iter()
            .any(|r| r.starts_with("PATCH /v1/pages/page-1")));
    }

    #[tokio::test]
    async fn re_export_clears_the_old_body_before_writing_the_new_one() {
        // Appending would leave the previous summary above the new one on the
        // same page — worse than a duplicate page, because it reads as if the
        // meeting was discussed twice.
        let server = FakeNotion::start();
        let notion = Notion::with_base_url(server.url(), "token");
        let existing = record("db-1", "page-1");

        export(&notion, &database(), &input(), Some(&existing))
            .await
            .unwrap();

        let requests = server.requests();
        let deleted = requests
            .iter()
            .position(|r| r.starts_with("DELETE /v1/blocks/block-old"));
        let appended = requests
            .iter()
            .position(|r| r.starts_with("PATCH /v1/blocks/page-1/children"));
        assert!(
            deleted.is_some(),
            "the old body was not removed: {requests:?}"
        );
        assert!(appended.is_some());
        assert!(
            deleted < appended,
            "wrote the new body before clearing the old"
        );
    }

    #[tokio::test]
    async fn a_repointed_database_gets_a_new_page() {
        let server = FakeNotion::start();
        let notion = Notion::with_base_url(server.url(), "token");
        let existing = record("db-old", "page-old");

        let result = export(&notion, &database(), &input(), Some(&existing))
            .await
            .unwrap();

        assert!(result.created);
        assert_eq!(result.page_id, "page-1");
    }

    #[tokio::test]
    async fn a_long_transcript_is_sent_in_several_requests() {
        // One request with thousands of blocks is rejected outright.
        let server = FakeNotion::start();
        let notion = Notion::with_base_url(server.url(), "token");

        let mut long = input();
        long.transcript = Some(
            (0..250)
                .map(|i| repo::Utterance {
                    id: i,
                    seq: i,
                    source: "mic".into(),
                    text: format!("line {i}"),
                    start_ms: i * 1_000,
                    end_ms: i * 1_000 + 500,
                    confidence: None,
                })
                .collect(),
        );

        let result = export(&notion, &database(), &long, None).await.unwrap();
        assert!(result.blocks > 250);

        let appends = server
            .requests()
            .iter()
            .filter(|r| r.contains("/children") && r.starts_with("PATCH"))
            .count();
        assert!(appends >= 2, "the body was not batched");
    }
}
