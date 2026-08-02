-- Vector index, backed by the sqlite-vec extension.
--
-- Separated from 0001 because it is the one part of the schema that fails if
-- the extension isn't registered, and because changing the embedding dimension
-- means dropping and rebuilding this table — which deserves its own migration
-- rather than a silent edit to the initial one.
--
-- 384 dimensions targets bge-small / MiniLM class models (SPEC section 16, G16).
-- Changing EMBEDDING_DIM in Rust without adding a migration here will fail loudly
-- on insert, which is the intent.

CREATE VIRTUAL TABLE embeddings USING vec0 (
    owner_type TEXT,
    owner_id TEXT,
    embedding float[384]
);
