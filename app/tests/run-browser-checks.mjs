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
  ["notification-menu", "browser-fixture.js", "notification-menu-browser-check.js"],
  ["simple-workspace", "browser-fixture.js", "simple-workspace-browser-check.js"],
  ["notification-archive", null, "notification-archive-browser-check.js"],
  ["simple-transcript", "browser-fixture.js", "simple-transcript-browser-check.js"],
  ["transcription-regeneration", "browser-fixture.js", "transcription-regeneration-browser-check.js"],
  ["analysis-refresh-errors", "browser-fixture.js", "analysis-refresh-errors-browser-check.js"],
  ["simple-models", "model-download-browser-fixture.tsx", "simple-models-browser-check.js"],
  ["plain-minutes-export", "plain-minutes-production-browser-fixture.js", "plain-minutes-export-browser-check.js"],
  ["plain-minutes-interval", "plain-minutes-interval-browser-fixture.js", "plain-minutes-interval-browser-check.js"],
  ["shared-settings", "settings-modal-browser-fixture.tsx", "settings-modal-browser-check.js"],
  ["keyboard-primary", "browser-fixture.js", "keyboard-primary-browser-check.js"],
  ["common", "common-modal-browser-fixture.tsx", "common-modal-browser-check.js"],
  ["action", "action-confirmation-browser-fixture.tsx", "action-confirmation-browser-check.js"],
  ["info", "info-popover-browser-fixture.tsx", "info-popover-browser-check.js"],
  ["import", "import-browser-fixture.js", "import-modal-browser-check.js"],
  ["data", "data-save-browser-fixture.js", "data-save-browser-check.js"],
  ["no-maintenance", "data-save-browser-fixture.js", "no-maintenance-browser-check.js"],
  ["data-layout", "data-save-browser-fixture.js", "data-save-layout-browser-check.js"],
  ["data-action", "data-save-browser-fixture.js", "data-action-confirmation-browser-check.js"],
  ["move", "data-save-browser-fixture.js", "storage-move-browser-check.js"],
  ["model-review", "model-deletion-browser-fixture.js", "model-deletion-browser-check.js"],
  ["model-rebinding", "model-rebinding-browser-fixture.js", "model-rebinding-browser-check.js"],
  ["info-production", "theme-browser-fixture.js", "info-production-browser-check.js"],
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
  ["recording-input-status", null, "recording-input-browser-check.js"],
  ["system-input-status", null, "system-input-browser-check.js"],
  ["input-notice-receipts", null, "input-notice-receipts-browser-check.js"],
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
  ["estimated-speakers", "browser-fixture.js", "estimated-speaker-colors-browser-check.js"],
  ["playback-cancellation", "browser-fixture.js", "playback-cancellation-browser-check.js"],
  ["display-follow", "browser-fixture.js", "display-follow-browser-check.js"],
  ["keyboard", "browser-fixture.js", "keyboard-browser-check.js"],
  ["toast-production", "browser-fixture.js", "toast-production-browser-check.js"],
  ["theme-toast", "theme-browser-fixture.js", "theme-toast-browser-check.js"],
  ["import-toast", "import-browser-fixture.js", "import-toast-browser-check.js"],
  ["job-notifications", "browser-fixture.js", "job-notifications-browser-check.js"],
  ["plain-minutes", null, "minutes-markdown-editor-browser-check.js"],
  ["plain-minutes-production", "plain-minutes-production-browser-fixture.js", "plain-minutes-production-browser-check.js"],
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
const indexRoute = new RegExp(localPort(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/.source));
const entryScript = /<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u;
const readiness = {
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
  // Explicit drivers for the production workspace: real tab/summary clicks,
  // never forcing hidden controls visible or replacing the application's UI.
  const detailsDriver = `const openDetails=async selector=>{const details=page.locator(selector);await details.waitFor({state:"attached"});if(!await details.evaluate(e=>e.open))await details.locator(":scope > summary").click();};const waitSpeakerLayout=()=>page.waitForFunction(()=>{const panel=document.querySelector(".workspace-panel-shell");return !panel||panel.dataset.narrow===String(matchMedia("(max-width: 1119px)").matches);});const detailUI={openMeetingRow:async(title="既存会議")=>{const row=page.locator(".meeting-table tbody tr").filter({has:page.locator(".meeting-name").filter({hasText:title})}).first();await row.waitFor();const actions=row.locator(".meeting-row-actions");const trigger=actions.locator(":scope > button");if(await trigger.getAttribute("aria-expanded")!=="true")await trigger.click();},openMeetingTools:()=>openDetails(".meeting-workspace-tools"),openTranscriptTools:async()=>{await openDetails(".transcript-workspace-tools");await detailUI.openSpeakerPanel();for(const selector of [".speaker-name-tools",".speaker-merge-tools"]){for(const details of await page.locator(selector).all())if(!await details.evaluate(e=>e.open))await details.locator(":scope > summary").click();}await detailUI.closeSpeakerPanel();for(const details of await page.locator(".utterance-edit-tools").all())if(!await details.evaluate(e=>e.open))await details.locator(":scope > summary").click();},openSpeakerPanel:async()=>{await waitSpeakerLayout();const button=page.getByRole("button",{name:"話者一覧を開く",exact:true});if(await button.isVisible()&&await button.getAttribute("aria-expanded")!=="true")await button.click();},closeSpeakerPanel:async()=>{await waitSpeakerLayout();const dialog=page.locator(".workspace-drawer[open]");if(await dialog.count())await dialog.getByRole("button",{name:"話者一覧を閉じる",exact:true}).click();},openDiarization:async()=>{const tools=page.locator(".speaker-diarization-tools");if(await tools.count()){await detailUI.openSpeakerPanel();await openDetails(".speaker-diarization-tools");}},openMinutes:async()=>{await page.getByRole("tab",{name:"議事録",exact:true}).click();await openDetails(".minutes-workspace-tools");}};`;
  const code = `async page=>{await page.goto("about:blank");await page.unrouteAll();await page.context().unrouteAll();page.setDefaultTimeout(10000);await page.setViewportSize({width:1200,height:900});${setup}${ready}const capturePath=path=>${capturePrefix}+path.split(/[\\/]/u).at(-1);${detailsDriver}detailUI.applyMinutesTheme=theme=>(${applyMinutesFixtureTheme.toString()})(page,theme);detailUI.applyPlaybackTheme=theme=>(${applyPlaybackFixtureTheme.toString()})(page,theme);const screenshot=page.screenshot.bind(page);page.screenshot=options=>screenshot({...options,path:capturePath(options.path)});try{return await (${check})(page,capturePath,detailUI);}finally{page.screenshot=screenshot;}}`;
  const wrapper = cli.endsWith(".sh");
  const run = spawnSync(wrapper ? cli : process.execPath, [...(wrapper ? [] : [cli]), `-s=${session}`, "run-code", code], { cwd: output, encoding: "utf8", env: { ...process.env, NO_UPDATE_NOTIFIER: "1" }, maxBuffer: 8 * 1024 * 1024 });
  const log = (run.stdout ?? "") + (run.stderr ?? "");
  writeFileSync(resolve(output, `${name}.log`), log);
  const match = /^### Result\r?\n([^\r\n]+)/m.exec(log);
  if (run.status !== 0 || !match) { process.stderr.write(`${name}: FAILED (${run.error ?? "see " + resolve(output, `${name}.log`)})\n`); process.exit(1); }
  const result = JSON.parse(match[1]), count = Array.isArray(result) ? result.length : result.passed ?? result.cases;
  if (!Number.isSafeInteger(count) || count < 1) throw Error(`${name}: invalid result count`);
  if (Array.isArray(result.results) && result.results.length !== count) throw Error(`${name}: result/count mismatch`);
  total += count;
  process.stdout.write(`${name}: ${count} passed\n`);
}
process.stdout.write(`TOTAL: ${total} cases in ${suites.length} suites\n`);
