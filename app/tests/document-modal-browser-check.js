// CLI-first integration check against browser-fixture.js and production panels.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const editor = () => page.getByRole("region", { name: "文字起こし編集", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const boot = async () => {
    await page.reload(); await page.getByRole("button", { name: "開く", exact: true }).first().click();
    await editor().getByRole("button", { name: "編集を開始", exact: true }).click();
    await page.waitForFunction(() => !!window.monBrowserFixture.draft);
  };
  const calls = command => page.evaluate(c => window.monBrowserFixture.calls.filter(x => x.command === c).length, command);
  const cancelled = async (name, trigger) => {
    await modal(name).waitFor({ state: "hidden" });
    await page.waitForFunction(() => !document.querySelector("dialog[open]"));
    check(await trigger.evaluate(e => document.activeElement === e), "cancel did not return to actual opener");
  };
  await boot();
  const add = editor().getByRole("button", { name: "話者を追加", exact: true });
  await add.click();
  check(await modal("新しい話者").getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "speaker safe focus missing");
  await page.keyboard.press("Enter"); await cancelled("新しい話者", add);
  check(await calls("save_transcript_draft") === 0, "initial Enter created speaker");
  await add.click(); await modal("新しい話者").getByRole("textbox", { name: "表示名" }).fill("取消する名前");
  await page.keyboard.press("Escape"); await cancelled("新しい話者", add);
  check(await calls("save_transcript_draft") === 0, "speaker cancellation saved draft");
  results.push("speaker initial Enter and Escape cancel without save, with opener focus");

  await add.click(); let box = await modal("新しい話者").boundingBox();
  await page.mouse.click(box.x - 10, box.y + 10); await cancelled("新しい話者", add);
  results.push("speaker backdrop cancellation uses existing no-save dismissal");

  await add.click(); await modal("新しい話者").getByRole("textbox", { name: "表示名" }).fill("新しい点検担当");
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 800; });
  await modal("新しい話者").getByRole("button", { name: "作成して保存", exact: true }).click();
  await page.keyboard.press("Escape"); check(await modal("新しい話者").isVisible(), "speaker closed during save");
  check(await modal("新しい話者").getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "speaker busy dismissal enabled");
  await cancelled("新しい話者", add);
  check(await calls("save_transcript_draft") === 1, "speaker write repeated");
  check(await page.evaluate(() => window.monBrowserFixture.draft.payload.manual_speakers.length === 1), "speaker not saved");
  results.push("speaker single explicit save, busy dismissal guard and focus after acknowledgement");

  await boot(); const split = editor().getByRole("button", { name: "発言を手動分割", exact: true }).first();
  await split.click(); await page.keyboard.press("Escape"); await cancelled("発言の手動分割", split);
  check(await calls("save_transcript_draft") === 0, "split cancellation changed draft");
  results.push("manual split Escape preserves original draft and returns focus");

  await split.click(); await modal("発言の手動分割").getByRole("textbox", { name: "分割位置（発言先頭からの秒）" }).fill("7");
  await modal("発言の手動分割").getByRole("spinbutton", { name: "本文の切れ目（文字数）" }).fill("5");
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 800; });
  await modal("発言の手動分割").getByRole("button", { name: "分割して保存", exact: true }).click();
  await page.keyboard.press("Escape"); check(await modal("発言の手動分割").isVisible(), "split closed during save");
  await modal("発言の手動分割").waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "文字起こし");
  check(await calls("save_transcript_draft") === 1, "split write repeated");
  check(await page.evaluate(() => {
    const split = window.monBrowserFixture.draft.payload.utterance_splits[0];
    return split.parts.length === 2 && split.parts[0].end_us === 7000000 && split.parts[1].start_us === 7000000 && split.parts.map(p => p.effective_text).join("") === "次の点検日は来週です。";
  }), "split time or original text lost");
  results.push("manual split single save retains exact text and time; removed opener falls back to panel heading");

  await boot(); await split.click(); await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
  await modal("発言の手動分割").getByRole("button", { name: "分割して保存", exact: true }).click();
  await modal("発言の手動分割").getByRole("button", { name: "分割の保存を再試行", exact: true }).waitFor();
  check(await modal("発言の手動分割").isVisible() && await page.evaluate(() => !window.monBrowserFixture.draft.payload.utterance_splits), "failed save reported commit");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; });
  await modal("発言の手動分割").getByRole("button", { name: "分割の保存を再試行", exact: true }).click();
  await modal("発言の手動分割").waitFor({ state: "hidden" });
  check(await calls("save_transcript_draft") === 2 && await page.evaluate(() => window.monBrowserFixture.draft.payload.utterance_splits.length === 1), "explicit split retry lost queued change");
  results.push("split save failure retains queued change; explicit retry commits same draft");

  await boot(); const review = page.getByRole("region", { name: "解析操作", exact: true }).getByRole("button", { name: "内容を確認", exact: true });
  await review.click(); await modal("候補版の採用確認 #2").waitFor();
  await page.keyboard.press("Escape"); await cancelled("候補版の採用確認 #2", review);
  check(await calls("adopt_analysis_candidate") === 0, "Escape adopted analysis");
  await review.click(); box = await modal("候補版の採用確認 #2").boundingBox();
  await page.mouse.click(box.x - 10, box.y + 10); await cancelled("候補版の採用確認 #2", review);
  results.push("analysis preview Escape and backdrop are read-only and restore opener");
  await review.click();
  await page.evaluate(() => { window.monBrowserFixture.confirmations = []; window.confirm = message => { window.monBrowserFixture.confirmations.push(message); return true; }; });
  await modal("候補版の採用確認 #2").getByRole("button", { name: "この候補を採用", exact: true }).click();
  await modal("候補版の採用確認 #2").waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "ローカル文字起こし");
  check(await calls("adopt_analysis_candidate") === 1 && await page.evaluate(() => window.monBrowserFixture.currentId === "candidate" && window.monBrowserFixture.previousId === "analysis" && !!window.monBrowserFixture.archivedDrafts.analysis), "adoption changed version or draft identity");
  check(await page.evaluate(() => window.monBrowserFixture.confirmations.length === 1 && window.monBrowserFixture.confirmations[0].includes("候補 #2 を採用")), "legacy explicit adoption confirmation missing");
  results.push("explicit analysis adoption is single and retains prior draft; removed opener focus fallback");
  return { passed: results.length, results };
}
