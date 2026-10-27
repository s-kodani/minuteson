// Actual production provider/cards under fake IPC; no native capture or audio.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(), body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const mic = () => page.locator(".toast-card").filter({ hasText: /録音中の履歴：.*マイク/u });
  const release = action => page.evaluate(action => { const f = window.closeFixture; f.queueHold[action] = false; const pending = f.queueReplies.filter(p => p.action === action); f.queueReplies = f.queueReplies.filter(p => p.action !== action); pending.forEach(p => p.resolve()); }, action);
  const calls = command => page.evaluate(command => window.closeFixture.calls.filter(c => c.command === command).length, command);
  const set = (count, both = false, loss = 0) => page.evaluate(([count, both, loss]) => {
    const f = window.closeFixture, ownerId = f.recording.sessionId, target = new URL(location.href).searchParams.get("window") === "recorder" ? "recorder" : "main";
    const event = (input, sequence) => {
      const interruptions = Math.ceil(sequence / 2), kind = sequence % 2 ? "interrupted" : "recovered", state = sequence % 2 ? "disconnected" : "connected";
      const status = { state, sourceEpoch: Math.floor(sequence / 2) + 1, interruptions, reason: sequence % 2 ? input === "microphone" ? "device_disconnected" : "stream_stopped" : null };
      return { id: `${ownerId}:${input}:${interruptions}:${kind}`, input, kind, interruptions, target, recipientId: null, claimedAt: null, delivery: { sequence, generation: 1, leaseAgeMs: null, receiveId: null, rendered: false, status } };
    };
    const events = input => Array.from({ length: count * 2 }, (_, i) => event(input, i + 1));
    f.queueState = { lanes: { microphone: events("microphone"), system: both ? events("system") : [] }, closed: {}, loss: { microphone: loss, system: both ? loss : 0 } };
    f.inputStatus = { schemaVersion: 4, ownerId, revision: 1, reportAgeMs: 0, microphone: { state: "connected", sourceEpoch: count + 1, interruptions: count, reason: null }, system: { state: "connected", sourceEpoch: both ? count + 1 : 1, interruptions: both ? count : 0, reason: null }, notices: [], noticeLoss: f.queueState.loss };
    f.refreshInputStatus = true;
  }, [count, both, loss]);
  const reload = async () => {
    await page.evaluate(() => window.closeFixture.persistInputQueueFixture()); await page.reload(); await page.waitForFunction(() => !!window.closeFixture);
    if (!new URL(page.url()).searchParams.has("window")) await page.getByRole("button", { name: "録音画面へ", exact: true }).click();
  };
  try {
    await page.goto("http://127.0.0.1:1420/"); await page.waitForFunction(() => !!window.closeFixture);
    await page.getByRole("button", { name: "新しい録音", exact: true }).click(); await page.getByRole("button", { name: "録音を開始", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "選択した会議の録音を開始しますか？", exact: true }); await dialog.getByRole("button", { name: "録音を開始する", exact: true }).click(); await dialog.waitFor({ state: "hidden" });
    await page.evaluate(() => { window.closeFixture.queueHold.receive = true; return window.closeFixture.recordChanged({ phase: "recording" }); }); await set(1);
    await page.waitForFunction(() => window.closeFixture.queueReplies.length === 1);
    await page.waitForFunction(() => document.querySelector('[data-input-kind="microphone"]')?.dataset.noticeReception === "unconfirmed");
    check(await mic().count() === 0 && await calls("render_recording_input_notice") === 0, "receive fabricated rendering");
    check(await page.locator('[data-input-kind="microphone"]').getAttribute("data-input-state") === "connected", "historical interruption became current disconnected status");
    results.push("held reception neither displays nor acknowledges a historical event; actual current input remains connected");

    await page.evaluate(() => { const d = document.createElement("dialog"); d.id = "render-cover"; d.textContent = "合成表示遮蔽";d.style.opacity="0"; document.body.append(d); d.showModal(); });
    await release("receive"); await page.waitForFunction(() => !!window.closeFixture.queueState.lanes.microphone[0].recipientId); await mic().waitFor({state:"attached"}); await page.waitForTimeout(400);
    check(await calls("render_recording_input_notice") === 0, "invisible modal card acknowledged rendering");
    await page.evaluate(() => { document.querySelector(".mon-notification-stack").style.opacity="0"; document.querySelector("#render-cover").close(); }); await page.waitForTimeout(400);
    check(await calls("render_recording_input_notice") === 0, "transparent ancestor acknowledged rendering");
    results.push("transparent ancestors prevent rendering confirmation despite an attached card");
    await page.evaluate(() => { const host=document.querySelector(".mon-notification-stack"); host.style.opacity="1"; host.style.transform="translateY(-2000px)"; }); await page.waitForTimeout(400);
    check(await calls("render_recording_input_notice") === 0, "off-viewport card acknowledged rendering");
    results.push("off-viewport cards do not acknowledge rendering");
    await page.evaluate(() => { document.querySelector(".mon-notification-stack").style.transform=""; document.querySelector("#render-cover").showModal(); });
    await page.evaluate(() => { window.closeFixture.showQueueBlockers(); document.querySelector("#render-cover").close(); });
    await page.waitForTimeout(500); check(await mic().count() === 0 && await calls("render_recording_input_notice") === 0, "queued card acknowledged before promotion");
    results.push("modal-covered and queued cards do not emit rendering confirmation");

    const old = await page.evaluate(() => window.closeFixture.calls.find(c => c.command === "receive_recording_input_notice").args);
    await page.evaluate(() => { const head = window.closeFixture.queueState.lanes.microphone[0]; head.claimedAt = Date.now() - 5100; }); await reload(); await mic().waitFor();
    await page.waitForFunction(() => window.closeFixture.queueState.lanes.microphone[0].delivery.rendered);
    check(await page.evaluate(old => { const h = window.closeFixture.queueState.lanes.microphone[0]; return h.delivery.generation === 2 && h.recipientId !== old.recipientId; }, old), "unshown event not handed off under new generation");
    check(await page.evaluate(async args => { try { await window.closeFixture.queuedAction("render_recording_input_notice", args); return false; } catch { return true; } }, old), "late old-generation render committed");
    results.push("unshown card survives WebView loss; generation-fenced handoff rejects the old recipient's late render");

    await mic().getByRole("button", { name: "注意通知を閉じる", exact: true }).click(); await mic().filter({ hasText: "復帰しました" }).waitFor();
    await page.waitForFunction(() => window.closeFixture.queueState.lanes.microphone[0].delivery.rendered);
    check(await page.evaluate(() => Object.values(window.closeFixture.queueState.closed)[0].delivery.sequence) === 1, "recovery bypassed interruption");
    results.push("only confirmed interruption closure advances the lane to its retained recovery event");
    await mic().getByRole("button", { name: "情報通知を閉じる", exact: true }).click(); await mic().waitFor({ state: "hidden" });
    await reload(); await page.waitForTimeout(600); check(await mic().count() === 0, "closed event replayed after reload");
    results.push("confirmed closed events do not replay in a replacement WebView");

    // A new synthetic recording owns new event identities and source counters.
    await page.evaluate(() => { const f = window.closeFixture; f.inputStatus = null; return f.recordChanged({ sessionId: crypto.randomUUID() }); }); await set(1);
    await page.evaluate(() => { window.closeFixture.queueLost.render = true; }); await mic().waitFor();
    await page.waitForFunction(() => window.closeFixture.queueState.lanes.microphone[0].delivery.rendered);
    await page.waitForFunction(() => document.querySelector('[data-input-kind="microphone"]')?.dataset.noticeReception === "received");
    await mic().getByRole("button", { name: "注意通知を閉じる", exact: true }).click(); await mic().filter({ hasText: "復帰しました" }).waitFor();
    results.push("authoritative readback of committed rendering permits closure after a lost render reply");

    await page.goto("http://127.0.0.1:1420/?window=recorder"); await page.waitForFunction(() => !!window.closeFixture); await page.setViewportSize({ width: 360, height: 116 });
    await page.evaluate(() => window.closeFixture.recordChanged({ phase: "recording", sessionId: crypto.randomUUID(), meetingId: crypto.randomUUID() })); await set(1, true, 1);
    await page.waitForFunction(()=>document.querySelector('[data-input-kind="microphone"]')?.dataset.inputState==="connected");await page.waitForTimeout(650);
    check(await calls("receive_recording_input_notice")===0&&await calls("render_recording_input_notice")===0&&await calls("close_recording_input_notice")===0,"mini consumed its unshown queue");
    await page.screenshot({ path: "input-queue-mini-360x116.png" });
    const bounds=await page.locator(".mini-shell").evaluate(e=>({height:e.clientHeight,scrollHeight:e.scrollHeight,width:e.clientWidth,scrollWidth:e.scrollWidth,rows:[...e.children].map(child=>({class:child.className,height:child.getBoundingClientRect().height,top:child.getBoundingClientRect().top,width:child.getBoundingClientRect().width}))}));
    check(bounds.scrollHeight <= bounds.height + 1 && bounds.scrollWidth <= bounds.width + 1, `queued mini notice overflowed 360x116: ${JSON.stringify(bounds)}`);
    check(await page.locator('[data-input-kind="microphone"]').getAttribute("data-notice-history-lost") === "true", "loss metadata disappeared");
    check(await page.locator('[data-input-kind="microphone"]').isVisible() && (await page.locator('[data-input-kind="microphone"]').getAttribute("title")).includes("通知履歴の一部"), "loss status icon lost its explanation");
    results.push("independent input telemetry and retained loss indication fit the native mini geometry");
    check(await page.evaluate(()=>["microphone","system"].every(input=>window.closeFixture.queueState.lanes[input].length===2&&window.closeFixture.queueState.lanes[input].every(n=>!n.delivery.rendered&&!n.delivery.receiveId))),"mini changed pending interruption or recovery evidence");
    results.push("mini leaves both pending lanes and their render evidence untouched for main presentation");
    const reads = await calls("get_recording_input_status"); await page.evaluate(() => { window.closeFixture.failInputStatus = true; }); await page.waitForFunction(() => document.querySelector('[data-input-kind="microphone"]')?.dataset.inputState === "unknown");
    check(await page.locator('[data-input-kind="microphone"]').getAttribute("data-notice-history-lost") === "true", "read failure cleared retained history loss");
    check(await calls("get_recording_input_status") > reads, "read failure stopped polling");
    await page.evaluate(() => window.closeFixture.recordChanged({ phase: "stopping" })); await page.locator(".mini-input-notice").waitFor({ state: "hidden" });
    check(await page.evaluate(() => window.closeFixture.calls.every(c => !["start_capture_test", "start_recording", "stop_recording"].includes(c.command))), "mini check requested capture");
    results.push("failed reads retain loss state without current connection proof; stop removes delivery and no capture is requested");
    return { passed: results.length, results };
  } catch (cause) { throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`); }
  finally { await page.context().unrouteAll(); await page.evaluate(() => sessionStorage.removeItem("MON_INPUT_QUEUE_FIXTURE")).catch(() => {}); }
}
