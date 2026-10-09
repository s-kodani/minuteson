async (page, capturePath = path => path) => {
  const context = page.context(), results = [], calls = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const entries = ["window_action", "confirmation", "theme_read", "info_read", "session_read"].map(source => ({ source, revision: 0, eventId: null, message: null, dismissed: false }));
  const snapshot = () => ({ schemaVersion: 1, entries: structuredClone(entries) });
  const recording = { revision: 1, phase: "recording", elapsedMs: 45000, microphoneConnected: true, systemAudioConnected: true, meetingId: "synthetic", sessionId: "00000000-0000-4000-8000-000000000001", errorMessage: null };
  const original = JSON.stringify(recording);
  let failOpen = true, failure = "合成ウィンドウ表示失敗：" + "長い原因説明を保持します。".repeat(30), lostPublish = false, failSession = false, failTheme = false;
  await context.route("http://127.0.0.1:1420/__synthetic_recorder_info", async route => {
    const { command, args, label } = route.request().postDataJSON(); calls.push({ command, args, label });
    let value, error;
    if (command === "get_recorder_operation_notices") value = snapshot();
    else if (command === "publish_recorder_operation_notice") {
      const e = entries.find(e => e.source === args.source);
      if (label !== "recorder" || !e || e.revision !== args.expectedRevision) error = "合成CAS競合";
      else { if (e.message !== args.message) { e.revision++; e.message = args.message; e.eventId = args.message ? `${e.source}:${e.revision}` : null; e.dismissed = false; } value = snapshot(); if (lostPublish) { lostPublish = false; error = "合成通知応答喪失"; } }
    } else if (command === "dismiss_recorder_operation_notice") {
      const e = entries.find(e => e.source === args.source);
      if (label !== "main" || !e || e.revision !== args.expectedRevision || e.eventId !== args.eventId || !e.message) error = "合成の古いcloseを拒否";
      else { if (!e.dismissed) { e.revision++; e.dismissed = true; } value = snapshot(); }
    } else if (command === "get_recording_snapshot") { if (failSession && label === "recorder") error = "合成録音状態の接続失敗"; else value = recording; }
    else if (command === "get_recording_confirmation") value = { revision: 0, id: null, phase: "idle", sessionId: null, meetingId: null, meetingTitle: null, origin: null, preparing: false, quitAfterSave: false, errorMessage: null };
    else if (command === "get_ui_preferences") { if (failTheme && label === "recorder") error = "合成表示設定の読込失敗"; else value = { id: "ui_preferences", lockVersion: 1, payload: { schemaVersion: 2, theme: "dark", textSize: 20, followPlayback: false } }; }
    else if (command === "get_recorder_info") value = false;
    else if (command === "open_main_window") { if (failOpen) error = failure; else value = null; }
    else if (command === "show_recorder_info") value = true;
    else if (command === "close_recorder_info") value = false;
    else error = `Unexpected operation fixture command: ${command}`;
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(error ? { error } : { value }) });
  });
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(); const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/recorder-info-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const main = await context.newPage(), info = await context.newPage();
  const toast = () => main.locator(".toast-card").filter({ hasText: "録音パネルのウィンドウ操作で要対応" });
  const open = () => page.getByRole("button", { name: "メインを開く", exact: true });
  const marker = () => page.getByRole("button", { name: "録音パネルの説明を開く", exact: true });
  const waitMarker = () => page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.dataset.helpError === "true");
  try {
    await page.setViewportSize({ width: 360, height: 116 }); await page.goto("http://127.0.0.1:1420/?window=recorder");
    await open().waitFor(); await page.getByText("00:00:45", { exact: true }).waitFor();
    await main.goto("http://127.0.0.1:1420/");
    await info.setViewportSize({ width: 360, height: 340 }); await info.goto("http://127.0.0.1:1420/?window=recorder-info");
    await open().click(); await toast().waitFor(); await waitMarker();
    check(await page.getByText("要対応", { exact: true }).isVisible(), "attention lacks visible short text");
    await info.getByText(failure, { exact: false }).waitFor();
    check(await page.locator(".mini-shell").evaluate(e => e.scrollHeight <= e.clientHeight + 1 && e.scrollWidth <= e.clientWidth + 1), "long error clipped mini controls");
    check(await page.locator('[data-input-kind="microphone"]').evaluate(e => e.clientWidth >= 75) && await page.locator('[data-input-kind="system"]').evaluate(e => e.clientWidth >= 65), "help/operation marker squeezed input labels");
    check(await page.getByRole("button", { name: "停止", exact: true }).isEnabled(), "diagnostic disabled actual stop");
    check(calls.filter(c => c.command === "open_main_window").length === 1, "window action automatically replayed");
    check(entries[0].message.includes(failure), "shared cause lost detail");
    await page.screenshot({ path: capturePath("operation-mini-error-360x116.png") });
    results.push("long window error routes to one main toast and scrollable info; compact marker does not clip stop");

    const first = structuredClone(entries[0]);
    await toast().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click(); await toast().waitFor({ state: "hidden" });
    await info.getByText("通知は閉じました。原因は確認待ちです。", { exact: true }).waitFor();
    check(entries[0].dismissed && entries[0].message === first.message, "dismissal resolved cause");
    await waitMarker(); await page.reload(); await waitMarker();
    check(entries[0].message === first.message && entries[0].dismissed, "blank remounted producer resolved retained cause");
    check(await toast().count() === 0, "dismissed error returned after reader/remount");
    results.push("dismissal retains cause/event identity, and blank reloaded mini cannot fabricate resolution");

    failOpen = false; await open().click();
    await page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.dataset.helpError === "false");
    check(entries[0].message === null && calls.filter(c => c.command === "open_main_window").length === 2, "explicit successful retry did not resolve exact operation");
    results.push("only an explicit successful original action resolves window-operation attention");

    failOpen = true; lostPublish = true; const beforePublishes = calls.filter(c => c.command === "publish_recorder_operation_notice" && c.args.source === "window_action").length;
    await open().click(); await toast().waitFor(); await waitMarker();
    check(calls.filter(c => c.command === "publish_recorder_operation_notice" && c.args.source === "window_action").length === beforePublishes + 1, "lost metadata response replayed publish");
    check(calls.filter(c => c.command === "open_main_window").length === 3, "lost metadata response replayed original action");
    results.push("lost publication response is authoritatively read back without replaying original action or metadata");

    const stale = structuredClone(entries[0]); failure = "新しい合成表示失敗"; await open().click();
    await info.getByText(failure, { exact: false }).waitFor();
    const current = structuredClone(entries[0]);
    const oldClose = await main.evaluate(async a => { try { await window.__TAURI_INTERNALS__.invoke("dismiss_recorder_operation_notice", a); return "accepted"; } catch (cause) { return String(cause); } }, { source: stale.source, expectedRevision: stale.revision, eventId: stale.eventId });
    const oldClear = await page.evaluate(async a => { try { await window.__TAURI_INTERNALS__.invoke("publish_recorder_operation_notice", a); return "accepted"; } catch (cause) { return String(cause); } }, { source: stale.source, expectedRevision: stale.revision, message: null });
    check(oldClose !== "accepted" && oldClear !== "accepted" && JSON.stringify(entries[0]) === JSON.stringify(current), "stale close/clear destroyed new error");
    results.push("stale toast close and stale producer resolution cannot dismiss or clear newer cause");

    failSession = true; await page.evaluate(() => window.dispatchEvent(new Event("focus"))); await info.getByText("合成録音状態の接続失敗", { exact: false }).waitFor();
    check(entries[4].message && entries[0].message, "session error replaced independent window cause");
    failSession = false; await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await info.getByText("合成録音状態の接続失敗", { exact: false }).waitFor({ state: "hidden" });
    check(entries[4].message === null && entries[0].message, "session reconnection cleared another source");
    results.push("session transport failure and authoritative reconnection affect only the session source");

    failTheme = true; await page.evaluate(() => window.dispatchEvent(new Event("focus"))); await info.getByText("合成表示設定の読込失敗", { exact: false }).waitFor();
    failTheme = false; await page.evaluate(() => window.dispatchEvent(new Event("focus"))); await info.getByText("合成表示設定の読込失敗", { exact: false }).waitFor({ state: "hidden" });
    check(entries[2].message === null && entries[0].message, "theme successful read cleared unrelated operation");
    check(JSON.stringify(recording) === original && !calls.some(c => /^(start_recording|stop_recording|decide_recording_confirmation|save_|delete_)/u.test(c.command)), "notifications mutated recording or data");
    check(!calls.some(c => c.command === "show_recorder_info"), "periodic updates automatically opened/focused help");
    results.push("theme read recovery preserves unrelated attention; notifications never mutate or auto-open recording/help");
    return results;
  } finally { await main.close(); await info.close(); }
}
