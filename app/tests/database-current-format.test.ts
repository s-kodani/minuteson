import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = readFileSync(new URL("../src-tauri/src/storage.rs", import.meta.url), "utf8");
const adapter = readFileSync(new URL("../src-tauri/src/storage/schema_40.rs", import.meta.url), "utf8");

test("database startup admits only current or exact schema39 before journal and recovery writes", () => {
  const journal = source.search(/connection\s*\.execute_batch\(\s*"PRAGMA journal_mode = WAL;/u);
  const admission = source.indexOf("empty_or_current_database(&connection)?");
  const initialization = source.indexOf("initialize_database_with_guidance(&mut connection, true)");
  assert(journal >= 0 && admission >= 0 && admission < journal);
  assert(initialization >= 0 && initialization < source.indexOf("deletion::startup::inspect(&connection"));
  assert.match(source, /schema_40::verify_startup\(connection\)\?/);
  assert.doesNotMatch(source + adapter, /apply_migrations|skip\(applied\)|pending upgrade|PRAGMA writable_schema/);
});

test("fresh schema40 is canonical and the only upgrade is an atomic verified39 trigger change", () => {
  assert.match(source, /SCHEMA_VERSION: usize = 40/);
  assert.match(source, /schema_40::current_layout\(connection\)\.map\(\|_\| \(\)\)/);
  assert.match(source, /transaction\.execute_batch\(canonical_schema_sql\(\)\)\?/);
  assert.match(source, /if !empty_or_current_database\(&transaction\)\?/);
  assert.match(source, /transaction\.commit\(\)\?/);
  assert.match(adapter, /PREVIOUS_VERSION: usize = 39/);
  for (const origin of ["PREVIOUS_CHECKSUM", "PREVIOUS_CRLF_CHECKSUM"]) {
    assert(adapter.includes(`${origin} if version == PREVIOUS_VERSION as i64 =>`));
  }
  assert.match(adapter, /version == SCHEMA_VERSION as i64 && schema_checksum\(\) == CURRENT_LF_CHECKSUM/);
  for (const current of ["CURRENT_LF_CHECKSUM", "CURRENT_CRLF_CHECKSUM"]) {
    assert(adapter.includes(`checksum == ${current}`));
  }
  assert.match(adapter, /objects\(db\)\? != objects\(&expected\)\?/);
  assert.match(adapter, /transaction_with_behavior\(rusqlite::TransactionBehavior::Immediate\)/);
  assert.match(adapter, /verify_previous\(&tx\)\? != layout/);
  assert.match(adapter, /DROP TRIGGER \{TRIGGER\}/);
  assert.match(adapter, /tx\.commit\(\)\?/);
  assert.doesNotMatch(source + adapter, /schema_upgrade|MIGRATIONS|upgrade_voice_registry|upgrade_speaker_completion/);
});
