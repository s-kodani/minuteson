-- Local durable recovery intent. No meeting text, audio or portable state.
CREATE TABLE maintenance_operations (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT REFERENCES meetings(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('restore_addition','delete_meeting','delete_assets','move_storage','prune_revisions')),
    state TEXT NOT NULL CHECK(state IN ('prepared','installing','cleanup','recovered','completed','attention')),
    restore_batch_id TEXT UNIQUE REFERENCES restore_batches(batch_id),
    target_manifest_json TEXT NOT NULL CHECK(json_valid(target_manifest_json) AND length(CAST(target_manifest_json AS BLOB))<=65536),
    target_hash TEXT NOT NULL CHECK(length(target_hash)=64),
    consent_at INTEGER,
    progress_json TEXT NOT NULL CHECK(json_valid(progress_json) AND length(CAST(progress_json AS BLOB))<=4096),
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((kind='restore_addition') = (restore_batch_id IS NOT NULL))
);
CREATE INDEX maintenance_operations_state ON maintenance_operations(kind,state);
CREATE TABLE maintenance_paths (
    operation_id TEXT NOT NULL REFERENCES maintenance_operations(id),
    relative_path TEXT NOT NULL CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 256 AND instr(relative_path,char(0))=0),
    kind TEXT NOT NULL CHECK(kind IN ('file','directory','shared_directory')),
    size_bytes INTEGER CHECK(size_bytes IS NULL OR size_bytes>=0),
    content_hash TEXT CHECK(content_hash IS NULL OR length(content_hash)=64),
    identity_json TEXT CHECK(identity_json IS NULL OR (json_valid(identity_json) AND length(CAST(identity_json AS BLOB))<=1024)),
    identity_hash TEXT CHECK(identity_hash IS NULL OR length(identity_hash)=64),
    PRIMARY KEY(operation_id,relative_path),
    CHECK((kind='file') = (size_bytes IS NOT NULL)),
    CHECK((kind='file') = (content_hash IS NOT NULL)),
    CHECK((identity_json IS NULL) = (identity_hash IS NULL))
);
CREATE TRIGGER maintenance_operation_fixed BEFORE UPDATE ON maintenance_operations
WHEN NEW.id!=OLD.id OR NEW.kind!=OLD.kind
 OR NEW.restore_batch_id IS NOT OLD.restore_batch_id
 OR NEW.target_manifest_json!=OLD.target_manifest_json OR NEW.target_hash!=OLD.target_hash
 OR NEW.consent_at IS NOT OLD.consent_at OR NEW.created_at!=OLD.created_at
 OR (OLD.state='completed' AND NEW.state!='completed')
BEGIN SELECT RAISE(ABORT,'maintenance intent is immutable'); END;
CREATE TRIGGER maintenance_operation_retained BEFORE DELETE ON maintenance_operations
BEGIN SELECT RAISE(ABORT,'maintenance intent must be retained'); END;
CREATE TRIGGER maintenance_path_prepared BEFORE INSERT ON maintenance_paths
WHEN NOT EXISTS(SELECT 1 FROM maintenance_operations WHERE id=NEW.operation_id AND state='prepared')
BEGIN SELECT RAISE(ABORT,'maintenance path requires prepared intent'); END;
CREATE TRIGGER maintenance_path_fixed BEFORE UPDATE ON maintenance_paths
WHEN NEW.operation_id!=OLD.operation_id OR NEW.relative_path!=OLD.relative_path OR NEW.kind!=OLD.kind
 OR NEW.size_bytes IS NOT OLD.size_bytes OR NEW.content_hash IS NOT OLD.content_hash
 OR NOT EXISTS(SELECT 1 FROM maintenance_operations WHERE id=OLD.operation_id AND
   ((state='installing' AND OLD.identity_json IS NULL AND NEW.identity_json IS NOT NULL)
    OR (state='cleanup' AND NEW.identity_json IS NULL)))
BEGIN SELECT RAISE(ABORT,'maintenance target is immutable except owned identity'); END;
CREATE TRIGGER maintenance_path_retained BEFORE DELETE ON maintenance_paths
BEGIN SELECT RAISE(ABORT,'maintenance targets must be retained'); END;
