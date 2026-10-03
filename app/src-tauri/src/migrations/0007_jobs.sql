CREATE TABLE processing_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('transcription', 'diarization', 'transcript_cleanup', 'minutes_generation')),
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
    finished_at INTEGER,
    CHECK (total_units IS NULL OR completed_units <= total_units),
    FOREIGN KEY (meeting_id, input_analysis_id) REFERENCES analysis_revisions(meeting_id, id)
);
CREATE INDEX processing_jobs_meeting_created ON processing_jobs(meeting_id, created_at);
CREATE INDEX processing_jobs_state_created ON processing_jobs(state, created_at);
CREATE UNIQUE INDEX processing_jobs_one_active ON processing_jobs((1)) WHERE state IN ('running', 'pausing');

CREATE TABLE job_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES processing_jobs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    completed_units INTEGER NOT NULL CHECK (completed_units >= 0),
    checkpoint_json TEXT NOT NULL CHECK (json_valid(checkpoint_json)),
    checkpoint_hash TEXT NOT NULL CHECK (length(checkpoint_hash) = 64),
    created_at INTEGER NOT NULL,
    UNIQUE (job_id, sequence)
);

CREATE TRIGGER job_input_committed BEFORE INSERT ON processing_jobs
WHEN NEW.input_analysis_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM analysis_revisions WHERE id = NEW.input_analysis_id
    AND meeting_id = NEW.meeting_id AND publication_state = 'committed'
    AND content_hash = NEW.input_hash)
BEGIN SELECT RAISE(ABORT, 'job input analysis is not fixed'); END;
CREATE TRIGGER job_spec_fixed BEFORE UPDATE OF meeting_id, kind, operation_id, retry_of_job_id,
    input_analysis_id, input_hash, profile_snapshot_json, parameters_json, spec_hash ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job inputs are immutable'); END;
CREATE TRIGGER job_checkpoint_progress BEFORE INSERT ON job_checkpoints
WHEN NEW.completed_units < (SELECT completed_units FROM processing_jobs WHERE id = NEW.job_id)
  OR NEW.sequence IS NOT (SELECT COALESCE(MAX(sequence), 0) + 1 FROM job_checkpoints WHERE job_id = NEW.job_id)
BEGIN SELECT RAISE(ABORT, 'job checkpoint is stale'); END;
