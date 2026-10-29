// Self-contained browser verification. Playwright owns Vite, browser and cleanup.
// Synthetic IPC only; this never opens the Tauri application or real recordings.
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const app = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = name => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
if (args.includes("--cli") || args.includes("--session")) throw Error("Browser sessions are now owned by this runner; use --only SUITES or Playwright options.");
const forwarded = args.filter((value, index) => !["--only", "--port"].includes(value) && !["--only", "--port"].includes(args[index - 1]));
const run = spawnSync(process.execPath, [resolve(app, "node_modules/@playwright/test/cli.js"), "test", ...forwarded], {
  cwd: app, stdio: "inherit", env: { ...process.env, ...(option("--only") ? { MON_BROWSER_SUITES: option("--only") } : {}), ...(option("--port") ? { MON_BROWSER_PORT: option("--port") } : {}) },
});
if (run.error) throw run.error;
process.exitCode = run.status ?? 1;
