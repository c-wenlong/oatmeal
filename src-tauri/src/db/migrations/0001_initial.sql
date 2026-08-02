-- Oatmeal initial schema. Mirrors docs/SPEC.md section 8.
--
-- Structural rule that everything else follows from: a meeting owns ONE
-- immutable transcript and N regenerable panels. Regenerating a summary or
-- switching templates must never be able to touch `utterances` or `note_blocks`.
--
-- Timestamps are epoch milliseconds (INTEGER). Times measured from the start of
-- a recording are suffixed `_ms` and are relative, not absolute.

-- ---------------------------------------------------------------- organisation

CREATE TABLE folders (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    parent_id  TEXT REFERENCES folders (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_folders_parent ON folders (parent_id);

-- ------------------------------------------------------------------- calendar

CREATE TABLE calendar_events (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,        -- google | microsoft
    external_id      TEXT NOT NULL,
    title            TEXT,
    starts_at        INTEGER NOT NULL,
    ends_at          INTEGER,
    conferencing_url TEXT,
    attendees_json   TEXT NOT NULL DEFAULT '[]',
    synced_at        INTEGER NOT NULL,
    UNIQUE (provider, external_id)
);

CREATE INDEX idx_calendar_events_starts_at ON calendar_events (starts_at);

-- ------------------------------------------------------------------- meetings

CREATE TABLE meetings (
    id                TEXT PRIMARY KEY,
    title             TEXT,
    folder_id         TEXT REFERENCES folders (id) ON DELETE SET NULL,
    started_at        INTEGER NOT NULL,
    ended_at          INTEGER,
    -- idle | armed | recording | processing | complete | failed
    status            TEXT NOT NULL DEFAULT 'recording',
    calendar_event_id TEXT REFERENCES calendar_events (id) ON DELETE SET NULL,
    trigger_source    TEXT NOT NULL DEFAULT 'manual',   -- calendar | mic | manual
    audio_path        TEXT,
    -- NULL means "keep forever"; the retention sweeper (G27) deletes the file
    -- once past this, and only the file — transcript and notes survive.
    audio_expires_at  INTEGER
);

CREATE INDEX idx_meetings_started_at ON meetings (started_at DESC);
CREATE INDEX idx_meetings_folder ON meetings (folder_id);
CREATE INDEX idx_meetings_audio_expiry ON meetings (audio_expires_at)
    WHERE audio_expires_at IS NOT NULL;

-- ----------------------------------------------------------------- transcript

-- Append-only. Nothing downstream may UPDATE or DELETE rows here.
CREATE TABLE utterances (
    id         INTEGER PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    -- Speaker attribution comes from the capture stream, not a diarization
    -- model: 'mic' is the user, 'system' is everyone else.
    source     TEXT NOT NULL CHECK (source IN ('mic', 'system')),
    text       TEXT NOT NULL,
    start_ms   INTEGER NOT NULL,
    end_ms     INTEGER NOT NULL,
    confidence REAL,
    UNIQUE (meeting_id, seq)
);

CREATE INDEX idx_utterances_meeting_time ON utterances (meeting_id, start_ms);

-- ---------------------------------------------------------------------- notes

CREATE TABLE note_blocks (
    id                 INTEGER PRIMARY KEY,
    meeting_id         TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    seq                INTEGER NOT NULL,
    text               TEXT NOT NULL,
    -- Relative to meeting start. `first_typed_at_ms` is the anchor the temporal
    -- linker keys on, so it must never be rewritten on edit.
    first_typed_at_ms  INTEGER,
    last_edited_at_ms  INTEGER,
    UNIQUE (meeting_id, seq)
);

CREATE INDEX idx_note_blocks_meeting ON note_blocks (meeting_id, seq);

-- Which transcript spans a note block came from. One block may link to several
-- utterances by several methods; `score` and `method` are kept so link quality
-- can be measured and tuned (G17) rather than guessed at.
CREATE TABLE note_links (
    id            INTEGER PRIMARY KEY,
    note_block_id INTEGER NOT NULL REFERENCES note_blocks (id) ON DELETE CASCADE,
    utterance_id  INTEGER NOT NULL REFERENCES utterances (id) ON DELETE CASCADE,
    method        TEXT NOT NULL CHECK (method IN ('temporal', 'semantic', 'llm')),
    score         REAL NOT NULL,
    UNIQUE (note_block_id, utterance_id, method)
);

CREATE INDEX idx_note_links_block ON note_links (note_block_id);
CREATE INDEX idx_note_links_utterance ON note_links (utterance_id);

-- ------------------------------------------------------------------- output

CREATE TABLE templates (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    prompt             TEXT NOT NULL,
    output_schema_json TEXT,
    is_builtin         INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL
);

-- A regenerable view over a meeting. Deleting every panel must leave the
-- transcript and notes intact.
CREATE TABLE panels (
    id           TEXT PRIMARY KEY,
    meeting_id   TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    template_id  TEXT REFERENCES templates (id) ON DELETE SET NULL,
    content_json TEXT NOT NULL,
    -- Denormalised for FTS; the JSON above stays the source of truth.
    plaintext    TEXT NOT NULL DEFAULT '',
    -- Which model produced this, so the privacy panel (G27) can report
    -- local-vs-cloud provenance per generation rather than per app.
    provider     TEXT,
    model        TEXT,
    generated_at INTEGER NOT NULL
);

CREATE INDEX idx_panels_meeting ON panels (meeting_id, generated_at DESC);

-- Every generated claim points at what it came from. Rows are only written
-- after the referenced ids are validated against the DB (SPEC section 7,
-- layer 3) — an unresolvable citation is dropped, never rendered.
CREATE TABLE panel_citations (
    id            INTEGER PRIMARY KEY,
    panel_id      TEXT NOT NULL REFERENCES panels (id) ON DELETE CASCADE,
    block_path    TEXT NOT NULL,
    utterance_id  INTEGER REFERENCES utterances (id) ON DELETE CASCADE,
    note_block_id INTEGER REFERENCES note_blocks (id) ON DELETE CASCADE,
    CHECK (utterance_id IS NOT NULL OR note_block_id IS NOT NULL)
);

CREATE INDEX idx_panel_citations_panel ON panel_citations (panel_id);

-- ------------------------------------------------------------------ settings

CREATE TABLE providers (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,   -- anthropic | openai | openrouter | ollama | lmstudio | bundled
    base_url     TEXT NOT NULL,
    model        TEXT,
    -- Reference into the macOS Keychain. Never the key itself.
    keychain_ref TEXT,
    is_default   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

-- Mic-activation detection is closed by default: an app produces a popup only
-- with an explicit 'allow' row here. A dictation tool the user marked 'ignore'
-- must never prompt again.
CREATE TABLE detection_rules (
    id         TEXT PRIMARY KEY,
    bundle_id  TEXT NOT NULL UNIQUE,
    app_name   TEXT,
    mode       TEXT NOT NULL CHECK (mode IN ('allow', 'ignore')),
    created_at INTEGER NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- ---------------------------------------------------------------- full text

-- External-content FTS: the base tables own the data, these mirror it. Kept in
-- sync by triggers so no write path can forget to update the index.
CREATE VIRTUAL TABLE utterances_fts USING fts5 (
    text,
    content = 'utterances',
    content_rowid = 'id',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER utterances_fts_ai AFTER INSERT ON utterances BEGIN
    INSERT INTO utterances_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER utterances_fts_ad AFTER DELETE ON utterances BEGIN
    INSERT INTO utterances_fts (utterances_fts, rowid, text)
    VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER utterances_fts_au AFTER UPDATE ON utterances BEGIN
    INSERT INTO utterances_fts (utterances_fts, rowid, text)
    VALUES ('delete', old.id, old.text);
    INSERT INTO utterances_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE VIRTUAL TABLE note_blocks_fts USING fts5 (
    text,
    content = 'note_blocks',
    content_rowid = 'id',
    tokenize = 'porter unicode61'
);

CREATE TRIGGER note_blocks_fts_ai AFTER INSERT ON note_blocks BEGIN
    INSERT INTO note_blocks_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER note_blocks_fts_ad AFTER DELETE ON note_blocks BEGIN
    INSERT INTO note_blocks_fts (note_blocks_fts, rowid, text)
    VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER note_blocks_fts_au AFTER UPDATE ON note_blocks BEGIN
    INSERT INTO note_blocks_fts (note_blocks_fts, rowid, text)
    VALUES ('delete', old.id, old.text);
    INSERT INTO note_blocks_fts (rowid, text) VALUES (new.id, new.text);
END;

-- Panels use a contentless-adjacent setup keyed by rowid because `panels.id` is
-- a TEXT uuid and FTS5 external content requires an INTEGER rowid.
CREATE VIRTUAL TABLE panels_fts USING fts5 (
    panel_id UNINDEXED,
    plaintext,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER panels_fts_ai AFTER INSERT ON panels BEGIN
    INSERT INTO panels_fts (panel_id, plaintext) VALUES (new.id, new.plaintext);
END;

CREATE TRIGGER panels_fts_ad AFTER DELETE ON panels BEGIN
    DELETE FROM panels_fts WHERE panel_id = old.id;
END;

CREATE TRIGGER panels_fts_au AFTER UPDATE ON panels BEGIN
    DELETE FROM panels_fts WHERE panel_id = old.id;
    INSERT INTO panels_fts (panel_id, plaintext) VALUES (new.id, new.plaintext);
END;
