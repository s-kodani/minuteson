-- Internal analysis snapshots are fixed generation inputs, not public history.
DROP TRIGGER minutes_input_committed;
CREATE TRIGGER minutes_input_committed BEFORE INSERT ON minutes_revisions
WHEN NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state IN ('committed', 'internal_snapshot'))
BEGIN SELECT RAISE(ABORT, 'minutes input is not fixed'); END;

DROP TRIGGER job_input_committed;
CREATE TRIGGER job_input_committed BEFORE INSERT ON processing_jobs
WHEN NEW.input_analysis_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state IN ('committed', 'internal_snapshot')
    AND content_hash = NEW.input_hash)
BEGIN SELECT RAISE(ABORT, 'job input analysis is not fixed'); END;

CREATE TABLE analysis_input_snapshots (
    analysis_id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    source_analysis_id TEXT NOT NULL,
    source_content_hash TEXT NOT NULL CHECK (length(source_content_hash) = 64),
    source_draft_json TEXT CHECK (source_draft_json IS NULL OR json_valid(source_draft_json)),
    working_content_hash TEXT NOT NULL CHECK (length(working_content_hash) = 64),
    meeting_title TEXT NOT NULL,
    meeting_created_at INTEGER NOT NULL,
    meeting_started_at_utc_ms INTEGER,
    FOREIGN KEY (meeting_id, analysis_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, source_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);
CREATE TRIGGER input_snapshot_valid BEFORE INSERT ON analysis_input_snapshots
WHEN NOT EXISTS (
    SELECT 1 FROM analysis_revisions a JOIN analysis_revisions p
    ON p.id = a.parent_revision_id AND p.meeting_id = a.meeting_id
    WHERE a.id = NEW.analysis_id AND a.meeting_id = NEW.meeting_id
    AND a.publication_state = 'internal_snapshot' AND a.origin = 'manual_snapshot'
    AND p.id = NEW.source_analysis_id AND p.content_hash = NEW.source_content_hash
    AND a.parent_content_hash = NEW.source_content_hash AND p.publication_state = 'committed')
BEGIN SELECT RAISE(ABORT, 'invalid input snapshot provenance'); END;
CREATE TRIGGER input_snapshot_fixed_update BEFORE UPDATE ON analysis_input_snapshots
BEGIN SELECT RAISE(ABORT, 'input snapshot metadata is immutable'); END;
CREATE TRIGGER input_snapshot_fixed_delete BEFORE DELETE ON analysis_input_snapshots
BEGIN SELECT RAISE(ABORT, 'input snapshot metadata cannot be deleted'); END;
CREATE TRIGGER analysis_internal_fixed_update BEFORE UPDATE ON analysis_revisions
WHEN OLD.publication_state = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis is immutable'); END;
CREATE TRIGGER analysis_internal_fixed_delete BEFORE DELETE ON analysis_revisions
WHEN OLD.publication_state = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis cannot be deleted'); END;

CREATE TRIGGER speakers_internal_insert BEFORE INSERT ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER speakers_internal_update BEFORE UPDATE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER speakers_internal_delete BEFORE DELETE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER segments_internal_insert BEFORE INSERT ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER segments_internal_update BEFORE UPDATE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER segments_internal_delete BEFORE DELETE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_insert BEFORE INSERT ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_update BEFORE UPDATE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER utterances_internal_delete BEFORE DELETE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_insert BEFORE INSERT ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_update BEFORE UPDATE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;

CREATE TRIGGER assignments_internal_delete BEFORE DELETE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'internal_snapshot'
BEGIN SELECT RAISE(ABORT, 'internal analysis content is immutable'); END;
