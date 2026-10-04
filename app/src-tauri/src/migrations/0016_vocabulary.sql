CREATE UNIQUE INDEX processing_jobs_meeting_identity ON processing_jobs(meeting_id, id);
CREATE TABLE vocabulary_scopes (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK(lock_version >= 0),
    updated_at INTEGER NOT NULL,
    CHECK ((id = 'global' AND meeting_id IS NULL) OR (meeting_id IS NOT NULL AND id = 'meeting:' || meeting_id))
);
CREATE TABLE vocabulary_entries (
    id TEXT PRIMARY KEY NOT NULL,
    scope_id TEXT NOT NULL REFERENCES vocabulary_scopes(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL CHECK(position >= 0),
    term TEXT NOT NULL CHECK(length(term) BETWEEN 1 AND 200),
    reading TEXT CHECK(reading IS NULL OR length(reading) BETWEEN 1 AND 200),
    note TEXT CHECK(note IS NULL OR length(note) BETWEEN 1 AND 1000),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    lock_version INTEGER NOT NULL CHECK(lock_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(scope_id, term),
    UNIQUE(scope_id, position)
);
CREATE TABLE vocabulary_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    job_id TEXT NOT NULL UNIQUE,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    created_at INTEGER NOT NULL,
    FOREIGN KEY(meeting_id, job_id) REFERENCES processing_jobs(meeting_id, id)
);
CREATE TRIGGER vocabulary_snapshot_no_update BEFORE UPDATE ON vocabulary_snapshots
BEGIN SELECT RAISE(ABORT, 'vocabulary snapshot is immutable'); END;
