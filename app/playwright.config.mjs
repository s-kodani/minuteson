import { defineConfig } from "@playwright/test";
import { browserOutputDirectory } from "./tests/run-browser-checks.mjs";

const port = Number(process.env.MON_BROWSER_PORT ?? 1420);
if (!Number.isSafeInteger(port) || port < 1 || port > 65535) throw Error("Invalid MON_BROWSER_PORT");
const diagnostic = process.env.MON_BROWSER_DIAGNOSTIC === "1";
const output = browserOutputDirectory(port, diagnostic);
const baseURL = `http://127.0.0.1:${port}`;
export default defineConfig({
  testDir: "./tests/browser",
  timeout: 120_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  workers: process.env.CI ? 2 : 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  outputDir: `${output}/results`,
  reporter: [["list"], ["html", { outputFolder: `${output}/report`, open: "never" }], ["json", { outputFile: `${output}/results.json` }]],
  // The runner records a full diagnostic trace only for failed cases, while
  // preserving the initial failure and its report. Successful cases avoid
  // thousands of DOM snapshots, including the 1,000-row log history.
  use: { baseURL, viewport: { width: 1200, height: 900 }, trace: diagnostic ? "on" : "off", screenshot: "only-on-failure" },
  projects: [{ name: process.platform === "darwin" ? "webkit" : "chromium", use: { browserName: process.platform === "darwin" ? "webkit" : "chromium" } }],
  webServer: { command: `npm run dev -- --port ${port}`, url: baseURL, reuseExistingServer: !process.env.CI, timeout: 30_000 },
});
