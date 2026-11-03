#!/usr/bin/env node
// Headless native entry check. Only synthetic owned processes and new fixtures.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, lstatSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import { once } from "node:events";
import { readVerifiedExecutable, stageVerifiedExecutable } from "./mon_verified_executable.mjs";

const ENTRY = "--mon-owned-engine-guard-v1", script = fileURLToPath(import.meta.url);
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const sleepGroup = "echo $$ > engine-pid; /bin/sleep 30 & echo $! > descendant-pid; wait";
function guarded(executable, root, source, parent = process.pid, owner = true) {
  const stdio = ["ignore", "pipe", "pipe"];
  if (owner) stdio.push("pipe");
  const child = spawn(executable, [ENTRY, String(parent), "3", "/bin/sh", "--", "-c", source], {
    cwd: root, stdio, env: { PATH: "/usr/bin:/bin", MON_FIXED_LITERAL: "literal ; $(not-executed)" },
  });
  let stdout = "", stderr = "";
  child.stdout.on("data", bytes => { stdout += bytes; assert.ok(stdout.length < 65536); });
  child.stderr.on("data", bytes => { stderr += bytes; assert.ok(stderr.length < 65536); });
  const done = once(child, "exit").then(([code, signal]) => ({ code, signal, stdout, stderr }));
  return { child, done };
}
async function wait(read) {
  const deadline = Date.now() + 8000;
  for (;;) { const value = read(); if (value) return value; assert.ok(Date.now() < deadline, "owned state not confirmed"); await pause(10); }
}
function pid(root, name) {
  try { const value = Number(readFileSync(join(root, name), "utf8").trim()); return Number.isInteger(value) && value > 1 ? value : null; } catch { return null; }
}
function gone(id) { try { process.kill(id, 0); return false; } catch (e) { return e.code === "ESRCH"; } }
async function nativeCheck(executable, root) {
  let cases = 0;
  const fixed = guarded(executable, root, "printf '%s' \"$MON_FIXED_LITERAL\"; printf 'error' >&2; exit 27");
  const result = await fixed.done;
  fixed.child.stdio[3].destroy();
  assert.equal(result.code, 27, JSON.stringify(result)); assert.equal(result.signal, null);
  assert.equal(result.stdout, "literal ; $(not-executed)"); assert.equal(result.stderr, "error"); cases++;

  const missing = guarded(executable, root, "touch must-not-exist", process.pid, false);
  assert.equal((await missing.done).code, 125); assert.equal(pid(root, "must-not-exist"), null); cases++;
  const wrong = guarded(executable, root, "touch must-not-exist", process.pid + 1);
  const refusal = await wrong.done; wrong.child.stdio[3].destroy();
  assert.equal(refusal.code, 125); assert.match(refusal.stderr, /no longer the direct parent/);
  assert.throws(() => lstatSync(join(root, "must-not-exist"))); cases++;

  const normal = guarded(executable, root, "echo $$ > engine-pid; /bin/sleep 30 & echo $! > descendant-pid; exit 0");
  assert.equal((await normal.done).code, 0); normal.child.stdio[3].destroy();
  const normalChild = await wait(() => pid(root, "descendant-pid"));
  await wait(() => gone(normalChild)); cases++;

  // New paths avoid adopting PID files from the previous completed operation.
  const droppedRoot = mkdtempSync(join(root, "closed-"));
  const closed = guarded(executable, droppedRoot, sleepGroup);
  const engine = await wait(() => pid(droppedRoot, "engine-pid"));
  const descendant = await wait(() => pid(droppedRoot, "descendant-pid"));
  assert.ok(!gone(engine) && !gone(descendant));
  closed.child.stdio[3].destroy();
  assert.equal((await closed.done).code, 125);
  await wait(() => gone(engine)); await wait(() => gone(descendant)); cases++;

  const killedRoot = mkdtempSync(join(root, "sigkill-"));
  const unrelated = spawn("/bin/sleep", ["30"], { stdio: "ignore" });
  const unrelatedDone = once(unrelated, "exit");
  const owner = spawn(process.execPath, [script, "--synthetic-owner", executable, killedRoot], { stdio: "ignore" });
  const ownerDone = once(owner, "exit");
  try {
    const guard = await wait(() => pid(killedRoot, "guard-pid"));
    const engine = await wait(() => pid(killedRoot, "engine-pid"));
    const descendant = await wait(() => pid(killedRoot, "descendant-pid"));
    assert.ok(!gone(guard) && !gone(engine) && !gone(descendant));
    assert.equal(owner.kill("SIGKILL"), true);
    assert.deepEqual(await ownerDone, [null, "SIGKILL"]);
    await wait(() => gone(guard)); await wait(() => gone(engine)); await wait(() => gone(descendant));
    assert.equal(unrelated.exitCode, null); assert.equal(unrelated.signalCode, null); assert.ok(!gone(unrelated.pid));
    cases++;
  } finally {
    if (owner.exitCode === null && owner.signalCode === null) { owner.kill("SIGKILL"); await ownerDone; }
    if (unrelated.exitCode === null && unrelated.signalCode === null) { unrelated.kill("SIGKILL"); await unrelatedDone; }
  }
  return cases;
}
if (process.argv[2] === "--synthetic-owner") {
  const [executable, root] = process.argv.slice(3);
  assert.ok(isAbsolute(executable) && isAbsolute(root));
  const active = guarded(executable, root, sleepGroup);
  writeFileSync(join(root, "guard-pid"), String(active.child.pid), { flag: "wx", mode: 0o600 });
  await active.done;
  active.child.stdio[3].destroy();
} else {
  assert.equal(process.argv[2], "--executable");
  const executable = process.argv[3]; assert.equal(process.argv.length, 6); assert.ok(isAbsolute(executable));
  assert.equal(process.argv[4], "--sha256");
  const expected = process.argv[5]; assert.match(expected, /^[0-9a-f]{64}$/);
  // An old MON without this entry would open its GUI. Refuse before spawn:
  // caller must pin the known headless-enabled binary, not any app pathname.
  const binary = readVerifiedExecutable(executable, expected, ENTRY);
  const root = mkdtempSync(join(tmpdir(), "mon-owned-engine-native-")), identity = lstatSync(root);
  writeFileSync(join(root, "fixture-marker"), "synthetic owned engine only", { flag: "wx", mode: 0o600 });
  const started = performance.now();
  try {
    const staged = stageVerifiedExecutable(binary, root);
    const cases = await nativeCheck(staged, root);
    assert.equal(readFileSync(join(root, "fixture-marker"), "utf8"), "synthetic owned engine only");
    console.log(JSON.stringify({ cases, passed: cases, elapsedSeconds: (performance.now() - started) / 1000, guiLaunched: false, realModels: false }));
  } finally {
    const current = lstatSync(root);
    assert.ok(!current.isSymbolicLink() && current.ino === identity.ino && current.dev === identity.dev && current.uid === identity.uid);
    assert.equal(readFileSync(join(root, "fixture-marker"), "utf8"), "synthetic owned engine only");
    rmSync(root, { recursive: true });
  }
}
