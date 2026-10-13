import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = readFileSync(new URL("../src-tauri/src/storage.rs", import.meta.url), "utf8");

test("database startup rejects old formats before journal changes and recovery writes", () => {
  assert(source.indexOf("empty_or_current_database(&connection)?") < source.indexOf('connection.execute_batch("PRAGMA journal_mode = WAL;'));
  assert(source.indexOf("initialize_database(&mut connection)?") < source.indexOf("deletion::startup::inspect(&connection"));
  assert.doesNotMatch(source, /apply_migrations|skip\(applied\)|pending upgrade/);
});

test("current database creation is empty-only and transactional", () => {
  assert.match(source, /name NOT GLOB 'sqlite_\*'/);
  assert.match(source, /verify_migration_history\(connection\)\? != MIGRATIONS\.len\(\)/);
  assert.match(source, /if !empty_or_current_database\(&transaction\)\?/);
  assert.match(source, /transaction\.commit\(\)\?/);
});
