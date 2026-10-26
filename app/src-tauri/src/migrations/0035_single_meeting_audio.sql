-- The sole playable source; input tracks are retained only as provenance.
ALTER TABLE recording_sessions ADD COLUMN audio_policy TEXT NOT NULL DEFAULT 'single_wav_v1'
    CHECK(audio_policy IN ('legacy','single_wav_v1'));
UPDATE recording_sessions SET audio_policy='legacy';
ALTER TABLE audio_imports ADD COLUMN audio_policy TEXT NOT NULL DEFAULT 'single_wav_v1'
    CHECK(audio_policy IN ('legacy','single_wav_v1'));
UPDATE audio_imports SET audio_policy='legacy';
ALTER TABLE analysis_flows ADD COLUMN initial_replacement_json TEXT
    CHECK(initial_replacement_json IS NULL OR json_valid(initial_replacement_json));
CREATE TABLE meeting_audio_commits (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id),
    track_id TEXT NOT NULL REFERENCES audio_tracks(id),
    manifest_asset_id TEXT NOT NULL REFERENCES file_assets(id),
    manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
    frames INTEGER NOT NULL CHECK(frames>0),
    source_json TEXT NOT NULL CHECK(json_valid(source_json)),
    cleanup_json TEXT NOT NULL CHECK(json_valid(cleanup_json)),
    phase TEXT NOT NULL CHECK(phase IN ('committed','cleaned')),
    created_at INTEGER NOT NULL
);
CREATE TABLE audio_upgrade_progress (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id),
    phase TEXT NOT NULL CHECK(phase IN ('pending','complete')),
    restart_analysis INTEGER NOT NULL CHECK(restart_analysis IN (0,1)),
    replacement_flow_id TEXT,
    replacement_target_json TEXT CHECK(replacement_target_json IS NULL OR json_valid(replacement_target_json)),
    updated_at INTEGER NOT NULL
);
