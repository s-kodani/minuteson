import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = readFileSync(new URL("../src-tauri/src/storage.rs", import.meta.url), "utf8");

test("database startup rejects old formats before journal changes and recovery writes", () => {
  const journal=source.search(/connection\s*\.execute_batch\(\s*"PRAGMA journal_mode = WAL;/u);
  assert(journal>=0&&source.indexOf("empty_or_current_database(&connection)?")<journal);
  assert(source.indexOf("initialize_database(&mut connection)?") < source.indexOf("deletion::startup::inspect(&connection"));
  assert.doesNotMatch(source, /apply_migrations|skip\(applied\)|pending upgrade/);
});

test("new database creation is transactional and only the current schema is accepted", () => {
  assert.match(source, /version != SCHEMA_VERSION as i64 \|\| checksum != schema_checksum\(\)/);
  assert.doesNotMatch(source, /schema_upgrade|MIGRATIONS|upgrade_voice_registry|upgrade_speaker_completion/);
  assert.match(source, /if !empty_or_current_database\(&transaction\)\?/);
  assert.match(source, /transaction\.commit\(\)\?/);
});
