-- Historical metadata is not evidence that any model is locally available.
CREATE TABLE model_packages (
    id TEXT PRIMARY KEY NOT NULL,
    engine TEXT NOT NULL,
    model_name TEXT NOT NULL,
    model_version TEXT NOT NULL,
    quantization TEXT,
    source_type TEXT NOT NULL CHECK(source_type IN ('download','manual_import','historical_snapshot')),
    manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json) AND length(CAST(manifest_json AS BLOB))<=65536),
    manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
    license_ref TEXT,
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible')),
    UNIQUE(source_type,manifest_hash)
);
CREATE TABLE model_files (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    relative_path TEXT NOT NULL,
    role TEXT NOT NULL,
    expected_size INTEGER CHECK(expected_size IS NULL OR expected_size>=0),
    actual_size INTEGER NOT NULL CHECK(actual_size>=0),
    expected_hash TEXT CHECK(expected_hash IS NULL OR length(expected_hash)=64),
    actual_hash TEXT NOT NULL CHECK(length(actual_hash)=64),
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible')),
    UNIQUE(package_id,relative_path),
    CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 4096 AND instr(relative_path,char(0))=0
      AND substr(relative_path,1,1)!='/' AND substr(relative_path,-1,1)!='/' AND instr(relative_path,char(92))=0
      AND instr('/'||relative_path||'/','/../')=0 AND instr('/'||relative_path||'/','/./')=0
      AND instr(relative_path,'//')=0 AND relative_path NOT GLOB '[A-Za-z]:*')
);
CREATE INDEX model_files_fingerprint ON model_files(actual_hash,actual_size,role);
CREATE TABLE model_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK(name IN ('standard','high_accuracy')),
    profile_version INTEGER NOT NULL CHECK(profile_version>0),
    state TEXT NOT NULL CHECK(state IN ('unavailable','preparing','ready','incompatible'))
);
CREATE TABLE profile_components (
    profile_id TEXT NOT NULL REFERENCES model_profiles(id),
    role TEXT NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    settings_json TEXT NOT NULL CHECK(json_valid(settings_json) AND length(CAST(settings_json AS BLOB))<=65536),
    PRIMARY KEY(profile_id,role)
);
CREATE TRIGGER historical_model_package_fixed BEFORE UPDATE ON model_packages
WHEN OLD.source_type='historical_snapshot' AND
 (NEW.id!=OLD.id OR NEW.engine!=OLD.engine OR NEW.model_name!=OLD.model_name OR NEW.model_version!=OLD.model_version
  OR NEW.quantization IS NOT OLD.quantization OR NEW.source_type!=OLD.source_type OR NEW.manifest_json!=OLD.manifest_json
  OR NEW.manifest_hash!=OLD.manifest_hash OR NEW.license_ref IS NOT OLD.license_ref OR NEW.state!='unavailable')
BEGIN SELECT RAISE(ABORT,'historical model metadata is immutable and unavailable'); END;
CREATE TRIGGER historical_model_file_fixed BEFORE UPDATE ON model_files
WHEN EXISTS(SELECT 1 FROM model_packages WHERE id=OLD.package_id AND source_type='historical_snapshot')
BEGIN SELECT RAISE(ABORT,'historical model file metadata is immutable'); END;
CREATE TABLE restore_omitted_assets (
    asset_id TEXT PRIMARY KEY NOT NULL REFERENCES file_assets(id),
    meeting_id TEXT NOT NULL REFERENCES meetings(id),
    reason TEXT NOT NULL CHECK(reason IN ('recreatable_pcm','native_log')),
    source_size INTEGER NOT NULL CHECK(source_size>=0),
    source_hash TEXT NOT NULL CHECK(length(source_hash)=64)
);
CREATE TRIGGER restore_omitted_asset_insert BEFORE INSERT ON restore_omitted_assets
WHEN NOT EXISTS(SELECT 1 FROM file_assets WHERE id=NEW.asset_id AND meeting_id=NEW.meeting_id
 AND state='missing' AND size_bytes=NEW.source_size AND content_hash=NEW.source_hash)
BEGIN SELECT RAISE(ABORT,'restore exclusion requires matching missing asset'); END;
CREATE TRIGGER restore_omitted_asset_fixed BEFORE UPDATE ON restore_omitted_assets
BEGIN SELECT RAISE(ABORT,'restore exclusion proof is immutable'); END;
