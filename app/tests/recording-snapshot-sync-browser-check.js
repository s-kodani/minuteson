async page => {
  const results = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const reads = () => page.evaluate(() => window.monBrowserFixture.calls.filter(
    call => call.command === "get_recording_snapshot").length);
  const snapshot = (revision, emit = true) => page.evaluate(async ({revision, emit}) => {
    const f = window.monBrowserFixture;
    f.recording = { revision, phase: "recording", elapsedMs: revision * 1000,
      microphoneConnected: false, systemAudioConnected: false, meetingId: "meeting-0",
      sessionId: "430d3e77-22f6-4b96-96fb-5bf3b4b1fd65", errorMessage: null };
    if (emit) await f.emitRecordingSnapshot();
  }, {revision, emit});
  // 起動と購読を完了させ、IPC 読み取りの時点を仮想時計で固定する。
  await page.getByRole("button", {name: "通知一覧", exact: true}).waitFor();
  await page.clock.pauseAt(new Date(await page.evaluate(() => Date.now()) + 100));
  check(await reads() > 0, "購読開始後の初回照会がない");
  let before = await reads();
  for (let revision = 1; revision <= 6; revision++) {
    await snapshot(revision);
    await page.clock.runFor(1000);
  }
  check(await reads() === before, "録音イベント受信中に定期照会が発生した");
  results.push("連続した録音イベントで定期照会を間引く");

  await snapshot(8);
  check(await reads() === before + 1, "revision の飛びで即時照会されなかった");
  results.push("revision が飛ぶと即時照会する");

  await snapshot(9, false);
  before = await reads();
  await page.clock.runFor(4999);
  check(await reads() === before, "5秒より早く定期照会した");
  await page.clock.runFor(1);
  check(await reads() === before + 1, "無通知が5秒続いても照会されなかった");
  await snapshot(10);
  check(await reads() === before + 1, "無通知時の照会が最新の revision を採用していない");
  results.push("無通知が5秒続くと最新の録音状態を取り直す");

  before = await reads();
  await page.evaluate(() => window.monBrowserFixture.emitWindowFocus());
  check(await reads() === before + 1, "Tauri のフォーカス復帰で照会されなかった");
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  check(await reads() === before + 2, "DOM のフォーカス復帰で照会されなかった");
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", {configurable: true, value: "hidden"});
    document.dispatchEvent(new Event("visibilitychange"));
  });
  check(await reads() === before + 2, "非表示になったときに照会が発生した");
  await page.evaluate(() => {
    delete document.visibilityState;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  check(await reads() === before + 3, "可視状態への復帰で照会されなかった");
  results.push("Tauri・DOM のフォーカスと可視状態への復帰で即時照会する");

  await page.evaluate(() => window.monBrowserFixture.emitRecordingSnapshot({revision: -1}));
  check(await reads() === before + 4, "壊れたイベントを受けても照会されなかった");
  results.push("壊れたイベントを受けると IPC で検証する");
  return {passed: results.length, results};
}
