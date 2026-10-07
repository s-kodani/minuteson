CREATE TABLE transcript_edit_recovery (
    draft_id TEXT PRIMARY KEY NOT NULL REFERENCES working_drafts(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    base_draft_lock_version INTEGER NOT NULL CHECK (base_draft_lock_version >= 0),
    base_draft_content_hash TEXT NOT NULL CHECK (length(base_draft_content_hash) = 64),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL CHECK (lock_version > 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
);

CREATE TRIGGER transcript_recovery_owner BEFORE INSERT ON transcript_edit_recovery
WHEN NOT EXISTS (SELECT 1 FROM working_drafts WHERE id = NEW.draft_id AND target_kind = 'transcript')
BEGIN SELECT RAISE(ABORT, 'Transcript recovery requires a transcript draft'); END;
