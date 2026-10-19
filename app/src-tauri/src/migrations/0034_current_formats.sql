-- A fresh database is required. These triggers prevent unsupported persisted
-- work from bypassing the same contracts used by the native job readers.
CREATE TRIGGER current_job_format BEFORE INSERT ON processing_jobs
WHEN (NEW.kind='transcription' AND json_extract(NEW.parameters_json,'$.schemaVersion') IS NOT 4)
 OR (NEW.kind='diarization' AND json_extract(NEW.parameters_json,'$.schemaVersion') IS NOT 3)
 OR (NEW.kind='minutes_generation' AND json_extract(NEW.parameters_json,'$.generation.schemaVersion') IS NOT 9)
 OR (NEW.kind IN ('voice_registration','voice_matching') AND json_extract(NEW.parameters_json,'$.voice.schemaVersion') IS NOT 4)
 OR (NEW.kind='speaker_completion' AND json_extract(NEW.parameters_json,'$.completion.schemaVersion') IS NOT 2)
BEGIN SELECT RAISE(ABORT, 'unsupported processing job format'); END;

CREATE TABLE model_reference_files (
 metadata_hash TEXT PRIMARY KEY NOT NULL CHECK(length(metadata_hash)=64),
 metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER reject_historical_model_package BEFORE INSERT ON model_packages
WHEN NEW.source_type='historical_snapshot'
BEGIN SELECT RAISE(ABORT,'unsupported historical model package'); END;
CREATE TRIGGER immutable_model_reference_update BEFORE UPDATE ON model_reference_files
BEGIN SELECT RAISE(ABORT,'immutable model reference'); END;
CREATE TRIGGER immutable_model_reference_delete BEFORE DELETE ON model_reference_files
BEGIN SELECT RAISE(ABORT,'immutable model reference'); END;
