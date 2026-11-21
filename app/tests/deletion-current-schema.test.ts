import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src-tauri/src/storage/deletion/${name}`, import.meta.url), "utf8");

test("deletion requires checksum-selected SQL objects and fences prior39 admission to read-only", () => {
  const code = source("database.rs");
  assert.doesNotMatch(code, /known_schemas|historical_scope_fixture|historical_schema_hash_fixture|legacy_without_journal/);
  const adapter = readFileSync(new URL("../src-tauri/src/storage/schema_40.rs", import.meta.url), "utf8");
  assert.match(code, /schema_40::current_layout\(db\)/);
  assert.match(code, /Ok\(layout\) => current_schema_for\(layout\)\?/);
  assert.match(code, /verify_readonly_previous\(db\)/);
  assert.match(code, /readonly_schema39\(layout\)\?/);
  const readOnly = adapter.slice(adapter.indexOf("pub(super) fn verify_readonly_previous"), adapter.indexOf("pub(super) fn deletion_v1_schema_hash"));
  assert.match(readOnly, /PRAGMA query_only/);
  assert.match(readOnly, /return Err/);
  assert.match(readOnly, /verify_previous\(db\)/);
  assert.match(adapter, /objects\(db\)\? != objects\(&expected\)\?/);
  assert.match(adapter, /trigger\.3\.replacen\(NEW_PREDICATE, OLD_PREDICATE, 1\)/);
  assert.match(code, /inspect_schema\(db\)\?\.hash != hash/);
  assert.match(code, /actual != expected\.objects/);
  for (const protection of ["verify_temp_schema(db)", "PRAGMA foreign_keys", "PRAGMA foreign_key_check", "SQLITE_DBCONFIG_ENABLE_TRIGGER"]) assert(code.includes(protection));
  assert.match(code, /OnceLock<Result<KnownSchema, String>>/);
});
test("missing deletion journals cannot be bypassed by legacy audio admission", () => {
  const code = source("audio_completion.rs");
  assert.doesNotMatch(code, /legacy_without_journal/);
  assert.match(code, /if !has_table \{\s*return if require_ledger \{\s*Err\("現行DBの音声削除台帳が欠落/);
  assert.match(code, /fn deleted\(db: &Connection, meeting: &str\)[\s\S]*?completed_for_meeting\(db, meeting, true\)\?/);
  assert.match(code, /fn salvage_deleted\([\s\S]*?if ids\.is_empty\(\) \{\s*return Ok\(false\);\s*\}[\s\S]*?verify_completed_for_meeting\(db, meeting, &ids, Some\(&anchor\), check\)\?/);
  for (const file of ["intent.rs", "audio_intent.rs", "completion.rs", "audio_removal.rs", "removal.rs"]) assert.doesNotMatch(source(file), /historical_completed_fixture|historical_progress_fixture|historical_manifest_fixture/);
});
