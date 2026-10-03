ALTER TABLE processing_jobs ADD COLUMN output_analysis_id TEXT REFERENCES analysis_revisions(id) ON DELETE RESTRICT;
ALTER TABLE processing_jobs ADD COLUMN pause_reason TEXT CHECK (pause_reason IN ('user', 'recording', 'exit'));

CREATE TRIGGER job_result_scope BEFORE UPDATE OF output_analysis_id ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL AND (
    NEW.kind != 'transcription' OR NEW.state != 'succeeded' OR NOT EXISTS (
        SELECT 1 FROM analysis_revisions WHERE id = NEW.output_analysis_id
            AND meeting_id = NEW.meeting_id AND audio_manifest_hash = NEW.input_hash))
BEGIN SELECT RAISE(ABORT, 'job result is not a matching analysis'); END;
CREATE TRIGGER job_result_fixed BEFORE UPDATE OF output_analysis_id ON processing_jobs
WHEN OLD.output_analysis_id IS NOT NULL AND NEW.output_analysis_id IS NOT OLD.output_analysis_id
BEGIN SELECT RAISE(ABORT, 'job result cannot change'); END;
