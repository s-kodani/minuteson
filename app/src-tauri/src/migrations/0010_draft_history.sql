-- Preserve draft IDs and edit chains while allowing an inactive draft per base.
-- There remains only one active draft of each kind per meeting.
DROP TRIGGER transcript_draft_base;
DROP TRIGGER minutes_draft_base;
DROP TRIGGER draft_base_fixed;
DROP TRIGGER edit_operation_chain;
ALTER TABLE edit_operations RENAME TO legacy_edit_operations;
ALTER TABLE working_drafts RENAME TO legacy_working_drafts;

CREATE TABLE working_drafts (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('transcript', 'minutes')),
    base_analysis_id TEXT,
    base_minutes_id TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK (lock_version >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    CHECK ((target_kind = 'transcript' AND base_analysis_id IS NOT NULL AND base_minutes_id IS NULL)
        OR (target_kind = 'minutes' AND base_minutes_id IS NOT NULL AND base_analysis_id IS NULL)),
    UNIQUE (meeting_id, target_kind, base_analysis_id),
    UNIQUE (meeting_id, target_kind, base_minutes_id),
    FOREIGN KEY (meeting_id, base_analysis_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, base_minutes_id) REFERENCES minutes_revisions(meeting_id, id)
);
CREATE UNIQUE INDEX one_active_draft ON working_drafts(meeting_id, target_kind) WHERE active = 1;
INSERT INTO working_drafts (id, meeting_id, target_kind, base_analysis_id, base_minutes_id, payload_json, schema_version, content_hash, lock_version, created_at, updated_at)
SELECT id, meeting_id, target_kind, base_analysis_id, base_minutes_id, payload_json, schema_version, content_hash, lock_version, created_at, updated_at FROM legacy_working_drafts;

CREATE TABLE edit_operations (
    id TEXT PRIMARY KEY NOT NULL,
    draft_id TEXT NOT NULL REFERENCES working_drafts(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    operation_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    inverse_json TEXT NOT NULL CHECK (json_valid(inverse_json)),
    before_hash TEXT NOT NULL CHECK (length(before_hash) = 64),
    after_hash TEXT NOT NULL CHECK (length(after_hash) = 64),
    created_at INTEGER NOT NULL,
    UNIQUE (draft_id, sequence)
);
INSERT INTO edit_operations SELECT * FROM legacy_edit_operations;
DROP TABLE legacy_edit_operations;
DROP TABLE legacy_working_drafts;

CREATE TRIGGER transcript_draft_base BEFORE INSERT ON working_drafts
WHEN NEW.target_kind = 'transcript' AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.base_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'transcript draft requires committed analysis'); END;
CREATE TRIGGER minutes_draft_base BEFORE INSERT ON working_drafts
WHEN NEW.target_kind = 'minutes' AND NOT EXISTS (
    SELECT 1 FROM minutes_revisions WHERE id = NEW.base_minutes_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'minutes draft requires committed minutes'); END;
CREATE TRIGGER draft_base_fixed BEFORE UPDATE OF meeting_id, target_kind, base_analysis_id, base_minutes_id ON working_drafts
BEGIN SELECT RAISE(ABORT, 'draft base must be rebased by a separate operation'); END;
CREATE TRIGGER edit_operation_chain BEFORE INSERT ON edit_operations
WHEN NEW.before_hash IS NOT (SELECT content_hash FROM working_drafts WHERE id = NEW.draft_id)
  OR NEW.sequence IS NOT (SELECT COALESCE(MAX(sequence), 0) + 1 FROM edit_operations WHERE draft_id = NEW.draft_id)
BEGIN SELECT RAISE(ABORT, 'draft edit chain is stale'); END;
