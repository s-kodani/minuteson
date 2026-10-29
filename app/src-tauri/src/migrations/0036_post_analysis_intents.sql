-- A saved transcript and its follow-up intent commit together. No historical
-- meeting is opted into automatic work by this schema migration.
CREATE TABLE post_analysis_intents (
 id TEXT PRIMARY KEY,
 meeting_id TEXT NOT NULL REFERENCES meetings(id),
 analysis_id TEXT NOT NULL UNIQUE REFERENCES analysis_revisions(id),
 current_analysis_id TEXT NOT NULL REFERENCES analysis_revisions(id),
 working_hash TEXT NOT NULL CHECK(length(working_hash)=64),
 spec_json TEXT NOT NULL CHECK(json_valid(spec_json) AND length(CAST(spec_json AS BLOB))<=1048576),
 spec_hash TEXT NOT NULL CHECK(length(spec_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('completion_pending','completion_enqueued','matching_pending','matching_enqueued','blocked','completed','stale','restored')),
 completion_operation_id TEXT NOT NULL UNIQUE,
 matching_operation_id TEXT NOT NULL UNIQUE,
 completion_job_id TEXT REFERENCES processing_jobs(id),
 matching_job_id TEXT REFERENCES processing_jobs(id),
 error_message TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE TRIGGER post_analysis_intent_fixed BEFORE UPDATE ON post_analysis_intents
WHEN NEW.id!=OLD.id OR NEW.meeting_id!=OLD.meeting_id OR NEW.analysis_id!=OLD.analysis_id
 OR NEW.spec_json!=OLD.spec_json OR NEW.spec_hash!=OLD.spec_hash
 OR NEW.completion_operation_id!=OLD.completion_operation_id OR NEW.matching_operation_id!=OLD.matching_operation_id
BEGIN SELECT RAISE(ABORT,'follow-up intent inputs are immutable'); END;
