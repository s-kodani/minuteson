// Actual main-window panels with theme-browser-fixture.js, not UI replicas.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const info = label => page.getByRole("button", { name: `${label}の説明を開く`, exact: true });
  const visit = async (label, expected) => {
    await info(label).click(); const popup = page.getByRole("dialog", { name: label, exact: true }); await popup.waitFor();
    check((await popup.innerText()).includes(expected), `${label}: explanation missing`);
    if(label==="話者の人数")check((await popup.innerText()).includes("各保存区間に同じ人数を割り当てません"), "fixed track count was incorrectly applied to each saved run");
    const box = await popup.boundingBox(), viewport = await page.evaluate(() => ({ width: document.documentElement.clientWidth, height: document.documentElement.clientHeight }));
    check(box.x >= 7.9 && box.y >= 7.9 && box.x + box.width <= viewport.width - 7.9 && box.y + box.height <= viewport.height - 7.9, `${label}: clipped help`);
    await page.keyboard.press("Escape"); await popup.waitFor({ state: "hidden" });
    check(await info(label).evaluate(e => e === document.activeElement), `${label}: focus missing`);
  };
  await page.reload(); await page.getByRole("combobox", { name: "表示テーマ", exact: true }).waitFor();
  await visit("録音セッション", "会議以外の音"); results.push("main recording range and consent help without recording");
  await page.locator(".meeting-name").first().click();
  await detailUI.openMeetingTools(); await detailUI.openDiarization();
  const expected = [["音声保存後の解析工程", "自動推定"], ["ローカル文字起こし", "現在版と編集"], ["会議の用語", "自動置換"], ["話者分離", "同一人物"], ["話者の人数", "トラック全体"], ["議事録", "根拠"]];
  for (const [label, text] of expected) { if (label === "議事録") await detailUI.openMinutes(); await visit(label, text); results.push(`production ${label}: keyboard dismissal and bounds`); }
  for (const [summary, label] of [["エンジン設定（端末内に保存）", "文字起こしの標準／高精度"], ["sherpa-onnx 設定（端末内に保存）", "話者分離の標準／高精度"], ["議事録エンジン設定（端末内に保存）", "議事録の標準／高精度"]]) {
    if (label === "話者分離の標準／高精度") await page.getByRole("tab", { name:"文字起こし",exact:true }).click();
    if (label === "議事録の標準／高精度") await detailUI.openMinutes();
    await page.getByText(summary, { exact: true }).click(); await visit(label, "未検証");
    results.push(`production ${label}: configuration values unchanged`);
  }
  await page.getByRole("tab", { name:"文字起こし", exact:true }).click();
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
  await editor.getByRole("button", { name: "音声を再生", exact: true }).click();
  await visit("発言の音声再生", "元トラック");
  check(!(await page.getByRole("region", { name: "会議の共通音声プレーヤー", exact: true }).innerText()).includes("同期未検証") && await page.locator(".toast-body").filter({hasText:"同期未検証"}).count()===1, "alignment warning is not an independent toast");
  results.push("production audio help keeps its explanation; unverified alignment is an independent toast, using synthetic silence only");
  await page.getByRole("button", { name: "設定", exact: true }).click();
  for (const [label, text] of [["表示テーマ", "OSの明るさ"], ["バックアップと追加復元", "置き換えません"], ["要確認データ", "別の操作"], ["会議削除", "完全削除"], ["音声だけの削除", "再解析"], ["モデル本体削除", "再準備"], ["共通用語", "次回"], ["AIモデル", "起動しません"], ["モデル配置記録", "実会議"], ["プロファイル構成記録", "保証しません"]]) {
    await visit(label, text); results.push(`production ${label}: explanation only, no native admission`);
  }
  for (const theme of ["light", "dark"]) {
    await page.getByRole("combobox", { name: "表示テーマ", exact: true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
    await page.setViewportSize({ width: 360, height: 640 }); await info("バックアップと追加復元").click();
    await page.getByRole("dialog", { name: "バックアップと追加復元", exact: true }).getByRole("link", { name: "詳しい説明", exact: true }).click();
    await page.screenshot({ path: `info-production-${theme}-360.png` });
    await page.keyboard.press("Escape"); await page.setViewportSize({ width: 1200, height: 900 });
    results.push(`production ${theme}: short and full explanations in 360px window`);
    await page.setViewportSize({ width: 960, height: 640 }); await info("バックアップと追加復元").click();
    await page.screenshot({ path: `info-production-${theme}-960.png` }); await page.keyboard.press("Escape");
    check(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), "supported 960px main window overflows");
    await page.setViewportSize({ width: 1200, height: 900 });
  }
  const commands = await page.evaluate(() => window.themeFixture.calls.filter(c => /^(start_|delete_|apply_|restore_|place_|retry_|save_(?!ui_preferences)|create_|confirm_)/u.test(c.command)).map(c => c.command));
  check(commands.length === 0, `help issued a mutation: ${commands}`);
  results.push("all help visits preserve model/data/settings except explicitly requested simulated theme changes");
  return { passed: results.length, results };
}
