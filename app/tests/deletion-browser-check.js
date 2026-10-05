// CLI-first browser verification, simulated IPC only; not a test spec.
async (page) => {
  const results = [], check = (v, message) => { if (!v) throw Error(message); };
  const set = values => page.evaluate(v => Object.assign(window.deletionFixture, v), values);
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(x => x.command === c), command);
  const region = () => page.getByRole("region", { name: "会議削除の確認と中断対応" });
  const dialog = () => page.getByRole("dialog", { name: "会議の完全削除を確認" });
  const understand = () => dialog().getByRole("checkbox", { name: "対象と削除範囲を確認し、アプリ内では復元できないことを理解しました。", exact: true }).check();
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "既存会議を削除", exact: true }).waitFor(); await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.getAttribute("aria-label") === "既存会議を削除" && !b.disabled)); };
  const review = async () => { await page.getByRole("button", { name: "既存会議を削除", exact: true }).click(); await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); };
  const deleted = () => region().getByRole("status").filter({ hasText: "会議の管理データを削除しました" }).waitFor();
  const publish = () => dialog().getByRole("button", { name: /会議を削除する|照合して残りの削除を続行/u }).click();
  const pending = async (phase, changed = false) => { await page.evaluate(([p, c]) => window.deletionFixture.pending(p, c), [phase, changed]); await region().getByRole("button", { name: "未完了削除の対象を確認", exact: true }).waitFor(); await region().getByRole("button", { name: "未完了削除の対象を確認", exact: true }).click(); await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); };

  await boot(); await review();
  const body = await dialog().innerText();
  check(body.includes("既存会議") && body.includes("下書き・手動編集") && body.includes("アプリ内では復元できません") && body.includes("旧保存先・保持コピー"), "delete scope/irreversibility missing");
  check((await calls("start_data_operation")).length === 0 && await dialog().getByRole("button", { name: "会議を削除する" }).isDisabled(), "review automatically deleted");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "既存会議を削除");
  check((await calls("start_data_operation")).length === 0, "Escape deleted"); results.push("scope, irreversible consent, Escape and opener focus");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish();
  await region().getByRole("button", { name: "安全な停止を要求", exact: true }).waitFor();
  check(await page.getByRole("button", { name: "残す会議を削除" }).isDisabled(), "other deletion enabled while native work runs");
  check(!await region().getByText(/会議の管理データを削除しました/u).count(), "partial work reported success");
  await page.evaluate(() => window.deletionFixture.complete()); await deleted();
  check(await page.getByRole("button", { name: "既存会議を削除" }).count() === 0 && await page.getByRole("button", { name: "残す会議を削除" }).count() === 1, "list did not remove only target");
  check((await calls("start_data_operation")).length === 1, "deletion ran twice"); results.push("one publication, busy protection, receipt-only success and target-only list refresh");

  await boot(); await set({ unknownCount: 1 }); await review(); await understand();
  check(await dialog().getByRole("button", { name: "会議を削除する" }).isDisabled(), "unknown files included by default");
  await dialog().getByRole("checkbox", { name: /確認時の一覧にある未登録ファイルも削除対象に含める/u }).check(); await publish(); await deleted();
  check((await calls("start_data_operation"))[0].args.request.action === "delete_including_unregistered", "unknown decision did not use separate action"); results.push("separate unregistered decision");

  await boot(); await set({ fileCount: 105 }); await page.getByRole("button", { name: "既存会議を削除" }).click(); await dialog().waitFor();
  await dialog().getByText("1〜50 / 107 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "次の50件" }).click(); await dialog().getByText("51〜100 / 107 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "次の50件" }).click(); await dialog().getByText("101〜107 / 107 件", { exact: true }).waitFor();
  check(await dialog().getByRole("button", { name: "次の50件" }).isDisabled(), "last page can advance");
  check((await calls("start_data_operation")).length === 0, "pagination caused deletion"); results.push("bounded metadata pagination without execution");

  await boot(); await set({ mode: "stale" }); await review(); await understand(); await publish();
  await region().getByRole("alert").filter({ hasText: "確認後に会議が変更" }).waitFor();
  const old = await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId);
  await region().getByRole("button", { name: "残りの状態を再検査", exact: true }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId) !== old, "refresh kept stale ID");
  check(await dialog().getByRole("button", { name: "会議を削除する" }).isDisabled(), "refresh inherited old consent");
  await understand(); await publish(); await deleted(); results.push("changed confirmation refresh requires new consent");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish();
  await region().getByRole("button", { name: "安全な停止を要求", exact: true }).click();
  check(await page.getByRole("button", { name: "残す会議を削除" }).isDisabled(), "stop request unlocked work before terminal");
  await page.evaluate(() => window.deletionFixture.complete());
  await region().getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
  check(await page.getByRole("textbox", { name: "会議名", exact: true }).isDisabled(), "partial deletion unlocked editing");
  await set({ mode: "success" }); await region().getByRole("button", { name: "残りの状態を再検査", exact: true }).click(); await dialog().waitFor();
  check(await dialog().getByRole("button", { name: "未開始のまま保持して中止" }).count() === 0, "partial deletion can be kept as if restored");
  await understand(); await publish(); await deleted(); results.push("partial cancellation retains fence, no fake restore, explicit resume");

  await boot(); await pending("not_started", true);
  check(await dialog().getByRole("button", { name: /照合して残りの削除/u }).isDisabled(), "changed unstarted contents can execute");
  await dialog().getByRole("button", { name: "未開始のまま保持して中止" }).click();
  await region().getByRole("status").filter({ hasText: "ファイル削除を開始せず保持して中止" }).waitFor();
  check(await page.getByRole("button", { name: "既存会議を削除" }).count() === 1 && await page.getByRole("button", { name: "既存会議を削除" }).isDisabled(), "keep deleted meeting or released cold startup fence");
  check(await region().getByRole("button", { name: "終了する（再起動は手動）" }).count() === 1, "restart guidance missing"); results.push("changed unstarted keep preserves meeting and cold restart fence");

  await boot(); await pending("database"); await understand(); await publish(); await deleted();
  check(await page.getByRole("textbox", { name: "会議名", exact: true }).isDisabled(), "cold completion resumed ordinary writes without restart"); results.push("cold database resume succeeds but requires restart");

  await boot(); await set({ mode: "unknown" }); await pending("files", true);
  check(await dialog().getByRole("button", { name: /照合して残りの削除/u }).isDisabled() && await dialog().getByRole("button", { name: "未開始のまま保持して中止" }).count() === 0, "unknown partial scope adopted or kept");
  check((await calls("start_data_operation")).length === 0, "unknown partial files executed"); results.push("partial unknown contents retained, no scope adoption");

  await boot(); await set({ mode: "unknown" }); await pending("files", true);
  const previousId = await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId);
  await set({ unknownCount: 1, fileCount: 2 });
  await dialog().getByRole("button", { name: "変更後の残り範囲を確認する（まだ削除しない）", exact: true }).click();
  const renewedDialog = () => page.getByRole("dialog", { name: "変更後の残り範囲への再同意", exact: true });
  await renewedDialog().waitFor(); await renewedDialog().getByText("1〜4 / 4 件", { exact: true }).waitFor();
  check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId) !== previousId, "remaining review reused old ID");
  check((await calls("start_data_operation")).length === 1 && (await calls("start_data_operation"))[0].args.request.action === "review_remaining", "remaining inspection performed deletion");
  check((await renewedDialog().innerText()).includes("元の同意と進捗を履歴として保持") && await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).isDisabled(), "old consent inherited or original progress missing");
  check((await renewedDialog().innerText()).includes("現在の確認段階") && (await renewedDialog().innerText()).includes("この一覧は再同意前の確認"), "temporary review labelled as durable consent");
  await page.setViewportSize({ width: 1200, height: 850 });
  await page.screenshot({ path: "output/playwright/deletion-reconfirmation-1200.png" });
  await page.setViewportSize({ width: 900, height: 700 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "remaining scope dialog causes horizontal overflow");
  await page.screenshot({ path: "output/playwright/deletion-reconfirmation-900.png" });
  await renewedDialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).check();
  check(await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).isDisabled(), "new unknown decision inherited");
  await renewedDialog().getByRole("checkbox", { name: /確認時の一覧にある未登録ファイルも削除対象に含める/u }).check();
  await renewedDialog().getByRole("button", { name: "残りの状態を再検査", exact: true }).click();
  await renewedDialog().getByText("1〜4 / 4 件", { exact: true }).waitFor();
  check(!await renewedDialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).isChecked() && !await renewedDialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).isChecked(), "reinspection retained renewed consent");
  await renewedDialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).check(); await renewedDialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).check();
  await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: "output/playwright/deletion-reconfirmation-consent-900.png" });
  await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).click(); await deleted();
  check((await calls("start_data_operation")).at(-1).args.request.action === "delete_including_unregistered", "new unknown decision not explicit");
  results.push("remaining scope inspection preserves original progress and requires fresh two-part consent after each ID change");

  await boot(); await set({ mode: "unknown" }); await pending("files", true); await set({ mode: "lostNoCommit" });
  await dialog().getByRole("button", { name: "変更後の残り範囲を確認する（まだ削除しない）", exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  await set({ mode: "unknown" }); await dialog().getByRole("button", { name: "同じ削除操作の結果を確認", exact: true }).click();
  await renewedDialog().waitFor(); await renewedDialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  const remainingAttempts = await calls("start_data_operation");
  check(remainingAttempts.length === 2 && JSON.stringify(remainingAttempts[0].args) === JSON.stringify(remainingAttempts[1].args) && await page.evaluate(() => window.deletionFixture.deleted) === 0, "lost remaining review changed request or deleted");
  results.push("lost remaining inspection reuses exact request without implicit deletion");

  await boot(); await review(); await understand();
  await page.evaluate(async () => { const { registerPlaybackStop } = await import("/src/playback-barrier.ts"); registerPlaybackStop(() => new Promise(resolve => { window.deletionFixture.releaseConsentWait = resolve; })); });
  await publish(); await page.waitForFunction(() => !!window.deletionFixture.releaseConsentWait);
  await page.evaluate(async () => { await window.dataFixture.changed({ deletionPreview: { ...window.dataFixture.snapshot.deletionPreview, confirmationId: crypto.randomUUID() } }); window.deletionFixture.releaseConsentWait(); });
  await dialog().getByRole("alert").filter({ hasText: "待機中に削除の確認が変更" }).waitFor();
  check((await calls("start_data_operation")).length === 0, "awaited playback rebound old consent to a new ID");
  results.push("playback wait cannot rebind consent to a changed confirmation");

  await boot(); await review(); await set({ mode: "lostNoCommit" }); await understand(); await publish();
  await dialog().getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  check(await page.getByRole("button", { name: "残す会議を削除" }).isDisabled(), "uncertain receipt allowed another deletion");
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "同じ削除操作の結果を確認" }).click(); await deleted();
  const attempts = await calls("start_data_operation");
  check(attempts.length === 2 && JSON.stringify(attempts[0].args) === JSON.stringify(attempts[1].args), "uncertain request replaced confirmation"); results.push("unknown unaccepted request retains exact resend identity");

  await boot(); await review(); await set({ mode: "lost" }); await page.evaluate(() => { window.dataFixture.suppressEvents = true; }); await understand(); await publish(); await deleted();
  check((await calls("start_data_operation")).length === 1, "lost reply caused duplicate execution"); results.push("lost response and missing events recovered by polling");

  await boot(); await review(); await set({ mode: "forged" }); await understand(); await publish();
  await dialog().getByRole("alert").filter({ hasText: "状態が不正" }).waitFor();
  check(!await region().getByText(/会議の管理データを削除しました/u).count(), "forged receipt reported success"); results.push("forged receipt refused without success");

  await boot(); await set({ listFail: true });
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.getAttribute("aria-label") === "既存会議を削除" && b.disabled));
  await region().getByRole("alert").filter({ hasText: "模擬削除一覧失敗" }).waitFor();
  await set({ listFail: false }); await region().getByRole("button", { name: "削除と書込停止の状態を再読込" }).click();
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.getAttribute("aria-label") === "既存会議を削除" && !b.disabled)); results.push("listing read failure locks ordinary writes until verified reread");

  await boot(); await set({ reviewLost: true }); await review();
  check((await calls("start_data_operation")).length === 0, "lost review automatically executed"); await understand(); await publish(); await deleted(); results.push("lost review recovered with explicit publication only");

  await boot(); await set({ pathsDelay: true }); await page.getByRole("button", { name: "既存会議を削除" }).click(); await dialog().waitFor();
  await dialog().getByRole("button", { name: "残りの状態を再検査" }).click(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  await page.evaluate(() => window.deletionFixture.releasePage());
  check(await dialog().getByRole("button", { name: "会議を削除する" }).isDisabled(), "stale page restored consent");
  check((await calls("start_data_operation")).every(c => c.args.request.action === "refresh"), "stale page executed"); results.push("late old path page ignored after refreshed confirmation");

  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => { const { registerPendingEdits } = await import("/src/draft-save.ts");
      window.deletionFixture.unregisterEdit = registerPendingEdits(() => fail ? Promise.reject(Error("模擬編集保存失敗")) : new Promise(resolve => { window.deletionFixture.releaseEdit = resolve; }));
    }, fail);
    await page.getByRole("button", { name: "既存会議を削除" }).click();
    if (fail) { await page.getByRole("alert").filter({ hasText: "模擬編集保存失敗" }).waitFor(); check((await calls("review_meeting_deletion")).length === 0, "save failure admitted review"); }
    else { check((await calls("review_meeting_deletion")).length === 0, "review did not wait for save"); await page.evaluate(() => window.deletionFixture.releaseEdit()); await dialog().waitFor(); }
    results.push(fail ? "save failure prevents review" : "review waits for pending edits");
  }
  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => { const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
      registerPlaybackStop(() => fail ? Promise.reject(Error("模擬音声停止失敗")) : new Promise(resolve => { window.deletionFixture.releasePlayback = resolve; }));
    }, fail);
    await page.getByRole("button", { name: "既存会議を削除" }).click();
    if (fail) { await region().getByRole("alert").filter({ hasText: "模擬音声停止失敗" }).waitFor(); check((await calls("review_meeting_deletion")).length === 0, "playback failure admitted review"); }
    else { check((await calls("review_meeting_deletion")).length === 0, "review did not wait for playback release"); await page.evaluate(() => window.deletionFixture.releasePlayback()); await dialog().waitFor(); }
    results.push(fail ? "playback release failure prevents review" : "review waits for playback token release");
  }
  await boot(); await set({ prepareHold: true, closeHold: true });
  await page.evaluate(() => window.deletionFixture.mountPlayback());
  await page.getByRole("button", { name: "模擬区間の再生準備" }).click();
  await page.waitForFunction(() => !!window.deletionFixture.releasePreparation);
  await page.getByRole("button", { name: "既存会議を削除" }).click();
  check((await calls("review_meeting_deletion")).length === 0, "real playback hook preparation did not fence review");
  await page.evaluate(() => window.deletionFixture.releasePreparation());
  await page.waitForFunction(() => !!window.deletionFixture.releaseClose);
  check((await calls("review_meeting_deletion")).length === 0, "review proceeded before close_playback acknowledgement");
  await page.evaluate(() => window.deletionFixture.releaseClose()); await dialog().waitFor();
  check(await page.evaluate(() => window.deletionFixture.liveTokens.size === 0), "preparation token was not released");
  results.push("production playback hook waits for late preparation and native close acknowledgement");

  await boot(); await set({ prepareHold: true, closeFail: true });
  await page.evaluate(() => window.deletionFixture.mountPlayback());
  await page.getByRole("button", { name: "模擬区間の再生準備" }).click();
  await page.waitForFunction(() => !!window.deletionFixture.releasePreparation);
  await page.evaluate(() => { window.deletionFixture.unmountPlayback(); window.deletionFixture.releasePreparation(); });
  await page.waitForFunction(() => window.deletionFixture.closes >= 2);
  await page.getByRole("button", { name: "既存会議を削除" }).click();
  await region().getByRole("alert").filter({ hasText: "模擬ネイティブ再生解放失敗" }).waitFor();
  check((await calls("review_meeting_deletion")).length === 0, "unmounted hook lost failed token release");
  await set({ closeFail: false }); await page.getByRole("button", { name: "既存会議を削除" }).click(); await dialog().waitFor();
  check(await page.evaluate(() => window.deletionFixture.liveTokens.size === 0), "detached token retry was not released");
  results.push("unmounted production playback hook retains failed release for explicit retry");

  await boot(); await pending("not_started"); await dialog().getByRole("button", { name: "未開始のまま保持して中止" }).click();
  await region().getByRole("button", { name: "終了する（再起動は手動）" }).waitFor(); await set({ quitFail: true });
  await region().getByRole("button", { name: "終了する（再起動は手動）" }).click();
  await region().getByRole("alert").filter({ hasText: "模擬終了失敗" }).waitFor();
  results.push("manual restart exit failure remains visible");
  for (const phase of ["database", "awaiting_ack", "database_files"]) {
    await boot(); await set({ mode: "unknown", unknownCount: 1 }); await pending(phase, true);
    check(await dialog().getByRole("button", { name: "未開始のまま保持して中止" }).count() === 0, "DB phase can falsely retain or restore deletion");
    const committed = phase !== "database";
    if (committed) check((await region().innerText()).includes("完了記録あり・残り処理の確認が必要"), "completed residual SQL marker shown as resolved");
    await dialog().getByRole("button", { name: "変更後の残り範囲を確認する（まだ削除しない）", exact: true }).click();
    await renewedDialog().waitFor(); await renewedDialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
    check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.phase) === "database_reconfirmation", "DB reconfirmation lost source stage");
    const text = await renewedDialog().innerText();
    check(text.includes("DB段階を保持した残り範囲への再同意待ち") && text.includes("元の確認時のDB行（削除済み）") === committed, "DB row commitment mislabelled");
    check(await page.evaluate(() => window.deletionFixture.deleted) === 0, "DB inspection deleted files");
    await renewedDialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).check();
    check(await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).isDisabled(), "DB unknown decision inherited");
    await renewedDialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).check();
    if (phase === "awaiting_ack") {
      await page.setViewportSize({ width: 1200, height: 850 });
      await renewedDialog().evaluate(el => { el.scrollTop = 0; });
      await page.screenshot({ path: "output/playwright/deletion-database-reconfirmation-1200.png" });
      await page.setViewportSize({ width: 900, height: 700 });
      await renewedDialog().evaluate(el => { el.scrollTop = 0; });
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "DB reconfirmation causes horizontal overflow");
      await page.screenshot({ path: "output/playwright/deletion-database-reconfirmation-900.png" });
      await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).scrollIntoViewIfNeeded();
      await page.screenshot({ path: "output/playwright/deletion-database-reconfirmation-consent-900.png" });
      await set({ mode: "partial" });
    }
    await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).click();
    if (phase === "awaiting_ack") {
      await region().getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
      check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.phase === "database_files" && window.dataFixture.snapshot.deletionPreview.rowPhaseCommitted), "partial DB files forgot committed rows");
      await set({ mode: "success" }); await region().getByRole("button", { name: "残りの状態を再検査", exact: true }).click();
      await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
      check(await dialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).count() === 0, "fixed DB files silently asks a new policy");
      await understand(); await publish();
    }
    await deleted();
    const started = (await calls("start_data_operation")).filter(c => ["publish", "delete_including_unregistered"].includes(c.args.request.action));
    check(started[0].args.request.action === "delete_including_unregistered", "DB current scope lacked explicit unknown decision");
    if (phase === "awaiting_ack") check(started[1].args.request.action === "publish", "DB fixed plan did not resume existing policy");
    results.push(`DB remaining scope ${phase} preserves row phase, explicit unknown consent and fixed-plan continuation`);
  }
  for (const foreignRootPage of [false, true]) {
    await boot(); await set({ reservedMode: true, foreignRootPage, mode: "unknown" }); await pending("files", true);
    const oldId = await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId);
    await dialog().getByRole("button", { name: "変更後の残り範囲を確認する（まだ削除しない）", exact: true }).click();
    await renewedDialog().waitFor();
    check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.confirmationId) !== oldId, "reserved review inherited old ID");
    check((await calls("start_data_operation")).every(c => c.args.request.action === "review_remaining"), "reserved inspection executed deletion");
    if (foreignRootPage) {
      await renewedDialog().getByRole("alert").filter({ hasText: "状態が不正" }).waitFor();
      check(await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).isDisabled(), "foreign sibling page enabled deletion");
      results.push("foreign operation reservation path is refused before consent or execution");
      continue;
    }
    await renewedDialog().getByText("1〜5 / 5 件", { exact: true }).waitFor();
    const paths = await renewedDialog().innerText();
    check(paths.includes("会議ツリー外の予約ルート") && paths.includes("/unknown.txt") && paths.includes("/audio/input.flac"), "coexisting reserved and logical roots not explained or listed");
    await renewedDialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).check();
    check(await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).isDisabled(), "reserved unknown consent was inherited");
    await renewedDialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).check();
    await page.setViewportSize({ width: 1200, height: 850 }); await renewedDialog().evaluate(el => { el.scrollTop = 0; });
    await page.screenshot({ path: "output/playwright/deletion-root-reconfirmation-1200.png" });
    await page.setViewportSize({ width: 900, height: 700 }); await renewedDialog().evaluate(el => { el.scrollTop = 0; });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "reserved scope page overflows horizontally");
    await page.screenshot({ path: "output/playwright/deletion-root-reconfirmation-900.png" });
    await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: "output/playwright/deletion-root-reconfirmation-consent-900.png" });
    await set({ mode: "partial" }); await renewedDialog().getByRole("button", { name: "新しい範囲へ再同意して削除を続行", exact: true }).click();
    await region().getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
    check(await page.evaluate(() => window.dataFixture.snapshot.deletionPreview.reservationRoots.length) === 1, "partial plan forgot reserved root");
    await set({ mode: "success" }); await region().getByRole("button", { name: "残りの状態を再検査", exact: true }).click();
    await dialog().waitFor(); await dialog().getByText("1〜5 / 5 件", { exact: true }).waitFor();
    check(await dialog().getByRole("checkbox", { name: /確認時の一覧にある未登録/u }).count() === 0, "fixed reserved plan changed unknown policy");
    await understand(); await publish(); await deleted();
    const started = (await calls("start_data_operation")).filter(c => ["publish", "delete_including_unregistered"].includes(c.args.request.action));
    check(started.length === 2 && started[0].args.request.action === "delete_including_unregistered" && started[1].args.request.action === "publish", "reserved plan publication/retry policy not fixed");
    check(await page.getByRole("button", { name: "残す会議を削除" }).count() === 1, "reserved deletion removed neighbor meeting");
    results.push("coexisting reserved root requires fresh unknown consent and retains fixed plan through partial stop");
  }
  await boot(); await review(); await page.setViewportSize({ width: 1200, height: 900 });
  await page.screenshot({ path: "output/playwright/deletion-confirm-1200.png" });
  await page.setViewportSize({ width: 900, height: 650 });
  await page.screenshot({ path: "output/playwright/deletion-confirm-900.png" });
  check(await dialog().evaluate(el => el.scrollWidth <= el.clientWidth), "confirmation horizontally clipped");
  await dialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).scrollIntoViewIfNeeded();
  check(await dialog().getByRole("checkbox", { name: /対象と削除範囲を確認/u }).evaluate(el => {
    const r = el.getBoundingClientRect(); return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === el;
  }), "consent checkbox covered by another action row");
  await page.screenshot({ path: "output/playwright/deletion-consent-900.png" });
  return { cases: results.length, results };
}
