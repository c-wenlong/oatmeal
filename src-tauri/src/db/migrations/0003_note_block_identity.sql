-- Stable identity for note blocks.
--
-- 0001 keyed blocks by `seq` alone. That breaks as soon as someone inserts a
-- line in the middle of their notes: every block below shifts down a seq, so an
-- upsert-by-seq would hand each one the *previous* occupant's
-- `first_typed_at_ms`. The temporal linker (SPEC section 7) keys on exactly that
-- timestamp, so the result would be notes anchored to the wrong moment in the
-- transcript — silently, and only visibly wrong much later.
--
-- `block_id` is assigned by the editor and never reused. `seq` stays, but now
-- only describes display order.

ALTER TABLE note_blocks ADD COLUMN block_id TEXT;

-- Existing rows (none in practice; this ships before any notes exist) get an
-- id derived from their current position so the column can be relied on.
UPDATE note_blocks SET block_id = 'legacy-' || id WHERE block_id IS NULL;

CREATE UNIQUE INDEX idx_note_blocks_block_id ON note_blocks (meeting_id, block_id);

-- `seq` is now a display ordinal that shifts freely, so it must not carry a
-- uniqueness constraint that a reorder would violate mid-update. SQLite cannot
-- drop a column constraint in place, so the table is rebuilt.
PRAGMA foreign_keys = OFF;

CREATE TABLE note_blocks_new (
    id                 INTEGER PRIMARY KEY,
    meeting_id         TEXT NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    block_id           TEXT NOT NULL,
    seq                INTEGER NOT NULL,
    text               TEXT NOT NULL,
    -- The anchor the temporal linker keys on. Set once, on first keystroke, and
    -- never rewritten on edit.
    first_typed_at_ms  INTEGER,
    last_edited_at_ms  INTEGER,
    UNIQUE (meeting_id, block_id)
);

INSERT INTO note_blocks_new
    (id, meeting_id, block_id, seq, text, first_typed_at_ms, last_edited_at_ms)
SELECT id, meeting_id, block_id, seq, text, first_typed_at_ms, last_edited_at_ms
FROM note_blocks;

DROP TABLE note_blocks;
ALTER TABLE note_blocks_new RENAME TO note_blocks;

CREATE INDEX idx_note_blocks_meeting ON note_blocks (meeting_id, seq);

PRAGMA foreign_keys = ON;

-- The FTS table and its triggers referenced the dropped table, so rebuild them.
DROP TRIGGER IF EXISTS note_blocks_fts_ai;
DROP TRIGGER IF EXISTS note_blocks_fts_ad;
DROP TRIGGER IF EXISTS note_blocks_fts_au;
DROP TABLE IF EXISTS note_blocks_fts;

CREATE VIRTUAL TABLE note_blocks_fts USING fts5 (
    text,
    content = 'note_blocks',
    content_rowid = 'id',
    tokenize = 'porter unicode61'
);

INSERT INTO note_blocks_fts (rowid, text) SELECT id, text FROM note_blocks;

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
