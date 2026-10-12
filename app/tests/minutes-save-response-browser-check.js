// Production React/queue, synthetic IPC commit followed by response failure.
// This does not simulate a native transport outage or change a real database.
async (page, captureDirectory = "output/playwright/minutes-response-20261012") => {
  const check = (value, message) => { if (!value) throw Error(message); };
  await page.reload();
  await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
  await page.getByRole("tab", { name: "議事録", exact: true }).click();
  await page.getByRole("button", { name: "議事録を編集", exact: true }).click();
  const field = page.getByRole("textbox", { name: "議事録ブロック1", exact: true });
  const primary = page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const finish = primary.getByRole("button", { name: "編集を終了", exact: true });
  const retry = primary.getByRole("button", { name: "議事録保存を再試行", exact: true });
  const status = page.getByRole("status", { name: "議事録の保存状態", exact: true });
  const saves = () => page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "save_minutes_draft").length);
  await field.evaluate(e => { window.responseField = e; });
  await page.evaluate(() => {
    window.responseBaseline = structuredClone(window.__mon.drafts.a);
    window.responseGenerated = JSON.stringify(window.__mon.views.a.content);
    window.__mon.lostMinutesSaveResponse = true;
  });
  await field.fill("QA応答喪失後も保持"); await finish.click(); await retry.waitFor();
  check(await status.textContent() === "保存要確認" && await field.textContent() === "QA応答喪失後も保持" && await saves() === 1, "lost response discarded text, marked saved or replayed");
  await finish.click(); check(await saves() === 1 && await field.isVisible(), "unconfirmed commit allowed exiting or automatic replay");
  await field.fill("QA最新追記を保持"); await retry.click();
  await page.waitForFunction(() => document.querySelector('[aria-label="議事録の保存状態"]')?.textContent === "保存済み");
  check(await saves() === 2 && await field.evaluate(e => e === window.responseField) && await field.textContent() === "QA最新追記を保持", "retry replaced field or saved stale input");
  check(await page.evaluate(() => window.__mon.history.a.length === 2 && window.__mon.drafts.a.lockVersion === window.responseBaseline.lockVersion + 2 && JSON.stringify(window.__mon.drafts.a.payload.evidence) === JSON.stringify(window.responseBaseline.payload.evidence) && JSON.stringify(window.__mon.views.a.content) === window.responseGenerated), "reconciliation duplicated history or changed fixed evidence/generated text");
  check(await page.evaluate(() => document.activeElement?.getAttribute("aria-label") === "編集を終了"), "retry lost focus");
  await page.screenshot({ path: `${captureDirectory}/minutes-response-retry-success.png` });

  await page.evaluate(() => { window.__mon.lostMinutesSaveResponse = true; });
  await field.fill("QA一時的な入力"); await finish.click(); await retry.waitFor();
  await field.fill("QA最新追記を保持"); await finish.click();
  check(await status.textContent() === "保存要確認" && await saves() === 3 && await field.isVisible(), "reverting cleared uncertain commit or exited");
  await retry.click();
  await page.waitForFunction(() => document.querySelector('[aria-label="議事録の保存状態"]')?.textContent === "保存済み");
  check(await saves() === 4 && await page.evaluate(() => window.__mon.drafts.a.payload.blocks[0].content[0].text === "QA最新追記を保持"), "reverted local text was not saved against committed version");

  await page.evaluate(() => { window.__mon.lostMinutesSaveResponse = true; });
  await field.fill("QA同じ入力の照合のみ"); await finish.click(); await retry.waitFor();
  await retry.click();
  await page.waitForFunction(() => document.querySelector('[aria-label="議事録の保存状態"]')?.textContent === "保存済み");
  check(await saves() === 5 && await page.evaluate(() => window.__mon.history.a.length === 5), "same committed input was written twice");

  await page.evaluate(() => { window.__mon.lostMinutesSaveResponse = true; });
  await field.fill("QA競合時もローカル入力を保持"); await finish.click(); await retry.waitFor();
  await page.evaluate(() => { window.__mon.drafts.a.lockVersion++; window.__mon.drafts.a.payload.blocks[0].content[0].text = "別画面の編集"; });
  await retry.click();
  check(await status.textContent() === "保存要確認" && await saves() === 6 && await field.textContent() === "QA競合時もローカル入力を保持", "conflict silently overwrote or discarded local input");
  check(await page.evaluate(() => window.__mon.drafts.a.payload.blocks[0].content[0].text === "別画面の編集" && JSON.stringify(window.__mon.views.a.content) === window.responseGenerated), "conflict overwrote saved/generated text");
  await page.screenshot({ path: `${captureDirectory}/minutes-response-conflict-preserved.png` });
  return { passed: 4, cases: ["newer edits after lost commit", "revert after lost commit", "acknowledge without duplicate write", "refuse conflicting version without overwrite"], saveRequests: await saves(), nativeDatabase: false };
}
