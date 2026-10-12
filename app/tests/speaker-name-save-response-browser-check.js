// Production name modal and recovery controller; synthetic IPC only.
async (page, capturePath = path => path) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const dialog = () => page.getByRole("dialog", { name: "話者名を変更", exact: true });
  const field = () => dialog().getByRole("textbox", { name: "表示名", exact: true });
  const save = () => dialog().getByRole("button", { name: "保存", exact: true });
  const recheck = () => dialog().getByRole("button", { name: "本文の保存結果を確認", exact: true });
  const writes = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft").length);
  const currentName = () => page.evaluate(() => window.monBrowserFixture.draft.payload.speaker_names[0]?.display_name);
  const boot = async () => {
    await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    await page.locator(".meeting-name").first().click();
    await page.getByRole("button", { name: "田中の名前を変更", exact: true }).click();
    await dialog().waitFor();
    check(await dialog().locator(".speaker-name-recovery-actions").count() === 0, "normal name modal exposes recovery controls or empty spacing");
  };
  const failedName = async (lost = true) => {
    await boot();
    await field().fill("先行する確定名");
    await field().evaluate(e => { window.nameResponseField = e; });
    await page.evaluate(lost => { window.monBrowserFixture[lost ? "lostTranscriptSaveResponse" : "failSave"] = true; }, lost);
    await save().click();
    await dialog().getByRole("alert").waitFor();
    await field().fill("保持する最新名");
    // Cancel may legitimately succeed after a read proves no body write. A
    // committed-but-unacknowledged save must remain protected instead.
    if (lost) await page.keyboard.press("Escape");
    check(await dialog().isVisible() && await field().inputValue() === "保持する最新名", "unknown name save was dismissed or newer input was lost");
    check(await writes() === 1, "cancel replayed name publication");
    check(await recheck().count() === 1, "name modal traps an unknown body result: recovery is only in the inert background");
  };

  await failedName();
  await recheck().click();
  check(await writes() === 1 && await currentName() === "先行する確定名" && await field().inputValue() === "保持する最新名", "read-back published new name or replaced retained input");
  check(await field().evaluate(e => e === window.nameResponseField), "read-back remounted name field");
  await save().click(); await dialog().waitFor({ state: "hidden" });
  check(await writes() === 2 && await currentName() === "保持する最新名", "new name could not be explicitly saved after lost response");
  results.push("lost committed name response is resolved inside modal, preserves newer input and saves it only on explicit save");

  await failedName(false);
  await recheck().click();
  check(await writes() === 1 && await currentName() === undefined && await field().inputValue() === "保持する最新名", "unchanged read-back published or erased input");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; });
  await save().click(); await dialog().waitFor({ state: "hidden" });
  check(await writes() === 2 && await currentName() === "保持する最新名", "unchanged read-back could not save explicit newer name");
  results.push("unchanged body result clears uncertainty without writing and requires a fresh explicit save");

  await failedName();
  await page.evaluate(() => { window.nameResponseDraft = structuredClone(window.monBrowserFixture.draft); });
  for (const mode of ["read-failure", "missing", "wrong-id", "wrong-meeting", "wrong-base", "newer-version", "different-payload"]) {
    await page.evaluate(mode => {
      const f = window.monBrowserFixture;
      f.draft = structuredClone(window.nameResponseDraft);
      if (mode === "read-failure") f.failDraftRead = true;
      if (mode === "missing") f.draft = null;
      if (mode === "wrong-id") f.draft.id = crypto.randomUUID();
      if (mode === "wrong-meeting") f.draft.meetingId = "other-meeting";
      if (mode === "wrong-base") f.draft.baseAnalysisId = "other-analysis";
      if (mode === "newer-version") f.draft.lockVersion++;
      if (mode === "different-payload") f.draft.payload.speaker_names[0].display_name = "別操作の確定名";
    }, mode);
    await recheck().click(); await dialog().getByRole("alert").waitFor();
    check(await writes() === 1 && await recheck().isVisible() && await field().inputValue() === "保持する最新名", `${mode} lost pending result or wrote name`);
    check(await field().evaluate(e => e === window.nameResponseField), `${mode} replaced field`);
    results.push(`${mode} refuses reconciliation without writing or discarding newer name`);
    await page.evaluate(() => { window.monBrowserFixture.failDraftRead = false; window.monBrowserFixture.draft = structuredClone(window.nameResponseDraft); });
  }
  await recheck().click(); await save().click(); await dialog().waitFor({ state: "hidden" });
  check(await currentName() === "保持する最新名" && await writes() === 2, "recovered read did not permit explicit save");

  await failedName(); await page.setViewportSize({ width: 360, height: 640 });
  check(await dialog().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "name recovery modal overflows narrow screen");
  const sizes = await dialog().locator(".speaker-name-recovery-actions button").evaluateAll(es => es.map(e => ({ height: e.getBoundingClientRect().height, font: getComputedStyle(e).fontSize, top: e.getBoundingClientRect().top })));
  check(sizes.length === 2 && sizes.every(s => Math.abs(s.height - sizes[0].height) <= 1 && Math.abs(s.top - sizes[0].top) <= 1 && s.font === sizes[0].font), "adjacent recovery button heights, alignment or fonts differ");
  await page.screenshot({ path: capturePath("name-response-narrow-error.png") });
  await dialog().getByRole("button", { name: "保存を再試行", exact: true }).click();
  check(await writes() === 1 && await field().inputValue() === "保持する最新名", "retry republished acknowledged name or confirmed raw input");
  await page.screenshot({ path: capturePath("name-response-narrow-recovered.png") });
  await save().click(); await dialog().waitFor({ state: "hidden" });
  check(await writes() === 2 && await currentName() === "保持する最新名", "narrow modal could not save latest name");
  results.push("explicit retry inside narrow modal resolves old request without duplicate body write or confirming raw input");

  await page.setViewportSize({ width: 1100, height: 800 });
  for (const lost of [false, true]) {
    await boot();
    await page.evaluate(lost => { window.monBrowserFixture.recovery[lost ? "lostSave" : "failSave"] = true; }, lost);
    await field().fill("回復待ちの確定名"); await save().click();
    await dialog().getByRole("alert").waitFor();
    await field().fill("回復後も保持する最新名");
    const read = dialog().getByRole("button", { name: "回復保存の結果を確認", exact: true });
    const retry = dialog().getByRole("button", { name: "未確定入力の保存を再試行", exact: true });
    check(await read.isVisible() && await writes() === 0, "journal failure hides recovery action or writes body");
    const attempts = await page.evaluate(() => window.monBrowserFixture.recovery.calls.filter(c => c.command === "save_transcript_edit_recovery").length);
    await read.click();
    // Exceed the 300ms recovery debounce: checking a result is not consent to
    // automatically send newer input, even after the read acknowledgement.
    await page.waitForTimeout(650);
    check(await writes() === 0 && await field().inputValue() === "回復後も保持する最新名", "journal read-back confirms raw name or discards input");
    check(await page.evaluate(() => window.monBrowserFixture.recovery.calls.filter(c => c.command === "save_transcript_edit_recovery").length) === attempts, "read-only journal check replays a write");
    await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = false; });
    if (await retry.count()) await retry.click();
    await dialog().getByRole("button", { name: "保存を再試行", exact: true }).click();
    await page.waitForFunction(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft").length === 1 && !document.querySelector('.speaker-name-dialog input')?.disabled);
    check(await writes() === 1 && await currentName() === "回復待ちの確定名" && await field().inputValue() === "回復後も保持する最新名", "journal recovery publishes newer raw name or loses frozen confirmed name");
    await save().click(); await dialog().waitFor({ state: "hidden" });
    check(await writes() === 2 && await currentName() === "回復後も保持する最新名", "explicit save cannot finish after journal recovery");
    results.push(`${lost ? "lost committed" : "failed unchanged"} recovery journal result can be checked and retried inside modal without confirming newer input`);
  }
  return { passed: results.length, results, nativeDatabase: false };
}
