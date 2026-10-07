// Real React and synthetic export receipts only; never opens an OS save dialog.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  const write = () => page.getByRole("button", { name: "Markdownを書き出す", exact: true }).click();
  const boot = async mode => { await page.reload(); await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openMinutes(); await page.getByRole("button", { name: "Markdownを書き出す", exact: true }).waitFor(); await page.evaluate(m => { window.__mon.exportMode = m; }, mode); };
  await boot("success"); await write(); await host.getByRole("status").filter({ hasText: "書き出し完了" }).waitFor();
  check((await host.innerText()).includes("成功：") && await page.evaluate(() => window.__mon.exports.length) === 1, "persisted export was not a single successful result");
  results.push("a successful export receipt emits a success toast once after persistence");
  await boot("cancel"); await write(); await host.getByRole("status").filter({ hasText: "書き出しを取り消しました" }).waitFor();
  check((await host.innerText()).includes("情報：") && await page.evaluate(() => window.__mon.exports.length) === 0, "cancellation fabricated success or an export record");
  results.push("cancelled export is information, not a successful file write");
  await boot("fail"); await write(); await host.getByRole("alert").filter({ hasText: "書き出し失敗" }).waitFor();
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  await page.getByText("Markdown出力履歴（最新50件）", { exact: true }).click();
  check(await page.getByLabel("議事録の書き出しを要確認の要対応状態", { exact: true }).isVisible() && await page.getByText("書き出し失敗", { exact: true }).isVisible(), "error close cleared its retained issue or failed output history");
  check(await page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "export_minutes_markdown").length) === 1, "closing failed export retried writing");
  results.push("failed export is important and retains its issue and failed history after dismissal without rewriting");
  for (const state of ["writing", "interrupted"]) {
    await boot("success"); await page.evaluate(s => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (cmd, args) => { const reply = await invoke(cmd, args); if (cmd === "export_minutes_markdown" && reply) { reply.state = s; window.__mon.exports[0].state = s; } return reply; };
    }, state);
    await write(); await host.getByRole("alert").filter({ hasText: "結果未確定" }).waitFor();
    check(!(await host.innerText()).includes("成功：") && await page.getByLabel("議事録の書き出しを要確認の要対応状態", { exact: true }).isVisible(), `${state} export was treated as succeeded`);
    results.push(`${state} export stays an important unknown outcome rather than a success`);
  }
  await boot("success"); await page.evaluate(() => { window.__mon.lostExportResponse = true; }); await write();
  await page.getByRole("button", { name: "前回の書き出し結果を確認", exact: true }).click(); await host.getByRole("status").filter({ hasText: "書き出し完了" }).waitFor();
  check(await page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "export_minutes_markdown").length) === 1 && await page.evaluate(() => window.__mon.exports.length) === 1, "lost receipt caused a second file write");
  results.push("a lost reply is resolved by the existing same-operation status read, without a second file write");
  return { passed: results.length, results };
}
