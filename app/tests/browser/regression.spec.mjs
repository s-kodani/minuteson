import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { basename } from "node:path";
import { browserSuites, fixtureReadiness } from "../browser-suites.mjs";
import { applyMinutesFixtureTheme } from "../minutes-browser-helpers.js";
import { applyPlaybackFixtureTheme } from "../playback-browser-helpers.js";

const selected = process.env.MON_BROWSER_SUITES ? new Set(process.env.MON_BROWSER_SUITES.split(",").flatMap(name => name === "internal-logs" ? browserSuites.filter(([suite]) => suite.startsWith("internal-logs-")).map(([suite]) => suite) : [name])) : null;
const suites = browserSuites.filter(([name]) => !selected || selected.has(name));
if (!suites.length || selected && suites.length !== selected.size) throw Error("Unknown or empty browser suite selection");
const failed = process.env.MON_BROWSER_FAILED_TESTS ? new Set(JSON.parse(process.env.MON_BROWSER_FAILED_TESTS)) : null;

for (const [name, fixture, source] of suites) {
  if (failed && !failed.has(name)) continue;
  test(name, async ({ page, baseURL }, info) => {
    page.setDefaultTimeout(name.startsWith("internal-logs-") ? 30_000 : 10_000);
    // Cold Vite module loads have a separate budget from control interactions.
    page.setDefaultNavigationTimeout(30_000);
    const port = new URL(baseURL).port;
    const localPort = value => value.replaceAll("1420", port);
    if (fixture) {
      const index = new RegExp(`^${baseURL.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/(?:\\?.*)?$`);
      await page.context().route(index, async route => {
        const response = await route.fetch();
        const body = (await response.text()).replace(/<script type="module" src="\/src\/(?:main|bootstrap)\.tsx[^"]*"><\/script>/u,
          `<script type="module">await import("/tests/${fixture}");</script>`);
        await route.fulfill({ response, body });
      });
      await page.goto(`${baseURL}/${["vad-settings", "all-model-choices", "whisper-variants", "qwen35-models"].includes(name) ? "?official=1" : ""}`);
      if (fixtureReadiness[fixture]) await page.waitForFunction(key => !!window[key], fixtureReadiness[fixture]);
    }
    const capturePath = path => info.outputPath(basename(path));
    const screenshot = page.screenshot.bind(page);
    page.screenshot = options => screenshot({ ...options, path: capturePath(options.path) });
    const detailUI = {
      approveModelLicense: async () => {
        const dialog = page.getByRole("dialog", { name: "利用条件を確認", exact: true });
        const checkbox = dialog.getByRole("checkbox", { name: "モデルと実行エンジンの利用条件を確認しました。", exact: true });
        await expect(checkbox).not.toBeChecked();
        await checkbox.check();
        await dialog.getByRole("button", { name: /^(ダウンロードして使用|実行環境を更新して使用|VADを追加して使用|使用)$/ }).click();
        await dialog.waitFor({ state: "hidden" });
      },
      internalLogsScenario: name.startsWith("internal-logs-") ? name.slice("internal-logs-".length) : undefined,
      prepareClipboard: async () => {
        if (info.project.name === "chromium") await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
        else await page.context().addInitScript(() => {
          let copied = "";
          Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { copied = text; }, readText: async () => copied } });
        });
      },
      openMeetingRow: async (title = "既存会議") => {
        const row = page.locator(".meeting-table tbody tr").filter({ has: page.locator(".meeting-name").filter({ hasText: title }) }).first();
        await row.waitFor();
        const trigger = row.locator(".meeting-row-actions").locator(":scope > button");
        if (await trigger.getAttribute("aria-expanded") !== "true") await trigger.click();
      },
      applyMinutesTheme: theme => applyMinutesFixtureTheme(page, theme),
      applyPlaybackTheme: theme => applyPlaybackFixtureTheme(page, theme),
      primaryModifier: process.platform === "darwin" ? "Meta" : "Control",
    };
    // Drivers are trusted, checked-in Node functions using visible controls.
    const driver = new Function(`return (${localPort(readFileSync(new URL(`../${source}`, import.meta.url), "utf8"))})`)();
    try {
      const result = await driver(page, capturePath, detailUI);
      const count = Array.isArray(result) ? result.length : result.passed ?? result.cases;
      expect(Number.isSafeInteger(count) && count > 0).toBe(true);
      if (Array.isArray(result.results)) expect(result.results).toHaveLength(count);
      await info.attach("assertions", { body: JSON.stringify(result), contentType: "application/json" });
    } finally { page.screenshot = screenshot; }
  });
}
