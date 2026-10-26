// CLI-first verification with mocked IPC; not a Playwright test spec.
// Route local index.html to data-save-browser-fixture.js, then run this file
// with playwright-cli run-code --filename <absolute path>.
async (page) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const set = values => page.evaluate(v => Object.assign(window.dataFixture, v), values);
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(x => x.command === c), command);
  const poll = async () => {
    const before = (await calls("get_data_operation")).length;
    await page.waitForFunction(n => window.dataFixture.calls.filter(c => c.command === "get_data_operation").length > n, before);
  };
  const boot = async () => {
    await page.reload();
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await page.getByRole("region", { name: "データと保存" }).waitFor();
  };
  const region = () => page.getByRole("region", { name: "データと保存" });
  const notifications = () => page.getByRole("complementary", { name: "通知", exact: true });
  const choose = kind => region().getByRole("button", { name: kind === "backup" ? "バックアップを作成" : "バックアップから追加復元", exact: true }).click();
  const done = async () => {
    await page.waitForFunction(() => window.dataFixture.snapshot.phase === "completed");
    await notifications().getByRole("status").filter({ hasText: /バックアップを作成しました|会議 2 件を追加復元しました/u }).waitFor();
  };
  const confirm = () => page.getByRole("dialog").getByRole("button", { name: "追加して復元", exact: true }).click();
  const ready = async () => { await page.getByRole("dialog").waitFor(); await page.getByRole("heading", { name: "バックアップから追加復元", exact: true }).waitFor(); };
  const stopChoice = () => page.getByRole("dialog", { name: "データ操作の安全な取消を確認", exact: true });
  const stop = async () => { await region().getByRole("button", { name: "処理を取り消す", exact: true }).click(); await stopChoice().getByRole("button", { name: "取消を要求して停止を待つ", exact: true }).click(); };

  await boot(); await page.getByRole("button", { name: "バックアップと追加復元の説明を開く", exact: true }).click();
  await page.getByRole("dialog", { name: "バックアップと追加復元", exact: true }).getByRole("link", { name: "詳しい説明", exact: true }).click();
  const info = await page.getByRole("dialog", { name: "バックアップと追加復元", exact: true }).innerText();
  check(info.includes("AIモデル本体") && info.includes("独自暗号化やクラウド同期ではありません") && info.includes("新しい別会議"), "data scope explanation missing");
  results.push("scope info: no model-body, encryption or replace-existing claim");

  await boot(); await set({ mode: "delay" }); await choose("backup");
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === "publishing");
  check((await calls("start_data_operation")).length === 1 && await page.getByRole("dialog").count() === 0, "new output did not publish once without overwrite modal");
  check(await page.getByRole("region", { name: "登録した声の自動適用", exact: true }).getByRole("spinbutton", { name: "類似度の下限", exact: true }).isDisabled(), "editing enabled during native work");
  check(await page.getByRole("navigation", { name: "メインナビゲーション", exact: true }).getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "navigation enabled during native work");
  check(!await notifications().getByRole("status").filter({ hasText: "バックアップを作成しました。" }).count(), "partial output labelled usable");
  await page.evaluate(() => window.dataFixture.complete()); await done();
  check(await page.evaluate(() => window.dataFixture.published) === 1, "backup duplicated");
  results.push("new backup: one admission, busy locks, terminal-only success");

  await boot(); await set({ deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4 }); await choose("backup"); await done();
  const deletedWarning = region().getByRole("alert").filter({ hasText: "音声削除済みの会議" });
  await deletedWarning.waitFor();
  const deletedText = await deletedWarning.innerText();
  check(deletedText.includes("会議 1 件") && deletedText.includes("音声資産 4 件") && deletedText.includes("保存済み文書と音声の欠落情報") && deletedText.includes("音声本体は含みません") && deletedText.includes("削除済み音声の再生・再解析はできません"), "deleted-source counts or restore limitation missing");
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 1000 });
    await deletedWarning.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `output/playwright/deleted-audio-backup-${width}.png` });
  }
  results.push("deleted source: explicit history-only counts and no audio replay/reanalysis claim");

  await boot(); await set({ deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4 }); await choose("restore"); await ready();
  const historyWarning = page.getByRole("dialog").getByRole("alert").filter({ hasText: "音声削除済みの会議" });
  await historyWarning.waitFor();
  const historyText = await historyWarning.innerText();
  check(historyText.includes("会議 1 件") && historyText.includes("音声資産 4 件") && historyText.includes("保存済み文書と音声の欠落情報") && historyText.includes("再生・再解析はできません"), "history-only restore confirmation missing");
  check((await calls("start_data_operation")).length === 0, "history-only preview published before confirmation");
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.screenshot({ path: `output/playwright/deleted-audio-restore-confirm-${width}.png` });
  }
  await set({ mode: "lost", suppressEvents: true }); await confirm(); await done();
  const restoredWarning = region().getByRole("alert").filter({ hasText: "保存済み文書と音声の欠落情報を復元しました" });
  await restoredWarning.waitFor();
  check((await restoredWarning.innerText()).includes("音声資産 4 件"), "published history-only receipt lost counts");
  check((await calls("start_data_operation")).length === 1 && await page.evaluate(() => window.dataFixture.published) === 1, "history-only restore duplicated on lost reply");
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 1000 });
    await restoredWarning.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `output/playwright/deleted-audio-restore-result-${width}.png` });
  }
  results.push("history-only restore: confirmation and terminal warning, lost-reply polling without duplicate admission");

  await boot(); await set({ deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4, mode: "unknownStats" }); await choose("restore"); await ready(); await confirm(); await done();
  check((await region().innerText()).includes("記録不明") && (await region().innerText()).includes("音声資産 4 件"), "terminal replay fabricated counts or omitted unavailable warning");
  results.push("history-only terminal replay: exact unavailable counts and unknown historical merge stats");

  await boot(); await set({ replacesExisting: true }); await choose("backup");
  await page.getByRole("dialog").waitFor();
  check((await calls("start_data_operation")).length === 0 && (await page.getByRole("dialog").innerText()).includes("/safe/会議.monbackup"), "overwrite ran without target confirmation");
  await page.keyboard.press("Escape"); await page.getByRole("dialog").waitFor({ state: "hidden" });
  check((await calls("start_data_operation")).length === 0, "Escape published backup");
  await page.waitForFunction(() => document.activeElement?.textContent === "バックアップを作成");
  results.push("overwrite selection: explicit target confirmation, Escape cancels and returns focus");

  await boot(); await set({ replacesExisting: true }); await choose("backup");
  await page.getByRole("dialog").getByRole("button", { name: "上書きして作成", exact: true }).click(); await done();
  check((await calls("start_data_operation")).length === 1, "overwrite not exactly once");
  results.push("overwrite: explicit confirmation publishes once");

  await boot(); await choose("restore"); await ready();
  const first = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.restorePreview));
  check((await calls("start_data_operation")).length === 0, "preview published meetings");
  const body = await page.getByRole("dialog").innerText();
  check(body.includes("会議 2 件") && body.includes("32 MiB") && body.includes("新しい別会議") && body.includes("既存を保持する競合 1 件"), "restore review missing");
  await confirm(); await done();
  check(await page.evaluate(() => window.dataFixture.meetings.length) === 3, "existing meeting replaced");
  await region().getByRole("button", { name: "復元した会議一覧を開く", exact: true }).click();
  await page.getByRole("heading", { name: "会議一覧", exact: true }).waitFor();
  check(await page.getByText("バックアップから復元", { exact: true }).count() === 2 && await page.getByText("既存会議", { exact: true }).count() === 0, "restored filter or provenance missing");
  await page.getByRole("button", { name: "すべての会議を表示", exact: true }).click();
  check(await page.getByText("既存会議", { exact: true }).count() === 1, "existing meeting lost from all-meetings view");
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await choose("restore"); await ready();
  const second = await page.evaluate(() => window.dataFixture.snapshot.restorePreview);
  check(first.batchId !== second.batchId && first.archiveId === second.archiveId, "explicit new operation changed source or reused batch");
  await confirm(); await done();
  check(await page.evaluate(() => window.dataFixture.meetings.length) === 5, "explicit second operation did not add distinct meetings");
  results.push("restore: review, existing preservation, provenance/filter and explicit new batch");

  await boot(); await set({ previewMode: "capacity" }); await choose("restore"); await ready();
  check(await page.getByRole("dialog").getByRole("button", { name: "追加して復元", exact: true }).isDisabled(), "insufficient capacity admitted");
  const old = await page.evaluate(() => window.dataFixture.snapshot.restorePreview);
  await page.getByRole("dialog").getByRole("button", { name: "同じバックアップを再検証", exact: true }).click(); await ready();
  const refreshed = await page.evaluate(() => window.dataFixture.snapshot.restorePreview);
  check(old.batchId === refreshed.batchId && old.confirmationId !== refreshed.confirmationId, "refresh lost batch or retained stale confirmation");
  await confirm(); await done();
  results.push("capacity refusal + same-batch refreshed confirmation");

  await boot(); await set({ previewMode: "invalid" }); await choose("restore");
  await notifications().getByRole("alert").filter({ hasText: "不正な形式" }).waitFor();
  check((await calls("start_data_operation")).length === 0 && await page.getByRole("dialog").count() === 0, "invalid archive exposed confirmation");
  await page.waitForFunction(() => document.activeElement?.textContent === "バックアップから追加復元" && !document.activeElement.disabled);
  results.push("invalid archive: no publication capability, enabled selection entry regains focus");

  await boot(); await set({ previewMode: "cancel" }); await choose("restore");
  await page.waitForFunction(() => document.activeElement?.textContent === "バックアップから追加復元" && !document.activeElement.disabled && window.dataFixture.snapshot.phase === "idle");
  check((await calls("start_data_operation")).length === 0 && await page.getByRole("dialog").count() === 0, "native chooser cancellation exposed publication capability");
  results.push("native chooser cancellation: idle completion restores focus without publication");

  await boot(); await set({ previewMode: "delay" }); await choose("restore");
  await stop();
  check(await page.getByRole("region", { name: "登録した声の自動適用", exact: true }).getByRole("spinbutton", { name: "類似度の下限", exact: true }).isDisabled(), "preview cancellation unlocked before actual completion");
  await page.evaluate(() => window.dataFixture.complete()); await stopChoice().waitFor({ state: "hidden" });
  await region().getByRole("status").filter({ hasText: "取消済み" }).waitFor();
  check((await calls("start_data_operation")).length === 0 && await page.getByRole("dialog").count() === 0, "late preview resurrected confirmation");
  results.push("cancel preview: wait for completion, no late resurrection");

  for (const mode of ["delay", "cancelRace"]) {
    await boot(); await set({ mode }); await choose("restore"); await ready(); await confirm();
    await stop();
    check(await page.getByRole("navigation", { name: "メインナビゲーション", exact: true }).getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "cancel released write protection early");
    await page.evaluate(() => window.dataFixture.complete()); await stopChoice().waitFor({ state: "hidden" });
    if (mode === "cancelRace") { await done(); check(await page.evaluate(() => window.dataFixture.meetings.length) === 3, "winning commit suppressed"); }
    else { await region().getByRole("status").filter({ hasText: "取消済み" }).waitFor(); check(await page.evaluate(() => window.dataFixture.meetings.length) === 1, "cancel published meetings"); }
    results.push(mode === "cancelRace" ? "commit/cancel race: commit remains successful" : "cancel publication: wait for cleanup, preserve existing");
  }

  await boot(); await choose("restore"); await ready(); await set({ mode: "lost", suppressEvents: true }); await confirm(); await done();
  check((await calls("start_data_operation")).length === 1 && await page.evaluate(() => window.dataFixture.published) === 1, "lost reply caused another publication");
  results.push("lost reply + missing events: polling recovers without new admission");

  await boot(); await choose("restore"); await ready(); await set({ mode: "lostNoCommit" }); await confirm();
  await notifications().getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  check(await region().getByRole("button", { name: "バックアップから追加復元", exact: true }).isDisabled(), "unknown request allowed a new operation");
  await set({ mode: "success" });
  await page.getByRole("dialog").getByRole("button", { name: "同じ操作の結果を確認", exact: true }).click(); await done();
  const attempts = await calls("start_data_operation");
  check(attempts.length === 2 && JSON.stringify(attempts[0].args) === JSON.stringify(attempts[1].args), "unknown receipt replaced confirmation identity");
  results.push("unknown unaccepted request: explicit retry retains exact identity");

  await boot(); await choose("restore"); await ready(); await set({ mode: "lostNoCommit" }); await confirm();
  await page.getByRole("dialog").getByRole("button", { name: "同じ操作を取り消して確認", exact: true }).click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.getAttribute("aria-label") === "会議一覧" && !b.disabled));
  check((await calls("start_data_operation")).length === 1 && (await calls("cancel_data_operation")).length === 1 && await page.evaluate(() => window.dataFixture.meetings.length) === 1, "unknown cancellation published or retried work");
  results.push("unknown unaccepted request: explicit native cancellation releases review without publication");

  await boot(); await choose("restore"); await ready(); await set({ mode: "forged" }); await confirm();
  await notifications().getByRole("alert").filter({ hasText: "状態が不正" }).waitFor();
  check(await region().getByRole("button", { name: "復元した会議一覧を開く", exact: true }).count() === 0, "forged completion displayed success");
  results.push("forged completion: no receipt, no success");

  await boot(); await set({ mode: "attention" }); await choose("backup");
  await region().getByRole("alert").filter({ hasText: "同じ操作での再上書きは行いません" }).waitFor();
  await poll();
  check((await calls("start_data_operation")).length === 1 && !await region().getByText("バックアップを作成しました。", { exact: true }).count(), "attention result was re-published or labelled successful");
  results.push("uncertain backup publication: manual verification, no automatic overwrite or success");

  await boot(); await set({ mode: "unknownStats" }); await choose("restore"); await ready(); await confirm(); await done();
  check((await region().innerText()).includes("追加 記録不明 件、完全一致の重複 記録不明 件、既存を保持した競合 記録不明 件"), "unknown ledger statistics replaced with preview or zero");
  results.push("replayed receipt: unknown actual vocabulary counts remain unknown");

  await boot(); await set({ selectLost: true, suppressEvents: true }); await choose("restore"); await ready();
  check((await calls("select_data_operation")).length === 1 && (await calls("start_data_operation")).length === 0, "lost selection auto-selected or published");
  await confirm(); await done();
  check((await calls("start_data_operation")).length === 1, "recovered selection not exactly once");
  results.push("lost selection reply: read-only polling recovers review, explicit publication only");

  await boot(); await set({ blocker: "AI処理中" });
  await page.waitForFunction(() => [...document.querySelectorAll('.data-save button')].some(b => b.textContent === "バックアップを作成" && b.disabled));
  check(await region().getByRole("button", { name: "バックアップを作成", exact: true }).isDisabled(), "active AI allowed data selection");
  check((await region().innerText()).includes("停止・完了後"), "stop/wait guidance missing");
  await set({ blocker: null, failRead: true });
  await page.waitForFunction(() => document.querySelector('section[aria-label="登録した声の自動適用"] input')?.matches(":disabled"));
  check(await page.getByRole("region", { name: "登録した声の自動適用", exact: true }).getByRole("spinbutton", { name: "類似度の下限", exact: true }).isDisabled(), "failed state read did not lock writes");
  await set({ failRead: false });
  await page.waitForFunction(() => document.querySelector('section[aria-label="登録した声の自動適用"] input')?.matches(":disabled") === false);
  check(await page.getByRole("region", { name: "登録した声の自動適用", exact: true }).getByRole("spinbutton", { name: "類似度の下限", exact: true }).isEnabled(), "successful reread did not recover writes");
  results.push("active processing guidance + fail-closed state read recovery");

  await boot(); await page.locator(".sidebar").getByRole("button", { name: "プロンプト", exact: true }).click(); await set({ failSave: true }); await page.getByRole("button", { name: "用語を追加", exact: true }).click();
  await page.getByRole("textbox", { name: "用語（必須）", exact: true }).fill("点検日"); await page.locator(".sidebar").getByRole("button", { name: "設定", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "編集内容はこの画面に保持" }).waitFor();
  check((await calls("select_data_operation")).length === 0 && await page.getByRole("textbox", { name: "用語（必須）", exact: true }).inputValue() === "点検日", "save failure lost edits or selected backup");
  await set({ failSave: false }); await page.getByRole("region", { name: "共通用語", exact: true }).getByRole("button", { name: "用語保存を再試行", exact: true }).click();
  await page.locator(".sidebar").getByRole("button", { name: "設定", exact: true }).click(); await choose("backup"); await done();
  results.push("draft save failure blocks data operation and preserves input");

  await boot(); await page.locator(".sidebar").getByRole("button", { name: "プロンプト", exact: true }).click(); await set({ saveDelay: 500 }); await page.getByRole("button", { name: "用語を追加", exact: true }).click();
  await page.getByRole("textbox", { name: "用語（必須）", exact: true }).fill("点検日"); await page.locator(".sidebar").getByRole("button", { name: "設定", exact: true }).click(); await choose("backup"); await done();
  const order = await page.evaluate(() => window.dataFixture.calls.map(c => c.command));
  check(order.indexOf("save_vocabulary_book") < order.indexOf("select_data_operation") && await page.evaluate(() => window.dataFixture.book.payload[0].term) === "点検日", "data snapshot did not wait for draft save");
  results.push("draft flush before file selection and publication");
  console.log(JSON.stringify({ passed: results.length, results }));
  return { passed: results.length, results };
}
