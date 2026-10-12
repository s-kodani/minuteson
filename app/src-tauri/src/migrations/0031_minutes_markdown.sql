-- One mutable Markdown document per meeting, not a revision/history store.
CREATE TABLE minutes_markdown (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    markdown TEXT NOT NULL CHECK (length(CAST(markdown AS BLOB)) <= 2097152),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK (lock_version BETWEEN 1 AND 9007199254740991),
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
    base_lock_version INTEGER NOT NULL CHECK (base_lock_version BETWEEN 0 AND 9007199254740990),
    base_content_hash TEXT CHECK (base_content_hash IS NULL OR length(base_content_hash) = 64),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK (lock_version = base_lock_version + 1),
    CHECK ((base_lock_version = 0) = (base_content_hash IS NULL))
);
