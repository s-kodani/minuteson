async (page, capturePath = path => `output/playwright/tap-panel-20261012/${path}`) => {
  const context = page.context(), results = [], calls = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const recording = { revision: 1, phase: "recording", elapsedMs: 45000, microphoneConnected: true, systemAudioConnected: true, meetingId: "synthetic", sessionId: "00000000-0000-4000-8000-000000000001", errorMessage: null };
  const before = JSON.stringify(recording);
  let visible = false, lostShow = false, failClose = false, badRead = false;
  const operations = ["window_action", "confirmation", "theme_read", "info_read", "session_read"].map(source => ({ source, revision: 0, eventId: null, message: null, dismissed: false }));
  await context.route("http://127.0.0.1:1420/__synthetic_recorder_info", async route => {
    const { command, args, label } = route.request().postDataJSON(); calls.push({ command, args, label });
    let value, error;
    if (command === "get_recorder_operation_notices") value = { schemaVersion: 1, entries: operations };
    else if (command === "publish_recorder_operation_notice") { const e = operations.find(e => e.source === args.source); check(label === "recorder" && e?.revision === args.expectedRevision, "bad operation relay"); if (e.message !== args.message) { e.revision++; e.message = args.message; e.eventId = args.message ? `${e.source}:${e.revision}` : null; e.dismissed = false; } value = { schemaVersion: 1, entries: operations }; }
    else if (command === "get_recording_snapshot") value = recording;
    else if (command === "get_recording_confirmation") value = { revision: 0, id: null, phase: "idle", sessionId: null, meetingId: null, meetingTitle: null, origin: null, preparing: false, quitAfterSave: false, errorMessage: null };
    else if (command === "get_ui_preferences") value = { id: "ui_preferences", lockVersion: 1, payload: { schemaVersion: 2, theme: "dark", textSize: 20, followPlayback: false } };
    else if (command === "get_recorder_info") value = badRead ? { forged: true } : visible;
    else if (command === "show_recorder_info") { check(label === "recorder", "wrong show role"); visible = true; value = true; if (lostShow) { lostShow = false; error = "合成表示応答喪失"; } }
    else if (command === "close_recorder_info") { check(["recorder", "recorder-info"].includes(label), "wrong close role"); if (failClose) error = "合成close失敗"; else { visible = false; value = false; } }
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(error ? { error } : { value }) });
  });
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(); const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/recorder-info-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const popup = await context.newPage();
  try {
    await page.setViewportSize({ width: 360, height: 116 });
    await page.goto("http://127.0.0.1:1420/?window=recorder");
    const trigger = page.getByRole("button", { name: "録音パネルの説明を開く", exact: true });
    await trigger.waitFor(); await page.getByText("00:00:45", { exact: true }).waitFor();
    await page.waitForFunction(() => document.documentElement.dataset.theme === "dark");
    check(calls.filter(c => c.command === "show_recorder_info").length === 0, "initial render showed help");
    check(await trigger.getAttribute("aria-expanded") === "false", "initial expanded state");
    await trigger.focus(); await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.getAttribute("aria-expanded") === "true");
    check(await page.getByRole("dialog").count() === 0, "help clipped into mini WebView");
    check(await page.getByRole("button", { name: "停止", exact: true }).isEnabled(), "read-only help disabled stop");
    for (const name of ["録音パネルの説明を開く", "停止", "メインを開く"]) {
      const rect = await page.getByRole("button", { name, exact: true }).boundingBox();
      check(rect && rect.x >= 0 && rect.y >= 0 && rect.x + rect.width <= 360 && rect.y + rect.height <= 116, `${name} clipped`);
    }
    results.push("actual recorder exposes keyboard info; no embedded dialog, capture mutation or clipped controls");

    await popup.setViewportSize({ width: 360, height: 340 });
    await popup.goto("http://127.0.0.1:1420/?window=recorder-info");
    const close = popup.getByRole("button", { name: "録音パネルの説明を閉じる", exact: true });
    await close.waitFor(); await popup.waitForFunction(() => document.documentElement.dataset.theme === "dark" && getComputedStyle(document.documentElement).getPropertyValue("--reading-font-size").trim() === "20px");
    await popup.evaluate(() => window.dispatchEvent(new Event("focus")));
    check(await close.evaluate(element => element === document.activeElement), "initial focus not close");
    check(await popup.getByRole("dialog").getAttribute("aria-modal") === "false", "help became confirmation");
    check((await popup.getByRole("dialog").innerText()).includes("文字起こし・話者分離は録音終了後"), "missing analysis explanation");
    check(await popup.getByRole("dialog").evaluate(e => document.documentElement.scrollWidth <= innerWidth && e.scrollWidth <= e.clientWidth + 1 && e.scrollHeight > e.clientHeight && getComputedStyle(e).overflowY === "auto"), "large text not vertically scrollable or horizontal clipping");
    await popup.screenshot({ path: capturePath("recorder-info-dark-20.png") });
    await popup.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.getAttribute("aria-expanded") === "false");
    check(calls.filter(c => c.command === "close_recorder_info").length === 1, "Escape not single close");
    results.push("separate nonmodal help follows dark/20px settings, close focus, scroll and Escape");

    lostShow = true; await trigger.click();
    await page.getByRole("alert").filter({ hasText: "合成表示応答喪失" }).waitFor();
    await page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.getAttribute("aria-expanded") === "true");
    check(calls.filter(c => c.command === "show_recorder_info").length === 2, "poll repeated show after lost response");
    await trigger.click(); await page.waitForFunction(() => document.querySelector(".recorder-info-trigger button")?.getAttribute("aria-expanded") === "false");
    results.push("lost show response is repaired by visibility read, not replayed window commands");

    failClose = true; await popup.evaluate(() => window.dispatchEvent(new Event("focus"))); await close.click();
    await popup.getByRole("alert").filter({ hasText: "合成close失敗" }).waitFor();
    check(await close.isEnabled(), "failed close stranded retry");
    failClose = false; await close.click();
    badRead = true;
    await page.getByRole("alert").filter({ hasText: "表示状態を確認できません" }).waitFor();
    check(await trigger.getAttribute("aria-expanded") === "false", "forged state fabricated visibility");
    check(await trigger.getAttribute("data-help-error") === "true", "error lost short visible marker");
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "error widened mini panel");
    check(JSON.stringify(recording) === before && !calls.some(c => /^(start_recording|stop_recording|decide_recording_confirmation|save_|delete_)/u.test(c.command)), "help mutated recording/data");
    results.push("close failure permits retry, forged reads stay errors and recording identity/state remain unchanged");
    return results;
  } finally { await popup.close(); }
}
