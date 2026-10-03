CREATE TABLE file_assets (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    storage_scope TEXT NOT NULL CHECK (storage_scope = 'meeting'),
    relative_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    hash_algorithm TEXT NOT NULL CHECK (hash_algorithm = 'sha256'),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    state TEXT NOT NULL CHECK (state IN ('ready', 'missing', 'corrupt', 'delete_pending')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (storage_scope, relative_path)
);
CREATE INDEX file_assets_meeting ON file_assets(meeting_id, state);
