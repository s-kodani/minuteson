CREATE TABLE analysis_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    parent_revision_id TEXT,
    revision_no INTEGER NOT NULL CHECK (revision_no > 0),
    parent_content_hash TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('engine', 'manual_snapshot')),
    publication_state TEXT NOT NULL CHECK (publication_state IN ('building', 'candidate', 'committed', 'internal_snapshot')),
    audio_manifest_hash TEXT NOT NULL CHECK (length(audio_manifest_hash) = 64),
    profile_snapshot_json TEXT NOT NULL CHECK (json_valid(profile_snapshot_json)),
    config_json TEXT NOT NULL CHECK (json_valid(config_json)),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    created_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id),
    UNIQUE (meeting_id, revision_no),
    FOREIGN KEY (meeting_id, parent_revision_id) REFERENCES analysis_revisions(meeting_id, id)
);

CREATE TABLE minutes_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    input_analysis_id TEXT NOT NULL,
    parent_revision_id TEXT,
    revision_no INTEGER NOT NULL CHECK (revision_no > 0),
    parent_content_hash TEXT,
    publication_state TEXT NOT NULL CHECK (publication_state IN ('building', 'candidate', 'committed', 'internal_snapshot')),
    generated_document_json TEXT NOT NULL CHECK (json_valid(generated_document_json)),
    effective_document_json TEXT NOT NULL CHECK (json_valid(effective_document_json)),
    document_schema_version INTEGER NOT NULL CHECK (document_schema_version > 0),
    input_content_hash TEXT NOT NULL CHECK (length(input_content_hash) = 64),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    model_snapshot_json TEXT NOT NULL CHECK (json_valid(model_snapshot_json)),
    template_version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id),
    UNIQUE (meeting_id, revision_no),
    FOREIGN KEY (meeting_id, input_analysis_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, parent_revision_id) REFERENCES minutes_revisions(meeting_id, id)
);

ALTER TABLE meetings ADD COLUMN current_analysis_id TEXT REFERENCES analysis_revisions(id);
ALTER TABLE meetings ADD COLUMN previous_analysis_id TEXT REFERENCES analysis_revisions(id);
ALTER TABLE meetings ADD COLUMN current_minutes_id TEXT REFERENCES minutes_revisions(id);
ALTER TABLE meetings ADD COLUMN previous_minutes_id TEXT REFERENCES minutes_revisions(id);

CREATE TRIGGER analysis_parent_hash BEFORE INSERT ON analysis_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND
     (NEW.parent_content_hash IS NULL OR NEW.parent_content_hash !=
      (SELECT content_hash FROM analysis_revisions WHERE id = NEW.parent_revision_id AND meeting_id = NEW.meeting_id))
BEGIN SELECT RAISE(ABORT, 'analysis parent hash mismatch'); END;

CREATE TRIGGER analysis_parent_committed BEFORE INSERT ON analysis_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.parent_revision_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'analysis parent is not committed'); END;

CREATE TRIGGER minutes_parent_hash BEFORE INSERT ON minutes_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND
     (NEW.parent_content_hash IS NULL OR NEW.parent_content_hash !=
      (SELECT content_hash FROM minutes_revisions WHERE id = NEW.parent_revision_id AND meeting_id = NEW.meeting_id))
BEGIN SELECT RAISE(ABORT, 'minutes parent hash mismatch'); END;

CREATE TRIGGER minutes_parent_committed BEFORE INSERT ON minutes_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM minutes_revisions WHERE id = NEW.parent_revision_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'minutes parent is not committed'); END;

CREATE TRIGGER minutes_input_committed BEFORE INSERT ON minutes_revisions
WHEN NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'minutes input is not committed'); END;

CREATE TRIGGER minutes_input_hash BEFORE INSERT ON minutes_revisions
WHEN NEW.input_content_hash !=
     (SELECT content_hash FROM analysis_revisions WHERE id = NEW.input_analysis_id AND meeting_id = NEW.meeting_id)
BEGIN SELECT RAISE(ABORT, 'minutes input hash mismatch'); END;

CREATE TRIGGER analysis_committed_immutable BEFORE UPDATE ON analysis_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis is immutable'); END;
CREATE TRIGGER analysis_committed_no_delete BEFORE DELETE ON analysis_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis cannot be deleted'); END;
CREATE TRIGGER minutes_committed_immutable BEFORE UPDATE ON minutes_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes are immutable'); END;
CREATE TRIGGER minutes_committed_no_delete BEFORE DELETE ON minutes_revisions
WHEN OLD.publication_state = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes cannot be deleted'); END;

CREATE TRIGGER meetings_revision_heads_valid BEFORE UPDATE OF
    current_analysis_id, previous_analysis_id, current_minutes_id, previous_minutes_id ON meetings
BEGIN
    SELECT RAISE(ABORT, 'analysis heads must differ')
    WHERE NEW.current_analysis_id IS NOT NULL AND NEW.current_analysis_id = NEW.previous_analysis_id;
    SELECT RAISE(ABORT, 'minutes heads must differ')
    WHERE NEW.current_minutes_id IS NOT NULL AND NEW.current_minutes_id = NEW.previous_minutes_id;
    SELECT RAISE(ABORT, 'invalid current analysis')
    WHERE NEW.current_analysis_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM analysis_revisions WHERE id = NEW.current_analysis_id AND meeting_id = NEW.id AND publication_state = 'committed');
    SELECT RAISE(ABORT, 'invalid previous analysis')
    WHERE NEW.previous_analysis_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM analysis_revisions WHERE id = NEW.previous_analysis_id AND meeting_id = NEW.id AND publication_state = 'committed');
    SELECT RAISE(ABORT, 'invalid current minutes')
    WHERE NEW.current_minutes_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM minutes_revisions WHERE id = NEW.current_minutes_id AND meeting_id = NEW.id AND publication_state = 'committed');
    SELECT RAISE(ABORT, 'invalid previous minutes')
    WHERE NEW.previous_minutes_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM minutes_revisions WHERE id = NEW.previous_minutes_id AND meeting_id = NEW.id AND publication_state = 'committed');
END;
