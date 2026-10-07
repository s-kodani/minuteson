// Production panels, browser-fixture.js IPC only. No native files or models.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const asr = () => page.getByRole("region", { name: "解析操作", exact: true });
  const diar = () => page.getByRole("region", { name: "話者分離操作", exact: true });
  const calls = command => page.evaluate(c => window.monBrowserFixture.calls.filter(x => x.command === c), command);
  const boot = async () => {
    await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(() => {
      const f = window.monBrowserFixture;
      f.jobs.push({ ...f.jobs[0], id: "diar-paused", kind: "diarization" }, { ...f.jobs[1], id: "diar-failed", kind: "diarization" });
      for (const key of ["global", "meeting-0"]) f.vocabulary[key] = { id: key, lockVersion: 0, payload: [{ id: `term-${key}`, term: "点検日", reading: "てんけんび", note: null, enabled: true }] };
    });
    await page.getByRole("button", { name: "開く", exact: true }).first().click();
    await asr().getByRole("button", { name: "同じ条件で再試行", exact: true }).waitFor();
  };
  const dismiss = async title => { await page.keyboard.press("Escape"); await modal(title).waitFor({ state: "hidden" }); };
  for (const [kind, panel, trigger, title, command, jobId] of [
    ["ASR", asr, "同じ条件で再試行", "文字起こしの再試行を確認", "retry_transcription", "job-failed"],
    ["diarization", diar, "話者分離を同じ条件で再試行", "話者分離の再試行を確認", "retry_diarization", "diar-failed"],
  ]) {
    await boot(); const button = panel().getByRole("button", { name: trigger, exact: true });
    await button.click(); await modal(title).waitFor();
    check((await modal(title).innerText()).includes(jobId) && (await modal(title).innerText()).includes("現在版"), `${kind} target or preservation omitted`);
    check((await calls(command)).length === 0, `${kind} retry started before acceptance`);
    await dismiss(title); check((await calls(command)).length === 0, `${kind} cancellation retried`);
    await button.click(); await modal(title).getByRole("button", { name: "同じ条件で再試行する", exact: true }).click();
    await modal(title).waitFor({ state: "hidden" });
    const requests = await calls(command);
    check(requests.length === 1 && requests[0].args.request.jobId === jobId && requests[0].args.request.meetingId === "meeting-0" && requests[0].args.request.expectedSpecHash === "b".repeat(64) && !!requests[0].args.request.operationId, `${kind} fixed retry identity changed`);
    check(await page.evaluate(() => window.monBrowserFixture.currentId === "analysis" && window.monBrowserFixture.previousId === null), `${kind} retry published candidate`);
    results.push(`${kind}: cancelled choice is read-only; one explicit retry preserves job, spec, operation and current version`);
  }
  for (const [kind, panel, trigger, title, accept, jobId] of [
    ["ASR", asr, "取消", "文字起こし処理の取消を確認", "処理を取り消す", "job-existing"],
    ["diarization", diar, "話者分離を取消", "話者分離の取消を確認", "話者分離を取り消す", "diar-paused"],
  ]) {
    await boot(); const button = panel().getByRole("button", { name: trigger, exact: true });
    await button.click(); await modal(title).waitFor(); await dismiss(title);
    check((await calls("cancel_analysis_job")).length === 0, `${kind} dismiss cancelled job`);
    await button.click(); await modal(title).getByRole("button", { name: accept, exact: true }).click(); await modal(title).waitFor({ state: "hidden" });
    const requests = await calls("cancel_analysis_job");
    check(requests.length === 1 && requests[0].args.jobId === jobId, `${kind} cancel identity changed`);
    check(await page.evaluate(() => window.monBrowserFixture.currentId === "analysis"), `${kind} cancel deleted head`);
    results.push(`${kind}: cancellation requires explicit decision and retains recording/current version`);
  }
  await boot(); await page.evaluate(() => window.monBrowserFixture.lostRetryResponse = true);
  await asr().getByRole("button", { name: "同じ条件で再試行", exact: true }).click();
  const retryTitle = "文字起こしの再試行を確認";
  await modal(retryTitle).getByRole("button", { name: "同じ条件で再試行する", exact: true }).click();
  await modal(retryTitle).getByRole("alert").waitFor();
  check((await calls("retry_transcription")).length === 1 && await modal(retryTitle).getByRole("button", { name: "同じ条件で再試行する", exact: true }).isDisabled(), "lost retry replayed or claimed cancellation");
  check(await page.evaluate(() => window.monBrowserFixture.jobs.filter(j => j.retryOfJobId === "job-failed").length === 1), "lost retry discarded committed job");
  await dismiss(retryTitle); check((await calls("retry_transcription")).length === 1, "closing lost retry repeated action");
  results.push("lost retry acknowledgement retains one committed job and visible uncertainty, without replay");

  await boot(); await diar().getByText("sherpa-onnx 設定（端末内に保存）", { exact: true }).click();
  const threads = diar().getByRole("spinbutton", { name: "話者分離CPUスレッド数", exact: true });
  await threads.fill("6"); await diar().getByRole("button", { name: "話者分離設定を再読込", exact: true }).click();
  const settingsTitle = "話者分離設定の入力破棄を確認"; await modal(settingsTitle).waitFor();
  await page.waitForTimeout(900); check((await calls("save_diarization_preferences")).length === 0, "diarization autosave ran during discard choice");
  await dismiss(settingsTitle); check(await threads.inputValue() === "6", "cancel discarded diarization settings");
  await threads.fill("7"); await diar().getByRole("button", { name: "話者分離設定を再読込", exact: true }).click();
  await modal(settingsTitle).getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click(); await modal(settingsTitle).waitFor({ state: "hidden" });
  check(await threads.inputValue() === "4" && (await calls("save_diarization_preferences")).length === 0, "explicit reload saved discarded input");
  results.push("diarization discard pauses autosave; cancel preserves input, explicit reload uses saved settings");

  for (const scope of ["meeting", "global"]) {
    await boot();
    let panel;
    if (scope === "global") { await page.getByRole("button", { name: "設定", exact: true }).click(); panel = page.getByRole("region", { name: "共通用語", exact: true }); }
    else { panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "会議の用語", exact: true }) }); await panel.getByText("会議別用語（任意）", { exact: true }).click(); }
    const term = panel.getByRole("textbox", { name: "用語（必須）", exact: true });
    await term.fill("未保存の点検日"); await panel.getByRole("button", { name: "用語を再読込", exact: true }).click();
    const title = "用語入力の破棄を確認"; await modal(title).waitFor();
    await page.waitForTimeout(900); check((await calls("save_vocabulary_book")).length === 0, `${scope} autosave ran while awaiting discard choice`);
    await dismiss(title); check(await term.inputValue() === "未保存の点検日", `${scope} cancelled reload lost input`);
    await panel.getByRole("button", { name: "用語を再読込", exact: true }).click();
    await modal(title).getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click(); await modal(title).waitFor({ state: "hidden" });
    check(await term.inputValue() === "点検日" && (await calls("save_vocabulary_book")).length === 0, `${scope} reload saved discarded terms`);
    results.push(`${scope} vocabulary: pending discard suspends autosave; cancel keeps input; acceptance reads saved book`);

    const remove = panel.getByRole("button", { name: "用語を削除", exact: true });
    await remove.click(); const deleteTitle = "用語の削除を確認"; await modal(deleteTitle).waitFor();
    check((await modal(deleteTitle).innerText()).includes("点検日") && (await modal(deleteTitle).innerText()).includes("固定コピー"), "deletion loss scope omitted");
    await dismiss(deleteTitle); check((await calls("save_vocabulary_book")).length === 0 && await term.inputValue() === "点検日", `${scope} cancel deleted term`);
    await page.evaluate(() => window.monBrowserFixture.persistenceDelayMs = 500);
    await remove.click(); await modal(deleteTitle).getByRole("button", { name: "用語を削除する", exact: true }).click();
    check(await modal(deleteTitle).getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "term deletion closed before actual save acknowledgement");
    await modal(deleteTitle).waitFor({ state: "hidden" });
    const writes = await calls("save_vocabulary_book"); const key = scope === "global" ? "global" : "meeting-0";
    check(writes.length === 1 && writes[0].args.meetingId === (scope === "global" ? null : "meeting-0") && await page.evaluate(k => window.monBrowserFixture.vocabulary[k].payload.length === 0, key), `${scope} delete wrong book or repeated save`);
    results.push(`${scope} vocabulary: cancelled deletion is read-only; one exact-book deletion waits for acknowledgement`);
  }
  return { passed: results.length, results };
}
