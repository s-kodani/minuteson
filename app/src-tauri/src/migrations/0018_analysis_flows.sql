-- A durable recording -> transcription -> diarization flow. Public heads and
-- edits are never changed by this coordinator; adoption remains explicit.
CREATE TABLE analysis_flows (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id),
    stage TEXT NOT NULL CHECK(stage IN ('pending','configuration','transcription','diarization','attention','review')),
    plan_json TEXT CHECK(plan_json IS NULL OR (json_valid(plan_json) AND length(CAST(plan_json AS BLOB)) <= 65536)),
    plan_hash TEXT CHECK(plan_hash IS NULL OR length(plan_hash)=64),
    transcription_job_id TEXT REFERENCES processing_jobs(id),
    diarization_job_id TEXT REFERENCES processing_jobs(id),
    error_message TEXT,
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK(lock_version>=0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((plan_json IS NULL) = (plan_hash IS NULL)),
    CHECK(stage NOT IN ('transcription','diarization','review') OR plan_json IS NOT NULL),
    CHECK(stage NOT IN ('diarization','review') OR transcription_job_id IS NOT NULL),
    CHECK(stage != 'review' OR diarization_job_id IS NOT NULL)
);
CREATE TRIGGER analysis_flow_plan_fixed BEFORE UPDATE ON analysis_flows
WHEN NEW.id != OLD.id OR NEW.meeting_id != OLD.meeting_id
 OR (OLD.plan_json IS NOT NULL AND (NEW.plan_json IS NOT OLD.plan_json OR NEW.plan_hash IS NOT OLD.plan_hash))
BEGIN SELECT RAISE(ABORT, 'analysis flow identity and fixed plan are immutable'); END;
CREATE TRIGGER analysis_flow_job_scope BEFORE UPDATE ON analysis_flows
WHEN (NEW.transcription_job_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM processing_jobs WHERE id=NEW.transcription_job_id AND meeting_id=NEW.meeting_id AND kind='transcription'))
 OR (NEW.diarization_job_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM processing_jobs WHERE id=NEW.diarization_job_id AND meeting_id=NEW.meeting_id AND kind='diarization'))
BEGIN SELECT RAISE(ABORT, 'analysis flow job has wrong kind or meeting'); END;

DROP TRIGGER input_snapshot_valid;
CREATE TRIGGER input_snapshot_valid BEFORE INSERT ON analysis_input_snapshots
WHEN NOT EXISTS (
 SELECT 1 FROM analysis_revisions a JOIN analysis_revisions p
 ON p.id=a.parent_revision_id AND p.meeting_id=a.meeting_id
 WHERE a.id=NEW.analysis_id AND a.meeting_id=NEW.meeting_id
 AND a.publication_state='internal_snapshot' AND a.origin='manual_snapshot'
 AND p.id=NEW.source_analysis_id AND p.content_hash=NEW.source_content_hash
 AND a.parent_content_hash=NEW.source_content_hash
 AND (p.publication_state='committed' OR (p.publication_state='candidate' AND EXISTS (
   SELECT 1 FROM analysis_flows f JOIN processing_jobs t ON t.id=f.transcription_job_id
   WHERE f.meeting_id=NEW.meeting_id AND t.meeting_id=f.meeting_id
   AND t.kind='transcription' AND t.state='succeeded' AND t.output_analysis_id=p.id
   AND t.input_hash=p.audio_manifest_hash))))
BEGIN SELECT RAISE(ABORT, 'invalid input snapshot provenance'); END;

DROP TRIGGER analysis_parent_committed;
CREATE TRIGGER analysis_parent_committed BEFORE INSERT ON analysis_revisions
WHEN NEW.parent_revision_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM analysis_revisions p WHERE p.id=NEW.parent_revision_id
 AND p.meeting_id=NEW.meeting_id AND (p.publication_state='committed' OR (
  p.publication_state='candidate' AND EXISTS (
   SELECT 1 FROM analysis_flows f JOIN processing_jobs t ON t.id=f.transcription_job_id
   WHERE f.meeting_id=NEW.meeting_id AND t.meeting_id=f.meeting_id
   AND t.kind='transcription' AND t.state='succeeded' AND t.output_analysis_id=p.id
   AND t.input_hash=p.audio_manifest_hash)
  AND ((NEW.origin='manual_snapshot' AND NEW.publication_state='building'
        AND json_extract(NEW.config_json,'$.inputSnapshot.sourceAnalysisId')=p.id)
    OR (NEW.origin='engine' AND NEW.publication_state='candidate' AND EXISTS (
      SELECT 1 FROM processing_jobs d JOIN analysis_input_snapshots s ON s.analysis_id=d.input_analysis_id
      WHERE d.id=json_extract(NEW.config_json,'$.jobId') AND d.meeting_id=NEW.meeting_id
      AND d.kind='diarization' AND d.state='running' AND s.source_analysis_id=p.id
      AND d.input_analysis_id=json_extract(NEW.config_json,'$.inputAnalysisId')
      AND d.input_hash=json_extract(NEW.config_json,'$.inputContentHash')))))))
BEGIN SELECT RAISE(ABORT, 'analysis parent is not committed or a verified flow source'); END;
