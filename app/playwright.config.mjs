import { defineConfig } from "@playwright/test";

const port = Number(process.env.MON_BROWSER_PORT ?? 1420);
if (!Number.isSafeInteger(port) || port < 1 || port > 65535) throw Error("Invalid MON_BROWSER_PORT");
const output = port === 1420 ? "../output/playwright" : `../output/playwright/port-${port}`;
const baseURL = `http://127.0.0.1:${port}`;
export default defineConfig({
  testDir: "./tests/browser",
  timeout: 120_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  outputDir: `${output}/results`,
  reporter: [["list"], ["html", { outputFolder: `${output}/report`, open: "never" }]],
  use: { baseURL, viewport: { width: 1200, height: 900 }, trace: "retain-on-failure", screenshot: "only-on-failure" },
  projects: [{ name: process.platform === "darwin" ? "webkit" : "chromium", use: { browserName: process.platform === "darwin" ? "webkit" : "chromium" } }],
  webServer: { command: `npm run dev -- --port ${port}`, url: baseURL, reuseExistingServer: !process.env.CI, timeout: 30_000 },
});
