// Unix-only headless test tooling. Never execute the mutable source pathname.
import assert from "node:assert/strict";
import { constants, openSync, closeSync, fstatSync, readSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";

export const MAX_EXECUTABLE_BYTES = 128 * 1024 * 1024;

export function readVerifiedExecutable(source, expected, entry) {
  assert.match(expected, /^[0-9a-f]{64}$/);
  assert.ok(constants.O_NOFOLLOW, "headless executable verification requires Unix O_NOFOLLOW");
  const fd = openSync(source, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const before = fstatSync(fd, { bigint: true });
    assert.ok(before.isFile() && before.size > 0n && before.size <= BigInt(MAX_EXECUTABLE_BYTES));
    const chunks = [], block = Buffer.alloc(64 * 1024);
    let total = 0;
    for (;;) {
      const count = readSync(fd, block, 0, Math.min(block.length, MAX_EXECUTABLE_BYTES + 1 - total), null);
      if (!count) break;
      total += count;
      assert.ok(total <= MAX_EXECUTABLE_BYTES, "executable grew beyond the byte limit");
      chunks.push(Buffer.from(block.subarray(0, count)));
    }
    const after = fstatSync(fd, { bigint: true });
    assert.ok(after.isFile());
    for (const field of ["dev", "ino", "size", "mtimeNs", "ctimeNs"]) assert.equal(after[field], before[field], "executable changed while reading");
    assert.equal(BigInt(total), before.size);
    const binary = Buffer.concat(chunks, total);
    assert.equal(createHash("sha256").update(binary).digest("hex"), expected);
    assert.ok(binary.includes(Buffer.from(entry)), "binary has no owned-engine entry");
    return binary;
  } finally { closeSync(fd); }
}

export function stageVerifiedExecutable(binary, privateRoot) {
  const staged = join(privateRoot, "verified-engine");
  writeFileSync(staged, binary, { flag: "wx", mode: 0o700 });
  return staged;
}
