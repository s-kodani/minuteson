ALTER TABLE meetings ADD COLUMN source_type TEXT CHECK (source_type IN ('recording', 'audio_import'));
CREATE UNIQUE INDEX file_assets_meeting_id_id ON file_assets(meeting_id, id);

CREATE TABLE recording_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'recording', 'stopping', 'saved', 'interrupted', 'failed')),
    capture_mode TEXT NOT NULL CHECK (capture_mode IN ('system', 'application')),
    capture_config_json TEXT NOT NULL CHECK (json_valid(capture_config_json)),
    start_operation_id TEXT NOT NULL UNIQUE,
    stop_operation_id TEXT UNIQUE,
    started_at_utc_ms INTEGER,
    ended_at_utc_ms INTEGER,
    duration_us INTEGER NOT NULL DEFAULT 0 CHECK (duration_us >= 0),
    audio_manifest_hash TEXT CHECK (audio_manifest_hash IS NULL OR length(audio_manifest_hash) = 64),
    last_committed_us INTEGER NOT NULL DEFAULT 0 CHECK (last_committed_us >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id)
);

CREATE TABLE audio_imports (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('copying', 'converting', 'ready', 'failed')),
    original_file_name TEXT NOT NULL,
    detected_format TEXT,
    duration_us INTEGER CHECK (duration_us IS NULL OR duration_us >= 0),
    original_asset_id TEXT,
    converted_asset_id TEXT,
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (meeting_id, id),
    FOREIGN KEY (meeting_id, original_asset_id) REFERENCES file_assets(meeting_id, id),
    FOREIGN KEY (meeting_id, converted_asset_id) REFERENCES file_assets(meeting_id, id)
);

CREATE TABLE audio_tracks (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE RESTRICT,
    session_id TEXT,
    import_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('mic', 'system', 'imported', 'processed')),
    format TEXT NOT NULL,
    sample_rate INTEGER NOT NULL CHECK (sample_rate > 0),
    channels INTEGER NOT NULL CHECK (channels > 0),
    state TEXT NOT NULL CHECK (state IN ('writing', 'ready', 'interrupted', 'failed')),
    device_label TEXT,
    processing_config_json TEXT CHECK (processing_config_json IS NULL OR json_valid(processing_config_json)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK ((session_id IS NOT NULL) != (import_id IS NOT NULL)),
    UNIQUE (meeting_id, id),
    FOREIGN KEY (meeting_id, session_id) REFERENCES recording_sessions(meeting_id, id),
    FOREIGN KEY (meeting_id, import_id) REFERENCES audio_imports(meeting_id, id)
);

CREATE TABLE audio_chunks (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    file_asset_id TEXT NOT NULL UNIQUE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    frame_count INTEGER NOT NULL CHECK (frame_count > 0),
    sample_rate INTEGER NOT NULL CHECK (sample_rate > 0),
    channels INTEGER NOT NULL CHECK (channels > 0),
    state TEXT NOT NULL CHECK (state IN ('ready', 'missing', 'corrupt')),
    created_at INTEGER NOT NULL,
    UNIQUE (track_id, id),
    UNIQUE (track_id, sequence),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (meeting_id, file_asset_id) REFERENCES file_assets(meeting_id, id)
);
CREATE INDEX audio_chunks_time ON audio_chunks(track_id, start_us);

CREATE TABLE track_clock_segments (
    id TEXT PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL,
    chunk_id TEXT NOT NULL,
    source_frame_start INTEGER NOT NULL CHECK (source_frame_start >= 0),
    source_frame_end INTEGER NOT NULL CHECK (source_frame_end > source_frame_start),
    timeline_start_us INTEGER NOT NULL CHECK (timeline_start_us >= 0),
    timeline_end_us INTEGER NOT NULL CHECK (timeline_end_us > timeline_start_us),
    mapping_json TEXT NOT NULL CHECK (json_valid(mapping_json)),
    FOREIGN KEY (track_id, chunk_id) REFERENCES audio_chunks(track_id, id)
);

CREATE TABLE audio_gaps (
    id TEXT PRIMARY KEY NOT NULL,
    track_id TEXT NOT NULL REFERENCES audio_tracks(id) ON DELETE RESTRICT,
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    reason_code TEXT NOT NULL,
    recovered INTEGER NOT NULL DEFAULT 0 CHECK (recovered IN (0, 1))
);

CREATE TABLE track_inputs (
    meeting_id TEXT NOT NULL,
    derived_track_id TEXT NOT NULL,
    input_track_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (derived_track_id, input_track_id, role),
    CHECK (derived_track_id != input_track_id),
    FOREIGN KEY (meeting_id, derived_track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (meeting_id, input_track_id) REFERENCES audio_tracks(meeting_id, id)
);

CREATE TRIGGER recording_source_match BEFORE INSERT ON recording_sessions
WHEN (SELECT source_type FROM meetings WHERE id = NEW.meeting_id) IS NOT 'recording'
BEGIN SELECT RAISE(ABORT, 'meeting is not a recording source'); END;
CREATE TRIGGER meeting_source_immutable BEFORE UPDATE OF source_type ON meetings
WHEN OLD.source_type IS NOT NULL AND NEW.source_type IS NOT OLD.source_type
BEGIN SELECT RAISE(ABORT, 'meeting source cannot change'); END;
CREATE TRIGGER import_source_match BEFORE INSERT ON audio_imports
WHEN (SELECT source_type FROM meetings WHERE id = NEW.meeting_id) IS NOT 'audio_import'
BEGIN SELECT RAISE(ABORT, 'meeting is not an import source'); END;
CREATE TRIGGER chunk_requires_ready_asset BEFORE INSERT ON audio_chunks
WHEN NOT EXISTS (SELECT 1 FROM file_assets WHERE id = NEW.file_asset_id AND meeting_id = NEW.meeting_id AND state = 'ready')
BEGIN SELECT RAISE(ABORT, 'chunk asset is not ready'); END;

CREATE TRIGGER track_inputs_no_cycle BEFORE INSERT ON track_inputs
WHEN EXISTS (
    WITH RECURSIVE inputs(id) AS (
        SELECT NEW.input_track_id
        UNION
        SELECT track_inputs.input_track_id FROM track_inputs JOIN inputs ON track_inputs.derived_track_id = inputs.id
    ) SELECT 1 FROM inputs WHERE id = NEW.derived_track_id
)
BEGIN SELECT RAISE(ABORT, 'track input cycle'); END;
CREATE TRIGGER track_inputs_no_update BEFORE UPDATE ON track_inputs
BEGIN SELECT RAISE(ABORT, 'replace track inputs by delete and insert'); END;
