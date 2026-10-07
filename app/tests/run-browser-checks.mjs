// Local CLI-first browser verification. Start Vite and a named CLI browser
// first; --cli selects an already installed CLI (no implicit npm download).
// All fixture IPC is synthetic. This does not run Tauri, CI or real recording.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const tests = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2), option = name => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
if (!args.includes("--cli") || !args.includes("--session")) throw Error("Provide --cli PATH and --session NAME for an existing local Playwright CLI browser.");
const cli = resolve(option("--cli")), session = option("--session"), output = resolve(option("--output") ?? resolve(tests, "../../output/playwright/regression"));
const selected = args.includes("--only") ? new Set(option("--only").split(",")) : null;
const suites = [
  ["common", "common-modal-browser-fixture.tsx", "common-modal-browser-check.js"],
  ["action", "action-confirmation-browser-fixture.tsx", "action-confirmation-browser-check.js"],
  ["info", "info-popover-browser-fixture.tsx", "info-popover-browser-check.js"],
  ["production", "browser-fixture.js", "action-confirmation-production-browser-check.js"],
  ["document", "browser-fixture.js", "document-modal-browser-check.js"],
  ["document-action", "browser-fixture.js", "document-action-confirmation-browser-check.js"],
  ["import", "import-browser-fixture.js", "import-modal-browser-check.js"],
  ["data", "data-save-browser-fixture.js", "data-save-browser-check.js"],
  ["data-action", "data-save-browser-fixture.js", "data-action-confirmation-browser-check.js"],
  ["move", "data-save-browser-fixture.js", "storage-move-browser-check.js"],
  ["recovery", "data-save-browser-fixture.js", "recovery-browser-check.js"],
  ["recovery-action", "data-save-browser-fixture.js", "recovery-action-confirmation-browser-check.js"],
  ["deletion", "deletion-browser-fixture.js", "deletion-browser-check.js"],
  ["model-review", "model-deletion-browser-fixture.js", "model-deletion-browser-check.js"],
  ["model-body", "model-body-deletion-browser-fixture.js", "model-body-deletion-browser-check.js"],
  ["model-profile", "model-profile-browser-fixture.js", "model-profile-browser-check.js"],
  ["model-rebinding", "model-rebinding-browser-fixture.js", "model-rebinding-browser-check.js"],
  ["model-preparation", "model-preparation-browser-fixture.js", "model-preparation-browser-check.js"],
  ["audio", "audio-deletion-browser-fixture.js", "audio-deletion-browser-check.js"],
  ["info-production", "theme-browser-fixture.js", "info-production-browser-check.js"],
  ["theme", "theme-browser-fixture.js", "theme-browser-check.js"],
  ["recording", null, "recording-confirmation-browser-check.js"],
  ["close", null, "application-close-browser-check.js"],
  ["workspace", "browser-fixture.js", "workspace-browser-check.js"],
  ["detail-tabs", "browser-fixture.js", "detail-tabs-browser-check.js"],
  ["speaker-panel", "browser-fixture.js", "speaker-panel-browser-check.js"],
  ["fixed-player", "browser-fixture.js", "fixed-player-browser-check.js"],
  ["meeting-audio", "browser-fixture.js", "meeting-audio-browser-check.js"],
  ["playback-speakers", "browser-fixture.js", "playback-speaker-browser-check.js"],
  ["estimated-speakers", "browser-fixture.js", "estimated-speaker-colors-browser-check.js"],
  ["playback-cancellation", "browser-fixture.js", "playback-cancellation-browser-check.js"],
  ["display-follow", "browser-fixture.js", "display-follow-browser-check.js"],
  ["keyboard", "browser-fixture.js", "keyboard-browser-check.js"],
  ["transcript-edit", "browser-fixture.js", "transcript-edit-browser-check.js"],
  ["transcript-recovery", "browser-fixture.js", "transcript-recovery-browser-check.js"],
  ["speaker-name", "browser-fixture.js", "speaker-name-browser-check.js"],
  ["speaker-name-recovery", "browser-fixture.js", "speaker-name-recovery-browser-check.js"],
  ["transcript-conflict", "browser-fixture.js", "transcript-conflict-browser-check.js"],
  ["toast-production", "browser-fixture.js", "toast-production-browser-check.js"],
  ["vocabulary-toast", "browser-fixture.js", "vocabulary-toast-browser-check.js"],
  ["theme-toast", "theme-browser-fixture.js", "theme-toast-browser-check.js"],
  ["import-toast", "import-browser-fixture.js", "import-toast-browser-check.js"],
  ["recovery-toast", "data-save-browser-fixture.js", "recovery-toast-browser-check.js"],
  ["model-preparation-toast", "model-preparation-browser-fixture.js", "model-preparation-toast-browser-check.js"],
  ["model-profile-toast", "model-profile-browser-fixture.js", "model-profile-toast-browser-check.js"],
  ["job-notifications", "browser-fixture.js", "job-notifications-browser-check.js"],
  ["publication-toast", "browser-fixture.js", "publication-toast-browser-check.js"],
  // addInitScript affects subsequent navigations in the selected browser page.
  ["minutes", "minutes-init", "minutes-modal-browser-check.js"],
  ["detail-minutes", "minutes-init", "detail-minutes-browser-check.js"],
  ["minutes-panel", "minutes-init", "minutes-panel-browser-check.js"],
  ["frozen-speakers", "minutes-init", "frozen-speaker-colors-browser-check.js"],
  ["meeting-minutes-audio", "minutes-init", "meeting-minutes-audio-browser-check.js"],
  ["display-minutes", "minutes-init", "display-minutes-browser-check.js"],
  ["minutes-edit", "minutes-init", "minutes-edit-confirmation-browser-check.js"],
  ["minutes-toast", "minutes-init", "minutes-toast-browser-check.js"],
  ["minutes-publication-toast", "minutes-init", "minutes-publication-toast-browser-check.js"],
  ["toast", "toast-browser-fixture.tsx", "toast-browser-check.js"],
].filter(([name]) => !selected || selected.has(name));
if (!suites.length || selected && suites.length !== selected.size) throw Error("Unknown or empty suite selection");
mkdirSync(output, { recursive: true });
let total = 0;
const indexRoute = /^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/;
const entryScript = /<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u;
const readiness = {
  "common-modal-browser-fixture.tsx": "modalFixture", "action-confirmation-browser-fixture.tsx": "actionFixture",
  "info-popover-browser-fixture.tsx": "infoFixture", "browser-fixture.js": "monBrowserFixture",
  "import-browser-fixture.js": "monImportFixture", "model-deletion-browser-fixture.js": "modelDeletionFixture",
  "data-save-browser-fixture.js": "dataFixture", "deletion-browser-fixture.js": "deletionFixture",
  "model-body-deletion-browser-fixture.js": "modelBodyFixture", "model-profile-browser-fixture.js": "modelProfileFixture",
  "model-rebinding-browser-fixture.js": "rebindingFixture", "model-preparation-browser-fixture.js": "modelPreparationFixture",
  "audio-deletion-browser-fixture.js": "audioDeletionFixture", "theme-browser-fixture.js": "themeFixture",
  "toast-browser-fixture.tsx": "toastFixture",
};
for (const [name, fixture, source] of suites) {
  const check = readFileSync(resolve(tests, source), "utf8");
  const setup = fixture === "minutes-init" ? `await page.goto("http://127.0.0.1:1420/"); await (${readFileSync(resolve(tests, "minutes-browser-fixture.js"), "utf8")})(page);`
    : fixture ? `await page.context().route(${indexRoute}, async route=>{const response=await route.fetch(); const body=(await response.text()).replace(${entryScript}, ${JSON.stringify(`<script type="module">await import("/tests/${fixture}");</script>`)}); await route.fulfill({response,body});}); await page.goto("http://127.0.0.1:1420/");` : "";
  const ready = readiness[fixture] ? `await page.waitForFunction(name=>!!window[name],${JSON.stringify(readiness[fixture])});` : "";
  // The browser daemon's cwd can differ from this runner's cwd. Always route
  // generated captures to the explicit output directory, not the repo root.
  const capturePrefix = JSON.stringify(resolve(output, name + "-"));
  // Explicit drivers for the production workspace: real tab/summary clicks,
  // never forcing hidden controls visible or replacing the application's UI.
  const detailsDriver = `const openDetails=async selector=>{const details=page.locator(selector);await details.waitFor({state:"attached"});if(!await details.evaluate(e=>e.open))await details.locator(":scope > summary").click();};const waitSpeakerLayout=()=>page.waitForFunction(()=>{const panel=document.querySelector(".workspace-panel-shell");return !panel||panel.dataset.narrow===String(matchMedia("(max-width: 1119px)").matches);});const detailUI={openMeetingTools:()=>openDetails(".meeting-workspace-tools"),openTranscriptTools:()=>openDetails(".transcript-workspace-tools"),openSpeakerPanel:async()=>{await waitSpeakerLayout();const button=page.getByRole("button",{name:"話者一覧を開く",exact:true});if(await button.isVisible()&&await button.getAttribute("aria-expanded")!=="true")await button.click();},closeSpeakerPanel:async()=>{await waitSpeakerLayout();const dialog=page.locator(".workspace-drawer[open]");if(await dialog.count())await dialog.getByRole("button",{name:"話者一覧を閉じる",exact:true}).click();},openDiarization:async()=>{const tools=page.locator(".speaker-diarization-tools");if(await tools.count()){await detailUI.openSpeakerPanel();await openDetails(".speaker-diarization-tools");}},openMinutes:async()=>{await page.getByRole("tab",{name:"議事録",exact:true}).click();await openDetails(".minutes-workspace-tools");}};`;
  const code = `async page=>{await page.goto("about:blank");await page.unrouteAll();await page.context().unrouteAll();page.setDefaultTimeout(10000);await page.setViewportSize({width:1200,height:900});${setup}${ready}const capturePath=path=>${capturePrefix}+path.split(/[\\/]/u).at(-1);${detailsDriver}const screenshot=page.screenshot.bind(page);page.screenshot=options=>screenshot({...options,path:capturePath(options.path)});try{return await (${check})(page,capturePath,detailUI);}finally{page.screenshot=screenshot;}}`;
  const run = spawnSync(process.execPath, [cli, `-s=${session}`, "run-code", code], { cwd: output, encoding: "utf8", env: { ...process.env, NO_UPDATE_NOTIFIER: "1" }, maxBuffer: 8 * 1024 * 1024 });
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
