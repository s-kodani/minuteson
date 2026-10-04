-- Import provenance is separate from a recording session. Old audio_imports
-- rows stay intact; they are not silently upgraded to verified imports.
CREATE TABLE audio_import_commits (
    import_id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE,
    source_size INTEGER NOT NULL CHECK(source_size > 0),
    source_hash TEXT NOT NULL CHECK(length(source_hash)=64),
    info_json TEXT NOT NULL CHECK(json_valid(info_json) AND length(CAST(info_json AS BLOB))<=8192),
    manifest_asset_id TEXT REFERENCES file_assets(id),
    manifest_hash TEXT CHECK(manifest_hash IS NULL OR length(manifest_hash)=64),
    error_message TEXT,
    CHECK((manifest_asset_id IS NULL)=(manifest_hash IS NULL)),
    FOREIGN KEY(meeting_id,import_id) REFERENCES audio_imports(meeting_id,id)
);
UPDATE audio_imports SET state='failed',error_code='interrupted' WHERE state IN ('copying','converting');
CREATE UNIQUE INDEX audio_import_single_active ON audio_imports((1)) WHERE state IN ('copying','converting');
CREATE TRIGGER audio_import_identity_fixed BEFORE UPDATE ON audio_import_commits
WHEN NEW.import_id!=OLD.import_id OR NEW.meeting_id!=OLD.meeting_id
 OR NEW.operation_id!=OLD.operation_id OR NEW.source_size!=OLD.source_size
 OR NEW.source_hash!=OLD.source_hash OR NEW.info_json!=OLD.info_json
 OR (OLD.manifest_asset_id IS NOT NULL AND (NEW.manifest_asset_id IS NOT OLD.manifest_asset_id OR NEW.manifest_hash IS NOT OLD.manifest_hash))
BEGIN SELECT RAISE(ABORT,'import provenance is immutable'); END;
CREATE TRIGGER audio_import_ready_fixed BEFORE UPDATE ON audio_imports
WHEN OLD.state='ready'
BEGIN SELECT RAISE(ABORT,'ready import is immutable'); END;
CREATE TRIGGER audio_import_ready_assets BEFORE UPDATE OF state ON audio_imports
WHEN NEW.state='ready' AND NOT EXISTS (
 SELECT 1 FROM audio_import_commits p
 JOIN file_assets m ON m.id=p.manifest_asset_id AND m.meeting_id=p.meeting_id AND m.state='ready' AND m.content_hash=p.manifest_hash AND m.media_type='application/json'
 JOIN file_assets o ON o.id=NEW.original_asset_id AND o.meeting_id=p.meeting_id AND o.state='ready' AND o.content_hash=p.source_hash AND o.size_bytes=p.source_size
 JOIN file_assets a ON a.id=NEW.converted_asset_id AND a.meeting_id=p.meeting_id AND a.state='ready' AND a.media_type='audio/flac'
 WHERE p.import_id=NEW.id AND p.meeting_id=NEW.meeting_id)
BEGIN SELECT RAISE(ABORT,'ready import requires verified scoped assets'); END;
CREATE TRIGGER audio_import_recording_exclusive BEFORE INSERT ON audio_imports
WHEN EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping'))
BEGIN SELECT RAISE(ABORT,'recording is active'); END;
CREATE TRIGGER recording_audio_import_exclusive BEFORE INSERT ON recording_sessions
WHEN EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting'))
BEGIN SELECT RAISE(ABORT,'audio import is active'); END;
CREATE TRIGGER imported_track_kind BEFORE INSERT ON audio_tracks
WHEN NEW.import_id IS NOT NULL AND NEW.kind NOT IN ('imported','processed')
BEGIN SELECT RAISE(ABORT,'import does not provide microphone or system tracks'); END;
CREATE TRIGGER imported_track_kind_update BEFORE UPDATE OF import_id,kind ON audio_tracks
WHEN NEW.import_id IS NOT NULL AND NEW.kind NOT IN ('imported','processed')
BEGIN SELECT RAISE(ABORT,'import does not provide microphone or system tracks'); END;
