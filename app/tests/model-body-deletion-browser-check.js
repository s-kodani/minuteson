// CLI-first browser verification against synthetic IPC, not native WebView.
async (page) => {
  page.setDefaultTimeout(10000);
  const results = [], check = (v, msg) => { if (!v) throw Error(msg); };
  const set = patch => page.evaluate(p => Object.assign(window.modelBodyFixture, p), patch);
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(v => v.command === c), command);
  const region = () => page.getByRole("region", { name: "モデル本体削除と中断対応", exact: true });
  const dialog = () => page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: /モデル本体の削除を確認|モデル本体の残り範囲への再同意/u }) });
  const trigger = () => page.getByRole("button", { name: "本体削除の実行確認へ", exact: true });
  const understand = () => dialog().getByRole("checkbox", { name: /対象を確認し、モデル本体は/u }).check();
  const unknown = () => dialog().getByRole("checkbox", { name: /確認時の一覧にある未登録パスも/u });
  const publish = () => dialog().getByRole("button", { name: /モデル本体を削除する|照合してモデル本体削除を続行|新しいモデル範囲へ再同意して削除を続行/u });
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click(); await trigger().waitFor(); await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "本体削除の実行確認へ" && !b.disabled)); };
  const review = async () => { await trigger().click(); await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); };
  const done = () => region().getByRole("status").filter({ hasText: "モデル本体を削除しました" }).waitFor();
  const pending = async (phase, altered = false) => {
    await page.evaluate(([p, a]) => window.modelBodyFixture.pending(p, a), [phase, altered]);
    await region().getByRole("button", { name: "未完了モデル削除の対象を確認", exact: true }).click();
    await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  };
  await boot(); await review();
  check((await dialog().innerText()).includes("会議音声・本文・議事録") && (await dialog().innerText()).includes("構成履歴・ジョブ履歴・現在の設定は保持"), "history/scope missing");
  check(await publish().isDisabled() && !(await calls("start_data_operation")).length, "review executed");
  check(await dialog().getByRole("heading", { name: "モデル本体の削除を確認" }).evaluate(e => e === document.activeElement), "heading focus missing");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "本体削除の実行確認へ");
  check(!(await calls("start_data_operation")).length, "Escape executed"); results.push("scope, history retained, consent, Escape and opener focus");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "モデル削除の安全な停止を要求", exact: true }).waitFor();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "navigation unlocked while busy");
  check(await page.getByRole("button", { name: "バックアップを作成", exact: true }).isDisabled(), "backup unlocked");
  check(!(await region().getByText(/モデル本体を削除しました/u).count()), "premature success");
  const before = await page.evaluate(() => window.modelBodyFixture.catalogReads);
  await page.evaluate(() => window.modelBodyFixture.complete()); await done();
  await page.waitForFunction(n => window.modelBodyFixture.catalogReads > n, before);
  check(await page.evaluate(() => window.dataFixture.meetings.length) === 1 && await trigger().isDisabled(), "meeting deleted or removed body selectable");
  check(!(await page.getByText("バックアップを作成しました。", { exact: false }).count()), "false backup success"); results.push("busy shared protection, receipt-only success, catalogue refresh, meeting retained");

  await boot(); await set({ unknownCount: 1 }); await review(); await understand(); check(await publish().isDisabled(), "unknown adopted implicitly");
  await unknown().check(); await publish().click(); await done();
  check((await calls("start_data_operation"))[0].args.request.action === "delete_including_unregistered", "unknown action missing"); results.push("separate fresh unregistered consent");

  await boot(); await set({ unknownCount: 1 }); await review(); await understand(); await unknown().check();
  const old = await page.evaluate(() => window.dataFixture.snapshot.modelDeletionPreview.confirmationId);
  await dialog().getByRole("button", { name: "モデル本体の残りを再検査", exact: true }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(await page.evaluate(() => window.dataFixture.snapshot.modelDeletionPreview.confirmationId) !== old && !await unknown().isChecked() && await publish().isDisabled(), "refresh inherited consent"); results.push("refresh resets both confirmation-bound consents");

  await boot(); await set({ fileCount: 105 }); await trigger().click(); await dialog().waitFor();
  await dialog().getByText("1〜50 / 106 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "モデル対象の次の50件" }).click(); await dialog().getByText("51〜100 / 106 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "モデル対象の次の50件" }).click(); await dialog().getByText("101〜106 / 106 件", { exact: true }).waitFor();
  check(await dialog().getByRole("button", { name: "モデル対象の次の50件" }).isDisabled() && !(await calls("start_data_operation")).length, "pagination boundary executed"); results.push("106 frozen paths, bounded 50-entry pagination");

  await boot(); await set({ unknownCount: 1 }); await pending("not_started");
  check(await unknown().count() === 0 && (await dialog().innerText()).includes("保存済みの方針: 未登録パスを含める"), "saved policy mutable");
  await understand(); await publish().click(); await done();
  check((await calls("start_data_operation"))[0].args.request.action === "publish", "pending policy replaced by new consent");
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "cold completion released write fence"); results.push("saved policy read-only, fixed publish resume and cold restart fence");

  await boot(); await pending("not_started", true); check(await publish().isDisabled(), "changed unstarted executed");
  await dialog().getByRole("button", { name: "モデル本体を未開始のまま保持して中止" }).click();
  await region().getByRole("status").filter({ hasText: "削除を開始せず保持して中止" }).waitFor();
  check(await page.evaluate(() => window.modelBodyFixture.kept === 1 && window.modelBodyFixture.deleted === 0 && window.dataFixture.meetings.length === 1), "keep deleted data"); results.push("unstarted keep preserves body and history, restart required");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "モデル削除の安全な停止を要求", exact: true }).click();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "stop unlocked prematurely");
  await page.evaluate(() => window.modelBodyFixture.complete());
  await region().getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
  await set({ mode: "success" }); await region().getByRole("button", { name: "モデル本体の残りを再検査", exact: true }).click(); await dialog().waitFor();
  check(await dialog().getByRole("button", { name: "モデル本体を未開始のまま保持して中止" }).count() === 0, "partial fake restore exposed");
  check((await dialog().innerText()).includes("処理済み") && (await dialog().innerText()).includes("現在の実在一覧ではありません"), "frozen processed paths unexplained");
  await understand(); await publish().click(); await done(); results.push("partial cancellation, continued fence, frozen progress, no fake keep, explicit resume");

  await boot(); await pending("files", true); await set({ unknownCount: 1 });
  await dialog().getByRole("button", { name: "モデル本体の変更後の残りを確認（まだ削除しない）" }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check((await dialog().innerText()).includes("以前の計画は 1 / 3") && await publish().isDisabled(), "old progress lost or renewed autoexec");
  await understand(); check(await publish().isDisabled(), "renewed unknown implicitly adopted"); await unknown().check(); await publish().click(); await done(); results.push("renewed scope retains former progress and requires new distinct consent");

  await boot(); await set({ mode: "lostNoCommit" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "同じモデル削除操作の結果を確認", exact: true }).waitFor();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled() && await page.getByRole("button", { name: "バックアップを作成", exact: true }).isDisabled(), "unknown response unlocked other writes");
  const original = (await calls("start_data_operation"))[0].args.request;
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "同じモデル削除操作の結果を確認", exact: true }).click(); await done();
  check(JSON.stringify((await calls("start_data_operation"))[1].args.request) === JSON.stringify(original), "lost request substituted"); results.push("unknown response protects shared actions and explicitly replays exact request");

  await boot(); await set({ mode: "forged" }); await review(); await understand(); await publish().click();
  await region().getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  check(!(await region().getByText(/モデル本体を削除しました/u).count()), "forged receipt accepted"); results.push("forged model receipt rejected without success");

  await boot(); await set({ pageFail: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByRole("alert").filter({ hasText: "模擬モデル対象一覧失敗" }).waitFor(); await understand(); check(await publish().isDisabled(), "missing page permitted delete");
  await set({ pageFail: false }); await dialog().getByRole("button", { name: "モデル対象一覧を再読込" }).click(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); results.push("page read failure fails closed and explicit retry recovers");

  await boot(); await set({ foreignPage: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByRole("alert").filter({ hasText: "モデル削除の状態が不正" }).waitFor(); await understand(); check(await publish().isDisabled(), "foreign model paths adopted"); results.push("foreign model page fails closed");

  await boot(); await set({ listFail: true }); await region().getByRole("button", { name: "モデル削除と書込停止の状態を再読込" }).click();
  await region().getByRole("alert").filter({ hasText: "模擬モデル削除一覧失敗" }).waitFor(); check(await trigger().isDisabled(), "invalid listing did not protect writes"); results.push("listing read failure locks writes");

  await boot(); await set({ reviewLost: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(!(await calls("start_data_operation")).length, "lost review auto published"); results.push("lost review recovered by shared state without deletion");

  await boot(); await set({ pathsDelay: true }); await trigger().click(); await dialog().waitFor();
  await page.waitForFunction(() => !!window.modelBodyFixture.releasePage);
  await dialog().getByRole("button", { name: "モデル本体の残りを再検査", exact: true }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  const newId = await page.evaluate(() => window.dataFixture.snapshot.modelDeletionPreview.confirmationId);
  await page.evaluate(() => window.modelBodyFixture.releasePage()); await understand();
  check(await publish().isEnabled() && await page.evaluate(() => window.dataFixture.snapshot.modelDeletionPreview.confirmationId) === newId, "late page replaced newer confirmation"); results.push("late old page ignored after refresh");

  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPendingEdits } = await import("/src/draft-save.ts");
      registerPendingEdits(() => fail ? Promise.reject(Error("模擬モデル前の編集保存失敗")) : new Promise(resolve => { window.modelBodyFixture.releaseEdit = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await page.getByRole("alert").filter({ hasText: "模擬モデル前の編集保存失敗" }).waitFor(); check(!(await calls("review_model_body_deletion")).length, "failed edit save admitted review"); }
    else { await page.waitForFunction(() => !!window.modelBodyFixture.releaseEdit); check(!(await calls("review_model_body_deletion")).length, "review skipped save wait"); await page.evaluate(() => window.modelBodyFixture.releaseEdit()); await dialog().waitFor(); }
    results.push(fail ? "save failure prevents model review" : "model review waits for edits");
  }
  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
      registerPlaybackStop(() => fail ? Promise.reject(Error("模擬モデル前の再生解放失敗")) : new Promise(resolve => { window.modelBodyFixture.releasePlayback = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await region().getByRole("alert").filter({ hasText: "模擬モデル前の再生解放失敗" }).waitFor(); check(!(await calls("review_model_body_deletion")).length, "playback failure admitted review"); }
    else { await page.waitForFunction(() => !!window.modelBodyFixture.releasePlayback); check(!(await calls("review_model_body_deletion")).length, "review skipped playback wait"); await page.evaluate(() => window.modelBodyFixture.releasePlayback()); await dialog().waitFor(); }
    results.push(fail ? "playback release failure prevents model review" : "model review waits for playback release");
  }
  await boot(); await review(); await understand();
  await page.evaluate(async () => {
    const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
    registerPlaybackStop(() => new Promise(resolve => { window.modelBodyFixture.releasePlayback = resolve; }));
  });
  await publish().click(); await page.waitForFunction(() => !!window.modelBodyFixture.releasePlayback);
  await page.evaluate(async () => { await window.dataFixture.changed({ modelDeletionPreview: { ...window.dataFixture.snapshot.modelDeletionPreview, confirmationId: crypto.randomUUID() } }); window.modelBodyFixture.releasePlayback(); });
  await dialog().getByRole("alert").filter({ hasText: "待機中にモデル削除の確認が変更" }).waitFor();
  check(!(await calls("start_data_operation")).length, "playback wait rebound old consent"); results.push("confirmation rechecked after playback wait, no consent rebinding");

  await boot(); await set({ mode: "lost" }); await page.evaluate(() => { window.dataFixture.suppressEvents = true; });
  await review(); await understand(); await publish().click(); await done();
  check(await page.evaluate(() => window.modelBodyFixture.deleted === 1) && (await calls("start_data_operation")).length === 1, "poll completion duplicated deletion"); results.push("lost response and suppressed events recovered by poll, single execution");

  await boot(); await pending("awaiting_ack"); await understand(); await publish().click(); await done();
  check(await page.evaluate(() => window.dataFixture.meetings.length === 1), "completion acknowledgement deleted meeting"); results.push("completed file stage acknowledged without meeting deletion");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await page.evaluate(async () => {
    window.confirm = message => { window.modelBodyFixture.closeMessage = message; return true; };
    const { emit } = await import("/@fs/Users/skodani/Codebase/mon/app/node_modules/@tauri-apps/api/event.js"); await emit("main-close-requested", { quit: true });
  });
  await page.waitForFunction(() => window.dataFixture.calls.some(c => c.command === "finish_main_close"));
  const closeMessage = await page.evaluate(() => window.modelBodyFixture.closeMessage);
  check(closeMessage.includes("モデル本体削除") && closeMessage.includes("復元しません"), "close used misleading backup wording"); results.push("main close uses model-specific irreversible stop confirmation");

  await boot(); await review();
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 800 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "horizontal dialog overflow");
    await page.screenshot({ path: `output/playwright/model-body-deletion-${width}.png` });
  }
  results.push("1200px and 900px modal layout screenshots");
  return { passed: results.length, results };
}
