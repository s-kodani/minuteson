import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { checkedStartupReport, acceptStartupReport, type StartupReport } from "../src/startup-attention-state.ts";
const ready: StartupReport = { schemaVersion: 1, phase: "ready", failure: null };
const blocked: StartupReport = { schemaVersion: 1, phase: "blocked", failure: { stage: "storage_admission", message: "台帳を保持", messageTruncated: false, meetingRoot: "/safe/root", configRoot: "/safe/config" } };
const source = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");
test("startup reader validates the full shape and rejects forged normal admission", () => {
  assert.deepEqual(checkedStartupReport(ready), ready); assert.deepEqual(checkedStartupReport(blocked), blocked);
  for (const value of [null, {}, { ...ready, phase: "ok" }, { ...ready, schemaVersion: 2 }, { ...ready, failure: blocked.failure }, { ...blocked, failure: null }, { ...ready, safe: true }]) assert.throws(() => checkedStartupReport(value));
});
test("diagnostic stages, byte limits and paths are checked without guessing error text", () => {
  for (const failure of [{ ...blocked.failure!, stage: "corrupt_database" }, { ...blocked.failure!, message: "音".repeat(6000) }, { ...blocked.failure!, message: "bad\0text" }, { ...blocked.failure!, meetingRoot: "../guess" }, { ...blocked.failure!, configRoot: "x".repeat(8193) }, { ...blocked.failure!, repaired: true }]) assert.throws(() => checkedStartupReport({ ...blocked, failure }));
  assert.equal(checkedStartupReport({ ...blocked, failure: { ...blocked.failure!, meetingRoot: null, configRoot: null, message: "" } }).failure!.meetingRoot, null);
});
test("starting may settle once; a blocked diagnostic cannot turn into ready or another cause", () => {
  const starting: StartupReport = { schemaVersion: 1, phase: "starting", failure: null };
  assert.deepEqual(acceptStartupReport(starting, ready), ready); assert.deepEqual(acceptStartupReport(starting, blocked), blocked);
  assert.throws(() => acceptStartupReport(blocked, ready)); assert.throws(() => acceptStartupReport(blocked, starting));
  assert.throws(() => acceptStartupReport(blocked, { ...blocked, failure: { ...blocked.failure!, message: "another" } }));
  assert.throws(() => acceptStartupReport(ready, blocked));
});
test("canonicalized field order preserves the same confirmed process report", () => {
  const shuffled = { failure: { configRoot: "/safe/config", messageTruncated: false, message: "台帳を保持", stage: "storage_admission", meetingRoot: "/safe/root" }, phase: "blocked", schemaVersion: 1 };
  assert.deepEqual(acceptStartupReport(blocked, shuffled), blocked);
});
test("bounded UTF8 detail may explicitly be truncated but does not claim repair", () => {
  const value = { ...blocked, failure: { ...blocked.failure!, message: "音".repeat(5461), messageTruncated: true } };
  assert.equal(checkedStartupReport(value).failure!.messageTruncated, true);
  assert.equal(checkedStartupReport(value).phase, "blocked");
});
test("main entry gates all workspace providers; attention route has only report and failure exit IPC", () => {
  const root = source("../src/main.tsx"), ui = source("../src/startup-attention.tsx");
  assert.ok(root.includes("<StartupGate>{desktopWindow}</StartupGate>"));
  assert.ok(ui.includes('state.report?.phase === "ready" ? children'));
  assert.deepEqual([...ui.matchAll(/invoke\("([^"]+)"/gu)].map(m => m[1]), ["get_startup_report", "quit_blocked_startup"]);
  assert.doesNotMatch(ui, /ThemeProvider|setInterval|localStorage|\.reload\(/u);
});
test("native dispatch guard runs before any generated command and admission refusal precedes workers", () => {
  const native = source("../src-tauri/src/lib.rs");
  const dispatch = native.split(".invoke_handler(|invoke|")[1];
  assert.ok(dispatch.indexOf(".admit_command(") < dispatch.indexOf("tauri::generate_handler!"));
  const refusal = native.split("Err(failure) =>")[1].split("#[cfg(not(target_os")[0];
  assert.ok(refusal.includes("startup_attention::show(app.handle());")); assert.ok(refusal.includes("return Ok(());"));
  assert.ok(native.indexOf("startup_attention::admit_storage") < native.indexOf("ImportController::recover(app.handle())"));
});
