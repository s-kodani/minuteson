// CLI-first browser verification against synthetic IPC, not native WebView.
async (page) => {
  page.setDefaultTimeout(10000);
  const results = [], check = (v, msg) => { if (!v) throw Error(msg); };
  const set = patch => page.evaluate(p => Object.assign(window.audioDeletionFixture, p), patch);
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(v => v.command === c), command);
  const region = () => page.getByRole("region", { name: "音声だけの削除と中断対応", exact: true });
  const dialog = () => page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: /音声だけの削除を確認|再出現した音声への新しい同意|残り音声への新しい同意/u }) });
  const trigger = () => page.getByRole("button", { name: "既存会議の音声だけを削除", exact: true });
  const understand = () => dialog().getByRole("checkbox", { name: /対象を確認し、音声の再生/u }).check();
  const unknown = () => dialog().getByRole("checkbox", { name: /一覧にある未登録音声も/u });
  const publish = () => dialog().getByRole("button", { name: /音声だけを削除する|照合して音声削除を続行|新しい音声範囲へ別同意して削除/u });
  const boot = async () => { await page.reload(); await trigger().waitFor(); await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "音声だけを削除" && !b.disabled)); };
  const review = async () => { await trigger().click(); await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); };
  const done = () => page.getByRole("complementary", { name: "通知", exact: true }).getByRole("status").filter({ hasText: "音声だけを削除しました" }).waitFor();
  const pending = async (phase, altered = false) => {
    await page.evaluate(([p, a]) => window.audioDeletionFixture.pending(p, a), [phase, altered]);
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().getByRole("button", { name: "未完了音声削除の対象を確認", exact: true }).click();
    await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  };
  await boot(); await review();
  check((await dialog().innerText()).includes("文字起こし・議事録・下書き・根拠テキスト") && (await dialog().innerText()).includes("音声の再生・解析・再解析") && (await dialog().innerText()).includes("会議名（一覧表示）: 既存会議"), "meeting identity or history/scope missing");
  check(await publish().isDisabled() && !(await calls("start_data_operation")).length, "review executed");
  check(await dialog().getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "safe initial cancellation focus missing");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "音声だけを削除");
  check(!(await calls("start_data_operation")).length, "Escape executed"); results.push("audio scope, retained text/tracks, consent, Escape and opener focus");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "音声削除の安全な停止を要求", exact: true }).waitFor();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "navigation unlocked while busy");
  check(await page.getByRole("button", { name: "既存会議を削除", exact: true }).isDisabled(), "full deletion unlocked");
  check(!(await region().getByText(/音声だけを削除しました/u).count()), "premature success");
  await page.evaluate(() => window.audioDeletionFixture.complete()); await done();
  check(await page.evaluate(() => window.dataFixture.meetings.length) === 1 && await trigger().isEnabled(), "meeting deleted or removed body selectable");
  await page.getByRole("button", { name: "設定", exact: true }).click();
  check(!(await page.getByText("バックアップを作成しました。", { exact: false }).count()), "false backup success"); results.push("busy shared protection, receipt-only success, meeting list refresh, meeting retained");

  await boot(); await set({ unknownCount: 1 }); await review(); await understand(); check(await publish().isDisabled(), "unknown adopted implicitly");
  await unknown().check(); await publish().click(); await done();
  check((await calls("start_data_operation"))[0].args.request.action === "delete_including_unregistered", "unknown action missing"); results.push("separate fresh unregistered consent");

  await boot(); await set({ unknownCount: 1 }); await review(); await understand(); await unknown().check();
  const old = await page.evaluate(() => window.dataFixture.snapshot.audioDeletionPreview.confirmationId);
  await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(await page.evaluate(() => window.dataFixture.snapshot.audioDeletionPreview.confirmationId) !== old && !await unknown().isChecked() && await publish().isDisabled(), "refresh inherited consent"); results.push("refresh resets both confirmation-bound consents");

  await boot(); await set({ fileCount: 105 }); await trigger().click(); await dialog().waitFor();
  await dialog().getByText("1〜50 / 106 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "音声対象の次の50件" }).click(); await dialog().getByText("51〜100 / 106 件", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: "音声対象の次の50件" }).click(); await dialog().getByText("101〜106 / 106 件", { exact: true }).waitFor();
  check(await dialog().getByRole("button", { name: "音声対象の次の50件" }).isDisabled() && !(await calls("start_data_operation")).length, "pagination boundary executed"); results.push("106 frozen paths, bounded 50-entry pagination");

  await boot(); await set({ unknownCount: 1 }); await pending("not_started");
  check(await unknown().count() === 0 && (await dialog().innerText()).includes("保存済みの方針: 未登録音声を含める"), "saved policy mutable");
  await understand(); await publish().click(); await done();
  check((await calls("start_data_operation"))[0].args.request.action === "publish", "pending policy replaced by new consent");
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "cold completion released write fence"); results.push("saved policy read-only, fixed publish resume and cold restart fence");

  await boot(); await pending("not_started", true); check(await publish().isDisabled(), "changed unstarted executed");
  await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止" }).click();
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("status").filter({ hasText: "削除を開始せず保持して中止" }).waitFor();
  check(await page.evaluate(() => window.audioDeletionFixture.kept === 1 && window.audioDeletionFixture.deleted === 0 && window.dataFixture.meetings.length === 1), "keep deleted data"); results.push("unstarted keep preserves audio and history, restart required");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "音声削除の安全な停止を要求", exact: true }).click();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "stop unlocked prematurely");
  await page.evaluate(() => window.audioDeletionFixture.complete());
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
  await set({ mode: "success" }); await region().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await dialog().waitFor();
  check(await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止" }).count() === 0, "partial fake restore exposed");
  check((await dialog().innerText()).includes("処理済み") && (await dialog().innerText()).includes("現在の実在一覧ではありません"), "frozen processed paths unexplained");
  await understand(); await publish().click(); await done(); results.push("partial cancellation, continued fence, frozen progress, no fake keep, explicit resume");

  await boot(); await pending("awaiting_ack", true); await set({ unknownCount: 1 });
  await dialog().getByRole("button", { name: "再出現した音声を確認（まだ削除しない）" }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check((await dialog().innerText()).includes("元の操作") && (await dialog().innerText()).includes("新しい操作") && await publish().isDisabled(), "old progress lost or renewed autoexec");
  await understand(); check(await publish().isDisabled(), "renewed unknown implicitly adopted"); await unknown().check(); await publish().click(); await done(); results.push("renewed scope retains former progress and requires new distinct consent");

  for (const phase of ["files", "availability"]) {
    await boot(); await pending(phase, true); await set({ unknownCount: 1 });
    const before = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
    await dialog().getByRole("button", { name: "残り音声を確認（まだ削除しない）", exact: true }).click();
    await dialog().getByRole("heading", { name: "残り音声への新しい同意", exact: true }).waitFor();
    await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
    const after = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
    check(after.operationId === before.operationId && after.sourceOperationId === before.operationId && after.confirmationId !== before.confirmationId && after.reconfirmationSourceKind === "native_audio_pending" && !after.priorDeletionCompleted, "remaining scope mislabeled or changed native ID");
    const body = await dialog().innerText();
    check(body.includes("同じ操作") && body.includes("中断位置") && !body.includes("音声削除完了済み") && !body.includes("新しい操作"), "remaining history incorrectly described");
    check(await publish().isDisabled() && await page.evaluate(() => window.audioDeletionFixture.deleted) === 0 && await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止" }).count() === 0, "remaining review deleted or exposed keep");
    if (phase === "files") {
      for (const width of [1200, 900]) {
        await page.setViewportSize({ width, height: 800 });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "remaining dialog overflow");
        await page.screenshot({ path: `output/playwright/audio-remaining-${width}.png` });
        await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
        await page.screenshot({ path: `output/playwright/audio-remaining-${width}-actions.png` });
        await dialog().evaluate(e => { e.scrollTop = 0; });
      }
      results.push("remaining consent warning, same ID, separate choices and actions at 1200px and 900px");
    }
    await understand(); await unknown().check();
    await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click();
    await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
    check(!await unknown().isChecked() && await publish().isDisabled(), "remaining refresh inherited consent");
    await understand(); check(await publish().isDisabled(), "remaining unknown adopted by prior policy");
    await unknown().check(); await publish().click(); await done();
    check(await page.evaluate(() => window.dataFixture.snapshot.audioDeletionReceipt.operationId) === before.operationId, "remaining result not bound to original operation");
    results.push(`${phase} remaining review is readonly, same native ID, fresh checkbox decisions and no false completion`);
  }

  await boot(); await pending("files", true); await set({ unknownCount: 1 });
  const pendingId = await page.evaluate(() => window.dataFixture.snapshot.audioDeletionPreview.operationId);
  await dialog().getByRole("button", { name: "残り音声を確認（まだ削除しない）", exact: true }).click();
  await dialog().getByRole("heading", { name: "残り音声への新しい同意", exact: true }).waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  await understand(); await unknown().check(); await set({ mode: "lostNoCommit" }); await publish().click();
  await dialog().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).waitFor();
  const lostRemaining = (await calls("start_data_operation")).at(-1).args.request;
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "remaining lost response released protection");
  await set({ mode: "unstarted" });
  await dialog().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).click();
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "再同意保存後に停止。残り音声を保持" }).waitFor();
  await region().getByRole("button", { name: "音声削除の確認内容を開く", exact: true }).click();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  const savedRemaining = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
  check(JSON.stringify((await calls("start_data_operation")).at(-1).args.request) === JSON.stringify(lostRemaining), "remaining retry replaced confirmation or choice");
  check(savedRemaining.phase === "files" && savedRemaining.operationId === pendingId && savedRemaining.includeUnregisteredRecorded === true && !savedRemaining.keepAllowed && await unknown().count() === 0, "remaining saved consent became unstarted or mutable");
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); await understand(); await publish().click(); await done();
  check((await calls("start_data_operation")).at(-1).args.request.action === "publish" && await page.evaluate(() => window.dataFixture.snapshot.audioDeletionReceipt.operationId) === pendingId, "remaining resume used a new choice or operation");
  results.push("remaining lost response replays exact consent; saved consent resumes original started operation without keep or new choice");

  await boot(); await set({ mode: "lostNoCommit" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).waitFor();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled() && await page.getByRole("button", { name: "既存会議を削除", exact: true }).isDisabled(), "unknown response unlocked other writes");
  const original = (await calls("start_data_operation"))[0].args.request;
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).click(); await done();
  check(JSON.stringify((await calls("start_data_operation"))[1].args.request) === JSON.stringify(original), "lost request substituted"); results.push("unknown response protects shared actions and explicitly replays exact request");

  await boot(); await set({ mode: "forged" }); await review(); await understand(); await publish().click();
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  check(!(await region().getByText(/音声だけを削除しました/u).count()), "forged receipt accepted"); results.push("forged audio receipt rejected without success");

  await boot(); await set({ pageFail: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByRole("alert").filter({ hasText: "模擬音声対象一覧失敗" }).waitFor(); await understand(); check(await publish().isDisabled(), "missing page permitted delete");
  await set({ pageFail: false }); await dialog().getByRole("button", { name: "音声対象一覧を再読込" }).click(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor(); results.push("page read failure fails closed and explicit retry recovers");

  await boot(); await set({ foreignPage: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByRole("alert").filter({ hasText: "音声削除の状態が不正" }).waitFor(); await understand(); check(await publish().isDisabled(), "foreign audio paths adopted"); results.push("foreign audio page fails closed");

  await boot(); await set({ listFail: true }); await region().getByRole("button", { name: "音声削除と書込停止の状態を再読込" }).click();
  await region().getByRole("alert").filter({ hasText: "模擬音声削除一覧失敗" }).waitFor(); check(await trigger().isDisabled(), "invalid listing did not protect writes"); results.push("listing read failure locks writes");

  await boot(); await set({ reviewLost: true }); await trigger().click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(!(await calls("start_data_operation")).length, "lost review auto published"); results.push("lost review recovered by shared state without deletion");

  await boot(); await set({ pathsDelay: true }); await trigger().click(); await dialog().waitFor();
  await page.waitForFunction(() => !!window.audioDeletionFixture.releasePage);
  await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await dialog().waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  const newId = await page.evaluate(() => window.dataFixture.snapshot.audioDeletionPreview.confirmationId);
  await page.evaluate(() => window.audioDeletionFixture.releasePage()); await understand();
  check(await publish().isEnabled() && await page.evaluate(() => window.dataFixture.snapshot.audioDeletionPreview.confirmationId) === newId, "late page replaced newer confirmation"); results.push("late old page ignored after refresh");

  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPendingEdits } = await import("/src/draft-save.ts");
      registerPendingEdits(() => fail ? Promise.reject(Error("模擬音声削除前の編集保存失敗")) : new Promise(resolve => { window.audioDeletionFixture.releaseEdit = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await page.getByRole("alert").filter({ hasText: "模擬音声削除前の編集保存失敗" }).waitFor(); check(!(await calls("review_meeting_audio_deletion")).length, "failed edit save admitted review"); }
    else { await page.waitForFunction(() => !!window.audioDeletionFixture.releaseEdit); check(!(await calls("review_meeting_audio_deletion")).length, "review skipped save wait"); await page.evaluate(() => window.audioDeletionFixture.releaseEdit()); await dialog().waitFor(); }
    results.push(fail ? "save failure prevents audio review" : "audio review waits for edits");
  }
  for (const fail of [true, false]) {
    await boot();
    await page.evaluate(async fail => {
      const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
      registerPlaybackStop(() => fail ? Promise.reject(Error("模擬音声削除前の再生解放失敗")) : new Promise(resolve => { window.audioDeletionFixture.releasePlayback = resolve; }));
    }, fail);
    await trigger().click();
    if (fail) { await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "模擬音声削除前の再生解放失敗" }).waitFor(); check(!(await calls("review_meeting_audio_deletion")).length, "playback failure admitted review"); }
    else { await page.waitForFunction(() => !!window.audioDeletionFixture.releasePlayback); check(!(await calls("review_meeting_audio_deletion")).length, "review skipped playback wait"); await page.evaluate(() => window.audioDeletionFixture.releasePlayback()); await dialog().waitFor(); }
    results.push(fail ? "playback release failure prevents audio review" : "audio review waits for playback release");
  }
  await boot(); await review(); await understand();
  await page.evaluate(async () => {
    const { registerPlaybackStop } = await import("/src/playback-barrier.ts");
    registerPlaybackStop(() => new Promise(resolve => { window.audioDeletionFixture.releasePlayback = resolve; }));
  });
  await publish().click(); await page.waitForFunction(() => !!window.audioDeletionFixture.releasePlayback);
  await page.evaluate(async () => { await window.dataFixture.changed({ audioDeletionPreview: { ...window.dataFixture.snapshot.audioDeletionPreview, confirmationId: crypto.randomUUID() } }); window.audioDeletionFixture.releasePlayback(); });
  await dialog().getByRole("alert").filter({ hasText: "待機中に音声削除の確認が変更" }).waitFor();
  check(!(await calls("start_data_operation")).length, "playback wait rebound old consent"); results.push("confirmation rechecked after playback wait, no consent rebinding");

  await boot(); await set({ mode: "lost" }); await page.evaluate(() => { window.dataFixture.suppressEvents = true; });
  await review(); await understand(); await publish().click(); await done();
  check(await page.evaluate(() => window.audioDeletionFixture.deleted === 1) && (await calls("start_data_operation")).length === 1, "poll completion duplicated deletion"); results.push("lost response and suppressed events recovered by poll, single execution");

  await boot(); await pending("awaiting_ack"); await understand(); await publish().click(); await done();
  check(await page.evaluate(() => window.dataFixture.meetings.length === 1), "completion acknowledgement deleted meeting"); results.push("completed file stage acknowledged without meeting deletion");

  await boot(); await pending("files", true);
  check(await publish().isDisabled() && await dialog().getByRole("button", { name: "残り音声を確認（まだ削除しない）", exact: true }).count() === 1 && await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止", exact: true }).count() === 0, "changed unfinished scope executed or falsely became unstarted");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled() && !(await calls("start_data_operation")).length, "closing pending scope released fence"); results.push("changed unfinished scope refuses old consent; readonly remaining review offered, close preserves fence");

  await boot();
  await page.evaluate(async () => {
    const d = window.audioDeletionFixture;
    d.blocked = true;
    d.items = [{ operationId: crypto.randomUUID(), state: "completed", meetingId: null, reviewAllowed: false, errorMessage: "復元履歴を検証できません", startupReason: "restored_audio_scope_reappeared" }];
  });
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await region().getByRole("alert").filter({ hasText: "元の利用不可記録には削除同意がありません" }).waitFor();
  check(await region().getByRole("button", { name: "未完了音声削除の対象を確認", exact: true }).isDisabled() && await page.getByRole("button", { name: "バックアップを作成", exact: true }).isDisabled(), "unsupported restored receipt granted authority"); results.push("restored invalid history stays visible without review or ordinary writes");

  await boot(); await pending("not_started");
  await page.evaluate(async () => {
    const { registerPendingEdits } = await import("/src/draft-save.ts");
    registerPendingEdits(() => Promise.reject(Error("書込停止中の編集を再保存しない")));
  });
  await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止", exact: true }).click();
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("status").filter({ hasText: "削除を開始せず保持して中止" }).waitFor();
  check(await page.evaluate(() => window.audioDeletionFixture.kept === 1), "pending recovery tried to rewrite fenced edits"); results.push("pending recovery never rewrites fenced document rows");

  await boot(); await set({ mode: "cancelRace" }); await review(); await understand(); await publish().click();
  await region().getByRole("button", { name: "音声削除の安全な停止を要求", exact: true }).click();
  await page.evaluate(() => window.audioDeletionFixture.complete()); await done();
  check(await page.evaluate(() => window.audioDeletionFixture.deleted === 1 && window.audioDeletionFixture.kept === 0), "late cancel replaced completion"); results.push("committed completion wins late cancellation");

  await boot(); await page.getByRole("button", { name: "開く", exact: true }).click();
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
  const text = editor.getByRole("textbox", { name: "00:00:00の発言", exact: true });
  await editor.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
  await text.waitFor(); await text.fill("修正済みの点検日です。");
  await editor.getByRole("searchbox", { name: "発言を検索", exact: true }).fill("点検");
  await text.evaluate(e => { window.audioDeletionFixture.editorNode = e; });
  const reads = await page.evaluate(() => [window.audioDeletionFixture.analysisReads, window.audioDeletionFixture.draftReads]);
  await review(); await understand(); await publish().click(); await done();
  check(await text.inputValue() === "修正済みの点検日です。" && await text.evaluate(e => e === window.audioDeletionFixture.editorNode), "audio completion remounted editor or lost text");
  check(await editor.getByRole("searchbox", { name: "発言を検索", exact: true }).inputValue() === "点検", "audio completion reset document UI state");
  check(JSON.stringify(await page.evaluate(() => [window.audioDeletionFixture.analysisReads, window.audioDeletionFixture.draftReads])) === JSON.stringify(reads), "audio completion reloaded document heads");
  check((await calls("save_transcript_draft")).some(c => c.args.payload.utterances[0]?.effective_text === "修正済みの点検日です。"), "deletion skipped actual transcript save"); results.push("actual transcript queue saves before deletion and mounted editor retains text and search state");

  await boot(); await set({ mode: "delay" }); await review(); await understand(); await publish().click();
  await page.evaluate(async () => {
    const { emit } = await import("/@fs/Users/skodani/Codebase/mon/app/node_modules/@tauri-apps/api/event.js"); await emit("main-close-requested", { quit: true });
  });
  const closing = page.getByRole("dialog", { name: "進行中の処理を停止して終了しますか？", exact: true });
  await closing.waitFor();
  const closeMessage = await closing.innerText();
  await closing.getByRole("button", { name: "安全な停止を待って終了する", exact: true }).dblclick();
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelling");
  check(!await page.evaluate(() => window.dataFixture.calls.some(c => c.command === "finish_main_close")), "close did not wait for actual audio worker stop");
  await page.evaluate(() => window.audioDeletionFixture.complete());
  await page.waitForFunction(() => window.dataFixture.calls.some(c => c.command === "finish_main_close"));
  check(closeMessage.includes("音声削除") && closeMessage.includes("復元しません"), "close used misleading backup wording"); results.push("main close uses audio-specific irreversible stop confirmation");

  await boot(); await review();
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 800 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "horizontal dialog overflow");
    await page.screenshot({ path: `output/playwright/audio-deletion-${width}.png` });
    await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
    await page.screenshot({ path: `output/playwright/audio-deletion-${width}-actions.png` });
    await dialog().evaluate(e => { e.scrollTop = 0; });
  }
  results.push("1200px and 900px modal layout screenshots");
  return { passed: results.length, results };
}
