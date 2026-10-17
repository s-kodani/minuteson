// Local CLI-first browser verification. Start Vite and a named CLI browser
// first; --cli selects an installed CLI or the explicitly supplied skill wrapper.
// All fixture IPC is synthetic. This does not run Tauri, CI or real recording.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { applyMinutesFixtureTheme } from "./minutes-browser-helpers.js";
import { applyPlaybackFixtureTheme } from "./playback-browser-helpers.js";
const tests = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2), option = name => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
if (!args.includes("--cli") || !args.includes("--session")) throw Error("Provide --cli PATH and --session NAME for an existing local Playwright CLI browser.");
const cli = resolve(option("--cli")), session = option("--session"), output = resolve(option("--output") ?? resolve(tests, "../../output/playwright/regression"));
const portText = option("--port") ?? "1420";
if (!/^[0-9]{1,5}$/u.test(portText) || Number(portText) < 1 || Number(portText) > 65535) throw Error("Provide a valid local server port.");
const port = Number(portText), origin = `http://127.0.0.1:${port}`;
const localPort = code => code.replaceAll(":1420", `:${port}`);
const selected = args.includes("--only") ? new Set(option("--only").split(",")) : null;
const suites = [
  ["voice", "voice-browser-fixture.js", "voice-browser-check.js"],
  ["voice-selection", "voice-browser-fixture.js", "voice-selection-browser-check.js"],
  ["voice-library", "voice-browser-fixture.js", "voice-library-browser-check.js"],
  ["notification-menu", "browser-fixture.js", "notification-menu-browser-check.js"],
  ["simple-workspace", "browser-fixture.js", "simple-workspace-browser-check.js"],
  ["notification-archive", null, "notification-archive-browser-check.js"],
  ["simple-transcript", "browser-fixture.js", "simple-transcript-browser-check.js"],
  ["transcript-consolidation", "browser-fixture.js", "transcript-groups-browser-check.js"],
  ["speaker-assignment", "browser-fixture.js", "speaker-assignment-browser-check.js"],
  ["speaker-structure", "browser-fixture.js", "speaker-structure-browser-check.js"],
  ["transcription-regeneration", "browser-fixture.js", "transcription-regeneration-browser-check.js"],
  ["analysis-refresh-errors", "browser-fixture.js", "analysis-refresh-errors-browser-check.js"],
  ["simple-models", "model-download-browser-fixture.tsx", "simple-models-browser-check.js"],
  ["plain-minutes-export", "plain-minutes-production-browser-fixture.js", "plain-minutes-export-browser-check.js"],
  ["minutes-audio-tags", "plain-minutes-interval-browser-fixture.js", "minutes-audio-tags-browser-check.js"],
  ["plain-minutes-interval", "plain-minutes-interval-browser-fixture.js", "plain-minutes-interval-browser-check.js"],
  ["shared-settings", "settings-modal-browser-fixture.tsx", "settings-modal-browser-check.js"],
  ["keyboard-primary", "browser-fixture.js", "keyboard-primary-browser-check.js"],
  ["common", "common-modal-browser-fixture.tsx", "common-modal-browser-check.js"],
  ["action", "action-confirmation-browser-fixture.tsx", "action-confirmation-browser-check.js"],
  ["info", "info-popover-browser-fixture.tsx", "info-popover-browser-check.js"],
  ["import", "import-browser-fixture.js", "import-modal-browser-check.js"],
  ["data", "data-save-browser-fixture.js", "data-save-browser-check.js"],
  ["data-admission", "data-save-browser-fixture.js", "data-admission-browser-check.js"],
  ["no-maintenance", "data-save-browser-fixture.js", "no-maintenance-browser-check.js"],
  ["data-layout", "data-save-browser-fixture.js", "data-save-layout-browser-check.js"],
  ["data-action", "data-save-browser-fixture.js", "data-action-confirmation-browser-check.js"],
  ["move", "data-save-browser-fixture.js", "storage-move-browser-check.js"],
  // Model deletion/rebinding dialogs were retired with maintenance UI.
  // Current download/selection and hidden cleanup have their own suites.
  ["info-production", "browser-fixture.js", "info-production-browser-check.js"],
  ["theme", "theme-browser-fixture.js", "theme-browser-check.js"],
  ["recording", null, "recording-confirmation-browser-check.js"],
  ["recorder-panel", null, "recorder-panel-browser-check.js"],
  ["recorder-info", null, "recorder-info-browser-check.js"],
  ["recorder-operations", null, "recorder-operation-browser-check.js"],
  ["startup-attention", null, "startup-attention-browser-check.js"],
  ["close", null, "application-close-browser-check.js"],
  ["microphone-input", null, "microphone-input-browser-check.js"],
  ["capture-scope", "capture-scope-browser-fixture.tsx", "capture-scope-browser-check.js"],
  ["application-scope", null, "application-scope-browser-check.js"],
  ["recording-preparation", null, "recording-preparation-browser-check.js"],
  ["input-levels", null, "input-level-browser-check.js"],
  ["recording-writes", null, "recording-write-browser-check.js"],
  ["current-input-status", null, "current-input-browser-check.js"],
  ["input-notice-queue", null, "input-notice-queue-browser-check.js"],
  ["archived-input-notices", null, "archived-input-browser-check.js"],
  ["input-journal-save", null, "input-journal-browser-check.js"],
  ["capture-test", null, "capture-test-browser-check.js"],
  ["capture-test-recovery", null, "capture-test-recovery-browser-check.js"],
  ["workspace", "browser-fixture.js", "workspace-browser-check.js"],
  ["detail-tabs", "browser-fixture.js", "detail-tabs-browser-check.js"],
  ["speaker-panel", "browser-fixture.js", "speaker-panel-browser-check.js"],
  ["fixed-player", "browser-fixture.js", "fixed-player-browser-check.js"],
  ["meeting-audio", "browser-fixture.js", "meeting-audio-browser-check.js"],
  ["meeting-missing", "browser-fixture.js", "meeting-missing-browser-check.js"],
  ["playback-speakers", "browser-fixture.js", "playback-speaker-browser-check.js"],
  // Estimated-segment review and manual merges were removed from the product.
  ["playback-cancellation", "browser-fixture.js", "playback-cancellation-browser-check.js"],
  ["playback-highlights", "browser-fixture.js", "current-playback-highlights-browser-check.js"],
  // Advanced persisted Undo/bulk editor shortcuts were removed. Visible
  // individual editing, IME and keyboard focus are covered by keyboard-primary.
  ["toast-production", "browser-fixture.js", "toast-production-browser-check.js"],
  ["theme-toast", "theme-browser-fixture.js", "theme-toast-browser-check.js"],
  ["import-toast", "import-browser-fixture.js", "import-toast-browser-check.js"],
  ["job-notifications", "browser-fixture.js", "job-notifications-browser-check.js"],
  ["plain-minutes", null, "minutes-markdown-editor-browser-check.js"],
  ["plain-minutes-production", "plain-minutes-production-browser-fixture.js", "plain-minutes-production-browser-check.js"],
  ["plain-minutes-close", "plain-minutes-production-browser-fixture.js", "plain-minutes-close-browser-check.js"],
  ["global-minutes-model", "plain-minutes-production-browser-fixture.js", "global-minutes-model-browser-check.js"],
  ["minutes-progress-toast", "plain-minutes-production-browser-fixture.js", "minutes-progress-toast-browser-check.js"],
  // Structured minutes/evidence/adoption suites no longer describe the product.
  // Plain-body saving, generation and leaving are covered above instead.
  ["toast", "toast-browser-fixture.tsx", "toast-browser-check.js"],
  ["toast-dedup", "toast-browser-fixture.tsx", "toast-dedup-browser-check.js"],
  ["navigation-menu", null, "navigation-menu-browser-check.js"],
  ["row-menu-confirmation", "audio-deletion-browser-fixture.js", "row-menu-confirmation-browser-check.js"],
  ["row-menu-activation", "deletion-browser-fixture.js", "row-menu-activation-browser-check.js"],
].filter(([name]) => !selected || selected.has(name));
if (!suites.length || selected && suites.length !== selected.size) throw Error("Unknown or empty suite selection");
mkdirSync(output, { recursive: true });
let total = 0;
const failures = [], continueOnFailure = args.includes("--continue-on-failure");
const indexRoute = new RegExp(localPort(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/.source));
const entryScript = /<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u;
const readiness = {
  "voice-browser-fixture.js": "monVoiceFixture",
  "common-modal-browser-fixture.tsx": "modalFixture", "action-confirmation-browser-fixture.tsx": "actionFixture",
  "info-popover-browser-fixture.tsx": "infoFixture", "browser-fixture.js": "monBrowserFixture",
  "import-browser-fixture.js": "monImportFixture", "model-deletion-browser-fixture.js": "modelDeletionFixture",
  "data-save-browser-fixture.js": "dataFixture", "deletion-browser-fixture.js": "deletionFixture",
  "model-profile-browser-fixture.js": "modelProfileFixture",
  "model-rebinding-browser-fixture.js": "rebindingFixture",
  "model-download-browser-fixture.tsx": "modelDownloadFixture",
  "audio-deletion-browser-fixture.js": "audioDeletionFixture", "theme-browser-fixture.js": "themeFixture",
  "toast-browser-fixture.tsx": "toastFixture",
  "capture-scope-browser-fixture.tsx": "scopeFixture",
};
for (const [name, fixture, source] of suites) {
  const check = localPort(readFileSync(resolve(tests, source), "utf8"));
  const setup = fixture ? `await page.context().route(${indexRoute}, async route=>{const response=await route.fetch(); const body=(await response.text()).replace(${entryScript}, ${JSON.stringify(`<script type="module">await import("/tests/${fixture}");</script>`)}); await route.fulfill({response,body});}); await page.goto("${origin}/");` : "";
  const ready = readiness[fixture] ? `await page.waitForFunction(name=>!!window[name],${JSON.stringify(readiness[fixture])});` : "";
  // The browser daemon's cwd can differ from this runner's cwd. Always route
  // generated captures to the explicit output directory, not the repo root.
  const capturePrefix = JSON.stringify(resolve(output, name + "-"));
  // Explicit drivers for the production workspace: real row-menu clicks,
  // never forcing hidden controls visible or replacing the application's UI.
  const detailsDriver = `const detailUI={openMeetingRow:async(title="既存会議")=>{const row=page.locator(".meeting-table tbody tr").filter({has:page.locator(".meeting-name").filter({hasText:title})}).first();await row.waitFor();const actions=row.locator(".meeting-row-actions");const trigger=actions.locator(":scope > button");if(await trigger.getAttribute("aria-expanded")!=="true")await trigger.click();}};`;
  const code = `async page=>{await page.goto("about:blank");await page.unrouteAll();await page.context().unrouteAll();page.setDefaultTimeout(10000);await page.setViewportSize({width:1200,height:900});${setup}${ready}const capturePath=path=>${capturePrefix}+path.split(/[\\/]/u).at(-1);${detailsDriver}detailUI.applyMinutesTheme=theme=>(${applyMinutesFixtureTheme.toString()})(page,theme);detailUI.applyPlaybackTheme=theme=>(${applyPlaybackFixtureTheme.toString()})(page,theme);const screenshot=page.screenshot.bind(page);page.screenshot=options=>screenshot({...options,path:capturePath(options.path)});try{return await (${check})(page,capturePath,detailUI);}finally{page.screenshot=screenshot;}}`;
  const wrapper = cli.endsWith(".sh");
  const run = spawnSync(wrapper ? cli : process.execPath, [...(wrapper ? [] : [cli]), `-s=${session}`, "run-code", code], { cwd: output, encoding: "utf8", env: { ...process.env, NO_UPDATE_NOTIFIER: "1" }, maxBuffer: 8 * 1024 * 1024 });
  const log = (run.stdout ?? "") + (run.stderr ?? "");
  writeFileSync(resolve(output, `${name}.log`), log);
  const match = /^### Result\r?\n([^\r\n]+)/m.exec(log);
  if (run.status !== 0 || !match) {
    process.stderr.write(`${name}: FAILED (${run.error ?? "see " + resolve(output, `${name}.log`)})\n`);
    failures.push(name);
    if (!continueOnFailure) process.exit(1);
    continue;
  }
  const result = JSON.parse(match[1]), count = Array.isArray(result) ? result.length : result.passed ?? result.cases;
  if (!Number.isSafeInteger(count) || count < 1) throw Error(`${name}: invalid result count`);
  if (Array.isArray(result.results) && result.results.length !== count) throw Error(`${name}: result/count mismatch`);
  total += count;
  process.stdout.write(`${name}: ${count} passed\n`);
}
process.stdout.write(`TOTAL: ${total} cases in ${suites.length - failures.length} passed suites\n`);
if (failures.length) { process.stderr.write(`FAILED SUITES: ${failures.join(", ")}\n`); process.exitCode = 1; }
