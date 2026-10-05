-- Portable source catalogs are historical metadata, never local profile heads.
-- Embedded origin UUIDs are provenance. Only this newly minted local ID is a PK.
CREATE TABLE model_catalog_history (
    id TEXT PRIMARY KEY NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(CAST(payload_json AS BLOB))<=33554432),
    payload_hash TEXT NOT NULL UNIQUE CHECK(length(payload_hash)=64),
    state TEXT NOT NULL CHECK(state='unavailable'),
    created_at INTEGER NOT NULL CHECK(created_at>=0)
);
CREATE TRIGGER model_catalog_history_fixed BEFORE UPDATE ON model_catalog_history
BEGIN SELECT RAISE(ABORT,'portable model catalog is immutable and unavailable'); END;
CREATE TRIGGER model_catalog_history_no_delete BEFORE DELETE ON model_catalog_history
BEGIN SELECT RAISE(ABORT,'portable model catalog history is retained'); END;
