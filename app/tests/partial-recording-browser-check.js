// CLI-first verification against synthetic IPC, not device capture or native WebView.
async page => {
  page.setDefaultTimeout(10000);
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const region = () => page.getByRole("region", { name: "中断録音の部分救出", exact: true });
  const dialog = () => page.getByRole("dialog", { name: "確定済み音声だけを救出する確認", exact: true });
  const trigger = () => region().getByRole("button", { name: "確定範囲と救出方法を確認", exact: true }).first();
  const recover = () => dialog().getByRole("button", { name: "同意して確定済み部分を救出", exact: true });
  const consent = () => dialog().getByRole("checkbox", { name: /正常終了・取得終了・末尾損失/u });
  const refresh = () => dialog().getByRole("button", { name: "最新の範囲を再確認", exact: true });
  const retry = () => page.getByRole("button", { name: "同じ部分救出操作の結果を確認", exact: true }).filter({ visible: true }).last();
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(v => v.command === c), command);
  const set = patch => page.evaluate(p => Object.assign(window.partialRecordingFixture, p), patch);
  const boot = async patch => {
    await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click(); await trigger().waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "確定範囲と救出方法を確認" && !b.disabled));
    if (patch) await set(patch);
  };
  const review = async () => { await trigger().click(); await dialog().waitFor(); await consent().waitFor(); };
  const success = () => page.getByRole("complementary", { name: "通知", exact: true }).getByRole("status").filter({ hasText: "検証済みの確定範囲だけを救出しました" }).waitFor();
  const nav = () => page.getByRole("button", { name: "会議一覧", exact: true });
  const terminal = phase => page.waitForFunction(p => window.dataFixture.snapshot.phase === p && !window.dataFixture.snapshot.busy, phase);

  await boot(); await review();
  const text = await dialog().innerText();
  check(text.includes("取得終了・UTC原点・末尾の損失時間は不明") && text.includes("末尾損失5秒以内は保証しません") && text.includes("0.050000秒〜5.000000秒"), "unknown closure or independent ranges lost");
  check(await recover().isDisabled() && !(await calls("start_data_operation")).length, "inspection recovered automatically");
  check(await page.evaluate(() => document.activeElement?.textContent === "キャンセル"), "initial focus is not cancel");
  await page.keyboard.press("Tab"); check(await consent().evaluate(e => e === document.activeElement), "forward focus escaped dialog");
  await page.keyboard.press("Shift+Tab"); await page.keyboard.press("Shift+Tab");
  check(await page.evaluate(() => document.activeElement?.textContent === "救出せず閉じる"), "reverse focus escaped dialog");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" }); await terminal("cancelled");
  await page.waitForFunction(() => document.activeElement?.textContent === "確定範囲と救出方法を確認");
  check(!(await calls("start_data_operation")).length, "Escape adopted audio"); results.push("read-only inspection, unknown loss, lane ranges, cancel focus, Tab wrapping and Escape restoration");

  await boot(); await review(); await consent().check();
  const old = await page.evaluate(() => window.dataFixture.snapshot.partialRecordingPreview.confirmationId);
  await refresh().click(); await dialog().waitFor();
  await page.waitForFunction(previous => window.dataFixture.snapshot.partialRecordingPreview.confirmationId !== previous, old);
  check(!await consent().isChecked() && await recover().isDisabled(), "refresh retained old consent"); results.push("fresh confirmation resets explicit consent");
  for (const unreadable of [false, true]) {
    await boot({ oneLane: true, unreadable }); await review();
    check((await dialog().innerText()).includes(unreadable ? "確定範囲を検証できません" : "確定範囲の記録なし"), "missing or corrupt input not explained");
    await consent().check(); await recover().click(); await success();
    check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 1, "verified input not recovered exactly once"); results.push(unreadable ? "unreadable lane remains excluded" : "missing lane remains excluded");
    const history = page.getByRole("region", { name: "保存済み部分救出の結果", exact: true });
    await history.waitFor();
    const starts = (await calls("start_data_operation")).length, cancels = (await calls("cancel_data_operation")).length;
    await history.getByRole("button", { name: "保存時の確定範囲を確認", exact: true }).click();
    const saved = page.getByRole("dialog", { name: "保存済み部分救出の記録", exact: true }); await saved.waitFor();
    check((await saved.innerText()).includes(unreadable ? "救出時に検証できず対象外" : "救出時に確定範囲の記録なし"), "saved missing/unreadable lane became recoverable");
    await page.keyboard.press("Escape"); await saved.waitFor({ state: "hidden" });
    check((await calls("start_data_operation")).length === starts && (await calls("cancel_data_operation")).length === cancels, "reading committed history replayed or cancelled recovery");
    results.push(unreadable ? "runtime committed history retains excluded unreadable lane without replay" : "runtime committed history retains missing lane without replay");
  }
  await boot({ empty: true }); await review(); await consent().check(); check(await recover().isDisabled(), "empty prefixes recoverable");
  await dialog().getByRole("button", { name: "救出せず保持", exact: true }).click(); await terminal("completed");
  check(await page.evaluate(() => window.partialRecordingFixture.kept === 1 && window.partialRecordingFixture.recovered === 0 && window.partialRecordingFixture.count === 1), "keep changed audio or removed candidate"); results.push("empty prefixes allow only non-destructive keep");
  check(await page.getByRole("region", { name: "保存済み部分救出の結果", exact: true }).count() === 0, "keep fabricated a committed history");

  await boot({ mode: "delay" }); await review(); await consent().check(); await recover().click();
  await region().getByRole("button", { name: "部分救出の安全な停止を要求", exact: true }).waitFor();
  check(await nav().isDisabled(), "publication released shared protection");
  await region().getByRole("button", { name: "部分救出の安全な停止を要求", exact: true }).click();
  check(await nav().isDisabled(), "stop request released shared protection prematurely");
  await page.evaluate(() => window.partialRecordingFixture.complete()); await terminal("cancelled");
  check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 0, "cancelled conversion registered audio");
  await set({ mode: "success" }); await region().getByRole("button", { name: "確定範囲と保存結果を再確認", exact: true }).click(); await dialog().waitFor();
  check(!await consent().isChecked() && await recover().isDisabled(), "cancelled work inherited consent"); await consent().check(); await recover().click(); await success(); results.push("cancel waits for actual completion; resume requires fresh consent");

  await boot({ mode: "cancelRace" }); await review(); await consent().check(); await recover().click();
  await region().getByRole("button", { name: "部分救出の安全な停止を要求", exact: true }).click();
  await page.evaluate(() => window.partialRecordingFixture.complete()); await success();
  check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 1, "late cancellation discarded published receipt"); results.push("published receipt wins late cancellation");

  await boot({ mode: "lostNoCommit" }); await review(); await consent().check(); await recover().click(); await retry().waitFor();
  check(await nav().isDisabled(), "lost request released protection"); const original = (await calls("start_data_operation")).at(-1).args.request;
  await set({ mode: "success" }); await retry().click(); await success();
  check(JSON.stringify((await calls("start_data_operation")).at(-1).args.request) === JSON.stringify(original), "unknown retry replaced consent or request"); results.push("unaccepted lost reply protects actions and explicitly replays exact request");
  await boot({ mode: "lost" }); await review(); await consent().check(); await recover().click(); await success();
  check((await calls("start_data_operation")).length === 1 && await page.evaluate(() => window.partialRecordingFixture.recovered) === 1, "accepted lost reply duplicated publication"); results.push("event before lost reply resolves without duplicate publication");
  check(await page.getByRole("alert").filter({ hasText: "受付結果を確認できません" }).count() === 0, "verified accepted event falsely reported as unknown");
  await boot({ mode: "lostRefresh" }); await review(); await consent().check(); await refresh().click(); await dialog().waitFor();
  check(!await consent().isChecked() && await recover().isDisabled(), "lost refresh kept old confirmation consent"); results.push("lost refresh reply still invalidates former consent");
  await boot({ reviewLost: true }); await review(); check(!(await calls("start_data_operation")).length && await recover().isDisabled(), "lost inspection auto recovered"); results.push("lost inspection reconciles newer shared state without recovery");

  await boot({ mode: "stale" }); await review(); await consent().check(); await recover().click(); await terminal("failed");
  check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 0, "changed input was published");
  await set({ mode: "success" }); await region().getByRole("button", { name: "確定範囲と保存結果を再確認", exact: true }).click(); await dialog().waitFor();
  check(!await consent().isChecked() && await recover().isDisabled(), "changed input reused old consent");
  await consent().check(); await recover().click(); await success(); results.push("changed input failure requires a fresh review and new consent");
  await boot({ reviewFail: true }); await trigger().click(); await terminal("failed");
  await page.getByRole("alert").filter({ hasText: "模擬確定範囲の検査失敗" }).waitFor();
  check(await dialog().count() === 0 && !(await calls("start_data_operation")).length, "failed inspection retained usable preview");
  await set({ reviewFail: false }); await review(); check(await recover().isDisabled(), "retry inspection inherited consent"); results.push("failed inspection cannot authorize recovery; fresh inspection remains possible");
  await boot({ mode: "stale" }); await review(); await consent().check(); await recover().click(); await terminal("failed");
  await trigger().click();
  const replacement = () => page.getByRole("dialog", { name: "部分救出の確認対象を変更", exact: true });
  await replacement().waitFor(); check((await calls("review_partial_recording")).length === 1, "existing review silently replaced");
  await replacement().getByRole("button", { name: "現在の確認を破棄して検査", exact: true }).click(); await dialog().waitFor();
  check((await calls("review_partial_recording")).length === 2 && !await consent().isChecked() && await recover().isDisabled(), "replacement kept old consent or was not inspected"); results.push("replacing a failed retained review requires explicit confirmation, then fresh consent");

  await boot({ cancelFail: true }); await review(); await page.keyboard.press("Escape"); await retry().waitFor();
  check(await nav().isDisabled(), "unaccepted stop released protection"); const stopped = (await calls("cancel_data_operation")).at(-1).args.token;
  await set({ cancelFail: false }); await retry().click(); await dialog().waitFor({ state: "hidden" }); await terminal("cancelled");
  check((await calls("cancel_data_operation")).at(-1).args.token === stopped && !(await calls("start_data_operation")).length, "stop retry adopted audio or changed token"); results.push("lost unaccepted stop replays exact cancellation, not recovery");
  await boot({ cancelLost: true }); await review(); await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" }); await terminal("cancelled");
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent.includes("会議一覧") && !b.disabled));
  check((await calls("cancel_data_operation")).length === 1, "accepted lost stop duplicated cancellation or remained locked"); results.push("accepted stop event before lost reply releases uncertainty after terminal state");
  check(await page.getByRole("alert").filter({ hasText: "停止受付を確認できません" }).count() === 0, "verified stopped event falsely reported as unknown");
  await boot({ mode: "delay", cancelLost: true }); await review(); await consent().check(); await recover().click();
  await region().getByRole("button", { name: "部分救出の安全な停止を要求", exact: true }).click(); check(await nav().isDisabled(), "lost busy stop released protection");
  await page.evaluate(() => window.partialRecordingFixture.complete()); await terminal("cancelled");
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent.includes("会議一覧") && !b.disabled));
  check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 0, "lost busy stop recovered audio"); results.push("lost busy stop remains protected until actual cancellation completes");

  await boot({ mode: "delay" }); await review(); await consent().check(); await recover().click();
  await page.evaluate(async () => { const { emit } = await import("/node_modules/@tauri-apps/api/event.js"); await emit("main-close-requested", { quit: true }); });
  const closing = page.getByRole("dialog", { name: "進行中の処理を停止して終了しますか？", exact: true });
  await closing.waitFor(); const closingText = await closing.innerText();
  check(closingText.includes("部分救出") && closingText.includes("原音声と途中の所有付きFLAC") && !closingText.includes("バックアップ／追加復元"), "close described the wrong operation");
  await closing.getByRole("button", { name: "安全な停止を待って終了する", exact: true }).dblclick();
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelling");
  check(!(await calls("finish_main_close")).length, "main closed before actual partial worker completion");
  await page.evaluate(() => window.partialRecordingFixture.complete());
  await page.waitForFunction(() => window.dataFixture.calls.some(c => c.command === "finish_main_close"));
  check(await page.evaluate(() => window.partialRecordingFixture.recovered) === 0 && (await calls("cancel_data_operation")).length === 1, "close duplicated cancel or adopted audio"); results.push("main close names partial recovery and waits for actual cancellation without duplicate admission");

  await boot({ mode: "forged" }); await review(); await consent().check(); await recover().click(); await retry().waitFor();
  check(await nav().isDisabled() && await page.evaluate(() => window.partialRecordingFixture.recovered) === 0, "foreign receipt accepted"); results.push("foreign receipt rejected without success");
  await boot({ listFail: true }); await region().getByRole("button", { name: "中断録音と救出結果を再読込", exact: true }).click();
  await region().getByRole("alert").filter({ hasText: "模擬部分救出一覧失敗" }).waitFor(); check(await trigger().isDisabled() && await nav().isDisabled(), "listing failure released writes"); results.push("listing failure fails closed");
  await boot({ count: 201 }); await region().getByRole("button", { name: "中断録音と救出結果を再読込", exact: true }).click();
  await region().getByRole("button", { name: "次の中断録音を表示", exact: true }).waitFor();
  check(await region().getByRole("listitem").count() === 200, "first listing is not bounded");
  await region().getByRole("button", { name: "次の中断録音を表示", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="中断録音の部分救出"] ul').children.length === 201);
  check(await region().getByRole("button", { name: "次の中断録音を表示", exact: true }).count() === 0, "pagination replayed final cursor");
  await set({ count: 1 }); await region().getByRole("button", { name: "中断録音と救出結果を再読込", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="中断録音の部分救出"] ul').children.length === 1); results.push("201 candidates paged 200 then 1, final cursor removed and old extra page reset");

  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPendingEdits } = await import("/src/draft-save.ts");
      registerPendingEdits(() => fail ? Promise.reject(Error("模擬部分救出前の編集保存失敗")) : new Promise(resolve => { window.partialRecordingFixture.releaseEdit = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await page.getByRole("alert").filter({ hasText: "模擬部分救出前の編集保存失敗" }).waitFor(); check(!(await calls("review_partial_recording")).length, "failed edits admitted inspection"); }
    else { await page.waitForFunction(() => !!window.partialRecordingFixture.releaseEdit); check(!(await calls("review_partial_recording")).length, "inspection skipped edit wait"); await page.evaluate(() => window.partialRecordingFixture.releaseEdit()); await dialog().waitFor(); }
    results.push(fail ? "edit save failure blocks inspection" : "inspection waits for pending edits");
  }
  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
      registerPlaybackStop(() => fail ? Promise.reject(Error("模擬部分救出前の再生停止失敗")) : new Promise(resolve => { window.partialRecordingFixture.releasePlayback = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await page.getByRole("alert").filter({ hasText: "模擬部分救出前の再生停止失敗" }).waitFor(); check(!(await calls("review_partial_recording")).length, "failed playback stop admitted inspection"); }
    else { await page.waitForFunction(() => !!window.partialRecordingFixture.releasePlayback); check(!(await calls("review_partial_recording")).length, "inspection skipped playback wait"); await page.evaluate(() => window.partialRecordingFixture.releasePlayback()); await dialog().waitFor(); }
    results.push(fail ? "playback stop failure blocks inspection" : "inspection waits for playback cleanup");
  }
  await boot();
  await page.evaluate(async () => {
    const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
    registerPlaybackStop(() => new Promise(resolve => { window.partialRecordingFixture.releasePlayback = resolve; }));
  });
  await trigger().click(); await page.waitForFunction(() => !!window.partialRecordingFixture.releasePlayback);
  await page.evaluate(async () => { await window.dataFixture.changed({ revision: window.dataFixture.snapshot.revision }); window.partialRecordingFixture.releasePlayback(); });
  await page.getByRole("alert").filter({ hasText: "待機中に操作が変更" }).waitFor(); check(!(await calls("review_partial_recording")).length, "stale operation admitted inspection"); results.push("shared state change during playback wait prevents stale inspection");

  await boot(); await review();
  for (const colorScheme of ["light", "dark"]) {
    await page.emulateMedia({ colorScheme });
    for (const width of [1200, 900]) {
      await page.setViewportSize({ width, height: 800 });
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "dialog horizontally overflows");
      await dialog().evaluate(e => { e.scrollTop = 0; });
      await page.screenshot({ path: `/Users/skodani/Codebase/mon/output/playwright/partial-controller/${colorScheme}-${width}.png` });
      await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
      await page.screenshot({ path: `/Users/skodani/Codebase/mon/output/playwright/partial-controller/${colorScheme}-${width}-actions.png` });
    }
  }
  results.push("light/dark 1200px/900px warnings and scrollable consent actions without horizontal overflow");
  return { passed: results.length, results };
}
