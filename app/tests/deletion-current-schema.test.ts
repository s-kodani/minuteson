import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src-tauri/src/storage/deletion/${name}`, import.meta.url), "utf8");

test("deletion history admits only the current schema and retains per-connection protections", () => {
  const code = source("database.rs");
  assert.doesNotMatch(code, /known_schemas|historical_scope_fixture|historical_schema_hash_fixture|legacy_without_journal/);
  assert.match(code, /version != crate::storage::MIGRATIONS\.len\(\)/);
  assert.match(code, /actual\.hash == hash/);
  assert.match(code, /actual != expected\.objects/);
  for (const protection of ["verify_temp_schema(db)", "PRAGMA foreign_keys", "PRAGMA foreign_key_check", "SQLITE_DBCONFIG_ENABLE_TRIGGER"]) assert(code.includes(protection));
  assert.match(code, /OnceLock<Result<KnownSchema, String>>/);
  assert.doesNotMatch(code, /OnceLock<Result<Vec<KnownSchema>/);
});
test("missing deletion journals cannot be bypassed by legacy audio admission", () => {
  const code = source("audio_completion.rs");
  assert.doesNotMatch(code, /legacy_without_journal/);
  assert.match(code, /if !has_table \{\s*return Err\("現行DBの音声削除台帳が欠落/);
  for (const file of ["intent.rs", "audio_intent.rs", "completion.rs", "audio_removal.rs", "removal.rs"]) assert.doesNotMatch(source(file), /historical_completed_fixture|historical_progress_fixture|historical_manifest_fixture/);
});
