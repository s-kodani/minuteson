-- Registered audio belongs to the registry, never to a source meeting.
CREATE TABLE voice_sample_audio (
    id TEXT PRIMARY KEY NOT NULL REFERENCES voice_samples(id) ON DELETE CASCADE,
    relative_path TEXT,
    size_bytes INTEGER,
    content_hash TEXT,
    frame_count INTEGER,
    state TEXT NOT NULL CHECK(state IN ('pending','ready','unavailable','failed')),
    error_message TEXT,
    CHECK(state='ready' OR (relative_path IS NULL AND size_bytes IS NULL AND content_hash IS NULL AND frame_count IS NULL)),
    CHECK(state!='ready' OR (relative_path IS NOT NULL AND size_bytes IS NOT NULL AND size_bytes>=44 AND content_hash IS NOT NULL AND length(content_hash)=64 AND frame_count IS NOT NULL AND frame_count>0))
);
-- Local recovery receipts are deliberately excluded from portable backups.
CREATE TABLE voice_audio_operations (
    id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    temporary_path TEXT,
    identity_json TEXT,
    content_hash TEXT,
    size_bytes INTEGER,
    kind TEXT NOT NULL CHECK(kind IN ('install','delete'))
);
INSERT INTO voice_sample_audio(id,state) SELECT id,'pending' FROM voice_samples;
