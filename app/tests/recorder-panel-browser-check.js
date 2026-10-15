// Production mini component with synthetic IPC. No native windows or capture.
async (page, capturePath = path => `output/playwright/tap-panel-20261012/${path}`) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  let readFailure = false, heldMain = false, releaseMain;
  let snapshot = { revision: 1, phase: "idle", elapsedMs: 45_000, microphoneConnected: true, systemAudioConnected: true, sessionId: null, meetingId: null, errorMessage: null };
  const decision = { revision: 0, id: null, phase: "idle", sessionId: null, meetingId: null, meetingTitle: null, origin: null, preparing: false, quitAfterSave: false, errorMessage: null };
  const calls = [];
  await page.context().route("http://127.0.0.1:1420/__synthetic_recording_controller", async route => {
    const { command, label } = route.request().postDataJSON();
    let value, error;
    if (command === "get_recording_snapshot") { if (readFailure) error = "合成状態読取失敗"; else value = snapshot; }
    else if (command === "get_recording_confirmation") value = decision;
    else if (["hide_recorder_panel", "open_main_window"].includes(command)) {
      check(label === "recorder", "wrong mini command role"); calls.push(command);
      if (command === "open_main_window" && heldMain) await new Promise(resolve => { releaseMain = resolve; });
      value = null;
    } else error = `unexpected synthetic command: ${command}`;
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(error ? { error } : { value }) });
  });
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/recording-confirmation-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  await page.setViewportSize({ width: 360, height: 116 });
  const boot = async () => {
    await page.goto("http://127.0.0.1:1420/?window=recorder");
    await page.getByRole("button", { name: "ミニパネルを閉じる", exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector(".mini-top strong")?.textContent !== "接続中");
  };
  const close = () => page.getByRole("button", { name: "ミニパネルを閉じる", exact: true });
  const checkClose = async () => {
    const before = JSON.stringify({ snapshot, decision }), count = calls.length;
    check(!await close().isDisabled(), "mini close was disabled");
    await close().click();
    await page.waitForFunction(() => !document.querySelector('[aria-label="ミニパネルを閉じる"]')?.disabled);
    check(calls.length === count + 1 && calls.at(-1) === "hide_recorder_panel", "close did not hide exactly once");
    check(JSON.stringify({ snapshot, decision }) === before, "close modified recording or stop decision");
  };
  const phases = ["idle", "preparing", "recording", "stopping", "saving", "error"];
  for (const phase of phases) {
    snapshot = { ...snapshot, revision: snapshot.revision + 1, phase, sessionId: phase === "idle" ? null : "synthetic-session", meetingId: phase === "idle" ? null : "synthetic-meeting", errorMessage: phase === "error" ? "tap does not supply Float32 PCM / no system audio samples were received" : null };
    await boot();
    check(await page.getByLabel("録音の書込み",{exact:true}).count()===0 && !(await page.locator('.mini-shell').innerText()).includes('書込みの確認が必要'), `${phase} mini retained write-diagnostic text`);
    const bounds = await page.locator(".mini-shell").evaluate(e => ({ overflow: e.scrollWidth > e.clientWidth || e.scrollHeight > e.clientHeight }));
    check(!bounds.overflow, `${phase} mini overflowed native geometry`);
    const buttons = await page.locator(".mini-bottom button").evaluateAll(list => list.map(e => { const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; }));
    check(buttons.every(r => r.x >= 0 && r.x + r.width <= 360 && r.y >= 0 && r.y + r.height <= 116 && Math.abs(r.y - buttons[0].y) < 1 && r.height === 36), `${phase} controls clipped or misaligned`);
    const layout = await page.locator(".mini-shell").evaluate(shell => {
      const rect = selector => { const r=shell.querySelector(selector).getBoundingClientRect(); return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,center:(r.top+r.bottom)/2}; };
      const grip=shell.querySelector(".mini-drag");
      return { row:rect(".mini-row"), summary:rect(".mini-summary"), grip:rect(".mini-drag"), main:rect('[aria-label="メインを開く"]'), info:rect(".info-trigger"), close:rect(".mini-close"), dragFont:getComputedStyle(grip).fontSize, dragRegion:grip.hasAttribute("data-tauri-drag-region") };
    });
    check(Math.abs(layout.row.center - 58)<1 && [layout.summary,layout.grip,layout.main,layout.info].every(r=>Math.abs(r.center-layout.row.center)<1), `${phase} content is not vertically centered`);
    check(layout.close.top===11 && layout.close.right===345 && layout.close.top<layout.info.top, `${phase} close is not independently top-right`);
    check(layout.info.right+8<=layout.close.left && layout.dragFont==="24px" && layout.grip.right-layout.grip.left===18 && layout.dragRegion, `${phase} larger grip or close clearance invalid`);
    if (["idle", "recording", "error"].includes(phase)) await page.screenshot({ path: capturePath(`mini-${phase}.png`) });
    await checkClose();
  }
  readFailure = true; await boot(); await checkClose();
  readFailure = false; heldMain = true; await boot();
  await page.getByRole("button", { name: "メインを開く", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="メインを開く"]')?.disabled === true);
  await checkClose();
  heldMain = false; releaseMain();
  await page.waitForFunction(() => !document.querySelector('[aria-label="メインを開く"]')?.disabled);
  snapshot = { ...snapshot, revision: snapshot.revision + 1, phase: "recording", errorMessage: null };
  Object.assign(decision, { revision: 1, id: "synthetic-decision", phase: "stopping", sessionId: snapshot.sessionId, meetingId: snapshot.meetingId, meetingTitle: "合成会議", origin: "recorder" });
  await boot();
  check(await page.getByRole("button", { name: "停止", exact: true }).isDisabled(), "pending stop allowed another stop");
  await checkClose();
  check(calls.filter(c => c === "hide_recorder_panel").length === 9, "missing phase/read-failure/busy/pending-stop hide");
  return { passed: 9, phases, cases: ["unreadable session", "independent close while main open pending", "pending stop remains guarded while close stays enabled"], nativeRecording: false, recordingAndDecisionUnchanged: true };
}
