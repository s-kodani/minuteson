CREATE TRIGGER job_retry_valid BEFORE INSERT ON processing_jobs
WHEN (NEW.retry_of_job_id IS NULL AND NEW.attempt != 1)
  OR (NEW.retry_of_job_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM processing_jobs p WHERE p.id = NEW.retry_of_job_id
      AND p.state IN ('failed', 'cancelled')
      AND p.meeting_id = NEW.meeting_id AND p.kind = NEW.kind
      AND p.input_analysis_id IS NEW.input_analysis_id AND p.input_hash = NEW.input_hash
      AND p.profile_snapshot_json = NEW.profile_snapshot_json
      AND p.parameters_json = NEW.parameters_json AND p.total_units IS NEW.total_units
      AND NEW.attempt = p.attempt + 1))
BEGIN SELECT RAISE(ABORT, 'retry inputs or source are not fixed'); END;

CREATE TRIGGER job_attempt_fixed BEFORE UPDATE OF attempt ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job retry attempt is immutable'); END;

CREATE TRIGGER job_total_units_fixed BEFORE UPDATE OF total_units ON processing_jobs
BEGIN SELECT RAISE(ABORT, 'job planned units are immutable'); END;

CREATE UNIQUE INDEX job_one_unfinished_retry ON processing_jobs(retry_of_job_id)
WHERE retry_of_job_id IS NOT NULL AND state IN ('queued', 'running', 'pausing', 'paused', 'interrupted');
