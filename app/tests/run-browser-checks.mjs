// Self-contained browser verification. Playwright owns Vite, browser and cleanup.
// Synthetic IPC only; this never opens the Tauri application or real recordings.
import { spawnSync } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const app = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function browserOutputDirectory(port, diagnostic = false) {
  const number = Number(port);
  if (!Number.isSafeInteger(number) || number < 1 || number > 65535) throw Error("Invalid MON_BROWSER_PORT");
  const output = number === 1420 ? "../output/playwright" : `../output/playwright/port-${number}`;
  return diagnostic ? `${output}/diagnostics` : output;
}

function failureMessage(test) {
  const result = test.results?.find(item => item.error?.message || item.errors?.some(error => error.message));
  return result?.error?.message ?? result?.errors?.map(error => error.message).filter(Boolean).join("\n") ?? "";
}

export function failedBrowserDetails(report) {
  const failures = new Map();
  const visit = suite => {
    for (const spec of suite.specs ?? []) {
      const test = spec.tests?.find(item => item.status === "unexpected");
      if (test && !failures.has(spec.title)) failures.set(spec.title, failureMessage(test));
    }
    for (const child of suite.suites ?? []) visit(child);
  };
  visit(report);
  return [...failures].map(([title, message]) => ({ title, message }));
}

export function failedBrowserTests(report) {
  return failedBrowserDetails(report).map(failure => failure.title);
}

// GitHub only promotes workflow commands to check annotations. The job log
// itself is not readable from the checks API, so a bare exit code hides the
// suite name. Keep this on one line; newlines and percents would be eaten.
function encodeWorkflow(value) {
  return value.replaceAll("%", "%25").replaceAll("\r", "%0D").replaceAll("\n", "%0A");
}

export function browserFailureCommands(report) {
  return failedBrowserDetails(report).map(failure => {
    const detail = failure.message.replace(/\s+/g, " ").trim();
    const text = (detail ? `${failure.title}: ${detail}` : failure.title).slice(0, 700);
    return `::error title=Browser regression::${encodeWorkflow(text)}`;
  });
}

export function runBrowserChecks(args, { spawn = spawnSync, read = readFileSync, remove = rmSync, env = process.env, log = console.error } = {}) {
  const option = name => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
  if (args.includes("--cli") || args.includes("--session")) throw Error("Browser sessions are now owned by this runner; use --only SUITES or Playwright options.");
  const forwarded = args.filter((value, index) => !["--only", "--port"].includes(value) && !["--only", "--port"].includes(args[index - 1]));
  const environment = { ...env, MON_BROWSER_DIAGNOSTIC: "0", ...(option("--only") ? { MON_BROWSER_SUITES: option("--only") } : {}), ...(option("--port") ? { MON_BROWSER_PORT: option("--port") } : {}) };
  delete environment.MON_BROWSER_FAILED_TESTS;
  const report = resolve(app, browserOutputDirectory(environment.MON_BROWSER_PORT ?? 1420), "results.json");
  // A startup failure must never replay failures from an earlier invocation.
  if (!args.includes("--list")) remove(report, { force: true });
  const command = [resolve(app, "node_modules/@playwright/test/cli.js"), "test"];
  const run = spawn(process.execPath, [...command, ...forwarded], { cwd: app, stdio: "inherit", env: environment });
  if (run.error) throw run.error;
  if (run.status !== 0 && !args.includes("--list")) {
    let failed = [];
    try {
      const parsed = JSON.parse(read(report, "utf8"));
      failed = failedBrowserTests(parsed);
      for (const command of browserFailureCommands(parsed)) log(command);
    } catch { /* Startup failures have no completed test report to replay. */ }
    if (failed.length) {
      log(`[browser] Capturing diagnostic traces for ${failed.join(", ")}; the original run remains failed.`);
      // Sharding and repeat-each apply to the first pass. Diagnostics replay
      // exactly its failed cases once, in an independent artifact directory.
      const replay = forwarded.filter((value, index) => value !== "--last-failed" && !/^--(?:shard|repeat-each|workers|trace|output|global-timeout)(?:=|$)/.test(value) && !["--shard", "--repeat-each", "--workers", "--trace", "--output", "--global-timeout"].includes(forwarded[index - 1]));
      // Bound the extra diagnostic work even when many cases fail together.
      const diagnostic = spawn(process.execPath, [...command, ...replay, "--workers", "1", "--trace", "on", "--global-timeout", "300000"], {
        cwd: app, stdio: "inherit", env: { ...environment, MON_BROWSER_DIAGNOSTIC: "1", MON_BROWSER_FAILED_TESTS: JSON.stringify(failed) },
      });
      if (diagnostic.error) log(`[browser] Diagnostic replay could not start: ${diagnostic.error.message}`);
    }
  }
  return run.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exitCode = runBrowserChecks(process.argv.slice(2));
