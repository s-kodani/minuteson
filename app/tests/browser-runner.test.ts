import test from "node:test";
import assert from "node:assert/strict";
import { browserFailureCommands, browserOutputDirectory, failedBrowserTests, runBrowserChecks } from "./run-browser-checks.mjs";

const report = { suites: [{ suites: [{ specs: [
  { title: "meeting-mix-progress", tests: [{ status: "unexpected" }] },
  { title: "internal-logs-history", tests: [{ status: "unexpected" }] },
  { title: "common", tests: [{ status: "expected" }] },
  { title: "skipped", tests: [{ status: "skipped" }] },
] }] }] };

test("diagnostics select only failed tests from nested Playwright reports", () => {
  assert.deepEqual(failedBrowserTests(report), ["meeting-mix-progress", "internal-logs-history"]);
  assert.notEqual(browserOutputDirectory(1420), browserOutputDirectory(1420, true));
  assert.equal(browserOutputDirectory(1432, true), "../output/playwright/port-1432/diagnostics");
});

test("failed suites are published as single-line GitHub annotations", () => {
  const detailed = { suites: [{ specs: [{ title: "workspace", tests: [{ status: "unexpected", results: [{ error: { message: "Error: header moved\npageWidth 100%" } }] }] }] }] };
  assert.deepEqual(browserFailureCommands(detailed), ["::error title=Browser regression::workspace: Error: header moved pageWidth 100%25"]);
  assert.deepEqual(browserFailureCommands(report), [
    "::error title=Browser regression::meeting-mix-progress",
    "::error title=Browser regression::internal-logs-history",
  ]);
});

test("a passing diagnostic replay cannot turn the original failure into success", () => {
  const calls: any[] = [];
  const logged: string[] = [];
  const code = runBrowserChecks(["--only", "internal-logs", "--port", "1432", "--shard=1/2", "--repeat-each", "3", "--workers=2", "--output", "initial-artifacts"], {
    env: { CI: "true", MON_BROWSER_FAILED_TESTS: '["old-filter"]' },
    remove: () => {},
    read: () => JSON.stringify(report),
    log: message => logged.push(message),
    spawn: (...args: any[]) => { calls.push(args); return { status: calls.length === 1 ? 1 : 0 }; },
  });
  assert.equal(code, 1);
  assert.deepEqual(logged.slice(0, 2), browserFailureCommands(report));
  assert.equal(calls.length, 2);
  assert.equal(calls[0][2].env.MON_BROWSER_DIAGNOSTIC, "0");
  assert.equal(calls[0][2].env.MON_BROWSER_FAILED_TESTS, undefined);
  assert.equal(calls[1][2].env.MON_BROWSER_DIAGNOSTIC, "1");
  assert.deepEqual(JSON.parse(calls[1][2].env.MON_BROWSER_FAILED_TESTS), ["meeting-mix-progress", "internal-logs-history"]);
  assert.equal(calls[1][2].env.MON_BROWSER_SUITES, "internal-logs");
  assert.equal(calls[1][2].env.MON_BROWSER_PORT, "1432");
  assert(!calls[1][1].some((arg: string) => arg.startsWith("--shard") || arg.startsWith("--repeat-each") || arg === "--output" || arg === "initial-artifacts"));
  assert.deepEqual(calls[1][1].slice(-6), ["--workers", "1", "--trace", "on", "--global-timeout", "300000"]);
});

test("successful runs and startup failures without a report do not replay", () => {
  for (const status of [0, 1]) {
    let calls = 0;
    assert.equal(runBrowserChecks([], {
      env: {},
      remove: () => {},
      spawn: () => { calls++; return { status }; },
      read: () => { throw Error("no report"); },
    }), status);
    assert.equal(calls, 1);
  }
});

test("a diagnostic startup error preserves the initial exit status", () => {
  let calls = 0;
  assert.equal(runBrowserChecks([], {
    env: {},
    remove: () => {},
    read: () => JSON.stringify(report),
    log: () => {},
    spawn: () => ++calls === 1 ? { status: 2 } : { status: null, error: Error("spawn failed") },
  }), 2);
});

test("listing preserves reports and invalid ports cannot select a filesystem path", () => {
  assert.equal(runBrowserChecks(["--list"], {
    env: {},
    spawn: () => ({ status: 0 }),
    remove: () => { throw Error("a listing must not remove the report"); },
  }), 0);
  for (const port of ["../../outside", "0", "65536"]) {
    assert.throws(() => browserOutputDirectory(port), /Invalid MON_BROWSER_PORT/);
  }
});
