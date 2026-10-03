CREATE TABLE app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK (lock_version > 0),
    updated_at INTEGER NOT NULL
);
