-- Keep the original retry invariants. The sole profile difference permitted
-- is a runtimeModel proof on a native retry; Rust validates its full typed
-- provenance and current destination records before the INSERT and on reads.
DROP TRIGGER job_retry_valid;
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

-- A restored interrupted parent can acquire a reviewed child. Its completed
-- history must never advance independently once that retry exists.
CREATE TRIGGER job_parent_retry_no_resume BEFORE UPDATE OF state ON processing_jobs
WHEN NEW.state = 'queued' AND OLD.state IN ('paused', 'interrupted')
  AND EXISTS(SELECT 1 FROM processing_jobs c WHERE c.retry_of_job_id = OLD.id)
BEGIN SELECT RAISE(ABORT, 'retry parent cannot resume'); END;
