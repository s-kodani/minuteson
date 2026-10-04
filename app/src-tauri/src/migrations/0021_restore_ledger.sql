-- Local operation state is not portable. Old keys below are provenance, not
-- primary keys to reuse when appending meetings from an archive.
CREATE TABLE restore_batches (
    batch_id TEXT PRIMARY KEY,
    archive_id TEXT NOT NULL,
    archive_fingerprint TEXT NOT NULL CHECK(length(archive_fingerprint)=64),
    mapping_hash TEXT NOT NULL CHECK(length(mapping_hash)=64),
    mapping_count INTEGER NOT NULL CHECK(mapping_count>=0),
    state TEXT NOT NULL CHECK(state IN ('prepared','published')),
    created_at INTEGER NOT NULL,
    published_at INTEGER,
    CHECK((state='published') = (published_at IS NOT NULL))
);
CREATE TABLE restore_identity_maps (
    batch_id TEXT NOT NULL REFERENCES restore_batches(batch_id),
    domain TEXT NOT NULL,
    source_scope TEXT NOT NULL,
    source_key TEXT NOT NULL,
    target_key TEXT NOT NULL,
    PRIMARY KEY(batch_id,domain,source_scope,source_key)
);
CREATE TRIGGER restore_batch_fixed BEFORE UPDATE ON restore_batches
WHEN NEW.batch_id != OLD.batch_id OR NEW.archive_id != OLD.archive_id
 OR NEW.archive_fingerprint != OLD.archive_fingerprint OR NEW.mapping_hash != OLD.mapping_hash
 OR NEW.mapping_count != OLD.mapping_count OR NEW.created_at != OLD.created_at
 OR OLD.state='published' OR NEW.state!='published'
BEGIN SELECT RAISE(ABORT, 'restore batch is immutable except publication'); END;
CREATE TRIGGER restore_batch_retained BEFORE DELETE ON restore_batches
BEGIN SELECT RAISE(ABORT, 'restore batch must be retained'); END;
CREATE TRIGGER restore_map_fixed BEFORE UPDATE ON restore_identity_maps
BEGIN SELECT RAISE(ABORT, 'restore identity map is immutable'); END;
CREATE TRIGGER restore_map_retained BEFORE DELETE ON restore_identity_maps
BEGIN SELECT RAISE(ABORT, 'restore identity map must be retained'); END;
CREATE TRIGGER restore_map_unpublished BEFORE INSERT ON restore_identity_maps
WHEN NOT EXISTS(SELECT 1 FROM restore_batches WHERE batch_id=NEW.batch_id AND state='prepared')
BEGIN SELECT RAISE(ABORT, 'restore identity map requires prepared batch'); END;

-- This history survives a further backup/restore. entity_key is scoped by
-- domain and meeting, and may describe a historical object already deleted.
CREATE TABLE restore_provenance (
    id TEXT PRIMARY KEY,
    meeting_id TEXT REFERENCES meetings(id),
    domain TEXT NOT NULL CHECK(length(domain) BETWEEN 1 AND 64),
    entity_key TEXT NOT NULL CHECK(length(CAST(entity_key AS BLOB)) BETWEEN 1 AND 256 AND instr(entity_key,char(0))=0),
    source_archive TEXT NOT NULL,
    source_scope TEXT NOT NULL CHECK(length(CAST(source_scope AS BLOB))<=256 AND instr(source_scope,char(0))=0),
    source_key TEXT NOT NULL CHECK(length(CAST(source_key AS BLOB)) BETWEEN 1 AND 256 AND instr(source_key,char(0))=0),
    restore_batch TEXT NOT NULL,
    restored_at INTEGER NOT NULL,
    UNIQUE(restore_batch,domain,source_scope,source_key,entity_key)
);
CREATE INDEX restore_provenance_meeting ON restore_provenance(meeting_id,domain,entity_key);
CREATE TRIGGER restore_provenance_fixed BEFORE UPDATE ON restore_provenance
BEGIN SELECT RAISE(ABORT, 'restore provenance is immutable'); END;
