import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, rmSync, symlinkSync, truncateSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { readVerifiedExecutable, stageVerifiedExecutable, MAX_EXECUTABLE_BYTES } from "../../tools/mon_verified_executable.mjs";

const entry = "--mon-owned-engine-guard-v1", bytes = Buffer.from(`headless fixture ${entry}`);
const digest = (value: Buffer) => createHash("sha256").update(value).digest("hex");
const unix = { skip: process.platform === "win32" };
function fixture(t: any) {
  const root = mkdtempSync(join(tmpdir(), "mon-verified-executable-"));
  t.after(() => rmSync(root, { recursive: true }));
  const source = join(root, "source"); writeFileSync(source, bytes);
  return { root, source };
}
test("verified bytes are staged exclusively and source replacement cannot change what executes", unix, t => {
  const { root, source } = fixture(t);
  const verified = readVerifiedExecutable(source, digest(bytes), entry);
  writeFileSync(source, "replacement must never execute");
  const staged = stageVerifiedExecutable(verified, root);
  assert.deepEqual(readFileSync(staged), bytes);
  assert.throws(() => stageVerifiedExecutable(Buffer.from("overwrite"), root));
  assert.deepEqual(readFileSync(staged), bytes);
  assert.equal(statSync(staged).mode & 0o777, 0o700);
});
test("wrong hashes, missing headless entry, symlinks and oversized files are rejected before staging", unix, t => {
  const { root, source } = fixture(t);
  assert.throws(() => readVerifiedExecutable(source, "0".repeat(64), entry));
  assert.throws(() => readVerifiedExecutable(source, digest(bytes), "absent headless entry"));
  const link = join(root, "link"); symlinkSync(source, link);
  assert.throws(() => readVerifiedExecutable(link, digest(bytes), entry));
  assert.throws(() => readVerifiedExecutable(root, digest(bytes), entry));
  truncateSync(source, MAX_EXECUTABLE_BYTES + 1);
  assert.throws(() => readVerifiedExecutable(source, digest(bytes), entry));
});
