CREATE TABLE audio_import_options (
    import_id TEXT PRIMARY KEY REFERENCES audio_import_commits(import_id),
    options_json TEXT NOT NULL CHECK(json_valid(options_json) AND length(CAST(options_json AS BLOB))<=1048576),
    options_hash TEXT NOT NULL CHECK(length(options_hash)=64)
);
INSERT INTO audio_import_options SELECT import_id,
 '{"profile":"standard","meetingVocabulary":[],"expectedGlobalVersion":null}',
 '8617ffc347c013dde1e355e4d3c68742779e5e01a9ed9d9692b086a5cbce4c4e'
 FROM audio_import_commits;
CREATE TRIGGER audio_import_options_fixed BEFORE UPDATE ON audio_import_options
BEGIN SELECT RAISE(ABORT,'initial import options are immutable'); END;
