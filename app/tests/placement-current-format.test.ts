import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = (name: string) => readFileSync(new URL(`../src-tauri/src/${name}`, import.meta.url), "utf8");

test("placement accepts only the current owned ledger and has no upgrade branch", () => {
  const ledger = source("bootstrap/placement.rs");
  assert.match(ledger, /version != 2 \|\| schema\(&db\)\? != expected_schema/);
  assert.doesNotMatch(ledger, /legacy_schema|version == 1/);
  assert.match(ledger, /PRAGMA user_version=2; COMMIT/);
  assert.match(ledger, /nodes\.values\(\)\.any\(\|n\| n\.state != "copied"\)/);
});

test("sealed placed images allow only the read-only exact39 adapter and preserve their census", () => {
  const backup = source("storage/backup.rs");
  const relocation = source("storage/backup/relocation.rs");
  const adapter = source("storage/schema_40.rs");
  assert.doesNotMatch(backup + adapter, /27\.\.MIGRATIONS|apply_migrations|PRAGMA writable_schema/);
  const placed = backup.slice(backup.indexOf("fn validate_placed_database"), backup.indexOf("fn validate_database_contents"));
  assert.match(placed, /verify_database_schema\(db\)\.is_err\(\)/);
  assert.match(placed, /verify_readonly_previous\(db\)/);
  assert.match(placed, /validate_database_contents\(db, cancelled\)/);
  assert.match(adapter, /PRAGMA query_only/);
  assert.match(adapter, /objects\(db\)\? != objects\(&expected\)\?/);
  const image = relocation.slice(relocation.indexOf("pub(crate) fn verify_placed_image_with_controls"));
  const hash = image.indexOf("expected.database_hash.clone()");
  const validation = image.indexOf("validate_placed_database(&mut db, cancelled)?");
  assert(hash >= 0 && validation > hash);
  const readOnly = image.indexOf('read_only(&root.join("meetings.sqlite3"))?');
  assert(readOnly >= 0 && readOnly < validation);
  assert.match(image, /census\(&db, cancelled\)\? != \(expected\.schema_hash\.clone\(\), expected\.tables\.clone\(\)\)/);
  assert.match(image, /assets\(&db, &copied\)\? != expected\.assets/);
  for (const protection of ["PRAGMA integrity_check", "PRAGMA foreign_key_check", "validate_provenance(db, cancelled)"]) assert(backup.includes(protection));
});

test("current handoff and native manifest version one are independent from the removed ledger", () => {
  assert.match(source("bootstrap/placement/history.rs"), /PRAGMA user_version=1; COMMIT/);
  assert.match(source("bootstrap/placement/activation.rs"), /if m\.version != 1/);
});
