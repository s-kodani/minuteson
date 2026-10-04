CREATE TABLE export_records (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    minutes_revision_id TEXT REFERENCES minutes_revisions(id) ON DELETE SET NULL,
    request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
    exported_content_hash TEXT NOT NULL CHECK(length(exported_content_hash)=64),
    output_content_hash TEXT NOT NULL CHECK(length(output_content_hash)=64),
    output_size_bytes INTEGER NOT NULL CHECK(output_size_bytes >= 0),
    source_draft_id TEXT,
    source_draft_lock_version INTEGER,
    format TEXT NOT NULL CHECK(format='markdown'),
    destination_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('writing','succeeded','failed','interrupted')),
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    exported_at INTEGER,
    CHECK((source_draft_id IS NULL) = (source_draft_lock_version IS NULL)),
    CHECK(source_draft_lock_version IS NULL OR source_draft_lock_version >= 0),
    CHECK((state='succeeded') = (exported_at IS NOT NULL))
);
CREATE INDEX export_records_meeting ON export_records(meeting_id,created_at DESC,id);
CREATE TRIGGER export_minutes_meeting_insert BEFORE INSERT ON export_records
WHEN NEW.minutes_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM minutes_revisions WHERE id=NEW.minutes_revision_id AND meeting_id=NEW.meeting_id AND publication_state='committed'
)
BEGIN SELECT RAISE(ABORT,'export minutes belongs to another meeting or is not committed'); END;
CREATE TRIGGER export_minutes_meeting_update BEFORE UPDATE OF minutes_revision_id,meeting_id ON export_records
WHEN NEW.minutes_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM minutes_revisions WHERE id=NEW.minutes_revision_id AND meeting_id=NEW.meeting_id AND publication_state='committed'
)
BEGIN SELECT RAISE(ABORT,'export minutes belongs to another meeting or is not committed'); END;
