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

test("all macOS flavors declare recording privacy purposes on the responsible parent app", () => {
  assert.equal(macos.bundle.macOS.infoPlist, "release-entitlements/Info.plist");
  const purposes = readFileSync(new URL(`../src-tauri/${macos.bundle.macOS.infoPlist}`, import.meta.url), "utf8");
  for (const key of ["NSMicrophoneUsageDescription", "NSAudioCaptureUsageDescription"]) {
    assert.match(purposes, new RegExp(`<key>${key}</key>\\s*<string>[^<]+</string>`, "u"));
  }
  // This is a usage declaration, not an entitlement or an OS permission grant.
  assert.doesNotMatch(purposes, /com\.apple\.security/);
});

test("native macOS packaging includes the generated headless embedding helper at the runtime resource path", () => {
  assert.equal(macos.bundle.resources["../../probes/speaker-embedding/.build/mon-speaker-embedding"], "mon-speaker-embedding");
  const build = readFileSync(new URL("../build-macos.sh", import.meta.url), "utf8");
  assert.ok(build.includes('probes/speaker-embedding/build.sh" --out-dir "$app_dir/../probes/speaker-embedding/.build"'));
  const runtime = readFileSync(new URL("../src-tauri/src/diarization/known.rs", import.meta.url), "utf8");
  assert.ok(runtime.includes('contents.join("Resources/mon-speaker-embedding")'));
});

test("native app packaging explicitly builds the capture helper in release and installs that same configuration", () => {
  const build = readFileSync(new URL("../build-macos.sh", import.meta.url), "utf8");
  assert.match(build, /^"\$app_dir\/\.\.\/probes\/macos-capture\/build-bundle\.sh" --configuration release --disable-sandbox$/m);
  const bundle = readFileSync(new URL("../../probes/macos-capture/build-bundle.sh", import.meta.url), "utf8");
  assert.ok(bundle.includes('swift build "$@"'));
  assert.ok(bundle.includes('binary_dir=$(swift build --show-bin-path "$@")'));
  assert.ok(bundle.includes('install -m 755 "$binary_dir/mon-macos-capture"'));
});

test("capture package keeps its CLI entry point out of the core imported by both test runners", () => {
  const pkg = readFileSync(new URL("../../probes/macos-capture/Package.swift", import.meta.url), "utf8");
  assert.ok(pkg.includes('.executable(name: "mon-macos-capture", targets: ["MONMacOSCaptureCLI"])'));
  assert.ok(pkg.includes('.executableTarget(name: "MONMacOSCaptureCLI", dependencies: ["MONMacOSCapture"])'));
  assert.match(pkg, /\.target\(\s*name: "MONMacOSCapture"/);
  const core = readFileSync(new URL("../../probes/macos-capture/Sources/MONMacOSCapture/App.swift", import.meta.url), "utf8");
  assert.doesNotMatch(core, /^@main$/m);
  const entry = readFileSync(new URL("../../probes/macos-capture/Sources/MONMacOSCaptureCLI/App.swift", import.meta.url), "utf8");
  assert.match(entry, /^@main$/m);
  assert.ok(entry.includes("await CaptureCommand.run()"));
});

test("distribution preparation is explicit, production-only and leaves QA/default signing unchanged", () => {
  const release = json("../src-tauri/tauri.release.conf.json");
  assert.deepEqual(Object.keys(release).sort(), ["$schema", "bundle"]);
  assert.equal(production.identifier, "jp.mon.desktop");
  assert.equal(qa.identifier, "jp.mon.desktop.qa");
  assert.deepEqual(release.bundle.targets, ["app"]);
  assert.equal(release.bundle.macOS.signingIdentity, "-");
  assert.equal(macos.bundle.macOS.signingIdentity, "-");
  assert.equal(release.bundle.macOS.infoPlist, "release-entitlements/Info.plist");
  assert.equal(pkg.scripts["tauri:release:build"], "python3 ../tools/mon_macos_release.py");
});

test("macOS packages offline dependency texts and an explicit unreviewed inventory without adding UI", () => {
  assert.equal(macos.bundle.resources["../../.cache/third-party-notices/THIRD_PARTY_NOTICES.txt"], "THIRD_PARTY_NOTICES.txt");
  assert.equal(macos.bundle.resources["../../.cache/third-party-notices/third-party-inventory.json"], "third-party-inventory.json");
  const build = readFileSync(new URL("../build-macos.sh", import.meta.url), "utf8");
  assert.match(build, /^python3 "\$app_dir\/\.\.\/tools\/mon_third_party_notices\.py"$/m);
});
