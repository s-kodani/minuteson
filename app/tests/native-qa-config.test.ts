import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const json = (path: string) => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
const production = json("../src-tauri/tauri.conf.json");
const qa = json("../src-tauri/tauri.qa.conf.json");
const macos = json("../src-tauri/tauri.macos.conf.json");
const pkg = json("../package.json");

test("native QA uses an explicit distinct bundle identifier and visible app name", () => {
  assert.equal(qa.identifier, "jp.mon.desktop.qa");
  assert.notEqual(qa.identifier, production.identifier);
  assert.equal(qa.productName, "MON QA");
  assert.notEqual(qa.productName, production.productName);
});
test("QA windows preserve production roles, routing and native presentation except titles", () => {
  const withoutTitle = ({ title, ...window }: Record<string, unknown>) => window;
  assert.deepEqual(qa.app.windows.map(withoutTitle), production.app.windows.map(withoutTitle));
  assert.ok(qa.app.windows.every((window: { title: string }) => window.title.startsWith("MON QA")));
});
test("QA does not override storage paths, security, permissions, helpers or signing", () => {
  assert.deepEqual(Object.keys(qa).sort(), ["$schema", "app", "identifier", "productName"]);
  assert.deepEqual(Object.keys(qa.app), ["windows"]);
  assert.equal(macos.bundle.active, true);
  assert.equal(macos.bundle.resources["../../probes/macos-capture/.build/MONCaptureProbe.app"], "MONCaptureProbe.app");
  assert.equal(macos.bundle.macOS.minimumSystemVersion, "15.0");
});
test("QA script builds with the QA flavor without launching or invoking CI", () => {
  assert.equal(pkg.scripts["tauri:qa:build"], "tauri build --config src-tauri/tauri.qa.conf.json");
  assert.equal(pkg.scripts.tauri, "tauri");
});

test("native macOS packaging includes the generated headless embedding helper at the runtime resource path", () => {
  assert.equal(macos.bundle.resources["../../probes/speaker-embedding/.build/mon-speaker-embedding"], "mon-speaker-embedding");
  const build = readFileSync(new URL("../build-macos.sh", import.meta.url), "utf8");
  assert.ok(build.includes('probes/speaker-embedding/build.sh" --out-dir "$app_dir/../probes/speaker-embedding/.build"'));
  const runtime = readFileSync(new URL("../src-tauri/src/diarization/known.rs", import.meta.url), "utf8");
  assert.ok(runtime.includes('contents.join("Resources/mon-speaker-embedding")'));
});
