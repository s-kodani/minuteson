-- Explicitly edited assignments, including an explicitly selected unknown,
-- accompany the immutable input. They cannot be added after a job owns it.
CREATE TABLE diarization_manual_assignments (
    analysis_id TEXT NOT NULL REFERENCES analysis_input_snapshots(analysis_id),
    utterance_id TEXT NOT NULL,
    PRIMARY KEY (analysis_id, utterance_id),
    FOREIGN KEY (analysis_id, utterance_id) REFERENCES utterances(analysis_revision_id, id)
);
CREATE TRIGGER diarization_manual_insert BEFORE INSERT ON diarization_manual_assignments
WHEN EXISTS (SELECT 1 FROM processing_jobs WHERE input_analysis_id = NEW.analysis_id)
  OR NOT EXISTS (SELECT 1 FROM analysis_revisions WHERE id = NEW.analysis_id AND publication_state = 'internal_snapshot')
BEGIN SELECT RAISE(ABORT, 'diarization assignments are not a new fixed input'); END;
CREATE TRIGGER diarization_manual_update BEFORE UPDATE ON diarization_manual_assignments
BEGIN SELECT RAISE(ABORT, 'diarization assignments are immutable'); END;
CREATE TRIGGER diarization_manual_delete BEFORE DELETE ON diarization_manual_assignments
BEGIN SELECT RAISE(ABORT, 'diarization assignments cannot be deleted'); END;

DROP TRIGGER job_result_scope;
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
CREATE TRIGGER job_analysis_result_insert BEFORE INSERT ON processing_jobs
WHEN NEW.output_analysis_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'analysis result must be finalized atomically'); END;
