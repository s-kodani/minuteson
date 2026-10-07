async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = () => page.getByRole("complementary", { name: "通知", exact: true });
  const status = text => host().getByRole("status").filter({ hasText: text });
  const alert = text => host().getByRole("alert").filter({ hasText: text });
  const owner = () => page.getByRole("region", { name: "解析の通知と要対応状態", exact: true });
  const calls = command => page.evaluate(command => window.monBrowserFixture.calls.filter(x => x.command === command).length, command);
  const boot = async () => { await page.reload(); await owner().waitFor({ state: "attached" }); await page.waitForFunction(() => window.monBrowserFixture.calls.some(x => x.command === "get_analysis_notification_jobs")); };
  const start = async (patch = {}) => {
    await page.evaluate(async patch => { const f = window.monBrowserFixture; f.notificationJobs = [f.makeNotificationJob(patch)]; await f.emitNotification(); }, patch);
    await status("受け付けました").waitFor();
  };
  const update = async patch => {
    const before = await calls("get_analysis_notification_jobs");
    await page.evaluate(async patch => { const f = window.monBrowserFixture; Object.assign(f.notificationJobs[0], { updatedAt: Date.now() }, patch); await f.emitNotification(); }, patch);
    await page.waitForFunction(n => window.monBrowserFixture.calls.filter(x => x.command === "get_analysis_notification_jobs").length > n, before);
  };
  const dismiss = async () => { for (const button of await host().getByRole("button", { name: /通知を閉じる$/ }).all()) if (await button.isVisible()) await button.click(); };
  const noWork = async () => check(await page.evaluate(() => !window.monBrowserFixture.calls.some(x => /^(start_|retry_|resume_analysis|cancel_analysis|adopt_|restore_)/.test(x.command))), "notification executed work/adoption/restoration");

  await boot(); await page.evaluate(async () => { const f = window.monBrowserFixture; f.notificationJobs = [f.makeNotificationJob({ state: "succeeded", createdAt: 1, updatedAt: 2, outputAnalysisId: "historical" })]; await f.emitNotification({ state: "succeeded", outputAnalysisId: "forged-event" }); });
  check(await host().locator(".toast-card").count() === 0 && await calls("get_analysis_notification_output") === 0, "historical/event payload announced success");
  results.push("startup/restored history and forged event payload never announce completion");

  await boot(); await start(); check(await status("候補を保存").count() === 0, "admission treated as publication"); await dismiss();
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await update({ state: "succeeded", completedUnits: 2, outputAnalysisId: "exact-output" }); await status("候補を保存").waitFor();
  check(await page.getByRole("button", { name: "設定", exact: true }).getAttribute("aria-current") === "page" && await calls("get_analysis_notification_output") === 1, "completion missed on settings or not verified");
  const savedId = await host().locator(".toast-card").getAttribute("data-toast-id");
  await page.evaluate(() => window.monBrowserFixture.emitNotification({ id: "foreign", state: "succeeded" }));
  check(await host().locator(".toast-card").getAttribute("data-toast-id") === savedId, "duplicate invalidation repeated notice");
  await noWork(); results.push("exact output publication is notified once across workspace navigation, without adopting");
  await host().getByRole("button", { name: "会議を開く", exact: true }).click(); await page.getByRole("textbox", { name: "会議名を変更", exact: true }).waitFor();
  check(await page.getByRole("textbox", { name: "会議名を変更", exact: true }).inputValue() === "会議A", "toast opened wrong meeting");
  await noWork(); results.push("safe notification action reads and opens the exact meeting only");

  for (const mode of ["lost", "foreign", "hidden"]) {
    await boot(); await start(); await dismiss(); await page.evaluate(mode => window.monBrowserFixture.notificationProofMode = mode, mode);
    await update({ state: "succeeded", outputAnalysisId: "unverified" }); await alert("解析通知の読込・結果確認").waitFor();
    check(await status("保存しました").count() === 0, "unverified output announced success");
    await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
    check(await owner().getByLabel("解析通知の読込・結果確認の要対応状態", { exact: true }).count() === 1, "dismissal erased pending proof/cause");
    await page.evaluate(() => window.monBrowserFixture.notificationProofMode = "valid"); await owner().getByRole("button", { name: "状態だけを再読込", exact: true }).click();
    await status("候補を保存").waitFor(); check(await owner().getByLabel("解析通知の読込・結果確認の要対応状態", { exact: true }).count() === 0, "valid exact proof did not resolve read issue");
    await noWork(); results.push(`${mode} proof holds success and watches exact job; read-only retry resolves without execution`);
  }

  await boot(); await start(); await dismiss(); await update({ state: "succeeded" }); await alert("解析通知の読込・結果確認").waitFor();
  check(await status("保存しました").count() === 0, "missing output ID announced success"); await noWork();
  results.push("succeeded state without output version never announces success");

  await boot(); await start(); await dismiss(); await update({ state: "failed", errorCode: "engine_failed" }); await alert("に失敗しました").waitFor();
  await page.waitForTimeout(4300); check(await alert("に失敗しました").count() === 1, "critical job failure auto-expired");
  await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click(); await page.getByRole("button", { name: "設定", exact: true }).click();
  await owner().getByText("解析の要対応：1件", { exact: true }).click();
  check(await owner().getByText("「会議A」の文字起こし：失敗", { exact: true }).count() === 1, "navigation lost dismissed failure");
  await owner().getByRole("button", { name: "「会議A」の文字起こしの要対応の説明を開く", exact: true }).click();
  check((await page.getByRole("dialog").innerText()).includes("engine_failed"), "retained failure lost exact cause"); await page.keyboard.press("Escape"); await noWork();
  results.push("failure is important, survives dismissal/navigation, and retains the exact cause");
  await page.evaluate(async () => { const f = window.monBrowserFixture; const old = f.notificationJobs[0]; f.notificationJobs.unshift(f.makeNotificationJob({ attempt: 2, retryOfJobId: old.id, createdAt: old.createdAt + 1, updatedAt: Date.now() })); await f.emitNotification(); });
  await owner().getByText("解析の要対応：1件", { exact: true }).waitFor({ state: "hidden" }); await status("受け付けました").waitFor(); await noWork();
  results.push("authoritative latest retry clears prior attention without notification-side retry");

  await boot(); await start(); await dismiss();
  await page.evaluate(async () => { const f = window.monBrowserFixture; f.notificationJobs.unshift(f.makeNotificationJob({ createdAt: f.notificationJobs[0].createdAt + 1, updatedAt: Date.now() })); await f.emitNotification(); });
  await status("受け付けました").waitFor(); await dismiss();
  await page.evaluate(async () => { const f = window.monBrowserFixture; Object.assign(f.notificationJobs[1], { state: "failed", errorCode: "older_engine_failed", updatedAt: Date.now() }); await f.emitNotification(); });
  await alert("に失敗しました").waitFor(); check(await owner().getByText("解析の要対応：1件", { exact: true }).count() === 1, "new distinct work hid older watched failure");
  await page.evaluate(() => window.monBrowserFixture.emitNotification());
  await page.waitForFunction(() => window.monBrowserFixture.calls.filter(x => x.command === "get_analysis_notification_jobs").at(-1).args.watchedJobIds.includes(window.monBrowserFixture.notificationJobs[1].id));
  await noWork(); results.push("new distinct work does not hide or stop watching an earlier tracked failure");

  await boot(); await start(); await dismiss(); await page.evaluate(() => window.monBrowserFixture.notificationProofMode = "lost");
  await update({ state: "succeeded", outputAnalysisId: "old-exact-output" }); await alert("解析通知の読込・結果確認").waitFor();
  await page.evaluate(async () => { const f = window.monBrowserFixture; const old = f.notificationJobs[0]; f.notificationJobs.unshift(f.makeNotificationJob({ createdAt: old.createdAt + 1, updatedAt: Date.now() })); await f.emitNotification(); });
  await page.waitForFunction(() => window.monBrowserFixture.calls.filter(x => x.command === "get_analysis_notification_jobs").at(-1).args.watchedJobIds.includes(window.monBrowserFixture.notificationJobs[1].id));
  await page.evaluate(() => window.monBrowserFixture.notificationProofMode = "valid"); await owner().getByRole("button", { name: "状態だけを再読込", exact: true }).click(); await status("候補を保存").waitFor();
  check(await page.evaluate(() => window.monBrowserFixture.notificationProofReads.includes(window.monBrowserFixture.notificationJobs[1].id)), "proof switched to newer queued job"); await noWork();
  results.push("unverified older completion stays watched beyond newer history and retries its own exact output");

  await boot(); await start(); await dismiss(); await page.evaluate(() => window.monBrowserFixture.notificationProofMode = "delay");
  await update({ state: "succeeded", outputAnalysisId: "delayed-output" }); await page.waitForFunction(() => !!window.monBrowserFixture.releaseNotificationProof);
  await page.evaluate(async () => { const f = window.monBrowserFixture; await Promise.all(Array.from({ length: 8 }, () => f.emitNotification({ state: "succeeded", outputAnalysisId: "foreign" }))); });
  check(await calls("get_analysis_notification_output") === 1 && await status("候補を保存").count() === 0, "concurrent hints spawned duplicate proof or premature success");
  await page.evaluate(() => { const f = window.monBrowserFixture; f.notificationProofMode = "valid"; f.releaseNotificationProof(); }); await status("候補を保存").waitFor();
  check(await calls("get_analysis_notification_output") === 1, "serialized refresh repeated confirmed output proof"); await noWork();
  results.push("in-flight proof coalesces rapid invalidations and publishes only after exact read completion");

  await boot(); await page.evaluate(async () => { const f = window.monBrowserFixture; f.notificationReadFail = true; await f.emitNotification({ state: "succeeded" }); }); await alert("解析通知の読込・結果確認に失敗").waitFor();
  check(!(await alert("解析通知の読込・結果確認に失敗").innerText()).includes("模擬解析通知読込失敗"), "long raw cause remained in the toast");
  await owner().getByRole("button", { name: "解析通知の読込・結果確認の説明を開く", exact: true }).click(); check((await page.getByRole("dialog").innerText()).includes("模擬解析通知読込失敗"), "short notification lost detailed read cause"); await page.keyboard.press("Escape");
  await page.evaluate(() => window.monBrowserFixture.notificationReadFail = false); await host().getByRole("button", { name: "状態だけを再読込", exact: true }).click(); await alert("解析通知の読込・結果確認に失敗").waitFor({ state: "hidden" });
  check(await status("保存しました").count() === 0, "read recovery fabricated success"); await noWork();
  results.push("census failure retains attention and a read-only toast retry resolves without fabricated completion");

  await boot(); await start(); await dismiss(); await update({ state: "pausing" }); check(await status("一時停止しました").count() === 0, "pause request treated as checkpoint completion");
  await update({ state: "paused", pauseReason: "recording" }); await status("録音・取り込み優先").waitFor(); await dismiss();
  await update({ state: "cancelled", pauseReason: null }); await status("取り消しました。現在版は保持").waitFor(); await noWork();
  results.push("checkpoint pause and terminal cancellation are separate from requests and preserve current version");

  for (const kind of ["diarization", "minutes_generation"]) {
    await boot(); await start({ kind, meetingId: "meeting-1", meetingTitle: "会議B" }); await dismiss();
    await update({ state: "succeeded", outputAnalysisId: kind === "diarization" ? "diar-output" : null, outputMinutesId: kind === "minutes_generation" ? "minutes-output" : null });
    await status(`「会議B」の${kind === "diarization" ? "話者分離" : "議事録生成"}の候補を保存`).waitFor(); await noWork();
    results.push(`${kind} verifies its own output kind and meeting instead of selected meeting`);
  }

  await boot(); await page.getByRole("button", { name: "会議一覧", exact: true }).click(); await page.getByRole("button", { name: "開く", exact: true }).first().click();
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
  await editor.getByRole("button", { name: "編集を開始", exact: true }).click();
  const text = editor.getByRole("textbox", { name: /の発言$/ }).first(); await page.evaluate(() => window.monBrowserFixture.failSave = true); await text.fill("通知で失われない編集中の本文");
  await start({ meetingId: "meeting-1", meetingTitle: "会議B" });
  await host().getByRole("button", { name: "会議を開く", exact: true }).click(); await page.getByRole("alert").filter({ hasText: "模擬保存競合" }).first().waitFor();
  check(await text.inputValue() === "通知で失われない編集中の本文" && await page.getByRole("textbox", { name: "会議名を変更", exact: true }).inputValue() === "会議A", "failed edit flush switched meeting/discarded input"); await noWork();
  results.push("notification navigation honors pending edit save failure and retains original meeting/input");

  await boot(); await start({ meetingId: "meeting-1", meetingTitle: "会議B" }); await page.evaluate(() => window.monBrowserFixture.missingMeetingId = "meeting-1");
  await host().getByRole("button", { name: "会議を開く", exact: true }).click(); await page.getByRole("alert").filter({ hasText: "通知対象の会議が見つかりません" }).waitFor();
  check(await page.getByRole("textbox", { name: "会議名を変更", exact: true }).count() === 0, "deleted meeting substituted another target"); await noWork();
  results.push("missing notified meeting fails explicitly without substituting another meeting");

  await boot(); await start(); await dismiss(); await update({ state: "interrupted", errorCode: "restored_preparation_required" }); await alert("再開の確認が必要").waitFor();
  await page.evaluate(async () => { const f = window.monBrowserFixture; f.notificationJobs = []; await f.emitNotification(); });
  await alert("再開の確認が必要").waitFor({ state: "hidden" }); check(await owner().getByText("解析の要対応：1件", { exact: true }).count() === 0, "deleted authoritative job left critical alert");
  results.push("interrupted work is important; authoritative removal resolves alert and retained state");

  await boot(); await page.evaluate(() => window.monBrowserFixture.emitWorkerError("模擬ワーカー停止：再起動後に状態を確認")); await alert("解析ワーカーから異常").waitFor();
  await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click(); await page.getByRole("button", { name: "設定", exact: true }).click();
  check(await owner().getByLabel("解析ワーカーの要対応状態", { exact: true }).count() === 1, "dismissal/read falsely resolved worker failure"); await noWork();
  results.push("worker error is retained globally, not silently resolved by a census read");
  await boot(); await page.evaluate(async () => { const f = window.monBrowserFixture; f.notificationJobs = Array.from({ length: 55 }, (_, i) => f.makeNotificationJob({ meetingId: `historical-${i}`, meetingTitle: `履歴会議${i + 1}`, createdAt: 1, updatedAt: 2, state: "failed", errorCode: `engine-${i}` })); await f.emitNotification(); });
  await owner().getByText("解析の要対応：55件", { exact: true }).click(); check(await owner().getByRole("listitem").count() === 50 && await host().locator(".toast-card").count() === 0, "historical attention unbounded or emitted startup error spam");
  await owner().getByRole("button", { name: "次へ", exact: true }).click(); check(await owner().getByRole("listitem").count() === 5, "last attention page not reachable");
  await page.waitForFunction(() => { const rows = document.querySelectorAll('.job-issues li'), a = rows[0]?.getBoundingClientRect(), b = rows[rows.length - 1]?.getBoundingClientRect(); return a?.top >= 64 && b?.bottom <= window.innerHeight; }); await noWork();
  results.push("historical attention is quiet and all 55 distinct failures remain accessible through bounded pages");
  for (const theme of ["light", "dark"]) {
    await page.getByRole("combobox", { name: "表示テーマ", exact: true }).selectOption(theme);
    await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme);
    await page.screenshot({ path: `job-notifications-attention-${theme}.png` });
  }
  return { passed: results.length, results };
}
