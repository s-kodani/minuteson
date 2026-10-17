-- Person identities are independent of meeting-local speaker labels.
CREATE TABLE voice_people (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 200),
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    created_at INTEGER NOT NULL
);
CREATE TABLE voice_samples (
    id TEXT PRIMARY KEY NOT NULL,
    person_id TEXT NOT NULL REFERENCES voice_people(id) ON DELETE CASCADE,
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    extractor_version TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK(dimension BETWEEN 1 AND 4096),
    vector BLOB NOT NULL CHECK(length(vector)=dimension*4),
    source_meeting_id TEXT,
    source_analysis_id TEXT,
    source_track_id TEXT,
    start_us INTEGER NOT NULL CHECK(start_us>=0),
    end_us INTEGER NOT NULL CHECK(end_us>start_us),
    created_at INTEGER NOT NULL
);
CREATE INDEX voice_samples_person ON voice_samples(person_id,model_hash);
CREATE TABLE voice_matches (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id),
    analysis_revision_id TEXT NOT NULL,
    speaker_id TEXT NOT NULL,
    person_id TEXT REFERENCES voice_people(id) ON DELETE SET NULL,
    name_snapshot TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('candidate','automatic','confirmed','dismissed')),
    similarity REAL CHECK(similarity IS NULL OR similarity BETWEEN -1 AND 1),
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    policy_version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(analysis_revision_id,speaker_id),
    FOREIGN KEY(analysis_revision_id,speaker_id) REFERENCES speakers(analysis_revision_id,id),
    FOREIGN KEY(meeting_id,analysis_revision_id) REFERENCES analysis_revisions(meeting_id,id)
);
-- No automatic identity policy ships without independent real-voice evaluation.
CREATE TABLE voice_policies (
    id TEXT PRIMARY KEY NOT NULL,
    model_hash TEXT NOT NULL CHECK(length(model_hash)=64),
    extractor_version TEXT NOT NULL,
    threshold REAL NOT NULL CHECK(threshold BETWEEN -1 AND 1),
    margin REAL NOT NULL CHECK(margin > 0 AND margin <= 2),
    evaluation_hash TEXT NOT NULL CHECK(length(evaluation_hash)=64),
    created_at INTEGER NOT NULL
);

-- Rebuild the CHECK constraint while retaining every job, FK, index and guard.
-- Upgrade callers disable FKs outside their transaction, verify them before
-- commit and restore enforcement. A new empty DB also supports this SQL.
PRAGMA legacy_alter_table=ON;
CREATE TABLE mon_voice_processing_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('transcription', 'diarization', 'transcript_cleanup', 'minutes_generation', 'voice_registration', 'voice_matching')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'pausing', 'paused', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    operation_id TEXT NOT NULL UNIQUE,
    retry_of_job_id TEXT REFERENCES processing_jobs(id) ON DELETE RESTRICT,
    input_analysis_id TEXT,
    input_hash TEXT NOT NULL CHECK (length(input_hash) = 64),
    profile_snapshot_json TEXT NOT NULL CHECK (json_valid(profile_snapshot_json)),
    parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)),
    spec_hash TEXT NOT NULL CHECK (length(spec_hash) = 64),
    completed_units INTEGER NOT NULL DEFAULT 0 CHECK (completed_units >= 0),
    total_units INTEGER CHECK (total_units IS NULL OR total_units >= 0),
    error_code TEXT,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER, output_analysis_id TEXT REFERENCES analysis_revisions(id) ON DELETE RESTRICT, pause_reason TEXT CHECK (pause_reason IN ('user', 'recording', 'exit')), output_minutes_id TEXT REFERENCES minutes_revisions(id) ON DELETE RESTRICT,
    CHECK (total_units IS NULL OR completed_units <= total_units),
    FOREIGN KEY (meeting_id, input_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);
INSERT INTO mon_voice_processing_jobs SELECT * FROM processing_jobs;
DROP TABLE processing_jobs;
ALTER TABLE mon_voice_processing_jobs RENAME TO processing_jobs;
CREATE UNIQUE INDEX job_one_unfinished_retry ON processing_jobs(retry_of_job_id)
WHERE retry_of_job_id IS NOT NULL AND state IN ('queued', 'running', 'pausing', 'paused', 'interrupted');
CREATE INDEX processing_jobs_meeting_created ON processing_jobs(meeting_id, created_at);
CREATE UNIQUE INDEX processing_jobs_meeting_identity ON processing_jobs(meeting_id, id);
CREATE UNIQUE INDEX processing_jobs_one_active ON processing_jobs((1)) WHERE state IN ('running', 'pausing');
CREATE INDEX processing_jobs_state_created ON processing_jobs(state, created_at);
CREATE TRIGGER job_analysis_result_insert BEFORE INSERT ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'analysis result must be finalized atomically'); END;
CREATE TRIGGER job_attempt_fixed BEFORE UPDATE OF attempt ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job retry attempt is immutable'); END;
CREATE TRIGGER job_input_committed BEFORE INSERT ON processing_jobs
WHEN NEW.input_analysis_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state IN ('committed', 'internal_snapshot')
    AND content_hash = NEW.input_hash)
BEGIN SELECT RAISE(ABORT, 'job input analysis is not fixed'); END;
CREATE TRIGGER job_minutes_result_fixed BEFORE UPDATE OF output_minutes_id ON processing_jobs
WHEN OLD.output_minutes_id IS NOT NULL AND NEW.output_minutes_id IS NOT OLD.output_minutes_id
BEGIN SELECT RAISE(ABORT, 'minutes result cannot change'); END;
CREATE TRIGGER job_minutes_result_insert BEFORE INSERT ON processing_jobs
WHEN NEW.output_minutes_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'minutes result must be finalized atomically'); END;
CREATE TRIGGER job_minutes_result_scope BEFORE UPDATE OF output_minutes_id, state ON processing_jobs
WHEN NEW.output_minutes_id IS NOT NULL AND (
    NEW.kind != 'minutes_generation' OR NEW.state != 'succeeded' OR NOT EXISTS (
        SELECT 1 FROM minutes_revisions WHERE id = NEW.output_minutes_id
        AND meeting_id = NEW.meeting_id AND input_analysis_id = NEW.input_analysis_id
        AND input_content_hash = NEW.input_hash))
BEGIN SELECT RAISE(ABORT, 'job result is not matching minutes'); END;
CREATE TRIGGER job_parent_retry_no_resume BEFORE UPDATE OF state ON processing_jobs
WHEN NEW.state = 'queued' AND OLD.state IN ('paused', 'interrupted')
  AND EXISTS(SELECT 1 FROM processing_jobs c WHERE c.retry_of_job_id = OLD.id)
BEGIN SELECT RAISE(ABORT, 'retry parent cannot resume'); END;
CREATE TRIGGER job_result_fixed BEFORE UPDATE OF output_analysis_id ON processing_jobs
WHEN OLD.output_analysis_id IS NOT NULL AND NEW.output_analysis_id IS NOT OLD.output_analysis_id
BEGIN SELECT RAISE(ABORT, 'job result cannot change'); END;
CREATE TRIGGER job_result_scope BEFORE UPDATE OF output_analysis_id, state ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL AND (
    NEW.state != 'succeeded' OR NOT (
        (NEW.kind = 'transcription' AND EXISTS (
            SELECT 1 FROM analysis_revisions a WHERE a.id = NEW.output_analysis_id
            AND a.meeting_id = NEW.meeting_id AND a.audio_manifest_hash = NEW.input_hash))
        OR (NEW.kind = 'diarization' AND EXISTS (
            SELECT 1 FROM analysis_revisions a
            JOIN analysis_revisions i ON i.id = NEW.input_analysis_id
            JOIN analysis_input_snapshots s ON s.analysis_id = i.id
            WHERE a.id = NEW.output_analysis_id AND a.meeting_id = NEW.meeting_id
            AND i.meeting_id = NEW.meeting_id AND s.meeting_id = NEW.meeting_id
            AND i.publication_state = 'internal_snapshot' AND i.content_hash = NEW.input_hash
            AND a.audio_manifest_hash = i.audio_manifest_hash
            AND a.parent_revision_id = s.source_analysis_id AND a.parent_content_hash = s.source_content_hash
            AND json_extract(a.config_json,'$.jobId') = NEW.id
            AND json_extract(a.config_json,'$.specHash') = NEW.spec_hash
            AND json_extract(a.config_json,'$.inputAnalysisId') = NEW.input_analysis_id
            AND json_extract(a.config_json,'$.inputContentHash') = NEW.input_hash))
    ))
BEGIN SELECT RAISE(ABORT, 'job result is not a matching analysis'); END;
CREATE TRIGGER job_retry_valid BEFORE INSERT ON processing_jobs
WHEN (NEW.retry_of_job_id IS NULL AND (NEW.attempt != 1
      OR json_type(NEW.profile_snapshot_json, '$.runtimeModel') IS NOT NULL))
  OR (NEW.retry_of_job_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM processing_jobs p WHERE p.id = NEW.retry_of_job_id
      AND (p.state IN ('failed', 'cancelled') OR (
        p.state = 'interrupted' AND p.error_code = 'restored_preparation_required'
        AND NEW.kind IN ('transcription', 'diarization', 'minutes_generation')
        AND json_type(NEW.profile_snapshot_json, '$.runtimeModel') = 'object'
        AND json_extract(NEW.profile_snapshot_json, '$.runtimeModel.schemaVersion') = 1))
      AND p.meeting_id = NEW.meeting_id AND p.kind = NEW.kind
      AND p.input_analysis_id IS NEW.input_analysis_id AND p.input_hash = NEW.input_hash
      AND (p.profile_snapshot_json = NEW.profile_snapshot_json OR (
        NEW.kind IN ('transcription', 'diarization', 'minutes_generation')
        AND json_type(NEW.profile_snapshot_json, '$.runtimeModel') = 'object'
        AND json_extract(NEW.profile_snapshot_json, '$.runtimeModel.schemaVersion') = 1
        AND json_remove(p.profile_snapshot_json, '$.runtimeModel') =
            json_remove(NEW.profile_snapshot_json, '$.runtimeModel')))
      AND p.parameters_json = NEW.parameters_json AND p.total_units IS NEW.total_units
      AND NEW.attempt = p.attempt + 1))
BEGIN SELECT RAISE(ABORT, 'retry inputs or source are not fixed'); END;
CREATE TRIGGER job_spec_fixed BEFORE UPDATE OF meeting_id, kind, operation_id, retry_of_job_id,
    input_analysis_id, input_hash, profile_snapshot_json, parameters_json, spec_hash ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job inputs are immutable'); END;
CREATE TRIGGER job_total_units_fixed BEFORE UPDATE OF total_units ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job planned units are immutable'); END;
PRAGMA legacy_alter_table=OFF;
