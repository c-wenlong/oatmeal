-- Where a meeting has been exported to.
--
-- One row per (meeting, destination). The Notion page id is the whole point:
-- G26's done-when is that regenerating a panel and re-exporting *updates* the
-- same page rather than creating a second one, and that is only possible if we
-- remember which page we made.
--
-- Keyed by meeting rather than by panel: a meeting has many regenerated panels
-- over its life and they all describe the same conversation. Exporting each to
-- its own page would litter the database with near-duplicates of one meeting.
CREATE TABLE notion_exports (
    meeting_id     TEXT PRIMARY KEY REFERENCES meetings (id) ON DELETE CASCADE,
    -- The Notion database the page lives in. Kept so a user who repoints the
    -- integration at a different database gets a new page rather than an
    -- update that fails against a schema which no longer matches.
    database_id    TEXT NOT NULL,
    page_id        TEXT NOT NULL,
    -- Which panel's content is currently on the page, so the UI can say
    -- "exported" versus "changed since export".
    panel_id       TEXT,
    exported_at    INTEGER NOT NULL
);

CREATE INDEX idx_notion_exports_page ON notion_exports (page_id);
