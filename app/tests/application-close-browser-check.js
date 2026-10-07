async (page, capturePath, detailUI) => {
  const context = page.context(), results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const title = "進行中の処理を停止して終了しますか？", startTitle = "選択した会議の録音を開始しますか？";
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const accept = () => modal(title).getByRole("button", { name: "安全な停止を待って終了する", exact: true });
  const start = () => modal(startTitle).getByRole("button", { name: "録音を開始する", exact: true });
  const count = command => page.evaluate(c => window.closeFixture.calls.filter(x => x.command === c).length, command);
  const boot = async (select = false) => {
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(() => !!window.closeFixture && !!document.querySelector(".meeting-list li"));
    if (select) { await page.locator(".meeting-list li").first().getByRole("button", { name: "開く", exact: true }).click(); await detailUI.openMeetingTools(); await page.waitForFunction(() => document.querySelector('[aria-label="録音状態"] button')?.disabled === false); }
  };
  const configureImport = busy => page.evaluate(async busy => window.closeFixture.importChanged({ token: crypto.randomUUID(), phase: busy ? "copying" : "ready", busy, fileName: "合成会議.m4a" }), busy);
  const configureData = () => page.evaluate(() => window.dataFixture.changed({ token: crypto.randomUUID(), kind: "backup", phase: "publishing", busy: true }));
  const request = () => page.evaluate(() => window.closeFixture.request(true));
  const dismissFailed = async name => { await modal(name).getByRole("alert").waitFor(); await modal(name).getByRole("button", { name: "確認を閉じる", exact: true }).click(); await modal(name).waitFor({ state: "hidden" }); };
  let stage = "setup";
  try {
    stage = "start-decline"; await boot(true); await page.getByRole("button", { name: "録音を開始", exact: true }).click(); await modal(startTitle).waitFor();
    check((await modal(startTitle).innerText()).includes("既存会議") && (await modal(startTitle).innerText()).includes("参加者の同意") && await modal(startTitle).getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "start omitted target or unsafe focus");
    await page.keyboard.press("Enter"); await modal(startTitle).waitFor({ state: "hidden" }); check(await count("start_recording") === 0, "initial Enter started recording");
    results.push("recording start reviews the exact meeting and permission scope; initial Enter cancels without device access");

    // Dispatch on the same original control: a physical second click can land
    // on the newly opened modal backdrop and correctly decline it instead.
    stage = "start-once"; await boot(true); await page.getByRole("button", { name: "録音を開始", exact: true }).evaluate(e => { e.click(); e.click(); }); await start().waitFor(); await start().dblclick(); await modal(startTitle).waitFor({ state: "hidden" });
    check(await count("start_recording") === 1 && await page.evaluate(() => window.closeFixture.recording.phase === "preparing"), "double activation repeated start");
    results.push("double activation starts exactly once after explicit consent and saved edits");

    for (const change of ["meeting", "import"]) {
      stage = `start-changed-${change}`; await boot(true); await page.getByRole("button", { name: "録音を開始", exact: true }).click(); await start().waitFor();
      await page.evaluate(change => { window.closeFixture.mutateDuringFlush = change; }, change); await start().click(); await dismissFailed(startTitle);
      check(await count("start_recording") === 0, "start adopted a changed target after saving");
      results.push(`recording start rejects a changed ${change} target after saving without sending start`);
    }
    stage = "start-save-failure"; await boot(true); await page.getByRole("button", { name: "録音を開始", exact: true }).click(); await start().waitFor(); await page.evaluate(() => { window.closeFixture.failFlush = true; }); await start().click(); await dismissFailed(startTitle);
    check(await count("start_recording") === 0 && await page.getByRole("textbox", { name: "会議名を変更", exact: true }).inputValue() === "既存会議", "save failure started or erased original meeting context");
    results.push("failed edit flush retains meeting context and does not call recording start");

    stage = "start-ack-lost"; await boot(true); await page.evaluate(() => { window.closeFixture.startLost = true; }); await page.getByRole("button", { name: "録音を開始", exact: true }).click(); await start().click(); await modal(startTitle).getByRole("alert").waitFor(); check(await start().isDisabled() && await count("start_recording") === 1, "lost accepted start was replayed"); await dismissFailed(startTitle);
    check(await page.getByRole("button", { name: "録音を開始", exact: true }).isDisabled() && await page.evaluate(() => window.closeFixture.recording.phase === "preparing"), "lost response fabricated idle");
    results.push("lost accepted-start acknowledgement retains canonical preparing state without replay");

    stage = "start-import-scope"; await boot(true); await configureImport(false); await page.getByRole("button", { name: "録音を開始", exact: true }).click(); await start().waitFor(); check((await modal(startTitle).innerText()).includes("合成会議.m4a") && (await modal(startTitle).innerText()).includes("元ファイルと保存済み会議を保持"), "start lost import scope"); await page.keyboard.press("Escape"); await modal(startTitle).waitFor({ state: "hidden" }); check(await count("start_recording") === 0 && await count("cancel_audio_import") === 0, "decline cancelled import");
    results.push("starting with a selected import explains exact loss and declining retains the import");

    stage = "close-decline"; await boot(); await configureImport(false); const snapshot = await page.evaluate(() => JSON.stringify(window.closeFixture.importing)); await request(); await modal(title).waitFor();
    check((await modal(title).innerText()).includes("合成会議.m4a") && (await modal(title).innerText()).includes("元ファイルは変更しません"), "close omitted reviewed import loss"); await page.keyboard.press("Enter"); await modal(title).waitFor({ state: "hidden" });
    check(await count("cancel_audio_import") === 0 && await count("finish_main_close") === 0 && await page.evaluate(old => JSON.stringify(window.closeFixture.importing) === old, snapshot), "decline altered import or quit");
    results.push("closing a ready import uses safe initial cancellation and preserves the exact review");

    stage = "close-import-wait"; await boot(); await configureImport(true); await Promise.all([request(), request()]); await accept().waitFor(); await accept().dblclick(); await page.waitForFunction(() => window.closeFixture.importing.phase === "cancelling");
    for (let i = 0; i < 5; i++) await page.keyboard.press("Escape"); check(await modal(title).isVisible() && await count("cancel_audio_import") === 1 && await count("finish_main_close") === 0 && await page.getByRole("button", { name: "概要", exact: true }).isDisabled(), "close dismissed or quit before actual import stop");
    await page.evaluate(() => window.closeFixture.finishImport(false)); await modal(title).waitFor({ state: "hidden" }); check(await count("finish_main_close") === 1, "actual import stop did not continue closing once");
    results.push("one same-token import cancellation waits through repeated Escape until actual worker termination");

    stage = "close-combined"; await boot(); await configureImport(true); await configureData(); await request(); await accept().waitFor(); check(await page.getByRole("dialog").count() === 1 && (await modal(title).innerText()).includes("バックアップ"), "combined work used multiple confirmations"); await accept().click(); await page.waitForFunction(() => window.closeFixture.importing.phase === "cancelling"); check(await count("cancel_data_operation") === 0, "data cancelled before import worker stopped");
    await page.evaluate(() => window.closeFixture.finishImport(true)); await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelling"); check(await count("finish_main_close") === 0, "data cancellation acknowledgement became completion"); await page.evaluate(() => window.closeFixture.finishData(true)); await modal(title).waitFor({ state: "hidden" });
    check(await count("cancel_audio_import") === 1 && await count("cancel_data_operation") === 1 && await count("finish_main_close") === 1 && await page.evaluate(() => !!window.closeFixture.importing.saved && !!window.dataFixture.snapshot.backupReceipt), "winning publication lost or closing repeated");
    check(await page.evaluate(() => { const c = window.closeFixture.calls.find(c => c.command === "finish_main_close"); return c.args.expected.importToken === window.closeFixture.importing.token && c.args.expected.dataToken === window.dataFixture.snapshot.token; }), "final native close lacked exact reviewed tokens");
    results.push("one combined close waits for both workers, preserves winning publications and sends exact final tokens");

    stage = "close-changed-target"; await boot(); await configureImport(false); await request(); await accept().waitFor(); await page.evaluate(() => window.closeFixture.importChanged({ token: crypto.randomUUID() })); await accept().click(); await dismissFailed(title); check(await count("cancel_audio_import") === 0 && await count("finish_main_close") === 0, "old close consent cancelled new import");
    results.push("a changed import token invalidates closing consent before cancellation");

    stage = "close-after-flush"; await boot(); await configureImport(false); await request(); await accept().waitFor(); await page.evaluate(() => { window.closeFixture.mutateDuringFlush = "import"; }); await accept().click(); await dismissFailed(title); check(await count("cancel_audio_import") === 0 && await count("finish_main_close") === 0, "flush-time target race was ignored");
    results.push("closing rereads targets after edit flush and refuses a newly admitted operation");

    for (const command of ["import", "data"]) {
      stage = `close-lost-${command}`; await boot(); if (command === "import") await configureImport(true); else await configureData();
      await page.evaluate(command => { window.closeFixture[command === "import" ? "cancelImportLost" : "cancelDataLost"] = true; }, command); await request(); await accept().click(); await modal(title).getByRole("alert").waitFor(); check(await accept().isDisabled() && await count(command === "import" ? "cancel_audio_import" : "cancel_data_operation") === 1 && await count("finish_main_close") === 0, "lost cancellation replayed or closed app"); await dismissFailed(title);
      if (command === "import") await page.evaluate(() => window.closeFixture.finishImport(false)); else await page.evaluate(() => window.closeFixture.finishData(false));
      check(await count("finish_main_close") === 0, "uncertain close automatically resumed after later termination"); results.push(`lost ${command} cancellation acknowledgement is retained without replay or automatic continuation`);
    }
    stage = "close-ack-lost"; await boot(); await page.evaluate(() => { window.closeFixture.activity.analysisBusy = true; window.closeFixture.finishLost = true; }); await request(); await accept().click(); await modal(title).getByRole("alert").waitFor(); check(await accept().isDisabled() && await count("finish_main_close") === 1, "lost close replayed"); await dismissFailed(title); check(await count("finish_main_close") === 1, "dismiss repeated close");
    results.push("lost final-close acknowledgement leaves the failure visible without resending exit");

    stage = "close-read-failure"; await boot(); await page.evaluate(() => { window.closeFixture.failRead = true; }); await request(); await page.getByRole("alert").filter({ hasText: "終了できませんでした" }).waitFor(); check(await count("finish_main_close") === 0 && await page.getByRole("dialog").count() === 0, "unknown native read fabricated quiet close");
    results.push("failed canonical close reads preserve the app and never fabricate quiet completion");

    stage = "quiet-save-failure"; await boot(); await page.getByRole("textbox", { name: "会議名", exact: true }).fill("保持する入力"); await page.evaluate(() => { window.closeFixture.failFlush = true; }); await request(); await page.getByRole("alert").filter({ hasText: "終了できませんでした" }).waitFor(); check(await count("finish_main_close") === 0 && await page.getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "保持する入力", "quiet close lost unsaved context");
    results.push("quiet close refuses failed edit flush and retains the current input");

    const savingTitle = "録音の保存完了後の終了を確認";
    const saving = () => page.evaluate(() => window.closeFixture.recordChanged({ phase: "saving", sessionId: crypto.randomUUID(), meetingId: window.dataFixture.meetings[0].id }));
    const saved = () => page.evaluate(() => window.closeFixture.recordChanged({ phase: "idle", sessionId: null, meetingId: null }));
    stage = "saving-decline"; await boot(); await saving(); await request(); await modal(savingTitle).waitFor(); await page.keyboard.press("Enter"); await modal(savingTitle).waitFor({ state: "hidden" }); await saved();
    check(await count("stop_recording") === 0 && await count("finish_main_close") === 0, "declined save-wait restarted stop or later closed app");
    results.push("declining exit during saving does not resend stop or automatically quit when saving later completes");

    stage = "saving-wait"; await boot(); await saving(); await request(); await modal(savingTitle).getByRole("button", { name: "保存完了を待って終了へ進む", exact: true }).dblclick(); await modal(savingTitle).getByRole("status").waitFor(); await page.keyboard.press("Escape");
    check(await modal(savingTitle).isVisible() && await count("finish_main_close") === 0 && await count("stop_recording") === 0, "saving acknowledgement became exit or resent stop");
    await saved(); await page.waitForFunction(() => window.closeFixture.calls.some(c => c.command === "finish_main_close")); check(await count("finish_main_close") === 1, "actual saved state did not close once");
    results.push("exit during saving waits read-only for actual idle before one checked close");

    stage = "saving-failure"; await boot(); await saving(); await request(); await modal(savingTitle).getByRole("button", { name: "保存完了を待って終了へ進む", exact: true }).click(); await modal(savingTitle).getByRole("status").waitFor(); await page.evaluate(() => window.closeFixture.recordChanged({ phase: "error", errorMessage: "模擬録音保存失敗" })); await dismissFailed(savingTitle);
    check(await count("finish_main_close") === 0 && await count("stop_recording") === 0 && await page.evaluate(() => window.closeFixture.recording.phase === "error"), "save failure closed app or fabricated idle");
    results.push("exit during saving retains canonical error and never continues closing after dismissal");

    stage = "quiet-close"; await boot(); await request(); await page.waitForFunction(() => window.closeFixture.calls.some(c => c.command === "finish_main_close")); check(await count("finish_main_close") === 1 && await page.getByRole("dialog").count() === 0 && await page.evaluate(() => window.closeFixture.flushes > 0), "quiet close missed save or prompted unnecessarily");
    results.push("quiet closing flushes edits and uses checked native tokens without a destructive prompt");

    stage = "close-layout"; await boot(); await page.evaluate(() => { window.closeFixture.activity.modelPreparationBusy = true; }); await request(); await modal(title).waitFor(); await page.setViewportSize({ width: 360, height: 640 });
    const box = await modal(title).boundingBox(); check(box.x >= 23.9 && box.x + box.width <= 336.1 && await modal(title).evaluate(e => e.scrollWidth <= e.clientWidth + 1), "close modal clipped on small screen"); await page.screenshot({ path: "application-close-360.png" }); await page.keyboard.press("Escape"); await modal(title).waitFor({ state: "hidden" }); check(await count("finish_main_close") === 0, "layout inspection quit");
    results.push("model preparation closing explains checkpoint retention and fits a 360px viewport");
    return { passed: results.length, results };
  } catch (cause) { throw Error(`${stage}: ${String(cause)}; body=${await page.locator("body").innerText()}`); }
  finally { await page.goto("about:blank"); await context.unrouteAll(); await page.setViewportSize({ width: 1280, height: 720 }); }
}
