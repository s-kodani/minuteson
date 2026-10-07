// Production TranscriptEditor and exact draft identities with synthetic IPC.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const editor = () => page.getByRole("region", { name: "文字起こし編集", exact: true });
  const modal = title => page.getByRole("dialog", { name: title, exact: true });
  const calls = command => page.evaluate(c => window.monBrowserFixture.calls.filter(x => x.command === c), command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openTranscriptTools(); await editor().getByRole("button", { name: "編集を開始", exact: true }).click(); await page.waitForFunction(() => !!window.monBrowserFixture.draft); await editor().getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click(); };
  await boot();
  const text = editor().getByRole("textbox", { name: "00:00:00の発言", exact: true });
  await text.fill("未保存の点検日を保持"); await editor().getByRole("button", { name: "再読込", exact: true }).click();
  const discard = "文字起こし編集の破棄を確認"; await modal(discard).waitFor(); await page.waitForTimeout(900);
  check((await calls("save_transcript_draft")).length === 0, "autosave ran during discard choice");
  await page.keyboard.press("Escape"); await modal(discard).waitFor({ state: "hidden" });
  check(await text.inputValue() === "未保存の点検日を保持", "cancel discarded local transcript");
  await editor().getByRole("button", { name: "再読込", exact: true }).click(); await modal(discard).getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click(); await modal(discard).waitFor({ state: "hidden" });
  check(await editor().getByLabel("00:00:00の発言本文", { exact: true }).textContent() === "次の点検日は来週です。" && await text.count() === 0 && (await calls("save_transcript_draft")).length === 0, "discard saved unwanted text");
  results.push("transcript discard suspends autosave; cancel preserves input, explicit reload discards only unsaved text");

  await boot();
  await editor().getByRole("combobox", { name: "統合元", exact: true }).selectOption("speaker-a");
  await editor().getByRole("combobox", { name: "統合先", exact: true }).selectOption("speaker-b");
  const mergeButton = editor().getByRole("button", { name: "話者を統合", exact: true }); await mergeButton.click();
  const mergeTitle = "話者の統合を確認"; await modal(mergeTitle).waitFor();
  check((await modal(mergeTitle).innerText()).includes("2発言") && (await calls("save_transcript_draft")).length === 0, "merge loss scope or premature save");
  await page.keyboard.press("Escape"); await modal(mergeTitle).waitFor({ state: "hidden" });
  check(await editor().getByRole("combobox", { name: "統合元", exact: true }).inputValue() === "speaker-a", "cancel cleared chosen source");
  await page.evaluate(() => window.monBrowserFixture.persistenceDelayMs = 500);
  await mergeButton.click(); await modal(mergeTitle).getByRole("button", { name: "話者を統合する", exact: true }).dblclick();
  await page.keyboard.press("Escape"); check(await modal(mergeTitle).isVisible(), "merge closed before save acknowledgement");
  await modal(mergeTitle).waitFor({ state: "hidden" });
  check((await calls("save_transcript_draft")).length === 1 && await page.evaluate(() => window.monBrowserFixture.draft.payload.speaker_merges[0].source_id === "speaker-a"), "merge duplicate or wrong fixed source");
  check(await text.inputValue() === "次の点検日は来週です。", "merge changed utterance text");
  await editor().getByRole("button", { name: "直前の編集を取り消す", exact: true }).click();
  await page.waitForFunction(() => !window.monBrowserFixture.draft.payload.speaker_merges?.length);
  results.push("one explicit durable merge preserves text and exact labels; native undo restores the prior draft");

  await boot(); await editor().getByRole("checkbox", { name: "00:00:20の発言を選択", exact: true }).check();
  await editor().getByRole("combobox", { name: "割り当て先", exact: true }).selectOption("speaker-b");
  const apply = editor().getByRole("button", { name: "選択した発言に適用", exact: true }); await apply.click();
  const bulk = "複数発言の話者変更を確認"; await modal(bulk).waitFor(); await page.keyboard.press("Escape"); await modal(bulk).waitFor({ state: "hidden" });
  check((await calls("save_transcript_draft")).length === 0 && await editor().getByRole("checkbox", { name: "00:00:20の発言を選択", exact: true }).isChecked(), "cancel changed bulk selection or saved");
  await page.evaluate(() => window.monBrowserFixture.failSave = true);
  await apply.click(); await modal(bulk).getByRole("button", { name: "選択した発言に適用する", exact: true }).click(); await modal(bulk).getByRole("alert").waitFor();
  check((await calls("save_transcript_draft")).length === 1 && await page.evaluate(() => window.monBrowserFixture.draft.payload.utterances.length === 0), "failed bulk claimed save or repeated command");
  await modal(bulk).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await page.evaluate(() => window.monBrowserFixture.failSave = false);
  await editor().getByRole("button", { name: "保存を再試行", exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.utterances.some(u => u.utterance_id === "speech-b"));
  check((await calls("save_transcript_draft")).length === 2 && await page.evaluate(() => { const u = window.monBrowserFixture.draft.payload.utterances[0]; return u.utterance_id === "speech-b" && u.speaker_ids.length === 1 && u.speaker_ids[0] === "speaker-b" && u.effective_text === "はい、確認します。"; }), "explicit queue retry changed target or text");
  results.push("bulk cancellation preserves selection; failed save retains the exact queued assignment and explicit retry");
  return { passed: results.length, results };
}
