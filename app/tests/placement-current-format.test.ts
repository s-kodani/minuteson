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

test("placed images use the normal current database validator without old schema exceptions", () => {
  assert.doesNotMatch(source("storage/backup.rs"), /validate_placed_database|27\.\.MIGRATIONS/);
  assert.match(source("storage/backup/relocation.rs"), /validate_database\(&mut db, cancelled\)\?/);
});

test("current handoff and native manifest version one are independent from the removed ledger", () => {
  assert.match(source("bootstrap/placement/history.rs"), /PRAGMA user_version=1; COMMIT/);
  assert.match(source("bootstrap/placement/activation.rs"), /if m\.version != 1/);
});
