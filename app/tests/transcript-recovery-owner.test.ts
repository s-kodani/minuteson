import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const panel = readFileSync(new URL("../src/transcript-recovery-panel.tsx", import.meta.url), "utf8");
const transcript = readFileSync(new URL("../src/transcript.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/prototype-alignment.css", import.meta.url), "utf8");

test("recovery inventory shares the edit-options native boundary without remounting its controller", () => {
  assert.match(panel, /renderPanel\?: \(entry: ReactNode, view: ModalReplacement \| null, persistentBody: ReactNode\) => ReactNode/);
  assert.match(panel, /if \(renderPanel\) return renderPanel\(entry, open \?/);
  assert.match(panel, /key: "transcript-recovery", labelledBy: titleId, content: null/);
  assert.match(panel, /className="transcript-recovery-view" hidden=\{!open\}>\{body\}/);
  assert.match(panel, /return <>\{entry\}<CommonModal/);
  assert.equal((panel.match(/new TranscriptRecoveryDiscard/g) ?? []).length, 1);
  assert.equal((transcript.match(/<TranscriptRecoveryPanel/g) ?? []).length, 1);
  assert.match(transcript, /renderPanel=\{\(recoveryEntry,recoveryView,persistentRecovery\) =>\s*<OptionsModal/);
  assert.match(transcript, /replacement=\{recoveryView\} persistentReplacement=\{persistentRecovery\}/);
  assert.match(transcript, /\{recoveryEntry\}<\/div>/);
});

test("recovery presentation preserves identity, unknown-result and destructive confirmation guards", () => {
  assert.match(panel, /result.operationId !== item.operationId \|\| result.lockVersion !== item.lockVersion \|\| result.contentHash !== item.contentHash/);
  assert.match(panel, /const destructiveBlocked = blocked \|\| busy \|\| !!state.pending \|\| state.busy \|\| \(current && !canChangeCurrent\)/);
  assert.match(panel, /validate: \(\) => preview === expected && !discard.snapshot\(\).pending/);
  assert.match(panel, /onDismiss: \(\) => setOpen\(false\), dismissLabel: "閉じる"/);
  assert.match(panel, /blocked: busy \|\| state.busy \|\| confirm.pending/);
  assert.equal((panel.match(/event.currentTarget.focus\(\{preventScroll:true\}\)/g) ?? []).length, 3);
  assert.match(transcript, /record.currentDraftLockVersion === current.lockVersion && record.currentDraftContentHash === current.contentHash/);
});

test("recovery navigation and record actions have explicit spacing instead of touching inline buttons", () => {
  assert.match(panel, /className="recovery-inventory-item"/);
  assert.equal((panel.match(/className="recovery-pagination"/g) ?? []).length, 2);
  assert.match(css, /\.recovery-pagination,\.recovery-actions\) \{ display:flex; flex-wrap:wrap; align-items:center; gap:8px/);
  assert.match(css, /\.recovery-inventory-item \{ display:grid; justify-items:start; gap:12px/);
});

test("visible-control recovery boundary regression is registered without destructive actions", () => {
  const driver = readFileSync(new URL("./transcript-recovery-owner-browser-check.js", import.meta.url), "utf8");
  const runner = readFileSync(new URL("./run-browser-checks.mjs", import.meta.url), "utf8");
  assert.match(runner, /\["transcript-recovery-owner", "browser-fixture.js", "transcript-recovery-owner-browser-check.js"\]/);
  assert.match(driver, /i < 2/);
  assert.match(driver, /inventoryEntry.evaluate\(e => e === document.activeElement\)/);
  assert.match(driver, /entry.evaluate\(e => e === document.activeElement\)/);
  assert.doesNotMatch(driver, /window\.monBrowserFixture|discard_transcript|この回復入力を破棄/);
  assert.doesNotThrow(() => new Function(`return (${driver})`));
});
