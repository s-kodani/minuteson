-- New imports publish decoded CAF directly to the canonical WAV. Keep the
-- legacy FLAC media type for completing an already staged version-1 import.
DROP TRIGGER audio_import_ready_assets;
CREATE TRIGGER audio_import_ready_assets BEFORE UPDATE OF state ON audio_imports
WHEN NEW.state='ready' AND NOT EXISTS (
 SELECT 1 FROM audio_import_commits p
 JOIN file_assets m ON m.id=p.manifest_asset_id AND m.meeting_id=p.meeting_id AND m.state='ready' AND m.content_hash=p.manifest_hash AND m.media_type='application/json'
 JOIN file_assets o ON o.id=NEW.original_asset_id AND o.meeting_id=p.meeting_id AND o.state='ready' AND o.content_hash=p.source_hash AND o.size_bytes=p.source_size
 JOIN file_assets a ON a.id=NEW.converted_asset_id AND a.meeting_id=p.meeting_id AND a.state='ready' AND a.media_type IN ('audio/flac','audio/x-caf')
 WHERE p.import_id=NEW.id AND p.meeting_id=NEW.meeting_id)
BEGIN SELECT RAISE(ABORT,'ready import requires verified scoped assets'); END;
