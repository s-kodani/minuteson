ALTER TABLE processing_jobs ADD COLUMN output_minutes_id TEXT REFERENCES minutes_revisions(id) ON DELETE RESTRICT;
CREATE TRIGGER job_minutes_result_scope BEFORE UPDATE OF output_minutes_id, state ON processing_jobs
WHEN NEW.output_minutes_id IS NOT NULL AND (
    NEW.kind != 'minutes_generation' OR NEW.state != 'succeeded' OR NOT EXISTS (
        SELECT 1 FROM minutes_revisions WHERE id = NEW.output_minutes_id
        AND meeting_id = NEW.meeting_id AND input_analysis_id = NEW.input_analysis_id
        AND input_content_hash = NEW.input_hash))
BEGIN SELECT RAISE(ABORT, 'job result is not matching minutes'); END;
CREATE TRIGGER job_minutes_result_insert BEFORE INSERT ON processing_jobs
WHEN NEW.output_minutes_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'minutes result must be finalized atomically'); END;
CREATE TRIGGER job_minutes_result_fixed BEFORE UPDATE OF output_minutes_id ON processing_jobs
WHEN OLD.output_minutes_id IS NOT NULL AND NEW.output_minutes_id IS NOT OLD.output_minutes_id
BEGIN SELECT RAISE(ABORT, 'minutes result cannot change'); END;
