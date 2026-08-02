-- Widen the vector index to 768 dimensions.
--
-- 0002 guessed 384 (bge-small class) before an embedding model was chosen. The
-- model actually used — nomic-embed-text v1.5, running locally through Ollama —
-- produces 768. Truncating it to fit would work (it is Matryoshka-trained) but
-- 384 is not one of its supported sizes, so it would quietly cost recall for no
-- reason other than an early guess.
--
-- 0002 said changing the dimension means rebuilding this table and deserves its
-- own migration. This is that migration.
--
-- Existing vectors cannot be reinterpreted at a new width, so they are dropped.
-- Nothing is lost that cannot be regenerated: embeddings are derived data, and
-- the backfill re-embeds every meeting from the transcript and notes that remain
-- untouched.

DROP TABLE IF EXISTS embeddings;

CREATE VIRTUAL TABLE embeddings USING vec0 (
    owner_type TEXT,
    owner_id TEXT,
    embedding float[768]
);
